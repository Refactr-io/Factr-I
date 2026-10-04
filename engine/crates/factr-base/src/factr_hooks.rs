//! Factr shell hooks (the `hooks:` block of `config.yaml`) on the engine's lifecycle events.
//!
//! The Factr event names are aliases onto the engine events in `hooks.rs`; a configured hook runs as
//! Factr documents (`agent/shell_hooks.py`): the command is split shell-style and executed directly
//! (never through a shell), it gets a JSON payload on stdin
//! `{hook_event_name, tool_name, tool_input, session_id, cwd, profile, extra}`, its stdout may carry a
//! directive, and it is killed (with its process group) at its `timeout`. Failure never stops a turn:
//! a missing command, a crash, a timeout or bad output only logs, with one exception, the one Factr
//! has: a `pre_tool_call` hook that exits 2 or prints a `block` directive denies that tool call (and a
//! `fail_closed` one denies it when it fails). `hooks_auto_accept` / `FACTR_ACCEPT_HOOKS` / the
//! `shell-hooks-allowlist.json` approval gate which hooks run, exactly as in Factr.
//!
//! Engine events a Factr hook can attach to (the rest of Factr's `VALID_HOOKS` have no engine
//! counterpart and are left to the Python backend, which reads the same file):
//!
//! | Factr event          | Engine event    | When                                          |
//! |-----------------------|-----------------|-----------------------------------------------|
//! | `pre_tool_call`       | `pre_tool`      | before a tool runs; may block or rewrite args |
//! | `post_tool_call`      | `post_tool`     | after a tool ran                              |
//! | `pre_llm_call`        | `turn_start`    | a turn begins (observer; no context return)   |
//! | `post_llm_call`       | `turn_end`      | a turn ended                                  |
//! | `on_session_end`      | `turn_end`      | Factr fires it per turn finalization         |
//! | `on_session_start`    | `session_start` | a session was created, attached or resumed    |
//! | `on_session_finalize` | `session_end`   | the session was closed                        |

use crate::factr_config::{self, HookSpec};
use serde_json::{Map, Value, json};
use std::collections::HashSet;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

const ALIASES: &[(&str, &str)] = &[
    ("pre_tool_call", "pre_tool"),
    ("post_tool_call", "post_tool"),
    ("pre_llm_call", "turn_start"),
    ("post_llm_call", "turn_end"),
    ("on_session_end", "turn_end"),
    ("on_session_start", "session_start"),
    ("on_session_finalize", "session_end"),
];

/// Engine tool -> the names Factr calls it by, so a Factr `matcher: "terminal|patch"` still matches.
const TOOL_ALIASES: &[(&str, &[&str])] = &[
    ("bash", &["terminal"]),
    ("write", &["write_file"]),
    ("edit", &["patch"]),
    ("replace", &["patch"]),
    ("apply_patch", &["patch"]),
    ("patch", &["patch"]),
    ("read", &["read_file"]),
    ("agentgrep", &["search_files"]),
    ("webfetch", &["web_extract"]),
    ("websearch", &["web_search"]),
    ("delegate", &["delegate_task"]),
    ("repl", &["execute_code"]),
    ("skill", &["skill_view", "skills_list"]),
];

const BLOCK_EXIT_CODE: i32 = 2;
const DEFAULT_BLOCK_MESSAGE: &str = "Blocked by shell hook.";
const STDERR_MESSAGE_LIMIT: usize = 400;
const ALLOWLIST: &str = "shell-hooks-allowlist.json";

/// The engine event a Factr event name stands for (`None`: the engine has no such event).
pub fn engine_event(factr_event: &str) -> Option<&'static str> {
    ALIASES.iter().find(|(h, _)| *h == factr_event).map(|(_, e)| *e)
}

/// The Factr event names the engine can run hooks for.
pub fn supported_events() -> impl Iterator<Item = &'static str> {
    ALIASES.iter().map(|(h, _)| *h)
}

fn factr_tool_name(engine_tool: &str) -> &str {
    TOOL_ALIASES.iter().find(|(e, _)| *e == engine_tool).map_or(engine_tool, |(_, names)| names[0])
}

/// Factr `matcher`: a regex that must match the whole tool name (either spelling); an invalid
/// regex is a literal comparison; no matcher matches every tool.
fn matches_tool(spec: &HookSpec, engine_tool: &str) -> bool {
    let Some(matcher) = spec.matcher.as_deref() else { return true };
    let names = std::iter::once(engine_tool).chain(TOOL_ALIASES.iter().filter(|(e, _)| *e == engine_tool).flat_map(|(_, n)| n.iter().copied()));
    match regex::Regex::new(&format!("^(?:{matcher})$")) {
        Ok(re) => names.into_iter().any(|n| re.is_match(n)),
        Err(_) => names.into_iter().any(|n| n == matcher),
    }
}

// ---- consent ---------------------------------------------------------------------------------

fn truthy(v: &str) -> bool {
    matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on")
}

fn allowlist_has(home: &Path, event: &str, command: &str) -> bool {
    std::fs::read_to_string(home.join(ALLOWLIST))
        .ok()
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .and_then(|v| v.get("approvals")?.as_array().cloned())
        .is_some_and(|entries| entries.iter().any(|e| e["event"] == event && e["command"] == command))
}

/// Whether `command` may run for `event`: auto-accepted by config or environment, or approved before.
fn approved(home: Option<&Path>, auto_accept: bool, event: &str, command: &str) -> bool {
    auto_accept
        || std::env::var("FACTR_ACCEPT_HOOKS").is_ok_and(|v| truthy(&v))
        || home.is_some_and(|h| allowlist_has(h, event, command))
}

/// Record an approval in Factr's allowlist (what Factr writes after a first-use "yes"). The owner
/// saving the hook through the authenticated Settings call is that consent.
pub fn record_approval(home: &Path, event: &str, command: &str) -> std::io::Result<()> {
    let file = home.join(ALLOWLIST);
    let mut data: Value = std::fs::read_to_string(&file).ok().and_then(|r| serde_json::from_str(&r).ok()).filter(Value::is_object).unwrap_or_else(|| json!({}));
    let mut approvals: Vec<Value> = data["approvals"].as_array().cloned().unwrap_or_default();
    approvals.retain(|e| !(e["event"] == event && e["command"] == command));
    approvals.push(json!({
        "event": event,
        "command": command,
        "approved_at": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        "script_mtime_at_approval": Value::Null,
    }));
    data["approvals"] = Value::Array(approvals);
    std::fs::create_dir_all(home)?;
    let tmp = file.with_extension(format!("json.{}.tmp", std::process::id()));
    std::fs::write(&tmp, serde_json::to_string_pretty(&data)?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(&tmp, &file)
}

static WARNED: Mutex<Option<HashSet<(String, String)>>> = Mutex::new(None);

fn warn_unapproved_once(event: &str, command: &str) {
    let first = WARNED.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(Default::default).insert((event.into(), command.into()));
    if first {
        crate::logging::warn(&format!(
            "Factr shell hook {event} -> {command} is not approved (hooks_auto_accept, FACTR_ACCEPT_HOOKS or {ALLOWLIST}); skipped"
        ));
    }
}

/// The approved hooks that run on `engine` (in file order); empty inside a hook process (recursion guard).
fn specs_for(engine: &str) -> Vec<HookSpec> {
    if std::env::var_os("FACTR_HOOKS_DISABLED").is_some() {
        return Vec::new();
    }
    let config = factr_config::current();
    if config.hooks.is_empty() {
        return Vec::new();
    }
    let home = factr_config::home();
    config
        .hooks
        .iter()
        .filter(|s| engine_event(&s.event) == Some(engine))
        .filter(|s| {
            let ok = approved(home.as_deref(), config.hooks_auto_accept, &s.event, &s.command);
            if !ok {
                warn_unapproved_once(&s.event, &s.command);
            }
            ok
        })
        .cloned()
        .collect()
}

/// Whether any approved Factr hook runs on the engine event.
pub fn configured(engine: &str) -> bool {
    !specs_for(engine).is_empty()
}

// ---- running one hook ------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct Outcome {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
    pub error: Option<String>,
}

fn expand_tilde(program: &str) -> PathBuf {
    match program.strip_prefix("~/").and_then(|rest| dirs::home_dir().map(|h| h.join(rest))) {
        Some(path) => path,
        None => PathBuf::from(program),
    }
}

#[cfg(unix)]
fn kill_group(child: &mut std::process::Child) {
    // SAFETY: plain signal to the hook's own process group (it leads one, see `run`).
    unsafe { libc::kill(-(child.id() as i32), libc::SIGKILL) };
    let _ = child.kill();
}

#[cfg(not(unix))]
fn kill_group(child: &mut std::process::Child) {
    let _ = child.kill();
}

/// Run `spec.command` with `stdin_json` on stdin, killing it (and its descendants) at the timeout.
pub fn run(spec: &HookSpec, stdin_json: &str, cwd: Option<&str>) -> Outcome {
    let fail = |error: String| Outcome { error: Some(error), ..Default::default() };
    let parts = match crate::terminal_launch::parse_hook_command(&spec.command) {
        Ok(parts) => parts,
        Err(error) => return fail(format!("command {:?} cannot be parsed: {error}", spec.command)),
    };
    let (program, args) = parts.split_first().expect("parse_hook_command guarantees at least one part");
    let mut cmd = Command::new(expand_tilde(program));
    cmd.args(args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    cmd.env("FACTR_HOOKS_DISABLED", "1");
    if let Some(home) = factr_config::home() {
        cmd.env("FACTR_CONFIG_HOME", home);
    }
    if let Some(dir) = cwd.filter(|d| Path::new(d).is_dir()) {
        cmd.current_dir(dir);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return fail("command not found".into()),
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => return fail("command not executable".into()),
        Err(e) => return fail(e.to_string()),
    };
    let started = Instant::now();
    let input = stdin_json.to_owned();
    let mut stdin = child.stdin.take();
    let writer = std::thread::spawn(move || {
        if let Some(stdin) = stdin.as_mut() {
            let _ = stdin.write_all(input.as_bytes());
        }
        drop(stdin); // EOF for hooks that read all of stdin
    });
    let drain = |pipe: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut text = String::new();
            if let Some(mut pipe) = pipe {
                let mut bytes = Vec::new();
                let _ = pipe.read_to_end(&mut bytes);
                text = String::from_utf8_lossy(&bytes).into_owned();
            }
            text
        })
    };
    let out = drain(child.stdout.take().map(|p| Box::new(p) as Box<dyn Read + Send>));
    let err = drain(child.stderr.take().map(|p| Box::new(p) as Box<dyn Read + Send>));
    let timeout = Duration::from_secs(spec.timeout_s.max(1));
    let (code, timed_out) = loop {
        match child.try_wait() {
            Ok(Some(status)) => break (status.code(), false),
            Ok(None) if started.elapsed() >= timeout => {
                kill_group(&mut child);
                let _ = child.wait();
                break (None, true);
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(e) => return fail(e.to_string()),
        }
    };
    let _ = writer.join();
    Outcome { code, stdout: out.join().unwrap_or_default(), stderr: err.join().unwrap_or_default(), timed_out, error: None }
}

// ---- pre_tool_call ----------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum Pre {
    Allow,
    Block(String),
    /// Replace the tool's arguments with this object.
    Modify(Value),
}

fn block_message(primary: &Value, secondary: &Value) -> String {
    [primary, secondary].iter().find_map(|v| v.as_str().filter(|s| !s.is_empty())).unwrap_or(DEFAULT_BLOCK_MESSAGE).to_string()
}

/// Factr `_parse_pre_tool_call`: the `action` dialect, then the Claude-Code `decision` one.
fn parse_pre(stdout: &str) -> Option<Pre> {
    let data: Value = serde_json::from_str(stdout.trim()).ok().filter(Value::is_object)?;
    for (verb, primary, secondary, _) in [("action", "message", "reason", "args"), ("decision", "reason", "message", "tool_input")] {
        if data[verb] == "block" {
            return Some(Pre::Block(block_message(&data[primary], &data[secondary])));
        }
    }
    for (verb, _, _, payload) in [("action", "message", "reason", "args"), ("decision", "reason", "message", "tool_input")] {
        if data[verb] == "modify" && data[payload].is_object() {
            return Some(Pre::Modify(data[payload].clone()));
        }
    }
    // `{"action": "approve"}` escalates to Factr's human-approval gate, which the engine does not
    // have for hooks: it is not a denial, so the call proceeds.
    None
}

fn evaluate_pre(spec: &HookSpec, o: &Outcome) -> Pre {
    let failed_closed = |reason: String| {
        if spec.fail_closed { Pre::Block(format!("hook {} failed closed: {reason}", spec.command)) } else { Pre::Allow }
    };
    if let Some(error) = &o.error {
        crate::logging::warn(&format!("Factr hook {} failed: {error}", spec.command));
        return failed_closed(error.clone());
    }
    if o.timed_out {
        crate::logging::warn(&format!("Factr hook {} timed out after {}s", spec.command, spec.timeout_s));
        return failed_closed(format!("timed out after {}s", spec.timeout_s));
    }
    if o.code == Some(BLOCK_EXIT_CODE) {
        if let Some(block @ Pre::Block(_)) = parse_pre(&o.stdout) {
            return block;
        }
        let stderr: String = o.stderr.trim().chars().take(STDERR_MESSAGE_LIMIT).collect();
        return Pre::Block(if stderr.is_empty() { DEFAULT_BLOCK_MESSAGE.into() } else { stderr });
    }
    if o.code != Some(0) {
        crate::logging::warn(&format!("Factr hook {} exited {:?}", spec.command, o.code));
    }
    let stdout = o.stdout.trim();
    match parse_pre(stdout) {
        Some(directive) => directive,
        None if spec.fail_closed && !stdout.is_empty() && !serde_json::from_str::<Value>(stdout).is_ok_and(|v| v.is_object()) => {
            failed_closed("unparseable stdout (expected a JSON object)".into())
        }
        None => Pre::Allow,
    }
}

fn payload(event: &str, session_id: &str, cwd: Option<&str>, tool: Option<(&str, Value)>, extra: Map<String, Value>) -> String {
    let cwd = cwd.map(str::to_owned).or_else(|| std::env::current_dir().ok().map(|d| d.display().to_string())).unwrap_or_default();
    let (tool_name, tool_input) = match tool {
        Some((name, input)) => (json!(factr_tool_name(name)), input),
        None => (Value::Null, Value::Null),
    };
    json!({
        "hook_event_name": event,
        "tool_name": tool_name,
        "tool_input": tool_input,
        "session_id": session_id,
        "cwd": cwd,
        "profile": "default",
        "extra": extra,
    })
    .to_string()
}

/// Run the `pre_tool_call` hooks for a tool call. A block wins at once; rewrites chain (each hook sees
/// the arguments the one before produced). Anything that goes wrong in a hook allows the call.
pub async fn pre_tool(session_id: &str, cwd: Option<&str>, tool: &str, input: &Value) -> Pre {
    let mut current = input.clone();
    let mut changed = false;
    for spec in specs_for("pre_tool").into_iter().filter(|s| matches_tool(s, tool)) {
        let mut extra = Map::new();
        extra.insert("engine_tool_name".into(), json!(tool));
        extra.insert("platform".into(), json!("factr"));
        let stdin = payload(&spec.event, session_id, cwd, Some((tool, current.clone())), extra);
        let cwd_owned = cwd.map(str::to_owned);
        let run_spec = spec.clone();
        let outcome = tokio::task::spawn_blocking(move || run(&run_spec, &stdin, cwd_owned.as_deref())).await.unwrap_or_default();
        match evaluate_pre(&spec, &outcome) {
            Pre::Block(message) => return Pre::Block(message),
            Pre::Modify(args) => {
                current = args;
                changed = true;
            }
            Pre::Allow => {}
        }
    }
    if changed { Pre::Modify(current) } else { Pre::Allow }
}

// ---- observers --------------------------------------------------------------------------------

fn observer_payload(spec: &HookSpec, event: &crate::hooks::HookEvent) -> String {
    let mut tool = None;
    let mut tool_input = Value::Null;
    let mut extra = Map::new();
    extra.insert("platform".into(), json!("factr"));
    for (key, value) in &event.fields {
        match *key {
            "TOOL_NAME" => tool = Some(value.as_str()),
            "TOOL_INPUT" => tool_input = serde_json::from_str(value).unwrap_or_else(|_| json!(value)),
            "LAST_ASSISTANT_TEXT" => {
                extra.insert("assistant_response".into(), json!(value));
            }
            "DURATION_MS" => {
                extra.insert("duration_ms".into(), value.parse::<u64>().map_or_else(|_| json!(value), |n| json!(n)));
            }
            "STATUS" => {
                extra.insert("completed".into(), json!(value == "ok"));
                extra.insert("failed".into(), json!(value != "ok"));
                extra.insert("interrupted".into(), json!(false));
                extra.insert("status".into(), json!(value));
            }
            other => {
                extra.insert(other.to_ascii_lowercase(), json!(value));
            }
        }
    }
    payload(&spec.event, event.session_id.as_deref().unwrap_or_default(), event.cwd.as_deref(), tool.map(|t| (t, tool_input)), extra)
}

/// Fire the Factr hooks attached to this engine event: each on its own thread, detached, so a slow
/// or broken hook never holds up the agent. Failures are logged.
pub fn dispatch_observer(event: &crate::hooks::HookEvent) {
    let tool = event.fields.iter().find(|(k, _)| *k == "TOOL_NAME").map(|(_, v)| v.as_str());
    for spec in specs_for(event.event) {
        if tool.is_some_and(|t| !matches_tool(&spec, t)) {
            continue;
        }
        let stdin = observer_payload(&spec, event);
        let cwd = event.cwd.clone();
        std::thread::spawn(move || {
            let outcome = run(&spec, &stdin, cwd.as_deref());
            if let Some(error) = &outcome.error {
                crate::logging::warn(&format!("Factr hook {} ({}) failed: {error}", spec.event, spec.command));
            } else if outcome.timed_out {
                crate::logging::warn(&format!("Factr hook {} ({}) timed out after {}s", spec.event, spec.command, spec.timeout_s));
            } else if outcome.code != Some(0) {
                crate::logging::warn(&format!("Factr hook {} ({}) exited {:?}", spec.event, spec.command, outcome.code));
            }
        });
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::hooks::{GateDecision, HookEvent};

    struct Home {
        dir: PathBuf,
        _guard: std::sync::MutexGuard<'static, ()>,
    }

    impl Home {
        /// A temp `FACTR_CONFIG_HOME` whose config.yaml is `yaml`, with `{dir}` replaced by the directory.
        fn new(name: &str, yaml: &str) -> Self {
            let guard = crate::storage::lock_test_env();
            let dir = std::env::temp_dir().join(format!("factr-hooks-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("config.yaml"), yaml.replace("{dir}", &dir.display().to_string())).unwrap();
            crate::env::set_var("FACTR_CONFIG_HOME", &dir);
            Home { dir, _guard: guard }
        }

        fn script(&self, name: &str, body: &str) -> String {
            use std::os::unix::fs::PermissionsExt;
            let path = self.dir.join(name);
            std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            path.display().to_string()
        }
    }

    impl Drop for Home {
        fn drop(&mut self) {
            crate::env::remove_var("FACTR_CONFIG_HOME");
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn wait_for(path: &Path) -> String {
        for _ in 0..300 {
            if let Ok(text) = std::fs::read_to_string(path) {
                if !text.is_empty() {
                    return text;
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("{} never written", path.display());
    }

    #[test]
    fn factr_names_alias_onto_engine_events() {
        assert_eq!(engine_event("pre_tool_call"), Some("pre_tool"));
        assert_eq!(engine_event("on_session_end"), Some("turn_end"));
        assert_eq!(engine_event("on_session_start"), Some("session_start"));
        assert_eq!(engine_event("on_session_finalize"), Some("session_end"));
        assert_eq!(engine_event("subagent_stop"), None);
        let spec = |m: &str| HookSpec { event: "pre_tool_call".into(), command: "x".into(), matcher: Some(m.into()), timeout_s: 5, fail_closed: false };
        assert!(matches_tool(&spec("terminal|patch"), "bash"), "Factr name for the engine's bash");
        assert!(matches_tool(&spec("bash"), "bash"));
        assert!(!matches_tool(&spec("terminal"), "read"));
        assert!(!matches_tool(&spec("term"), "bash"), "whole-name match, as in Factr");
    }

    #[test]
    fn an_observer_gets_the_factr_payload_on_stdin_and_the_recursion_guard_in_its_env() {
        let home = Home::new("observer", "hooks_auto_accept: true\nhooks:\n  on_session_start:\n    - command: '{dir}/start.sh'\n");
        let out = home.dir.join("seen.json");
        home.script("start.sh", &format!("cat > {o}.tmp; echo >> {o}.tmp; echo \"$FACTR_HOOKS_DISABLED|$FACTR_CONFIG_HOME\" >> {o}.tmp; mv {o}.tmp {o}", o = out.display()));
        crate::hooks::dispatch_observer(HookEvent::new("session_start").session_id("s-1").cwd(home.dir.display().to_string()).field("MODEL", "m-1"));
        let seen = wait_for(&out);
        let (json_part, env_part) = seen.trim_end().split_once('\n').unwrap();
        let payload: Value = serde_json::from_str(json_part).unwrap();
        assert_eq!(payload["hook_event_name"], "on_session_start");
        assert_eq!(payload["session_id"], "s-1");
        assert_eq!(payload["cwd"], home.dir.display().to_string());
        assert_eq!(payload["profile"], "default");
        assert_eq!(payload["tool_name"], Value::Null);
        assert_eq!(payload["extra"]["model"], "m-1");
        assert_eq!(env_part, format!("1|{}", home.dir.display()));
    }

    #[test]
    fn two_factr_events_share_the_engine_turn_end() {
        let home = Home::new("turnend", "hooks_auto_accept: true\nhooks:\n  post_llm_call: [{command: '{dir}/a.sh'}]\n  on_session_end: [{command: '{dir}/b.sh'}]\n");
        let (a, b) = (home.dir.join("a.out"), home.dir.join("b.out"));
        home.script("a.sh", &format!("cat > {}", a.display()));
        home.script("b.sh", &format!("cat > {}", b.display()));
        crate::hooks::dispatch_observer(HookEvent::new("turn_end").session_id("s").field("STATUS", "ok").field("DURATION_MS", "1200").field("LAST_ASSISTANT_TEXT", "done"));
        let (a, b): (Value, Value) = (serde_json::from_str(&wait_for(&a)).unwrap(), serde_json::from_str(&wait_for(&b)).unwrap());
        assert_eq!((a["hook_event_name"].as_str(), b["hook_event_name"].as_str()), (Some("post_llm_call"), Some("on_session_end")));
        assert_eq!((&a["extra"]["completed"], &a["extra"]["duration_ms"], &a["extra"]["assistant_response"]), (&json!(true), &json!(1200), &json!("done")));
    }

    #[tokio::test]
    async fn exit_code_2_or_a_block_directive_denies_a_matching_tool_and_nothing_else_does() {
        let home = Home::new("gate", "hooks_auto_accept: true\nhooks:\n  pre_tool_call:\n    - {matcher: terminal, command: '{dir}/deny.sh'}\n");
        home.script("deny.sh", "cat >/dev/null; echo 'no rm -rf' >&2; exit 2");
        let input = json!({ "command": "rm -rf /" });
        assert_eq!(pre_tool("s", None, "bash", &input).await, Pre::Block("no rm -rf".into()));
        assert_eq!(pre_tool("s", None, "read", &input).await, Pre::Allow, "the matcher does not match read");
        // Through the engine's gate, which is what the tool runner calls.
        let gate = crate::hooks::run_pre_tool_gate("s", None, "bash", &input.to_string()).await;
        assert_eq!(gate, GateDecision::Block { reason: "no rm -rf".into() }, "a Factr deny wins before the approval gate is asked");

        home.script("deny.sh", "cat >/dev/null; echo '{\"decision\":\"block\",\"reason\":\"policy\"}'");
        assert_eq!(pre_tool("s", None, "bash", &input).await, Pre::Block("policy".into()), "a JSON block needs no exit code");
    }

    #[tokio::test]
    async fn a_pre_tool_hook_can_rewrite_the_arguments_and_the_gate_reports_it() {
        let home = Home::new("modify", "hooks_auto_accept: true\nhooks:\n  pre_tool_call:\n    - command: '{dir}/fix.sh'\n");
        home.script("fix.sh", "cat >/dev/null; echo '{\"action\":\"modify\",\"args\":{\"command\":\"echo safe\"}}'");
        let input = json!({ "command": "echo unsafe" });
        assert_eq!(pre_tool("s", None, "bash", &input).await, Pre::Modify(json!({ "command": "echo safe" })));
        struct Allow;
        #[async_trait::async_trait]
        impl crate::hooks::ApprovalHook for Allow {
            async fn approve(&self, _: &crate::hooks::ApprovalCall<'_>) -> Result<(), String> {
                Ok(())
            }
        }
        crate::hooks::set_approval_hook(Some(std::sync::Arc::new(Allow)));
        let gate = crate::hooks::run_pre_tool_gate("s", None, "bash", &input.to_string()).await;
        crate::hooks::set_approval_hook(None);
        assert_eq!(gate, GateDecision::Modify { input_json: json!({ "command": "echo safe" }).to_string() });
    }

    #[tokio::test]
    async fn the_approval_gate_judges_the_rewritten_arguments_not_the_original() {
        struct Refuse;
        #[async_trait::async_trait]
        impl crate::hooks::ApprovalHook for Refuse {
            async fn approve(&self, call: &crate::hooks::ApprovalCall<'_>) -> Result<(), String> {
                if call.input.to_string().contains("rm -rf") { Err("refused".into()) } else { Ok(()) }
            }
        }
        let home = Home::new("rewrite-gate", "hooks_auto_accept: true\nhooks:\n  pre_tool_call:\n    - command: '{dir}/fix.sh'\n");
        home.script("fix.sh", "cat >/dev/null; echo '{\"action\":\"modify\",\"args\":{\"command\":\"rm -rf build\"}}'");
        crate::hooks::set_approval_hook(Some(std::sync::Arc::new(Refuse)));
        let gate = crate::hooks::run_pre_tool_gate("s", None, "bash", &json!({ "command": "echo ok" }).to_string()).await;
        crate::hooks::set_approval_hook(None);
        assert_eq!(gate, GateDecision::Block { reason: "refused".into() });
    }

    #[tokio::test]
    async fn hook_failures_never_block_except_for_a_fail_closed_gate() {
        let home = Home::new(
            "fail",
            "hooks_auto_accept: true\nhooks:\n  pre_tool_call:\n    - {matcher: 'open', command: '{dir}/missing.sh'}\n    - {matcher: 'slow', command: '{dir}/slow.sh', timeout: 1}\n    - {matcher: 'strict', command: '{dir}/missing.sh', fail_closed: true}\n    - {matcher: 'garbage', command: '{dir}/garbage.sh', fail_closed: true}\n    - {matcher: 'crash', command: '{dir}/crash.sh'}\n",
        );
        home.script("slow.sh", "sleep 30");
        home.script("garbage.sh", "echo 'Traceback (most recent call last)'");
        home.script("crash.sh", "exit 1");
        let input = json!({});
        assert_eq!(pre_tool("s", None, "open", &input).await, Pre::Allow, "missing command fails open");
        assert_eq!(pre_tool("s", None, "crash", &input).await, Pre::Allow, "a crash fails open");
        let started = Instant::now();
        assert_eq!(pre_tool("s", None, "slow", &input).await, Pre::Allow, "a timeout fails open");
        assert!(started.elapsed() < Duration::from_secs(10), "the hook was killed at its timeout");
        assert!(matches!(pre_tool("s", None, "strict", &input).await, Pre::Block(m) if m.contains("failed closed")));
        assert!(matches!(pre_tool("s", None, "garbage", &input).await, Pre::Block(m) if m.contains("unparseable")));
    }

    #[test]
    fn an_unapproved_hook_does_not_run_until_it_is_approved() {
        let home = Home::new("consent", "hooks:\n  on_session_start:\n    - command: '{dir}/start.sh'\n");
        assert!(!configured("session_start"), "no auto-accept, no allowlist entry");
        record_approval(&home.dir, "on_session_start", &format!("{}/start.sh", home.dir.display())).unwrap();
        assert!(configured("session_start"), "approved through the allowlist");
        // Factr's own file shape.
        let list: Value = serde_json::from_str(&std::fs::read_to_string(home.dir.join(ALLOWLIST)).unwrap()).unwrap();
        assert_eq!(list["approvals"][0]["event"], "on_session_start");
    }

    #[test]
    fn nothing_runs_inside_a_hook_process() {
        let home = Home::new("guard", "hooks_auto_accept: true\nhooks:\n  on_session_start:\n    - command: '{dir}/start.sh'\n");
        assert!(configured("session_start"));
        crate::env::set_var("FACTR_HOOKS_DISABLED", "1");
        let off = configured("session_start");
        crate::env::remove_var("FACTR_HOOKS_DISABLED");
        assert!(!off);
        drop(home);
    }
}
