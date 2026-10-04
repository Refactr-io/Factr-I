//! Stop nudges when a turn ends with no tool calls.
//!
//! Ported from Factr `agent/turn_stop_gates.py` + `verification_stop.py`
//! (nudge after code edits unless tests ran) and its tool-use-enforcement path
//! (a text-only turn that announces an action). The test-command detection is
//! the idea of `coding_context.py`. At most two nudges per turn (the second only after tool calls in between), and never in a
//! turn without edits (verify) or without prior tool use (action).

const EDIT_TOOLS: [&str; 4] = ["write", "edit", "apply_patch", "replace"];
const TEST_MARKERS: [&str; 23] = [
    "jest", "vitest", "bun test", "deno test", "node --test", "swift test", "dotnet test",
    "gradle test", "tox",
    "pytest", "unittest", "cargo test", "cargo nextest", "go test", "npm test", "npm run test",
    "yarn test", "pnpm test", "make test", "gradlew test", "ctest", "mvn test", "rspec",
];
const CODE_EXTS: [&str; 26] = [
    "rs", "py", "js", "jsx", "ts", "tsx", "mjs", "cjs", "go", "java", "kt", "swift", "c", "h",
    "cc", "cpp", "hpp", "cs", "rb", "php", "scala", "sh", "lua", "dart", "zig", "ex",
];
const ACTION_PHRASES: [&str; 17] = [
    "i will", "i'll", "let me", "now i'll", "now, i'll", "next i'll", "next, i'll", "first, i'll",
    "first i'll", "then i'll", "i am going to", "i'm going to", "i need to", "i should", "i can now",
    "plan:", "next steps:",
];

const VERIFY_NUDGE: &str = "<system-reminder>Before finishing: re-read the task's explicit requirements (exact names, paths, commands, formats) and check each against your result; run or exercise what you built (use python3 -m unittest if pytest is missing); fix anything that fails. If you cannot fully solve it, write your best-effort result to the requested output first.</system-reminder>";
const STEER: &str = "Two attempts have not fixed this. Step back: read the failing output again, write down two different hypotheses for the cause, test the most likely one with a quick command before editing again, and do not repeat an edit you already tried.";
const QUESTION_NUDGE: &str = "<system-reminder>No user is available to answer. Act on the most likely reading of the task now, do the work, and verify it.</system-reminder>";
const FAILED_CHECK_NUDGE: &str = "<system-reminder>Your last check failed and nothing changed since. Fix it or explain precisely what blocks you, and still write your best result.</system-reminder>";
const OFFER_PHRASES: [&str; 9] = [
    "i can run", "i can do", "if you want", "if you'd like", "would you like", "do you want me to",
    "shall i", "should i", "let me know if you want me to",
];
const SNIPPET_NUDGE: &str = "<system-reminder>Your answer rests on search result snippets alone. Open the source behind what it depends on and check it there.</system-reminder>";
const DECLINE_PHRASES: [&str; 9] = [
    "unable to determine", "unable to verify", "unable to find", "cannot determine", "can't determine",
    "cannot be determined", "could not find", "couldn't find", "not possible to",
];
// A decline that names an unreadable input is an honest limitation, not a give-up.
const UNREADABLE_PHRASES: [&str; 10] = [
    "unreadable", "inaccessible", "cannot read", "can't read", "could not read", "couldn't read",
    "unable to read", "cannot open", "can't open", "unable to access",
];
const DECLINE_NUDGE: &str = "<system-reminder>Try one different route (another source, tool or method). Then give your best-supported answer and state the uncertainty in one sentence. If the blocker is an input you cannot read, say so plainly.</system-reminder>";
const ACTION_NUDGE: &str = "<system-reminder>Continue by calling the tool now, or state the final result.</system-reminder>";

type Runner<'a> = &'a dyn Fn(&str, &std::path::Path, std::time::Duration) -> factr_learn::agent_loop::GateResult;

#[derive(Default)]
pub(super) struct StopNudge {
    edited: bool,
    used_tools: bool,
    sent: u8,
    sent_seq: u32,
    pending: Option<String>,
    // Auto-verify gate state. `seq` orders edits against test runs.
    seq: u32,
    last_edit: u32,
    last_test: u32,
    last_test_exit: i64,
    tests_touched: bool,
    cwd: Option<std::path::PathBuf>,
    gate_ok: bool,
    baseline: Vec<String>,
    rounds: u32,
    last_fail: Option<u64>,
    last_count: Option<usize>,
    gate_ran: bool,
    passed: bool,
    last_fail_stop: bool,
    no_marker_logged: bool,
    // Headless runs (no user to answer) and the last check-looking bash run.
    headless: bool,
    last_write: u32,
    last_check: u32,
    last_check_exit: i64,
    // Literal labelled line the task demands in the final reply (e.g. `RESULT:`).
    format_label: Option<String>,
    // Search/fetch tally for the snippet-only-answer nudge (sent once per turn).
    searches: u32,
    fetched: bool,
    snippet_sent: bool,
    // The 70% time-budget reminder was sent (set by the turn loop); decline_sent: one per turn.
    late: bool,
    decline_sent: bool,
}

fn enabled() -> bool {
    std::env::var("FACTR_VERIFY_ON_STOP").map_or(true, |v| v != "0")
        && crate::config::config().agents.verify_on_stop
}

/// Default: headless runs only (an interactive chat gets the test nudge, not surprise test runs).
/// `FACTR_AUTO_VERIFY` (set from `FACTR_AUTO_VERIFY`): `0` forces off, `1` forces on everywhere.
fn auto_verify_enabled(headless: bool) -> bool {
    auto_verify_gate(std::env::var("FACTR_AUTO_VERIFY").ok().as_deref(), headless, crate::config::config().agents.auto_verify)
}

fn auto_verify_gate(env: Option<&str>, headless: bool, configured: bool) -> bool {
    match env {
        Some("0") => false,
        Some("1") => true,
        _ => headless && configured,
    }
}

fn looks_like_test_run(command: &str) -> bool {
    let c = command.to_ascii_lowercase();
    TEST_MARKERS.iter().any(|m| c.contains(m))
}

/// Failing-test count from a runner's output (pytest/cargo `N failed`, unittest
/// `FAILED (failures=N, errors=M)`, go `FAIL` lines); output length when unparsable.
fn fail_count(out: &str) -> usize {
    let words: Vec<&str> = out.split(|c: char| c.is_whitespace() || matches!(c, ',' | ';' | '(' | ')')).filter(|w| !w.is_empty()).collect();
    let mut n = 0;
    let mut found = false;
    for (i, w) in words.iter().enumerate() {
        if *w == "failed" && i > 0 {
            if let Ok(k) = words[i - 1].parse::<usize>() {
                n += k;
                found = true;
            }
        }
        for key in ["failures=", "errors="] {
            if let Some(k) = w.strip_prefix(key).and_then(|v| v.parse::<usize>().ok()) {
                n += k;
                found = true;
            }
        }
    }
    let go = out.lines().filter(|l| l.trim_start().starts_with("--- FAIL")).count();
    if go > 0 {
        return n + go;
    }
    if found { n } else { out.len() }
}

fn looks_like_check(command: &str) -> bool {
    let c = command.to_ascii_lowercase();
    looks_like_test_run(command) || ["test", "check", "verify", "validate", "lint"].iter().any(|m| c.contains(m))
}

fn is_code_path(p: &str) -> bool {
    let p = p.trim().trim_matches(['"', '\'']);
    p.rsplit_once('.')
        .is_some_and(|(_, e)| CODE_EXTS.contains(&e.to_ascii_lowercase().as_str()))
}

/// Paths an edit-tool call touched; `None` when the call names no readable
/// path (apply_patch bodies are scanned for source-file paths instead).
fn edit_paths(input: &serde_json::Value) -> Vec<String> {
    for k in ["file_path", "path", "filePath"] {
        if let Some(p) = input[k].as_str() {
            return vec![p.to_string()];
        }
    }
    input["patch_text"]
        .as_str()
        .or_else(|| input["patch"].as_str())
        .or_else(|| input["input"].as_str())
        .map(|t| {
            t.lines()
                .filter(|l| l.starts_with("***") || l.starts_with("+++"))
                .filter_map(|l| l.split_whitespace().last().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// Whether an edit-tool call touched a source file. Calls with no readable
/// path are assumed to.
fn edits_code(input: &serde_json::Value) -> bool {
    let paths = edit_paths(input);
    if paths.is_empty() {
        return input["file_path"].is_null()
            && input["path"].is_null()
            && input["filePath"].is_null()
            && !(input["patch_text"].is_string() || input["patch"].is_string() || input["input"].is_string());
    }
    paths.iter().any(|p| is_code_path(p))
}

/// Hash of a test run's output with timing tokens (`0.02s`, `(0.00s)`, `1.5`) dropped,
/// so the same failure compares equal across runs ("1 failed in 0.02s" vs "0.03s").
fn hash_str(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for line in s.lines() {
        for tok in line.split_whitespace() {
            let t = tok.trim_matches(|c: char| "()[],".contains(c));
            let t = t.strip_suffix("ms").or_else(|| t.strip_suffix('s')).unwrap_or(t);
            if t.contains('.') && t.parse::<f64>().is_ok() {
                continue;
            }
            tok.hash(&mut h);
        }
        '\n'.hash(&mut h);
    }
    h.finish()
}

impl super::Agent {
    /// Goal sessions run their own gates.
    pub(super) fn in_goal(&self) -> bool {
        factr_base::storage::factr_dir()
            .ok()
            .and_then(|h| factr_learn::agent_loop::ControlStore::open_cached(&h).ok())
            .is_some_and(|st| {
                use factr_learn::agent_loop::GoalStatus;
                st.get_goal(&self.session.id).ok().flatten().is_some_and(|g| g.status == GoalStatus::Active)
            })
    }

    /// Per-turn nudge/gate state. The gate is off for subagents, goal
    /// sessions (their own gates), past a hard deadline, and when
    /// the cwd is missing, `/`, or a home directory.
    pub(super) fn new_stop_nudge(&self) -> StopNudge {
        let mut n = StopNudge::default();
        let cwd = self.session.working_dir.as_deref().map(std::path::PathBuf::from);
        let is_home = |d: &std::path::Path| {
            d == std::path::Path::new("/")
                || std::env::var_os("HOME").is_some_and(|h| d == std::path::Path::new(&h))
                || factr_base::storage::factr_dir().is_ok_and(|h| d == h)
        };
        let past_deadline = std::env::var("FACTR_HARD_DEADLINE_UNIX")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .is_some_and(|d| {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .is_ok_and(|t| t.as_secs() >= d)
            });
        n.gate_ok = auto_verify_enabled(factr_base::headless::is(&self.session.id))
            && self.session.parent_id.is_none()
            && !past_deadline
            && cwd.as_deref().is_some_and(|d| d.is_dir() && !is_home(d))
            && !self.in_goal();
        n.headless = factr_base::headless::is(&self.session.id);
        if self.session.parent_id.is_none() && !self.in_goal() {
            n.format_label = first_task_text(
                self.session
                    .messages
                    .iter()
                    .filter(|m| m.role == factr_message_types::Role::User)
                    .map(|m| m.content.as_slice()),
            )
            .and_then(required_label);
        }
        if n.gate_ok {
            n.baseline = super::auto_verify::git_changed(cwd.as_deref().unwrap());
            n.cwd = cwd;
        }
        n
    }
}

impl StopNudge {
    /// Record a finished tool call. `exit_code` is the tool's exit status
    /// (see `auto_verify::exit_code_of`).
    pub(super) fn observe(&mut self, tool: &str, input: &serde_json::Value, exit_code: i64) {
        self.used_tools = true;
        self.seq += 1;
        if exit_code == 0 && EDIT_TOOLS.contains(&tool) {
            self.last_write = self.seq;
            if edits_code(input) {
                self.edited = true;
                self.last_edit = self.seq;
                self.passed = false;
            }
            if edit_paths(input).iter().any(|p| super::auto_verify::is_test_path(p)) {
                self.tests_touched = true;
            }
        }
        match tool {
            "websearch" => self.searches += 1,
            "webfetch" if exit_code == 0 => self.fetched = true,
            _ => {}
        }
        if tool == "bash" && input["command"].as_str().is_some_and(looks_like_test_run) {
            self.last_test = self.seq;
            self.last_test_exit = exit_code;
        }
        if tool == "bash" && input["command"].as_str().is_some_and(looks_like_check) {
            self.last_check = self.seq;
            self.last_check_exit = exit_code;
        }
    }

    /// Fallback for edits made through a non-edit tool (python, shell):
    /// code files that became changed in git since the turn began.
    fn git_edited(&mut self) -> bool {
        let Some(dir) = self.cwd.as_deref().filter(|_| self.used_tools) else { return false };
        let changed: Vec<String> = super::auto_verify::git_changed(dir)
            .into_iter()
            .filter(|p| !self.baseline.contains(p))
            .collect();
        if changed.iter().any(|p| super::auto_verify::is_test_path(p)) {
            self.tests_touched = true;
        }
        changed.iter().any(|p| is_code_path(p))
    }

    fn guard_span(&self, session_id: &str, reason: &str, cmd: &str, ms: u64) -> factr_base::obs_sink::Span {
        factr_base::obs_sink::Span::new("loop.guard")
            .session(session_id)
            .attr("reason", reason)
            .attr("round", self.rounds)
            .attr("command", cmd)
            .attr("tests_touched", self.tests_touched)
            .took_ms(ms)
    }

    /// Run the project's tests once more when the turn ends after edits.
    /// `None`: the gate does not apply (caller falls back to the nudge).
    /// `Some(true)`: failure feedback queued, continue the turn.
    /// `Some(false)`: end the turn (passed, or stopped early).
    fn auto_verify(&mut self, session_id: &str, run: Runner) -> Option<bool> {
        if !self.gate_ok {
            return None;
        }
        let tested_ok = self.last_test > self.last_edit && self.last_test_exit == 0;
        if self.edited {
            if tested_ok {
                return Some(false);
            }
        } else if self.passed || !self.git_edited() {
            return None;
        }
        let dir = self.cwd.clone()?;
        let Some((name, cmd)) = super::auto_verify::detect_test_command(&dir) else {
            if !self.no_marker_logged {
                self.no_marker_logged = true;
                factr_base::obs_sink::emit(self.guard_span(session_id, "auto_verify_skip_no_marker", "", 0));
            }
            return None;
        };
        self.gate_ran = true;
        let cfg = &crate::config::config().agents;
        if self.rounds >= cfg.auto_verify_rounds.max(1) || self.last_fail_stop {
            return Some(false);
        }
        self.rounds += 1;
        let started = std::time::Instant::now();
        // A cold build (Cargo, Gradle, Maven, CMake) takes far longer than a script test run, so the
        // first run would falsely time out at the base limit.
        let slow_build = ["cargo ", "gradle ", "./gradlew ", "mvn ", "cmake "].iter().any(|b| cmd.starts_with(b));
        let base = cfg.auto_verify_timeout_s.max(1);
        let r = run(&cmd, &dir, std::time::Duration::from_secs(if slow_build { base.saturating_mul(3) } else { base }));
        let ms = started.elapsed().as_millis() as u64;
        let reason = match (r.passed, r.exit_code) {
            (true, _) => "auto_verify_pass",
            (_, 124) => "auto_verify_timeout",
            _ => "auto_verify_fail",
        };
        let span = self.guard_span(session_id, reason, name, ms);
        // A pass that skipped tests is recorded, never acted on: tests are the user's to un-skip.
        factr_base::obs_sink::emit(if r.passed { span.attr("skipped", super::auto_verify::skipped_count(&r.output)) } else { span });
        if r.passed {
            // The pass is now the latest test run; later edits re-arm the gate.
            self.seq += 1;
            self.last_test = self.seq;
            self.last_test_exit = 0;
            self.passed = true;
            return Some(false);
        }
        let h = hash_str(&r.output);
        if self.last_fail.replace(h) == Some(h) {
            self.last_fail_stop = true;
            return Some(false);
        }
        let count = fail_count(&r.output);
        let steer = self.last_count.replace(count).is_some_and(|prev| count >= prev);
        if steer {
            factr_base::obs_sink::emit(self.guard_span(session_id, "auto_verify_steer", name, 0));
        }
        self.pending = Some(format!(
            "<system-reminder>Auto-verify: `{cmd}` failed (exit {}, round {}/{}). Last output:\n{}\nFix the failures, then finish.{}</system-reminder>",
            r.exit_code, self.rounds, cfg.auto_verify_rounds.max(1), r.output,
            if steer { format!("\n\n{STEER}") } else { String::new() }
        ));
        Some(true)
    }

    /// Called when the model ended its turn with no tool calls. Returns true
    /// when a nudge or gate feedback is queued and the loop should continue.
    pub(super) fn on_text_only_stop(&mut self, session_id: &str, text: &str) -> bool {
        self.on_text_only_stop_with(session_id, text, &real_runner)
    }

    fn on_text_only_stop_with(&mut self, session_id: &str, text: &str, run: Runner) -> bool {
        if self.auto_verify(session_id, run) == Some(true) {
            return true;
        }
        // Max 2 nudges per turn; the second only after tool calls made progress.
        if self.sent >= 2 || (self.sent == 1 && self.seq == self.sent_seq) || !enabled() {
            return false;
        }
        let lower = text.to_ascii_lowercase();
        let format_nudge;
        let (nudge, reason) = if self.edited && self.last_test <= self.last_edit && !self.gate_ran {
            (VERIFY_NUDGE, "verify_nudge")
        } else if !self.gate_ran && self.last_check > self.last_write && self.last_check_exit != 0 {
            (FAILED_CHECK_NUDGE, "failed_check_nudge")
        } else if self.used_tools && announces_action(&lower) {
            (ACTION_NUDGE, "action_nudge")
        } else if self.headless && !self.edited && asks_or_offers(&lower) {
            (QUESTION_NUDGE, "question_nudge")
        } else if self.headless && !self.edited && self.snippet_only() {
            self.snippet_sent = true;
            (SNIPPET_NUDGE, "snippet_nudge")
        } else if self.headless && !self.edited && !self.late && !self.decline_sent && declines(&lower) {
            self.decline_sent = true;
            (DECLINE_NUDGE, "decline_nudge")
        } else if let Some(l) = self.format_label.as_deref().filter(|l| !has_label_line(text, l) && !text.trim().is_empty()) {
            format_nudge = format!("<system-reminder>Your reply must end with the required line `{l} <value>` exactly as specified in the task (a single value unless the task asks for a list). Restate your final result in that format now.</system-reminder>");
            self.format_label = None; // one format nudge per turn
            (format_nudge.as_str(), "format_nudge")
        } else {
            return false;
        };
        self.sent += 1;
        self.sent_seq = self.seq;
        self.pending = Some(nudge.to_string());
        factr_base::obs_sink::emit(
            factr_base::obs_sink::Span::new("loop.guard")
                .session(session_id)
                .attr("reason", reason)
                .attr("tests_touched", self.tests_touched),
        );
        true
    }

    /// The turn loop reports whether the 70% deadline stage has been reached.
    pub(super) fn set_late(&mut self, late: bool) {
        self.late = late;
    }

    /// Searched, never fetched a page. At `SEARCH_STALL_AFTER` searches the repeat guard has
    /// already sent its own search nudge this turn, so this one stays quiet.
    fn snippet_only(&self) -> bool {
        !self.snippet_sent && !self.fetched && (1..super::repeat_guard::SEARCH_STALL_AFTER).contains(&self.searches)
    }

    pub(super) fn take_pending(&mut self) -> Option<String> {
        self.pending.take()
    }
}

fn real_runner(cmd: &str, dir: &std::path::Path, t: std::time::Duration) -> factr_learn::agent_loop::GateResult {
    let run = || factr_learn::agent_loop::run_command(cmd, Some(dir), t);
    match tokio::runtime::Handle::try_current() {
        Ok(h) if h.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread => tokio::task::block_in_place(run),
        _ => run(),
    }
}

/// Text of the first user message that is the task itself: headless runs put a
/// `<system-reminder>` session-context message first, which carries no task.
fn first_task_text<'a>(user_messages: impl Iterator<Item = &'a [factr_message_types::ContentBlock]>) -> Option<&'a str> {
    user_messages
        .filter_map(|blocks| {
            blocks.iter().find_map(|b| match b {
                factr_message_types::ContentBlock::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
        })
        .find(|t| !t.trim_start().starts_with("<system-reminder>"))
}

/// Literal uppercase label (e.g. `RESULT:`) the task says the reply must
/// end with / contain / start with: quoted, or the first unquoted label after a
/// `[`/`<` placeholder, within a short window after a formatting verb.
fn required_label(task: &str) -> Option<String> {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| regex::Regex::new(r#"(["'`\u{2018}\u{201C}])?\b([A-Z]{2,}(?: [A-Z]{2,}){0,3}:)(\s*[\[<])?"#).unwrap());
    const VERBS: [&str; 10] = ["end", "finish", "conclude", "finali", "report", "respond", "reply", "answer", "template", "format"];
    for c in re.captures_iter(task) {
        let m = c.get(2)?;
        if c.get(1).is_none() && c.get(3).is_none() {
            continue;
        }
        let start = task[..m.start()].char_indices().rev().nth(120).map_or(0, |(i, _)| i);
        let before = task[start..m.start()].to_ascii_lowercase();
        if VERBS.iter().any(|v| before.contains(v)) {
            return Some(m.as_str().to_string());
        }
    }
    None
}

/// Some line of the reply starts with the label (markdown emphasis ignored).
fn has_label_line(text: &str, label: &str) -> bool {
    text.lines().any(|l| l.trim_start_matches(|c: char| c.is_whitespace() || matches!(c, '*' | '`' | '_' | '#' | '>' | '-')).starts_with(label))
}

/// The final paragraph asks the user something or offers to do the work.
fn asks_or_offers(lower: &str) -> bool {
    let body = lower.trim_end();
    let para = body.rsplit("\n\n").next().unwrap_or(body);
    para.ends_with('?') || OFFER_PHRASES.iter().any(|p| para.contains(p))
}

/// The final paragraph gives up on the question, without blaming an unreadable input.
fn declines(lower: &str) -> bool {
    let body = lower.trim_end();
    let para = body.rsplit("\n\n").next().unwrap_or(body);
    DECLINE_PHRASES.iter().any(|p| para.contains(p)) && !UNREADABLE_PHRASES.iter().any(|p| para.contains(p))
}

/// The reply ends on a promise to act: its last sentence, or the lead-in line
/// of its last paragraph (a numbered plan), carries the phrase.
fn announces_action(lower: &str) -> bool {
    let body = lower.trim_end();
    let para = body.rsplit("\n\n").next().unwrap_or(body);
    if para.trim_end().ends_with('?') || para.contains("summary") {
        return false;
    }
    let last = body.rsplit(['.', '!', '?', '\n']).find(|s| !s.trim().is_empty()).unwrap_or("").trim();
    let lead = para.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
    let promises = |s: &str| !s.starts_with("let me know") && ACTION_PHRASES.iter().any(|p| s.starts_with(p));
    promises(last) || promises(lead) || body.ends_with(':')
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn label_detected_from_phrasings() {
        let l = |t: &str| required_label(t);
        let want = Some("RESULT:".to_string());
        assert_eq!(l("Report your answer. Finish your answer with the following template: RESULT: [YOUR VALUE]"), want);
        assert_eq!(l("Please answer with 'RESULT: ...' at the end."), want);
        assert_eq!(l("Conclude your reply with `RESULT: <value>`."), want);
        assert_eq!(l("Respond in this format: \"RESULT: x\""), want);
        assert_eq!(l("Fix the bug in parse(); NOTE: tests are slow."), None);
        assert_eq!(l("Write a poem. Format: 'haiku' only."), None);
    }

    #[test]
    fn task_text_skips_leading_session_context_reminder() {
        use factr_message_types::Message;
        let msgs = [
            Message::user("<system-reminder>\n# Session Context\nDate: x\n</system-reminder>"),
            Message::user("Finish your answer with: RESULT: [YOUR VALUE]"),
        ];
        let task = first_task_text(msgs.iter().map(|m| m.content.as_slice()));
        assert_eq!(task.and_then(required_label), Some("RESULT:".to_string()));
    }

    #[test]
    fn format_nudge_once_and_only_when_missing() {
        let task = "Finish your answer with the following template: RESULT: [YOUR VALUE]";
        let mk = || StopNudge { format_label: required_label(task), ..Default::default() };
        let mut n = mk();
        assert!(n.on_text_only_stop("s", "It is 42."));
        assert!(n.take_pending().unwrap().contains("`RESULT: <value>`"));
        n.observe("bash", &json!({"command": "ls"}), 0);
        assert!(!n.on_text_only_stop("s", "Still 42."), "format nudge fires once");
        let mut n = mk();
        assert!(!n.on_text_only_stop("s", "Reasoning.\n\n**RESULT: 42**"));
        let mut n = StopNudge::default();
        assert!(!n.on_text_only_stop("s", "It is 42."));
    }

    #[test]
    fn snippet_only_answer_nudged_once_headless_when_nothing_was_fetched() {
        let searched = |n: u32, fetch: Option<i64>, headless: bool| {
            let mut s = StopNudge::default();
            s.headless = headless;
            for _ in 0..n {
                s.observe("websearch", &json!({"query": "q"}), 0);
            }
            if let Some(code) = fetch {
                s.observe("webfetch", &json!({"url": "u"}), code);
            }
            s
        };
        let mut n = searched(1, None, true);
        assert!(n.on_text_only_stop("s", "It is 42."));
        assert_eq!(n.take_pending(), Some(SNIPPET_NUDGE.to_string()));
        n.observe("bash", &json!({"command": "ls"}), 0);
        assert!(!n.on_text_only_stop("s", "It is 42."), "once per turn");
        assert!(!searched(1, None, false).on_text_only_stop("s", "It is 42."), "interactive");
        assert!(!searched(0, None, true).on_text_only_stop("s", "It is 42."), "no search");
        assert!(!searched(1, Some(0), true).on_text_only_stop("s", "It is 42."), "fetched");
        assert!(searched(1, Some(1), true).on_text_only_stop("s", "It is 42."), "failed fetch is not a source");
        let stalled = super::super::repeat_guard::SEARCH_STALL_AFTER;
        assert!(!searched(stalled, None, true).on_text_only_stop("s", "It is 42."), "the search-stall nudge already fired");
        // After code work (edited, then tested), a looked-up API answer is not a research answer.
        let mut coded = searched(1, None, true);
        coded.observe("edit", &json!({"file_path": "a.rs"}), 0);
        coded.observe("bash", &json!({"command": "cargo test"}), 0);
        assert!(!coded.on_text_only_stop("s", "Done: the parser now streams."), "code work");
    }

    #[test]
    fn decline_nudged_once_headless_unless_late_or_input_unreadable() {
        let mk = |late: bool| {
            let mut n = StopNudge::default();
            n.headless = true;
            n.set_late(late);
            n
        };
        let mut n = mk(false);
        assert!(n.on_text_only_stop("s", "Checked two pages.\n\nI was unable to determine the value."));
        assert_eq!(n.take_pending(), Some(DECLINE_NUDGE.to_string()));
        n.observe("bash", &json!({"command": "ls"}), 0);
        assert!(!n.on_text_only_stop("s", "I could not find it."), "once per turn");
        assert!(!mk(true).on_text_only_stop("s", "I could not find it."), "past the 70% stage");
        assert!(!mk(false).on_text_only_stop("s", "I cannot determine the total: the attached file is unreadable."));
        assert!(!mk(false).on_text_only_stop("s", "It is not possible to open the file (inaccessible)."));
        assert!(!mk(false).on_text_only_stop("s", "I could not find it, but I could not find it.\n\nThe answer is 42."), "only the last paragraph counts");
        let mut interactive = StopNudge::default();
        assert!(!interactive.on_text_only_stop("s", "I could not find it."));
        // After code work (edited, then tested), "could not find" reports on the code, not a give-up.
        let mut coded = mk(false);
        coded.observe("edit", &json!({"file_path": "a.rs"}), 0);
        coded.observe("bash", &json!({"command": "cargo test"}), 0);
        assert!(!coded.on_text_only_stop("s", "Fixed. I could not find any other caller."), "code work");
    }

    #[test]
    fn passing_gate_records_skipped_count_on_the_guard_span_without_nudging() {
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = seen.clone();
        factr_base::obs_sink::install(move |s| sink.lock().unwrap().push(s));
        let (mut n, _d) = gate_nudge(&[("Cargo.toml", "")]);
        n.observe("edit", &json!({"file_path": "a.rs"}), 0);
        let run = |_: &str, _: &std::path::Path, _: std::time::Duration| GateResult {
            passed: true,
            exit_code: 0,
            output: "test result: ok. 3 passed; 0 failed; 2 ignored;".into(),
        };
        assert!(!n.on_text_only_stop_with("skip-span", "Done.", &run));
        assert!(n.take_pending().is_none(), "recorded, never nudged");
        let seen = seen.lock().unwrap();
        let span = seen.iter().find(|s| s.session_id.as_deref() == Some("skip-span") && s.attributes["reason"] == "auto_verify_pass").unwrap();
        assert_eq!(span.attributes["skipped"], 2);
    }

    #[test]
    fn failure_hash_ignores_timings() {
        assert_eq!(hash_str("1 failed in 0.02s\nok (0.00s)"), hash_str("1 failed in 0.03s\nok (0.01s)"));
        assert_ne!(hash_str("1 failed in 0.02s"), hash_str("2 failed in 0.02s"));
    }

    #[test]
    fn verify_nudge_once_after_edit_without_tests() {
        let mut n = StopNudge::default();
        n.observe("edit", &json!({}), 0);
        assert!(n.on_text_only_stop("s", "Done."));
        assert_eq!(n.take_pending(), Some(VERIFY_NUDGE.to_string()));
        assert!(!n.on_text_only_stop("s", "Done."));
    }

    #[test]
    fn no_nudge_when_tests_ran_or_no_edits() {
        let mut n = StopNudge::default();
        n.observe("write", &json!({}), 0);
        n.observe("bash", &json!({"command": "cd x && cargo test -p foo"}), 0);
        assert!(!n.on_text_only_stop("s", "Done."));
        let mut n = StopNudge::default();
        n.observe("read", &json!({}), 0);
        assert!(!n.on_text_only_stop("s", "All good."));
        let mut n = StopNudge::default();
        n.observe("edit", &json!({}), 1);
        assert!(!n.on_text_only_stop("s", "failed."));
    }

    #[test]
    fn action_nudge_needs_prior_tools_and_trailing_promise() {
        let mut n = StopNudge::default();
        assert!(!n.on_text_only_stop("s", "Now I'll fix it."));
        n.observe("read", &json!({}), 0);
        assert!(n.on_text_only_stop("s", "Found the bug. Now I'll fix it."));
        assert_eq!(n.take_pending(), Some(ACTION_NUDGE.to_string()));
    }

    #[test]
    fn plan_only_endings_are_caught() {
        for t in [
            "I'll implement the parser, then run the tests.",
            "Next, I'll fix the bug and run the tests",
            "I need to update the config.",
            "Plan:\n1. edit a.py\n2. run the tests",
            "I'll do this:\n1. edit a.py\n2. run the tests.",
            "Found it. First, I'll patch the loader.",
        ] {
            assert!(announces_action(&t.to_ascii_lowercase()), "{t}");
        }
        for t in [
            "Let me know if you want more.",
            "Fixed and tested. Summary: all good.",
            "Should I continue?",
            "I'll implement it.\n\nDone. Anything else?",
        ] {
            assert!(!announces_action(&t.to_ascii_lowercase()), "{t}");
        }
    }

    #[test]
    fn second_nudge_only_after_progress_max_two() {
        let mut n = StopNudge::default();
        n.observe("read", &json!({}), 0);
        assert!(n.on_text_only_stop("s", "I'll fix it."));
        assert!(!n.on_text_only_stop("s", "I'll fix it."), "no progress");
        n.observe("read", &json!({}), 0);
        assert!(n.on_text_only_stop("s", "Next, I'll fix it."));
        n.observe("read", &json!({}), 0);
        assert!(!n.on_text_only_stop("s", "I'll fix it."), "max 2");
    }

    #[test]
    fn headless_question_ending_nudged_once_interactive_not() {
        let mut n = StopNudge::default();
        n.headless = true;
        n.observe("bash", &json!({"command": "ls"}), 0);
        assert!(n.on_text_only_stop("s", "I can't tell.\n\nWhat operating system are you using?"));
        assert_eq!(n.take_pending(), Some(QUESTION_NUDGE.to_string()));
        assert!(!n.on_text_only_stop("s", "Would you like me to?"), "no progress since");
        let mut n = StopNudge::default();
        n.observe("bash", &json!({"command": "ls"}), 0);
        assert!(!n.on_text_only_stop("s", "What operating system are you using?"));
        let mut n = StopNudge::default();
        n.headless = true;
        assert!(n.on_text_only_stop("s", "I can run that fix if you want."));
        let mut n = StopNudge::default();
        n.headless = true;
        assert!(!n.on_text_only_stop("s", "Done. Let me know if you need anything else."));
    }

    #[test]
    fn failing_last_check_nudged_unless_fixed_or_passed() {
        let mut n = StopNudge::default();
        n.observe("bash", &json!({"command": "python3 check.py"}), 1);
        assert!(n.on_text_only_stop("s", "Done."));
        assert_eq!(n.take_pending(), Some(FAILED_CHECK_NUDGE.to_string()));
        let mut n = StopNudge::default();
        n.observe("bash", &json!({"command": "python3 check.py"}), 1);
        n.observe("write", &json!({"file_path": "out.txt"}), 0);
        assert!(!n.on_text_only_stop("s", "Done."));
        let mut n = StopNudge::default();
        n.observe("bash", &json!({"command": "python3 check.py"}), 1);
        n.observe("bash", &json!({"command": "python3 check.py"}), 0);
        assert!(!n.on_text_only_stop("s", "Done."));
    }

    #[test]
    fn let_me_know_is_not_a_promise() {
        let mut n = StopNudge::default();
        n.observe("read", &json!({}), 0);
        assert!(!n.on_text_only_stop("s", "All fixed. Let me know if you need anything else."));
        assert!(!n.on_text_only_stop("s", "I think it works, so I'll stop here."));
    }

    #[test]
    fn docs_edits_do_not_need_tests_and_new_runners_count() {
        let mut n = StopNudge::default();
        n.observe("edit", &json!({"file_path": "README.md"}), 0);
        n.observe("write", &json!({"file_path": "a.json"}), 0);
        assert!(!n.on_text_only_stop("s", "Done."));
        let mut n = StopNudge::default();
        n.observe("edit", &json!({"file_path": "src/a.ts"}), 0);
        n.observe("bash", &json!({"command": "bun test"}), 0);
        assert!(!n.on_text_only_stop("s", "Done."));
    }

    use factr_learn::agent_loop::GateResult;
    use std::cell::RefCell;

    fn gate_nudge(files: &[(&str, &str)]) -> (StopNudge, std::path::PathBuf) {
        let d = std::env::temp_dir().join(format!("sn-{}-{}", std::process::id(), std::thread::current().name().unwrap_or("t").replace("::", "_")));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        for (f, c) in files {
            std::fs::write(d.join(f), c).unwrap();
        }
        let mut n = StopNudge::default();
        n.gate_ok = true;
        n.cwd = Some(d.clone());
        (n, d)
    }

    fn fail(out: &str) -> GateResult {
        GateResult { passed: false, exit_code: 1, output: out.into() }
    }
    const OK: fn() -> GateResult = || GateResult { passed: true, exit_code: 0, output: String::new() };

    #[test]
    fn gate_feeds_back_failure_then_ends_on_pass() {
        let (mut n, _d) = gate_nudge(&[("test_a.py", "")]);
        n.observe("edit", &json!({"file_path": "a.py"}), 0);
        let results = RefCell::new(vec![OK(), fail("FAIL: test_x")]);
        let run = |_: &str, _: &std::path::Path, _: std::time::Duration| results.borrow_mut().pop().unwrap();
        assert!(n.on_text_only_stop_with("s", "Done.", &run));
        let msg = n.take_pending().unwrap();
        assert!(msg.contains("FAIL: test_x") && msg.contains("round 1/3"));
        n.observe("edit", &json!({"file_path": "a.py"}), 0);
        assert!(!n.on_text_only_stop_with("s", "Fixed.", &run));
        assert!(n.take_pending().is_none(), "gate replaces the verify nudge");
        assert!(!n.on_text_only_stop_with("s", "Fixed.", &run));
        assert!(results.borrow().is_empty());
    }

    #[test]
    fn steer_when_failure_count_does_not_improve() {
        let run_with = |outs: Vec<&'static str>| {
            let (mut n, _d) = gate_nudge(&[("Cargo.toml", "")]);
            n.observe("edit", &json!({"file_path": "a.rs"}), 0);
            let outs = RefCell::new(outs);
            let run = |_: &str, _: &std::path::Path, _: std::time::Duration| fail(outs.borrow_mut().remove(0));
            assert!(n.on_text_only_stop_with("s", "x", &run));
            let first = n.take_pending().unwrap();
            n.observe("edit", &json!({"file_path": "a.rs"}), 0);
            assert!(n.on_text_only_stop_with("s", "x", &run));
            (first, n.take_pending().unwrap())
        };
        let (a, b) = run_with(vec!["test result: FAILED. 0 passed; 2 failed; in 0.1s", "x\ntest result: FAILED. 0 passed; 2 failed;"]);
        assert!(!a.contains("Two attempts") && b.contains("Two attempts"));
        let (_, b) = run_with(vec!["3 failed in 1s", "1 failed in 1s, other"]);
        assert!(!b.contains("Two attempts"));
    }

    #[test]
    fn gate_stops_on_identical_failure_and_after_max_rounds() {
        let (mut n, _d) = gate_nudge(&[("Cargo.toml", "")]);
        n.observe("edit", &json!({"file_path": "a.rs"}), 0);
        let calls = RefCell::new(0);
        let run = |_: &str, _: &std::path::Path, _: std::time::Duration| {
            *calls.borrow_mut() += 1;
            fail("same")
        };
        assert!(n.on_text_only_stop_with("s", "x", &run));
        assert!(n.on_text_only_stop_with("s", "x", &run) == false);
        assert_eq!(*calls.borrow(), 2);
        assert!(!n.on_text_only_stop_with("s", "x", &run));
        assert_eq!(*calls.borrow(), 2, "no more runs after the identical-failure stop");

        let (mut n, _d) = gate_nudge(&[("Cargo.toml", "")]);
        n.observe("edit", &json!({"file_path": "a.rs"}), 0);
        let k = RefCell::new(0);
        let run = |_: &str, _: &std::path::Path, _: std::time::Duration| {
            *k.borrow_mut() += 1;
            fail(&format!("different {}", k.borrow()))
        };
        for _ in 0..3 {
            assert!(n.on_text_only_stop_with("s", "x", &run));
        }
        assert!(!n.on_text_only_stop_with("s", "x", &run));
        assert_eq!(*k.borrow(), 3);
    }

    #[test]
    fn no_marker_falls_back_to_nudge() {
        let (mut n, _d) = gate_nudge(&[("main.py", "")]);
        n.observe("edit", &json!({"file_path": "a.py"}), 0);
        let run = |_: &str, _: &std::path::Path, _: std::time::Duration| -> GateResult { panic!("ran") };
        assert!(n.on_text_only_stop_with("s", "Done.", &run));
        assert_eq!(n.take_pending(), Some(VERIFY_NUDGE.to_string()));
    }

    #[test]
    fn exit_code_aware_skip_and_edit_resets_tested() {
        let (mut n, _d) = gate_nudge(&[("Cargo.toml", "")]);
        let calls = RefCell::new(0);
        let run = |_: &str, _: &std::path::Path, _: std::time::Duration| {
            *calls.borrow_mut() += 1;
            OK()
        };
        n.observe("edit", &json!({"file_path": "a.rs"}), 0);
        n.observe("bash", &json!({"command": "cargo test"}), 0);
        assert!(!n.on_text_only_stop_with("s", "Done.", &run));
        assert_eq!(*calls.borrow(), 0, "green test after last edit: skip");
        // A later edit makes the earlier run stale.
        n.observe("edit", &json!({"file_path": "a.rs"}), 0);
        n.on_text_only_stop_with("s", "Done.", &run);
        assert_eq!(*calls.borrow(), 1);
        // A failing test run after the edit does not skip the gate.
        let (mut n, _d) = gate_nudge(&[("Cargo.toml", "")]);
        n.observe("edit", &json!({"file_path": "a.rs"}), 0);
        n.observe("bash", &json!({"command": "cargo test"}), 101);
        n.on_text_only_stop_with("s", "Done.", &run);
        assert_eq!(*calls.borrow(), 2);
    }

    #[test]
    fn auto_verify_defaults_to_headless_and_env_overrides() {
        assert!(auto_verify_gate(None, true, true));
        assert!(!auto_verify_gate(None, false, true));
        assert!(!auto_verify_gate(None, true, false));
        assert!(!auto_verify_gate(Some("0"), true, true));
        assert!(auto_verify_gate(Some("1"), false, true));
        assert!(auto_verify_gate(Some("1"), false, false));
    }

    #[test]
    fn tests_touched_flag() {
        let mut n = StopNudge::default();
        n.observe("edit", &json!({"file_path": "src/a.rs"}), 0);
        assert!(!n.tests_touched);
        n.observe("edit", &json!({"file_path": "tests/a.rs"}), 0);
        assert!(n.tests_touched);
    }
}
