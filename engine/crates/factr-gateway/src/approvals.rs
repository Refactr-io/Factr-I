//! Human approval for risky tool calls (Factr parity), the engine's one approval gate.
//!
//! The registry asks [`HubGate`] (installed once at startup through `factr_base::hooks`) before every
//! `bash`, `repl` and `factr` call, in process. [`gate`] judges the call: shell commands by the bash
//! tool's own verdict (`factr_app_core::tool::bash_verdict`, the single deterministic gate) plus the
//! Factr dangerous-pattern list; REPL cells where the platform has no sandbox; Factr bridge calls
//! that act on the outside world. Anything that is not plainly fine goes to [`Hub::decide_in`], which
//! shows a Factr `approval` request on the desktop windows that have the session open and waits.
//!
//! Fail closed everywhere: no hook installed, no desktop client, a timeout or a malformed call all
//! mean "deny". Answers come only from an authenticated desktop socket.
//!
//! Unattended runs (cron, bot turns, goals with no window) cannot wait for an
//! answer: they follow the Factr approval config (`approvals.mode: off`,
//! `cron_mode` / `unattended_mode: approve`) and otherwise deny, parking the
//! blocked command as a normal `approval` prompt on the desktop so the user can
//! approve it later (see docs/SAFETY_SYSTEM.md).

use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::sync::{Mutex, mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;

/// How long a prompt waits for the user.
pub const DECISION_TIMEOUT: Duration = Duration::from_secs(240);
/// How long a denied unattended command stays open for a late approval.
const PARK_TTL: Duration = Duration::from_secs(24 * 3600);
const MAX_PARKED: usize = 50;
/// Internal answer: the window showing a prompt closed, so it is parked like an unattended one.
const PARK: &str = "\0park";

/// Whether the user's Factr config (`$FACTR_CONFIG_HOME/config.yaml`) lets an unattended `surface`
/// ("cron", "bot", "goal") run approval-gated commands: `approvals.mode: off`, or `cron_mode`
/// (cron) / `unattended_mode` (everything else) set to `approve`. Default and any read error: no.
fn policy_allows(home: &Path, surface: &str) -> bool {
    let approvals = std::fs::read_to_string(home.join("config.yaml"))
        .ok()
        .and_then(|raw| serde_yaml::from_str::<serde_yaml::Value>(&raw).ok())
        .map(|cfg| cfg["approvals"].clone())
        .unwrap_or_default();
    let key = if surface == "cron" { "cron_mode" } else { "unattended_mode" };
    approvals["mode"].as_str() == Some("off") || approvals[key].as_str() == Some("approve")
}

/// Factr's `command_allowlist` matching (`approval_floors._command_matches_permanent_allowlist`):
/// the exact command text, or a `*` / `?` glob, never for a compound command. Stricter than Factr
/// where it is cheaper to be: any shell metacharacter disqualifies, and `[...]` classes only match
/// literally. Factr also keeps dangerous-pattern keys ("recursive delete") in this list; they are
/// not command text, so they never match here.
fn has_shell_operator(command: &str) -> bool {
    command.chars().any(|c| matches!(c, ';' | '&' | '|' | '<' | '>' | '`' | '$' | '(' | ')' | '{' | '}' | '\\' | '\n' | '\r'))
}

fn glob_match(pattern: &[u8], text: &[u8]) -> bool {
    match pattern.split_first() {
        None => text.is_empty(),
        Some((b'*', rest)) => (0..=text.len()).any(|i| glob_match(rest, &text[i..])),
        Some((b'?', rest)) => !text.is_empty() && glob_match(rest, &text[1..]),
        Some((c, rest)) => text.first() == Some(c) && glob_match(rest, &text[1..]),
    }
}

fn config_allowlist(home: &Path) -> Vec<String> {
    let cfg = std::fs::read_to_string(home.join("config.yaml")).ok().and_then(|raw| serde_yaml::from_str::<serde_yaml::Value>(&raw).ok());
    cfg.and_then(|c| c["command_allowlist"].as_sequence().cloned())
        .map(|list| list.iter().filter_map(|v| v.as_str().map(|s| s.trim().to_string())).collect())
        .unwrap_or_default()
}

fn allowlisted(home: &Path, command: &str) -> bool {
    let command = command.trim();
    !command.is_empty()
        && !has_shell_operator(command)
        && config_allowlist(home).iter().any(|p| {
            !p.is_empty() && (p == command || (!p.contains('[') && p.contains(['*', '?']) && glob_match(p.as_bytes(), command.as_bytes())))
        })
}

/// `text` with `command` added to `command_allowlist` (the format Factr itself writes for an `always`
/// answer: a block list of command text). A plain text edit so the rest of the file is untouched.
/// `Ok(None)`: already there. `Err`: a shape it does not recognise, left alone.
fn add_to_allowlist(text: &str, command: &str) -> Result<Option<String>, ()> {
    let valid = |t: &str| serde_yaml::from_str::<serde_yaml::Value>(t).ok().filter(|v| v.is_mapping() || v.is_null());
    let cfg = valid(text).ok_or(())?;
    if cfg["command_allowlist"].as_sequence().is_some_and(|l| l.iter().any(|v| v.as_str().map(str::trim) == Some(command))) {
        return Ok(None);
    }
    if !cfg["command_allowlist"].is_null() && !cfg["command_allowlist"].is_sequence() {
        return Err(());
    }
    let item = |indent: &str| format!("{indent}- {}", serde_json::to_string(command).unwrap_or_default());
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    match lines.iter().position(|l| l.starts_with("command_allowlist:")) {
        None => {
            lines.push("command_allowlist:".into());
            lines.push(item(""));
        }
        Some(at) => {
            let rest = lines[at]["command_allowlist:".len()..].split('#').next().unwrap_or("").trim().to_string();
            if rest == "[]" {
                lines[at] = "command_allowlist:".into();
                lines.insert(at + 1, item(""));
            } else if rest.is_empty() {
                let is_item = |l: &String| l.trim_start().starts_with("- ") || l.trim() == "-";
                let last = (at + 1..lines.len()).take_while(|&i| is_item(&lines[i])).last().unwrap_or(at);
                let indent: String = lines.get(at + 1).filter(|l| is_item(l)).map(|l| l.chars().take_while(|c| *c == ' ').collect()).unwrap_or_default();
                lines.insert(last + 1, item(&indent));
            } else {
                return Err(());
            }
        }
    }
    let edited = lines.join("\n") + "\n";
    if !valid(&edited).is_some_and(|v| v["command_allowlist"].as_sequence().is_some_and(|l| l.iter().any(|x| x.as_str() == Some(command)))) {
        return Err(());
    }
    Ok(Some(edited))
}

/// Add `command` to Factr's `command_allowlist` in `$home/config.yaml`. Factr has no cross-process
/// lock on this file (its `_CONFIG_LOCK` is in-process, its writer an atomic rename), so this is
/// edit, write a temp file, re-read and rename only if the file is still what the edit was made
/// from, else redo the edit on the new content: a concurrent Python save is merged, not overwritten.
fn allow_permanently(home: &Path, command: &str) -> bool {
    allow_permanently_with(home, command, || {})
}

/// `between` runs after the edit is staged and before the re-read (a test seam for the race).
fn allow_permanently_with(home: &Path, command: &str, mut between: impl FnMut()) -> bool {
    let command = command.trim();
    if command.is_empty() || command.contains('\n') || has_shell_operator(command) {
        return false;
    }
    let path = home.join("config.yaml");
    let tmp = path.with_extension(format!("yaml.{}.tmp", std::process::id()));
    for _ in 0..5 {
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let edited = match add_to_allowlist(&text, command) {
            Ok(Some(edited)) => edited,
            Ok(None) => return true,
            Err(()) => return false,
        };
        let staged = std::fs::write(&tmp, edited).is_ok()
            && std::fs::metadata(&path).map_or(true, |m| std::fs::set_permissions(&tmp, m.permissions()).is_ok());
        if !staged {
            break;
        }
        between();
        if std::fs::read_to_string(&path).unwrap_or_default() == text {
            if std::fs::rename(&tmp, &path).is_ok() {
                return true;
            }
            break;
        }
    }
    let _ = std::fs::remove_file(&tmp);
    false
}

/// What kind of risk a command carries, by factr's classifier: the distinct reasons it flags
/// (they name the program, never a path), or "general". Factr keys a session grant on its
/// pattern category the same way, so allowing `rm -rf build` for the session does not also allow
/// a command that pipes paths into a delete.
fn risk_category(command: &str) -> String {
    let assessment = factr_command_risk::assess(command, &factr_command_risk::RiskContext::from_env(None));
    let mut reasons: Vec<&str> = assessment.findings.iter().map(|f| f.reason.as_str()).collect();
    reasons.sort_unstable();
    reasons.dedup();
    if reasons.is_empty() { "general".into() } else { reasons.join(" | ") }
}

/// A desktop connection able to show prompts.
pub struct Client {
    pub id: u64,
    pub to_ws: mpsc::Sender<Message>,
    /// Sessions this client has open (created or attached).
    pub sessions: Mutex<HashSet<String>>,
}

#[derive(Default)]
pub struct Hub {
    clients: Mutex<Vec<Arc<Client>>>,
    pending: Mutex<HashMap<String, (String, oneshot::Sender<String>)>>,
    /// Params of open prompts, for `approval.pending`.
    shown: Mutex<HashMap<String, (String, Value)>>,
    /// (session, risk category) pairs where the user chose "session": allowed for the rest of the
    /// session, but only for commands of the same kind (see [`risk_category`]).
    session_grants: Mutex<HashSet<(String, String)>>,
    next: AtomicU64,
    /// Sessions running unattended (`/api/agent/run`) -> their surface ("cron" | "bot"). No desktop
    /// prompt ever waits on them; see [`Hub::unattended`].
    headless: Mutex<HashMap<String, &'static str>>,
    /// Exact commands the user approved after the fact: consumed by the next unattended run needing
    /// it (`once`), or good until the engine restarts (`session`; `always` only when Factr's config
    /// can't take it, see [`allow_permanently`]).
    // (session, command); an empty session is the in-memory stand-in for an "always" Factr can't store.
    once_grants: Mutex<HashSet<(String, String)>>,
    sticky_grants: Mutex<HashSet<(String, String)>>,
    /// Open `clarify` questions (`clarify-N`) -> (session, answer channel; `None` = the window cancelled).
    clarifying: Mutex<HashMap<String, (String, oneshot::Sender<Option<String>>)>>,
    observer: std::sync::Mutex<Option<std::sync::Arc<crate::observability::Observer>>>,
    /// Where parked unattended prompts persist (factr.db), so a restart keeps them.
    store: std::sync::Mutex<Option<Arc<factr_learn::entries::EntryStore>>>,
}

/// The lowest-numbered (oldest) parked unattended prompt.
fn oldest_parked(shown: &HashMap<String, (String, Value)>) -> Option<(String, String)> {
    let number = |id: &String| id.strip_prefix("approval-").and_then(|n| n.parse::<u64>().ok()).unwrap_or(u64::MAX);
    shown.iter().filter(|(_, (_, p))| p["unattended"] == true).min_by_key(|(id, _)| number(id)).map(|(id, (session, _))| (id.clone(), session.clone()))
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

impl Hub {
    pub fn set_observer(&self, observer: std::sync::Arc<crate::observability::Observer>) {
        *self.observer.lock().unwrap_or_else(|e| e.into_inner()) = Some(observer);
    }

    fn store(&self) -> Option<Arc<factr_learn::entries::EntryStore>> {
        self.store.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Persist parked prompts in `store` and re-arm the ones still inside their 24 h window.
    pub async fn set_store(self: &Arc<Self>, store: Arc<factr_learn::entries::EntryStore>) {
        *self.store.lock().unwrap_or_else(|e| e.into_inner()) = Some(store.clone());
        let ttl_ms = PARK_TTL.as_millis() as i64;
        for (request_id, session, params, created) in store.park_load(now_ms() - ttl_ms) {
            let Ok(params) = serde_json::from_str::<Value>(&params) else { continue };
            // Ids restart at 0 with the process: keep new ones clear of the restored ones.
            if let Some(n) = request_id.strip_prefix("approval-").and_then(|n| n.parse::<u64>().ok()) {
                self.next.fetch_max(n + 1, Ordering::Relaxed);
            }
            let left = Duration::from_millis((created + ttl_ms - now_ms()).max(0) as u64);
            let (tool, command) = (params["tool_name"].as_str().unwrap_or("bash").to_string(), params["command"].as_str().unwrap_or_default().to_string());
            self.shown.lock().await.insert(request_id.clone(), (session.clone(), params));
            self.arm(request_id, session, tool, command, left).await;
        }
    }

    fn audit(&self, session_id: &str, tool: &str, command: &str, decision: &str, actor: &str) {
        if let Some(observer) = self.observer.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            observer.record_approval(session_id, tool, command, decision, actor);
        }
    }
    pub async fn broadcast_text(&self, frame: String) {
        for client in self.clients.lock().await.iter() {
            let _ = client.to_ws.send(Message::Text(frame.clone())).await;
        }
    }

    pub async fn add(&self, client: Arc<Client>) {
        // A desktop that connects later still sees what unattended runs were denied meanwhile.
        for (id, (_, params)) in self.shown.lock().await.iter() {
            if params["unattended"] == true {
                let frame = json!({ "jsonrpc": "2.0", "id": id, "method": "approval", "params": params });
                let _ = client.to_ws.send(Message::Text(frame.to_string())).await;
            }
        }
        self.clients.lock().await.push(client);
    }

    /// Whether a desktop window has `session` open (the driver's engine link
    /// yields to it: the window renders, observes and prompts).
    pub async fn has_window(&self, session_id: &str) -> bool {
        for client in self.clients.lock().await.iter() {
            if client.sessions.lock().await.contains(session_id) {
                return true;
            }
        }
        false
    }

    pub async fn remove(&self, id: u64) {
        self.clients.lock().await.retain(|c| c.id != id);
        // A prompt only the closed window showed would time out to a plain deny: hand its waiter to
        // the unattended path instead, so the command is parked and can be approved later.
        let open: Vec<(String, String)> = {
            let shown = self.shown.lock().await;
            self.pending.lock().await.iter().filter(|(rid, _)| shown.get(*rid).is_some_and(|(_, p)| p["unattended"] != true)).map(|(rid, (sid, _))| (rid.clone(), sid.clone())).collect()
        };
        // A clarify only the closed window could answer is cancelled (its asker sees TimedOut).
        let asking: Vec<(String, String)> = self.clarifying.lock().await.iter().map(|(rid, (sid, _))| (rid.clone(), sid.clone())).collect();
        for (rid, sid) in asking {
            if !self.has_window(&sid).await {
                self.clarifying.lock().await.remove(&rid);
            }
        }
        for (rid, sid) in open {
            if !self.has_window(&sid).await {
                if let Some((_, tx)) = self.pending.lock().await.remove(&rid) {
                    let _ = tx.send(PARK.into());
                }
            }
        }
    }

    pub fn next_client_id(&self) -> u64 {
        self.next.fetch_add(1, Ordering::Relaxed)
    }

    pub async fn mark_headless(&self, session_id: &str, surface: &'static str) {
        factr_base::headless::mark(session_id);
        self.headless.lock().await.insert(session_id.to_string(), surface);
    }

    pub async fn unmark_headless(&self, session_id: &str) {
        factr_base::headless::unmark(session_id);
        self.headless.lock().await.remove(session_id);
    }

    pub async fn is_headless(&self, session_id: &str) -> bool {
        self.headless.lock().await.contains_key(session_id)
    }

    async fn sticky(&self, session_id: &str, command: &str) -> bool {
        let grants = self.sticky_grants.lock().await;
        grants.contains(&(session_id.to_string(), command.to_string())) || grants.contains(&(String::new(), command.to_string()))
    }

    /// An approval nobody can answer now (a cron / bot turn, or a goal with no desktop): allowed by
    /// the user's Factr config or an earlier late approval (`once`), otherwise denied and parked as
    /// a normal desktop `approval` prompt so the user can approve it later.
    pub(crate) async fn unattended(self: &Arc<Self>, session_id: &str, tool: &str, command: &str, reason: &str, cwd: Option<&std::path::Path>) -> String {
        let surface = self.headless.lock().await.get(session_id).copied().unwrap_or("goal");
        let granted = self.once_grants.lock().await.remove(&(session_id.to_string(), command.to_string())) || self.sticky(session_id, command).await;
        let home = factr_base::factr_config::home();
        let allowlisted = home.as_deref().is_some_and(|home| allowlisted(home, command));
        if granted || allowlisted || home.is_some_and(|home| policy_allows(&home, surface)) {
            self.audit(session_id, tool, command, "once", if granted { "user-later" } else if allowlisted { "allowlist" } else { "policy" });
            return "once".into();
        }
        // Tidying the session's own working directory needs no one's approval.
        if cwd.is_some_and(|cwd| factr_command_risk::confined_to_workdir(command, &factr_command_risk::RiskContext::from_env(Some(cwd.to_path_buf())))) {
            self.audit(session_id, tool, command, "once", "workdir");
            return "once".into();
        }
        self.audit(session_id, tool, command, "deny", "headless-deny");
        self.park(session_id, tool, command, reason, surface).await;
        "deny".into()
    }

    /// Show a denied unattended command on the desktop; an approval records a late grant and, for a
    /// goal session, resumes it.
    async fn park(self: &Arc<Self>, session_id: &str, tool: &str, command: &str, reason: &str, surface: &str) {
        let request_id = format!("approval-{}", self.next.fetch_add(1, Ordering::Relaxed));
        let params = json!({
            "session_id": session_id,
            "request_id": request_id,
            "command": command.chars().take(4000).collect::<String>(),
            "description": format!("Blocked while unattended ({surface}): {reason}. Approve to let it run next time."),
            "tool_name": tool,
            "choices": ["once", "session", "always", "deny"],
            "allow_permanent": true,
            "allow_session": true,
            "unattended": true,
        });
        let evicted: Option<(String, String)>;
        {
            let mut shown = self.shown.lock().await;
            let parked = shown.values().filter(|(_, p)| p["unattended"] == true);
            if parked.clone().any(|(sid, p)| sid == session_id && p["command"] == command) {
                return;
            }
            // At the cap the oldest parked prompt makes room (request ids only grow), and the user is told.
            evicted = (parked.count() >= MAX_PARKED).then(|| oldest_parked(&shown)).flatten();
            if let Some((old, _)) = &evicted {
                shown.remove(old);
            }
            shown.insert(request_id.clone(), (session_id.to_string(), params.clone()));
        }
        if let Some((old, old_session)) = evicted {
            self.pending.lock().await.remove(&old); // its waiter times out to a plain deny
            if let Some(store) = self.store() {
                store.park_delete(&old);
            }
            let note = json!({ "jsonrpc": "2.0", "method": "event", "params": { "type": "status.update", "session_id": old_session,
                "payload": { "kind": "approval", "text": format!("Too many blocked commands are waiting for approval; the oldest ({old}) was dropped.") } } });
            self.broadcast_text(note.to_string()).await;
        }
        if let Some(store) = self.store() {
            let _ = store.park_save(&request_id, session_id, &params.to_string(), now_ms());
        }
        let frame = json!({ "jsonrpc": "2.0", "id": request_id, "method": "approval", "params": params }).to_string();
        self.arm(request_id, session_id.to_string(), tool.to_string(), command.to_string(), PARK_TTL).await;
        self.broadcast_text(frame).await;
    }

    /// Wait up to `ttl` for the user's answer to a parked prompt, then record the grant.
    async fn arm(self: &Arc<Self>, request_id: String, session: String, tool: String, command: String, ttl: Duration) {
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(request_id.clone(), (session.clone(), tx));
        let hub = self.clone();
        tokio::spawn(async move {
            let choice = tokio::time::timeout(ttl, rx).await.ok().and_then(Result::ok).unwrap_or_default();
            hub.pending.lock().await.remove(&request_id);
            hub.shown.lock().await.remove(&request_id);
            if let Some(store) = hub.store() {
                store.park_delete(&request_id);
            }
            match choice.as_str() {
                "once" => drop(hub.once_grants.lock().await.insert((session.clone(), command.clone()))),
                "session" => drop(hub.sticky_grants.lock().await.insert((session.clone(), command.clone()))),
                // Permanent grants belong to Factr's config; in memory only if it can't take them.
                "always" => {
                    let saved = factr_base::factr_config::home().is_some_and(|home| allow_permanently(&home, &command));
                    if !saved {
                        hub.sticky_grants.lock().await.insert((String::new(), command.clone()));
                    }
                }
                _ => return,
            }
            hub.audit(&session, &tool, &command, &choice, "user-later");
            crate::rpc::resume_after_approval(&session, &command);
        });
    }

    /// The windows that can answer for `session_id`: those showing it, else those showing its
    /// nearest ancestor (delegate children), with the session they show.
    async fn windows_for(&self, session_id: &str) -> (String, Vec<Arc<Client>>) {
        let all = self.clients.lock().await.clone();
        let mut current = session_id.to_string();
        for _ in 0..MAX_ANCESTORS {
            let mut showing = Vec::new();
            for client in &all {
                if client.sessions.lock().await.contains(&current) {
                    showing.push(client.clone());
                }
            }
            if !showing.is_empty() {
                return (current, showing);
            }
            let id = current.clone();
            match tokio::task::spawn_blocking(move || parent_of(&id)).await.ok().flatten() {
                Some(parent) => current = parent,
                None => break,
            }
        }
        (session_id.to_string(), Vec::new())
    }

    /// Ask the user whether `command` may run. Returns the Factr choice
    /// (`once` / `session` / `always` / `deny`).
    pub async fn decide(self: &Arc<Self>, session_id: &str, tool: &str, command: &str, reason: &str) -> String {
        self.decide_in(session_id, tool, command, reason, None).await
    }

    /// [`Hub::decide`] for a command run from `cwd`, so an unattended run may allow what stays inside it.
    pub async fn decide_in(self: &Arc<Self>, session_id: &str, tool: &str, command: &str, reason: &str, cwd: Option<&std::path::Path>) -> String {
        if self.headless.lock().await.contains_key(session_id) {
            return self.unattended(session_id, tool, command, reason, cwd).await;
        }
        // A late `once` approval is for this session's next attempt at exactly this command: spend it
        // here, so a stale grant can't wave through some later unattended run.
        if self.once_grants.lock().await.remove(&(session_id.to_string(), command.to_string())) {
            self.audit(session_id, tool, command, "once", "user-later");
            return "once".into();
        }
        // "always" is per command, as in Factr: its permanent allowlist (or, when the config
        // can't take the grant, a sticky in-memory one) — never a blanket allow-everything.
        let home = factr_base::factr_config::home();
        // `approvals.mode: off` or a YOLO flag (config.set yolo) waves the prompt through; catastrophic
        // commands were already blocked by the gate before it asked.
        // The per-session flag is in the engine store (the one `config.set yolo` wrote); Factr's dir only has the yaml.
        if crate::rpc::yolo_active(home.as_deref(), self.store().as_deref(), session_id) {
            self.audit(session_id, tool, command, "once", "yolo");
            return "once".into();
        }
        if home.as_deref().is_some_and(|home| allowlisted(home, command)) || self.sticky(session_id, command).await {
            self.audit(session_id, tool, command, "always", "allowlist");
            return "always".into();
        }
        // A shell command's grant is keyed on what kind of risk it carries; any other tool's on the tool.
        let category = if tool == "bash" { risk_category(command) } else { format!("tool:{tool}") };
        if self.session_grants.lock().await.contains(&(session_id.to_string(), category.clone())) {
            let choice = "session".to_string();
            self.audit(session_id, tool, command, &choice, "session-grant");
            return choice;
        }
        // Only a window showing this chat can answer for it. A delegate child is never in a window's
        // set, so its prompt goes to the window of the nearest ancestor that has one, and is keyed
        // to that session: that is the id the window answers (`approval.respond`) and reloads with.
        // Prompting unrelated windows would be denied after the timeout and never parked, so no
        // window anywhere up the chain means unattended (parked).
        let (prompt_session, clients) = self.windows_for(session_id).await;
        if clients.is_empty() {
            return self.unattended(session_id, tool, command, reason, cwd).await;
        }
        let request_id = format!("approval-{}", self.next.fetch_add(1, Ordering::Relaxed));
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(request_id.clone(), (prompt_session.clone(), tx));
        let params = json!({
            "session_id": prompt_session,
            "request_id": request_id,
            "command": command.chars().take(4000).collect::<String>(),
            "description": reason,
            "tool_name": tool,
            "choices": ["once", "session", "always", "deny"],
            "allow_permanent": true,
            "allow_session": true,
        });
        self.shown.lock().await.insert(request_id.clone(), (prompt_session.clone(), params.clone()));
        let frame = json!({ "jsonrpc": "2.0", "id": request_id, "method": "approval", "params": params }).to_string();
        for client in &clients {
            let _ = client.to_ws.send(Message::Text(frame.clone())).await;
        }
        let choice = match tokio::time::timeout(DECISION_TIMEOUT, rx).await {
            Ok(Ok(choice)) => choice,
            _ => "deny".into(),
        };
        self.pending.lock().await.remove(&request_id);
        self.shown.lock().await.remove(&request_id);
        if choice == PARK {
            return self.unattended(session_id, tool, command, reason, cwd).await;
        }
        match choice.as_str() {
            "session" => {
                self.session_grants.lock().await.insert((session_id.to_string(), category));
            }
            "always" => {
                let saved = home.as_deref().is_some_and(|home| allow_permanently(home, command));
                if !saved {
                    self.sticky_grants.lock().await.insert((String::new(), command.to_string()));
                }
            }
            _ => {}
        }
        let final_choice = if matches!(choice.as_str(), "once" | "session" | "always") {
            choice
        } else {
            "deny".into()
        };
        self.audit(session_id, tool, command, &final_choice, "user");
        final_choice
    }

    /// Ask the person a question for the `clarify` tool and wait for the answer. A run nobody watches
    /// (cron, bot, goal without a window) gets `NoUser` at once instead of waiting for a reply that
    /// cannot come.
    pub async fn clarify(&self, session_id: &str, question: &str, choices: &[String]) -> factr_app_core::tool::factr_bridge::ClarifyReply {
        use factr_app_core::tool::factr_bridge::ClarifyReply;
        if self.headless.lock().await.contains_key(session_id) {
            return ClarifyReply::NoUser;
        }
        let mut showing = Vec::new();
        for client in self.clients.lock().await.clone() {
            if client.sessions.lock().await.contains(session_id) {
                showing.push(client);
            }
        }
        if showing.is_empty() {
            return ClarifyReply::NoUser;
        }
        let request_id = format!("clarify-{}", self.next.fetch_add(1, Ordering::Relaxed));
        let (tx, rx) = oneshot::channel();
        self.clarifying.lock().await.insert(request_id.clone(), (session_id.to_string(), tx));
        let mut params = json!({ "session_id": session_id, "question": question });
        if !choices.is_empty() {
            params["choices"] = json!(choices);
        }
        let frame = json!({ "jsonrpc": "2.0", "id": request_id, "method": "clarify", "params": params }).to_string();
        for client in &showing {
            let _ = client.to_ws.send(Message::Text(frame.clone())).await;
        }
        let reply = tokio::time::timeout(DECISION_TIMEOUT, rx).await;
        self.clarifying.lock().await.remove(&request_id);
        match reply {
            Ok(Ok(answer)) => ClarifyReply::Answer(answer.unwrap_or_default()),
            _ => ClarifyReply::TimedOut,
        }
    }

    /// A window answered a `clarify` request (`result.answer`; anything else is a skip). Only a
    /// client attached to the request's session may answer it.
    pub async fn answer_clarify(&self, from: &Client, request_id: &str, frame: &Value) -> bool {
        let mut clarifying = self.clarifying.lock().await;
        let Some((session, _)) = clarifying.get(request_id) else { return false };
        if !from.sessions.lock().await.contains(session) {
            return false;
        }
        match clarifying.remove(request_id) {
            Some((_, tx)) => tx.send(frame["result"]["answer"].as_str().map(str::to_string)).is_ok(),
            None => false,
        }
    }

    /// Open prompts for a session (`approval.pending`, e.g. after a reload).
    pub async fn pending_for(&self, session_id: &str) -> Vec<Value> {
        self.shown
            .lock()
            .await
            .iter()
            .filter(|(_, (sid, _))| sid == session_id)
            .map(|(id, (_, params))| {
                // PendingApproval carries no session_id (the caller named it).
                let mut p = params.clone();
                if let Some(map) = p.as_object_mut() {
                    map.remove("session_id");
                }
                p["request_id"] = json!(id);
                p
            })
            .collect()
    }

    /// A desktop answered a prompt by replying to the server request.
    pub async fn answer(&self, request_id: &str, choice: &str) -> bool {
        match self.pending.lock().await.remove(request_id) {
            Some((_, tx)) => tx.send(choice.to_string()).is_ok(),
            None => false,
        }
    }

    /// `approval.respond`: resolve this session's prompts (one, or all).
    pub async fn answer_session(&self, session_id: &str, request_id: Option<&str>, choice: &str) -> usize {
        let mut pending = self.pending.lock().await;
        let keys: Vec<String> = pending
            .iter()
            .filter(|(id, (sid, _))| sid == session_id && request_id.is_none_or(|r| r == id.as_str()))
            .map(|(id, _)| id.clone())
            .collect();
        let mut resolved = 0;
        for key in keys {
            if let Some((_, tx)) = pending.remove(&key) {
                resolved += usize::from(tx.send(choice.to_string()).is_ok());
            }
        }
        resolved
    }
}

/// Factr bridge tools that act on the outside world (messages, devices, the desktop) ask first,
/// like a risky shell command. The bridge itself no longer keeps this list.
pub(crate) fn factr_tool_needs_approval(name: &str) -> bool {
    matches!(name, "send_message" | "computer_use" | "ha_call_service")
        || name.starts_with("discord")
        || name.starts_with("yb_send_")
        || matches!(name, "feishu_drive_reply_comment" | "feishu_drive_add_comment")
}

/// What the approval gate does with a tool call.
pub(crate) mod gate {
    use factr_base::hooks::ApprovalCall;
    use std::path::PathBuf;

    #[derive(Debug, PartialEq, Eq)]
    pub(crate) enum Decision {
        Allow,
        /// Refuse without asking the person (reflection prompt or hard deny).
        Block(String),
        /// The person decides: what to show, and why.
        Ask { summary: String, reason: String },
    }

    /// A shell command: the bash tool's verdict (the one deterministic gate, so the tool and this
    /// gate cannot disagree), with Factr's dangerous-pattern list on top. Safe: allow, unless a
    /// Factr pattern matches, which asks (never downgrades). Low, or a justified Confirm: ask.
    /// Confirm without a justification: reflect-block. Catastrophic: deny.
    pub(super) fn decide(command: &str, justification: Option<&str>, cwd: Option<PathBuf>) -> Decision {
        use factr_app_core::tool::BashVerdict;
        let ask = |reason: String| Decision::Ask { summary: command.to_string(), reason };
        let factr = crate::dangerous::dangerous_reason(command);
        match factr_app_core::tool::bash_verdict(command, justification, cwd) {
            BashVerdict::Refuse(text) => Decision::Block(text),
            BashVerdict::Run => factr.map_or(Decision::Allow, |reason| ask(reason.to_string())),
            BashVerdict::Ask(reason) => ask(factr.map(str::to_string).unwrap_or(reason)),
        }
    }

    /// Judge a registry call to `bash`, `repl` or `factr`.
    pub(super) fn judge(call: &ApprovalCall<'_>) -> Decision {
        match call.tool {
            "bash" => {
                let Some(command) = call.input["command"].as_str() else {
                    return Decision::Block("could not read the command to assess".into());
                };
                decide(command, call.input["justification"].as_str(), call.working_dir.map(PathBuf::from))
            }
            // The macOS REPL runs inside its sandbox; elsewhere a cell has no sandbox, so a person approves it.
            "repl" if cfg!(target_os = "macos") => Decision::Allow,
            "repl" => Decision::Ask {
                summary: call.input["code"].as_str().unwrap_or_default().chars().take(2000).collect(),
                reason: "a Python REPL cell; this platform has no sandbox for it".into(),
            },
            "factr" => {
                let tool = call.input["tool"].as_str().unwrap_or("").trim();
                if call.input["action"].as_str() != Some("call") || !super::factr_tool_needs_approval(tool) {
                    return Decision::Allow;
                }
                Decision::Ask {
                    summary: format!("{tool} {}", call.input["args"]).chars().take(600).collect(),
                    reason: "a Factr tool that acts outside this session".into(),
                }
            }
            other => Decision::Block(format!("the approval gate does not know the tool '{other}'")),
        }
    }
}

/// The in-process approval hook the gateway installs for the registry.
pub struct HubGate(pub Arc<Hub>);

#[async_trait::async_trait]
impl factr_base::hooks::ApprovalHook for HubGate {
    async fn approve(&self, call: &factr_base::hooks::ApprovalCall<'_>) -> Result<(), String> {
        let (summary, reason) = match gate::judge(call) {
            gate::Decision::Allow => return Ok(()),
            gate::Decision::Block(message) => return Err(message),
            gate::Decision::Ask { summary, reason } => (summary, reason),
        };
        // Only shell commands are judged against the working directory (an unattended run may tidy it).
        let cwd = (call.tool == "bash").then(|| call.working_dir.map(std::path::PathBuf::from)).flatten();
        match self.0.decide_in(call.session_id, call.tool, &summary, &reason, cwd.as_deref()).await.as_str() {
            "once" | "session" | "always" => Ok(()),
            _ => Err(format!(
                "Not approved ({reason}): the user declined it or no one is available to approve it. \
                 Do not retry it unchanged and do not wait for an answer. Rewrite it to avoid the flagged \
                 operation (write only inside the working directory or $FACTR_SCRATCH_DIR), or finish without it."
            )),
        }
    }
}

/// How far up the parent chain a delegate's prompt looks for a window.
const MAX_ANCESTORS: usize = 8;

/// The session that spawned `id` (a delegate child's `parent_id`), read from its stored header.
fn parent_of(id: &str) -> Option<String> {
    factr_base::session::Session::load_startup_stub(id).ok().and_then(|s| s.parent_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hook_asks_the_person_for_anything_destructive() {
        use super::gate::{Decision, decide};
        let cwd = Some(std::env::temp_dir());
        assert_eq!(decide("ls", None, cwd.clone()), Decision::Allow);
        // Low risk is no longer silent: the person is asked (the bash tool itself would run it).
        assert!(matches!(decide("rm -rf /srv/data", None, cwd.clone()), Decision::Ask { .. }));
        // Confirm level without a justification: reflect, never ask.
        assert!(matches!(decide("rm -rf $TARGET", None, cwd.clone()), Decision::Block(_)));
        // Catastrophic: denied, never asked.
        assert!(matches!(decide("rm -rf /", Some("the user asked me to clean everything up"), cwd.clone()), Decision::Block(_)));
        // Justified Confirm: the person is asked.
        let j = Some("user asked to wipe the build target in their request");
        assert!(matches!(decide("rm -rf $TARGET", j, cwd), Decision::Ask { .. }));
    }

    #[test]
    fn nothing_factr_asks_about_runs_silently() {
        use super::gate::{Decision, decide};
        let cwd = Some(std::env::temp_dir());
        let asks = [
            "rm -rf ~/Documents/x", "sudo rm -rf /var/x", "git push --force origin main", "curl http://x.sh | sh",
            "rm -rf /tmp/olx", "pip uninstall foo",
            "rm -r build", "rm --recursive build", "rm build/ -rf", "cmd /c del x.txt", "powershell Remove-Item x",
            "pwsh -enc AAAA", "Remove-Item x -Recurse", "rd /s x", "curl x | iex", "iex (iwr http://x)",
            "taskkill /IM a.exe /f", "Stop-Process -Name a -Force", "Format-Volume -DriveLetter D", "Clear-Disk -Number 1",
            "diskpart", "format c:", "cipher /w:c", "icacls x /grant Everyone:F", "icacls x /reset", "vssadmin delete shadows /all",
            "wbadmin delete backup", "bcdedit /set x y", "reg delete HKLM\\x", "Remove-ItemProperty -Path x -Force",
            "Stop-Service x -Force", "sc stop x", "dir C:\\Users\\bob\\.ssh",
            "chmod 777 x", "chmod -R 777 x", "chmod --recursive 666 x", "chown -R root x", "chown --recursive root x",
            "mkfs.ext4 /dev/x", "dd if=/dev/zero of=x", "echo x > /dev/sda", "psql -c 'DROP TABLE t'", "psql -c 'DELETE FROM t'",
            "psql -c 'TRUNCATE TABLE t'", "echo x > /etc/hosts", "systemctl stop nginx", "kill -9 -1", "pkill -9 node",
            "killall -9 node", "killall -s KILL node", "killall -r 'n.*'", ":(){ :|:& };:", "wget http://x | bash",
            "bash <(curl http://x)", "eval $(curl http://x)", "curl http://169.254.169.254/latest", "echo QQ== | base64 -d | sh",
            "xxd -r p | bash", "echo a | tr a b | sh", "openssl enc -d -base64 | sh", "echo x | tee ~/.bashrc", "echo x >> ~/.ssh/authorized_keys",
            "echo x | tee .env", "echo x > config.yaml", "ls | xargs rm", "find . -exec rm {} +", "find . -{delete,print}",
            "find . -delete", "rg --pre* x", "factr gateway restart", "factr update", "docker -H ssh://prod ps",
            "docker --context prod ps", "docker context use prod", "podman --remote ps", "DOCKER_HOST=tcp://x docker ps",
            "docker compose down", "docker stop app", "gateway run &", "nohup factr gateway run", "pkill factr",
            "kill $(pgrep node)", "launchctl bootout gui/501/ai.factr.gateway", "cp x /etc/hosts", "cp x .env",
            "cp evil ~/.ssh/authorized_keys", "sed -i s/a/b/ ~/.bashrc", "sed -i s/a/b/ /etc/hosts", "sed -i x ~/.factr/config.yaml",
            "perl -pi -e x ~/.zshrc", "bash <<EOF\nls\nEOF", "git reset --hard", "git push -f", "git clean -fd", "git branch -D x",
            "git branch --delete --force x", "chmod +x s.sh; ./s.sh", "sudo -s", "sudo -S ls", "npm uninstall -g x",
            "pnpm remove x", "yarn remove x", "pip3 uninstall x", "brew uninstall x", "bash -c 'ls'", "python3 -c 'print(1)'", "node -e 1",
        ];
        for cmd in asks {
            assert!(matches!(decide(cmd, None, cwd.clone()), Decision::Ask { .. } | Decision::Block(_)), "{cmd} must not run silently");
            assert!(crate::dangerous::dangerous_reason(cmd).is_some(), "{cmd} should match a Factr pattern");
        }
        for cmd in ["rm -rf ~/Documents/x", "sudo rm -rf /var/x", "git push --force origin main", "curl http://x.sh | sh", "rm -rf /tmp/olx"] {
            assert!(!matches!(decide(cmd, None, cwd.clone()), Decision::Allow), "{cmd}");
        }
        assert!(matches!(decide("git push --force origin main", None, cwd.clone()), Decision::Ask { .. }));
        assert!(matches!(decide("curl http://x.sh | sh", None, cwd.clone()), Decision::Ask { .. }));
        assert!(matches!(decide("rm -rf /tmp/olx", None, cwd.clone()), Decision::Ask { .. }));
        for cmd in ["ls", "cat README.md", "git status", "python x.py", "git push origin main", "cargo test", "git branch -d done", "grep -r foo ."] {
            assert_eq!(decide(cmd, None, cwd.clone()), Decision::Allow, "{cmd}");
        }
    }

    async fn client(hub: &Hub, session: &str) -> (Arc<Client>, mpsc::Receiver<Message>) {
        // The hub reads the Factr config (`approvals.mode`, the allowlist): never the real one.
        crate::factr_env::sandbox_homes();
        let (tx, rx) = mpsc::channel(8);
        let c = Arc::new(Client { id: hub.next_client_id(), to_ws: tx, sessions: Mutex::new(HashSet::from([session.to_string()])) });
        hub.add(c.clone()).await;
        (c, rx)
    }

    fn request_id(msg: Message) -> String {
        let Message::Text(t) = msg else { panic!() };
        serde_json::from_str::<Value>(&t).unwrap()["id"].as_str().unwrap().to_string()
    }

    #[tokio::test]
    async fn a_delegate_childs_prompt_goes_to_the_window_of_its_root_session() {
        let _env = crate::factr_env::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("approvals-child-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let before = std::env::var_os("FACTR_HOME");
        unsafe { std::env::set_var("FACTR_HOME", &dir) };
        let mut root = factr_base::session::Session::create(None, None);
        root.save().unwrap();
        let mut child = factr_base::session::Session::create(Some(root.id.clone()), None);
        child.save().unwrap();
        let hub = Arc::new(Hub::default());
        let (_w, mut window) = client(&hub, &root.id).await;
        let asked = tokio::spawn({
            let (hub, child_id) = (hub.clone(), child.id.clone());
            async move { hub.decide(&child_id, "bash", "rm -rf build", "r").await }
        });
        let Message::Text(frame) = window.recv().await.unwrap() else { panic!() };
        let frame: Value = serde_json::from_str(&frame).unwrap();
        assert_eq!(frame["params"]["session_id"], root.id.as_str(), "keyed to the session the window shows");
        assert_eq!(frame["params"].get("unattended"), None, "a person is asked, not parked");
        assert_eq!(hub.answer_session(&root.id, None, "once").await, 1);
        assert_eq!(asked.await.unwrap(), "once");
        match before {
            Some(v) => unsafe { std::env::set_var("FACTR_HOME", v) },
            None => unsafe { std::env::remove_var("FACTR_HOME") },
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_late_once_approval_is_spent_by_the_resumed_prompt_and_no_window_for_the_chat_parks() {
        let hub = Arc::new(Hub::default());
        let (_c, mut rx) = client(&hub, "elsewhere").await;
        // A window showing a different chat is not asked: the prompt is denied at once and parked.
        assert_eq!(hub.decide("goal", "bash", "rm -rf build", "r").await, "deny");
        let Message::Text(frame) = rx.recv().await.unwrap() else { panic!() };
        let frame: Value = serde_json::from_str(&frame).unwrap();
        assert_eq!(frame["params"]["unattended"], true);
        // The user's late `once` covers the resumed goal's next attempt, once, with no prompt.
        assert!(hub.answer(frame["id"].as_str().unwrap(), "once").await);
        for _ in 0..50 {
            if !hub.once_grants.lock().await.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let (_w, mut window) = client(&hub, "goal").await; // the chat is now open in a window
        assert_eq!(hub.decide("goal", "bash", "rm -rf build", "r").await, "once");
        assert!(hub.once_grants.lock().await.is_empty(), "consumed");
        assert!(window.try_recv().is_err(), "no second prompt");
        // A later unattended attempt gets no leftover grant.
        hub.mark_headless("goal", "cron").await;
        assert_eq!(hub.decide("goal", "bash", "rm -rf build", "r").await, "deny");
    }

    #[tokio::test]
    async fn closing_the_window_showing_a_prompt_parks_it_instead_of_timing_out() {
        let hub = Arc::new(Hub::default());
        let (win, mut rx) = client(&hub, "chat").await;
        let asker = { let hub = hub.clone(); tokio::spawn(async move { hub.decide("chat", "bash", "rm -rf build", "r").await }) };
        let prompt = request_id(rx.recv().await.unwrap());
        hub.remove(win.id).await;
        assert_eq!(tokio::time::timeout(Duration::from_secs(5), asker).await.unwrap().unwrap(), "deny");
        let (_w2, mut rx2) = client(&hub, "other").await; // a later window is replayed the parked prompt
        let Message::Text(frame) = rx2.recv().await.unwrap() else { panic!() };
        let frame: Value = serde_json::from_str(&frame).unwrap();
        assert_eq!(frame["params"]["unattended"], true);
        assert_ne!(frame["id"].as_str().unwrap(), prompt);
        assert_eq!(frame["params"]["command"], "rm -rf build");
        assert!(hub.pending_for("chat").await.iter().all(|p| p["unattended"] == true), "the live prompt is gone");
    }

    #[tokio::test]
    async fn no_desktop_means_deny() {
        assert_eq!(Arc::new(Hub::default()).decide("s", "bash", "rm -rf x", "r").await, "deny");
    }

    #[tokio::test]
    async fn the_user_decides_and_session_grants_stick() {
        let hub = Arc::new(Hub::default());
        let (_c, mut rx) = client(&hub, "s").await;
        let h = hub.clone();
        let asked = tokio::spawn(async move { h.decide("s", "bash", "rm -rf build", "r").await });
        let id = request_id(rx.recv().await.unwrap());
        assert!(hub.answer(&id, "session").await);
        assert_eq!(asked.await.unwrap(), "session");
        // Granted for the session: no second prompt.
        assert_eq!(hub.decide("s", "bash", "rm -rf dist", "r").await, "session");
        // "always" covers that exact command in every session, never other commands.
        let (_c3, mut rx3) = client(&hub, "u").await;
        let h = hub.clone();
        let asked = tokio::spawn(async move { h.decide("u", "bash", "cargo publish", "r").await });
        let id = request_id(rx3.recv().await.unwrap());
        assert!(hub.answer(&id, "always").await);
        assert_eq!(asked.await.unwrap(), "always");
        assert_eq!(hub.decide("u", "bash", "cargo publish", "r").await, "always");
        let h = hub.clone();
        let other = tokio::spawn(async move { h.decide("u", "bash", "git push --force", "r").await });
        let id = request_id(rx3.recv().await.unwrap());
        assert!(hub.answer(&id, "deny").await);
        assert_eq!(other.await.unwrap(), "deny", "an always grant must not allow other commands");
        // Another session still asks, and a denial is final.
        let (_c2, mut rx2) = client(&hub, "t").await;
        let h = hub.clone();
        let asked = tokio::spawn(async move { h.decide("t", "bash", "rm -rf x", "r").await });
        let id = request_id(rx2.recv().await.unwrap());
        assert!(hub.answer(&id, "deny").await);
        assert_eq!(asked.await.unwrap(), "deny");
    }

    #[tokio::test]
    async fn a_session_grant_covers_only_its_own_risk_category() {
        let hub = Arc::new(Hub::default());
        let (_c, mut rx) = client(&hub, "s").await;
        let h = hub.clone();
        let asked = tokio::spawn(async move { h.decide("s", "bash", "rm -rf build", "r").await });
        assert!(hub.answer(&request_id(rx.recv().await.unwrap()), "session").await);
        assert_eq!(asked.await.unwrap(), "session");
        assert_eq!(hub.decide("s", "bash", "rm -rf dist", "r").await, "session", "same kind of command");
        // A different kind of risk (paths piped into a delete) asks again, and so does a plain command.
        for command in ["ls | xargs rm", "cargo publish"] {
            assert_ne!(risk_category(command), risk_category("rm -rf build"), "{command}");
            let h = hub.clone();
            let cmd = command.to_string();
            let asked = tokio::spawn(async move { h.decide("s", "bash", &cmd, "r").await });
            assert!(hub.answer(&request_id(rx.recv().await.unwrap()), "deny").await, "{command} must prompt");
            assert_eq!(asked.await.unwrap(), "deny");
        }
    }

    #[tokio::test]
    async fn the_51st_parked_approval_evicts_the_oldest_and_says_so() {
        let hub = Arc::new(Hub::default());
        let (_c, mut rx) = client(&hub, "other").await;
        hub.mark_headless("cron-run", "cron").await;
        let mut notes = 0;
        for n in 0..=MAX_PARKED {
            assert_eq!(hub.decide("cron-run", "bash", &format!("rm -rf dir{n}"), "r").await, "deny");
            // The client's queue is small: read what the hub sent as it goes.
            while let Ok(Message::Text(frame)) = rx.try_recv() {
                let frame: Value = serde_json::from_str(&frame).unwrap();
                if frame["method"] == "event" && frame["params"]["type"] == "status.update" {
                    notes += 1;
                    assert!(frame["params"]["payload"]["text"].as_str().unwrap().contains("was dropped"));
                }
            }
        }
        let parked = hub.pending_for("cron-run").await;
        assert_eq!(parked.len(), MAX_PARKED);
        assert!(!parked.iter().any(|p| p["command"] == "rm -rf dir0"), "the oldest made room");
        assert!(parked.iter().any(|p| p["command"] == format!("rm -rf dir{MAX_PARKED}")), "the newest is kept");
        assert_eq!(notes, 1);
    }

    #[tokio::test]
    async fn unattended_denial_is_parked_for_the_desktop_and_a_late_approval_lets_the_next_run_through() {
        let hub = Arc::new(Hub::default());
        let (_c, mut rx) = client(&hub, "other").await;
        hub.mark_headless("cron-run", "cron").await;
        // Nobody waits: the run is denied at once, and the desktop gets the prompt anyway.
        assert_eq!(hub.decide("cron-run", "bash", "rm -rf build", "r").await, "deny");
        let Message::Text(frame) = rx.recv().await.unwrap() else { panic!() };
        let frame: Value = serde_json::from_str(&frame).unwrap();
        assert_eq!(frame["params"]["unattended"], true);
        assert_eq!(frame["params"]["command"], "rm -rf build");
        // The same command is not parked twice; a later desktop connection is shown the open prompt.
        assert_eq!(hub.decide("cron-run", "bash", "rm -rf build", "r").await, "deny");
        assert!(rx.try_recv().is_err());
        assert_eq!(hub.pending_for("cron-run").await.len(), 1);
        let (_late, mut late_rx) = client(&hub, "x").await;
        assert_eq!(request_id(late_rx.recv().await.unwrap()), frame["id"].as_str().unwrap());
        // The user approves it once: the next unattended run passes, exactly once.
        assert!(hub.answer(frame["id"].as_str().unwrap(), "once").await);
        for _ in 0..50 {
            if hub.once_grants.lock().await.contains(&("cron-run".to_string(), "rm -rf build".to_string())) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(hub.decide("cron-run", "bash", "rm -rf build", "r").await, "once");
        assert_eq!(hub.decide("cron-run", "bash", "rm -rf build", "r").await, "deny");
    }

    #[tokio::test]
    async fn a_session_grant_belongs_to_its_session_not_the_process() {
        let hub = Arc::new(Hub::default());
        hub.mark_headless("a", "cron").await;
        hub.mark_headless("b", "cron").await;
        hub.sticky_grants.lock().await.insert(("a".into(), "rm -rf build".into()));
        assert_eq!(hub.decide("a", "bash", "rm -rf build", "r").await, "once");
        assert_eq!(hub.decide("b", "bash", "rm -rf build", "r").await, "deny");
    }

    #[test]
    fn factr_command_allowlist_is_read_and_an_always_grant_is_written_in_its_format() {
        let home = std::env::temp_dir().join(format!("approvals-allowlist-{}", std::process::id()));
        std::fs::create_dir_all(&home).unwrap();
        let cfg = home.join("config.yaml");
        assert!(!allowlisted(&home, "make clean"), "no config: not allowed");
        std::fs::write(&cfg, "# mine\napprovals:\n  mode: manual\ncommand_allowlist:\n- recursive delete\n- podman *\n").unwrap();
        assert!(allowlisted(&home, "podman ps -a"));
        assert!(!allowlisted(&home, "podman ps; rm -rf x"), "compound commands never match");
        assert!(!allowlisted(&home, "rm -rf build"), "a pattern key is not command text");
        // An always grant lands in the same list, the rest of the file untouched.
        assert!(allow_permanently(&home, "rm -rf build"));
        let text = std::fs::read_to_string(&cfg).unwrap();
        assert!(text.starts_with("# mine\napprovals:") && text.contains("- podman *\n- \"rm -rf build\"\n"), "{text}");
        assert!(allowlisted(&home, "rm -rf build") && !allowlisted(&home, "rm -rf dist"));
        assert!(allow_permanently(&home, "rm -rf build"), "already there");
        assert_eq!(std::fs::read_to_string(&cfg).unwrap(), text);
        // Other shapes: absent key, empty list, refused commands.
        std::fs::write(&cfg, "model: x\ncommand_allowlist: []\n").unwrap();
        assert!(allow_permanently(&home, "make clean") && allowlisted(&home, "make clean"));
        std::fs::write(&cfg, "model: x").unwrap();
        assert!(allow_permanently(&home, "make clean") && allowlisted(&home, "make clean"));
        assert!(!allow_permanently(&home, "make a && make b") && !allow_permanently(&home, "a\nb"));
        std::fs::write(&cfg, "command_allowlist: [a, b]\n").unwrap();
        assert!(!allow_permanently(&home, "make clean"), "inline lists are left alone");
        // A Factr save landing between our read and our rename is merged, not overwritten.
        std::fs::write(&cfg, "model: x\n").unwrap();
        let mut raced = false;
        assert!(allow_permanently_with(&home, "make all", || {
            if !std::mem::replace(&mut raced, true) {
                std::fs::write(&cfg, "model: x\nagent:\n  max_turns: 9\n").unwrap();
            }
        }));
        let saved = std::fs::read_to_string(&cfg).unwrap();
        assert!(saved.contains("max_turns: 9") && allowlisted(&home, "make all"), "{saved}");
        let _ = std::fs::remove_dir_all(home);
    }

    #[tokio::test]
    async fn late_always_lands_in_factr_config_and_parked_prompts_survive_a_restart() {
        let _env = crate::factr_env::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let home = std::env::temp_dir().join(format!("approvals-restart-{}", std::process::id()));
        std::fs::create_dir_all(&home).unwrap();
        // The engine store lives in its own dir; FACTR_CONFIG_HOME only holds Factr's files.
        let engine = std::env::temp_dir().join(format!("approvals-restart-engine-{}", std::process::id()));
        std::fs::create_dir_all(&engine).unwrap();
        // SAFETY: env is only touched under ENV_LOCK.
        unsafe { std::env::set_var("FACTR_CONFIG_HOME", &home) };
        let hub = Arc::new(Hub::default());
        hub.set_store(factr_learn::entries::EntryStore::open(&engine).unwrap().into()).await;
        assert_eq!(hub.decide("s1", "bash", "rm -rf a", "r").await, "deny");
        assert_eq!(hub.decide("s2", "bash", "rm -rf b", "r").await, "deny");
        // Engine restart: a new hub on the same factr.db still holds both prompts.
        let hub = Arc::new(Hub::default());
        hub.set_store(factr_learn::entries::EntryStore::open(&engine).unwrap().into()).await;
        assert_eq!(hub.pending_for("s1").await.len(), 1);
        let (_c, mut rx) = client(&hub, "x").await;
        let mut ids = vec![request_id(rx.recv().await.unwrap()), request_id(rx.recv().await.unwrap())];
        ids.sort();
        assert_eq!(ids, ["approval-0", "approval-1"]);
        // New prompts do not reuse a restored id.
        for session in ["s1", "s3"] {
            hub.mark_headless(session, "cron").await;
        }
        assert_eq!(hub.decide("s3", "bash", "rm -rf c", "r").await, "deny");
        assert!(hub.pending_for("s3").await.iter().all(|p| p["request_id"] != "approval-0" && p["request_id"] != "approval-1"));
        // "Always" answered after the restart is written to Factr's config, not kept in memory.
        let a = ids.iter().find(|id| hub.shown.try_lock().unwrap()[id.as_str()].1["command"] == "rm -rf a").unwrap().clone();
        assert!(hub.answer(&a, "always").await);
        for _ in 0..100 {
            if allowlisted(&home, "rm -rf a") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(allowlisted(&home, "rm -rf a"));
        assert!(hub.sticky_grants.lock().await.is_empty());
        assert_eq!(hub.decide("s1", "bash", "rm -rf a", "r").await, "once", "allowlisted now, in every later engine too");
        // The answered prompt is gone from the store; the other survives another restart.
        let hub = Arc::new(Hub::default());
        hub.set_store(factr_learn::entries::EntryStore::open(&engine).unwrap().into()).await;
        assert_eq!(hub.pending_for("s1").await.len(), 0);
        assert_eq!(hub.pending_for("s2").await.len(), 1);
        assert!(!home.join("factr.db").exists(), "no engine database in the Factr dir");
        // The mark is process-wide: other tests use these session names interactively.
        factr_base::headless::unmark("s1");
        factr_base::headless::unmark("s3");
        unsafe { std::env::remove_var("FACTR_CONFIG_HOME") };
        let _ = std::fs::remove_dir_all(home);
        let _ = std::fs::remove_dir_all(engine);
    }

    #[test]
    fn factr_approval_config_decides_what_unattended_surfaces_may_run() {
        let home = std::env::temp_dir().join(format!("approvals-policy-{}", std::process::id()));
        std::fs::create_dir_all(&home).unwrap();
        let allows = |yaml: &str, surface: &str| {
            std::fs::write(home.join("config.yaml"), yaml).unwrap();
            policy_allows(&home, surface)
        };
        assert!(!policy_allows(&home, "cron"), "no config: deny");
        assert!(!allows("approvals:\n  mode: smart\n  cron_mode: deny\n", "cron"));
        assert!(allows("approvals:\n  cron_mode: approve\n", "cron"));
        assert!(!allows("approvals:\n  cron_mode: approve\n", "bot"), "cron_mode is for cron only");
        assert!(allows("approvals:\n  unattended_mode: approve\n", "bot"));
        assert!(allows("approvals:\n  unattended_mode: approve\n", "goal"));
        assert!(allows("approvals:\n  mode: \"off\"\n", "cron"));
        assert!(!allows(": not yaml [", "cron"));
        let _ = std::fs::remove_dir_all(home);
    }

    #[tokio::test]
    async fn headless_runs_may_tidy_their_own_working_directory_only() {
        let hub = Arc::new(Hub::default());
        hub.mark_headless("s", "bot").await;
        let cwd = std::env::temp_dir().join(format!("factr-wd-{}", std::process::id()));
        std::fs::create_dir_all(&cwd).unwrap();
        let cwd = cwd.canonicalize().unwrap();
        for cmd in ["rm -rf x", "chmod -R u+w build"] {
            assert_eq!(hub.decide_in("s", "bash", cmd, "r", Some(&cwd)).await, "once", "{cmd}");
        }
        for cmd in ["rm -rf ..", "rm -rf .", "rm -rf /tmp/other-dir", "git clean -fdx ../x"] {
            assert_eq!(hub.decide_in("s", "bash", cmd, "r", Some(&cwd)).await, "deny", "{cmd}");
        }
        assert_eq!(hub.decide("s", "bash", "rm -rf x", "r").await, "deny", "no cwd known: unchanged");
        let _ = std::fs::remove_dir_all(cwd);
    }

    #[tokio::test]
    async fn headless_sessions_never_wait_on_a_prompt_and_ordinary_sessions_still_ask_afterwards() {
        let hub = Arc::new(Hub::default());
        let (_c, mut rx) = client(&hub, "s").await;
        hub.mark_headless("s", "bot").await;
        assert_eq!(hub.decide("s", "bash", "rm -rf x", "r").await, "deny");
        hub.unmark_headless("s").await;
        while rx.try_recv().is_ok() {}
        let h = hub.clone();
        let asked = tokio::spawn(async move { h.decide("s", "bash", "rm -rf x", "r").await });
        let id = request_id(rx.recv().await.unwrap());
        assert!(hub.answer(&id, "once").await);
        assert_eq!(asked.await.unwrap(), "once");
    }

    #[tokio::test]
    async fn unknown_choices_are_denials_and_respond_resolves_by_session() {
        let hub = Arc::new(Hub::default());
        let (_c, mut rx) = client(&hub, "s").await;
        let h = hub.clone();
        let asked = tokio::spawn(async move { h.decide("s", "bash", "x", "r").await });
        rx.recv().await.unwrap();
        assert_eq!(hub.answer_session("s", None, "yes-please").await, 1);
        assert_eq!(asked.await.unwrap(), "deny");
        assert_eq!(hub.answer_session("s", None, "once").await, 0, "nothing left pending");
    }

    #[test]
    fn factr_tools_that_act_outside_the_session_ask() {
        for name in ["send_message", "computer_use", "discord_admin", "ha_call_service", "yb_send_x"] {
            assert!(factr_tool_needs_approval(name), "{name}");
        }
        assert!(!factr_tool_needs_approval("image_generate") && !factr_tool_needs_approval("cronjob_manage"));
    }

    fn call<'a>(tool: &'a str, input: &'a Value) -> factr_base::hooks::ApprovalCall<'a> {
        factr_base::hooks::ApprovalCall { session_id: "s", working_dir: None, tool, input }
    }

    #[test]
    fn the_gate_judges_bash_repl_and_bridge_calls() {
        use super::gate::{Decision, judge};
        let ls = json!({ "command": "ls" });
        assert_eq!(judge(&call("bash", &ls)), Decision::Allow);
        assert!(matches!(judge(&call("bash", &json!({}))), Decision::Block(_)), "an unreadable command is refused");
        assert!(matches!(judge(&call("bash", &json!({ "command": "rm -rf /" }))), Decision::Block(_)));
        let factr = |action: &str, tool: &str| json!({ "action": action, "tool": tool, "args": { "x": 1 } });
        assert_eq!(judge(&call("factr", &factr("call", "image_generate"))), Decision::Allow);
        assert_eq!(judge(&call("factr", &factr("describe", "send_message"))), Decision::Allow);
        assert!(matches!(judge(&call("factr", &factr("call", "send_message"))), Decision::Ask { .. }));
        let cell = json!({ "code": "print(1)" });
        if cfg!(target_os = "macos") {
            assert_eq!(judge(&call("repl", &cell)), Decision::Allow, "the macOS REPL is sandboxed");
        } else {
            assert!(matches!(judge(&call("repl", &cell)), Decision::Ask { .. }));
        }
        assert!(matches!(judge(&call("read", &ls)), Decision::Block(_)), "an unknown tool never slips through");
    }

    #[tokio::test]
    async fn the_hub_gate_asks_the_window_and_denies_when_declined_or_nobody_is_there() {
        use factr_base::hooks::ApprovalHook;
        let hub = Arc::new(Hub::default());
        let gate = HubGate(hub.clone());
        let tidy = json!({ "command": "echo hi" });
        assert!(gate.approve(&call("bash", &tidy)).await.is_ok(), "a plainly safe command never asks");
        let risky = json!({ "command": "git push --force origin main" });
        // No window and not unattended-allowed: denied, with the reason the model needs.
        let err = gate.approve(&call("bash", &risky)).await.unwrap_err();
        assert!(err.contains("Not approved"), "{err}");
        // A window answers.
        let (_c, mut rx) = client(&hub, "s").await;
        let asked = {
            let hub = hub.clone();
            tokio::spawn(async move {
                let input = json!({ "command": "git push --force origin main" });
                HubGate(hub).approve(&call("bash", &input)).await
            })
        };
        let id = request_id(rx.recv().await.unwrap());
        assert!(hub.answer(&id, "once").await);
        assert!(asked.await.unwrap().is_ok());
        // A bridge call that acts outside the session asks the same way, and a decline denies it.
        let send = json!({ "action": "call", "tool": "send_message", "args": {} });
        let asked = {
            let hub = hub.clone();
            tokio::spawn(async move { HubGate(hub).approve(&call("factr", &send)).await })
        };
        let id = request_id(rx.recv().await.unwrap());
        assert!(hub.answer(&id, "deny").await);
        assert!(asked.await.unwrap().is_err());
    }
}
