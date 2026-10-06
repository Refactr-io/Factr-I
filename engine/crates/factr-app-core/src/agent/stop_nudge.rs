//! Stop nudges when a turn ends with no tool calls.
//!
//! Ported from Factr `agent/turn_stop_gates.py` + `verification_stop.py`
//! (nudge after code edits unless tests ran) and its tool-use-enforcement path
//! (a text-only turn that announces an action). The test-command detection is
//! the idea of `coding_context.py`. At most two nudges per turn (the second only after tool calls in between), and never in a
//! turn without edits (verify) or without prior tool use (action).

pub(super) const EDIT_TOOLS: [&str; 4] = ["write", "edit", "apply_patch", "replace"];
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

const VERIFY_NUDGE: &str = "<system-reminder>Run or read back what you produced and check it against each stated requirement, then finish.</system-reminder>";
const STEER: &str = "Two attempts have not fixed this. Step back: read the failing output again, write down two different hypotheses for the cause, test the most likely one with a quick command before editing again, and do not repeat an edit you already tried.";
const NO_TOOLS_QUESTION_NUDGE: &str = "<system-reminder>No user is available to answer. Give your answer now, without tools.</system-reminder>";
const QUESTION_NUDGE: &str = "<system-reminder>No user is available to answer. Act on the most likely reading of the task now, do the work, and verify it.</system-reminder>";
const FAILED_CHECK_NUDGE: &str = "<system-reminder>Your last check failed and nothing changed since. Fix it or explain precisely what blocks you, and still write your best result.</system-reminder>";
const OFFER_PHRASES: [&str; 9] = [
    "i can run", "i can do", "if you want", "if you'd like", "would you like", "do you want me to",
    "shall i", "should i", "let me know if you want me to",
];
const SNIPPET_NUDGE: &str = "<system-reminder>Your answer rests on search result snippets alone. Open the source behind what it depends on and check it there.</system-reminder>";
// Shortfall detector: a negation or shortfall word and an object word in one sentence of the last paragraph.
const SHORTFALL_NEG: [&str; 14] = [
    "could not", "couldn't", "unable", "cannot", "can't", "not yet", "below", "short of", "fails to", "failed to",
    "not verified", "not re-tested", "not retested", "did not reach",
];
const OBJECT_WORDS: [&str; 20] = [
    "target", "threshold", "requirement", "test", "verify", "score", "expected", "determine", "find", "answer",
    "calculate", "compute", "solve", "complete", "value", "result", "check", "access", "install", "reach",
];
// Self-sufficient shortfall phrases (negation and object in one).
const SHORTFALL_PHRASES: [&str; 9] = [
    "not possible", "not feasible", "not installed", "impossible to", "insufficient information",
    "not enough information", "no way to", "cannot be determined", "cannot be verified",
];
// A decline that names an unreadable input is an honest limitation, not a give-up.
const UNREADABLE_PHRASES: [&str; 6] = ["unreadable", "inaccessible", "cannot read", "can't read", "could not read", "couldn't read"];
// Only an exemption when the paragraph also names a file that exists in the cwd.
const UNREADABLE_IF_FILE: [&str; 4] = ["unable to read", "cannot open", "can't open", "unable to access"];
const DECLINE_NUDGE: &str = "<system-reminder>List what you tried, then try two different routes (a reformulated query, a primary source, another site, another tool or method). If a step produced an impossible or ambiguous intermediate value, re-derive the step that fed it. If the blocker is an input you cannot read, say so plainly. Then give your best answer in the requested form.</system-reminder>";
const DECLINE_NUDGE_V1: &str = "<system-reminder>List what you tried, then try two different routes (a reformulated query, a primary source, another site, another tool or method). If the blocker is an input you cannot read, say so plainly.</system-reminder>";
const DECLINE_MANY_ROUTES_NUDGE: &str = "<system-reminder>You have already tried several routes. Re-read the question and what your sources actually say, then give your answer.</system-reminder>";
const DECLINE_NO_TOOLS_NUDGE: &str = "<system-reminder>Give your best answer in the requested form now, without tools.</system-reminder>";
const DECLINE_NO_TOOLS_NUDGE_V1: &str = "<system-reminder>If the blocker is an input you cannot read, say so plainly.</system-reminder>";
const ITERATE_NUDGE: &str = "<system-reminder>You reported a shortfall with most of the budget left. Try a different method, or install or implement the missing piece, re-run the check, then finish.</system-reminder>";
const COMPUTE_GUARD_NUDGE: &str = "<system-reminder>Your result is a number and no code has run on the data since you read it. Recompute it from the data with the repl or bash.</system-reminder>";
const SKIPPED_NUDGE: &str = "<system-reminder>The tests passed, but some were skipped or ignored. If the task's requirements cover them, enable them (remove just the skip marker, never delete or weaken a test), run the full suite and fix failures. Otherwise finish.</system-reminder>";
const NO_RUNNER_NOTE: &str = "The test runner is not available here. Fall back to python3 -m unittest, node --test, or compile and run the test file directly.";
/// The v0.0.1 phrase list, kept for the `FACTR_GUARD_NOTOOLS=0` ablation: no compute guard or tool advice then.
const TOOL_FORBIDS: [&str; 14] = [
    "without tools", "without using tools", "do not use tools", "don't use tools", "no tools", "without any tools",
    "do not use code", "don't use code", "without code", "without using code", "no code", "by hand", "in your head",
    "without a calculator",
];
/// Output markers of a run in which no test executed: such a run does not count as an exec.
const ZERO_TESTS: [&str; 5] = ["ran 0 tests", "no tests ran", "collected 0 items", "running 0 tests", "0 passing"];
const UNVERIFIED_NUDGE: &str = "<system-reminder>Your todo list reports verified work but no tool call inspected the inputs or checked the result. Look at the actual data and check the result with a tool, then finish.</system-reminder>";
const MEASURE_NUDGE: &str = "<system-reminder>State what you measured and why it equals what was asked; check its size against the record count. If it does not, work out the real quantity from the data and answer with that.</system-reminder>";
/// A top-level cwd file this large (bytes) counts as a large input; the system prompt's ~20K rule.
const LARGE_INPUT_BYTES: u64 = 20_000;
const ACTION_NUDGE: &str = "<system-reminder>Continue by calling the tool now, or give your result.</system-reminder>";
/// A reply this short after an earlier, longer reply is a bare stub, not an answer.
const STUB_CHARS: usize = 40;

/// Guard switches, all default ON: `FACTR_GUARD_<NAME>=0` turns one off without a rebuild.
pub(super) fn guard_on(name: &str) -> bool {
    std::env::var(format!("FACTR_GUARD_{name}")).map_or(true, |v| v != "0")
}

type Runner<'a> = &'a dyn Fn(&str, &std::path::Path, std::time::Duration) -> factr_learn::agent_loop::GateResult;

#[derive(Default)]
pub(super) struct StopNudge {
    edited: bool,
    used_tools: bool,
    sent: u8,
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
    // The task's label, kept after the format nudge clears `format_label`, for the final-text rule.
    task_label: Option<String>,
    format_sent: bool,
    // Non-todo tool calls so far: the second chain nudge needs progress, a todo write is not one.
    progress: u32,
    sent_progress: u32,
    // Replies of this turn that were followed by a nudge (kept for the final-text rule), nudge count.
    replies: Vec<String>,
    nudges_total: u32,
    first_value: Option<String>,
    compute_sent: bool,
    skipped_sent: bool,
    verify_sent: bool,
    shortfall_sent: bool,
    unread_sent: bool,
    // bash or the REPL can run (policy and install), and the task forbids tools.
    pub(super) compute_tool: bool,
    forbids_tools: bool,
    // `FACTR_GUARD_NOTOOLS=0`: the no-tools gating (and the shared detector) are off.
    notools_off: bool,
    cwd_files: Vec<String>,
    // A promoted (timed-out) or background check is still running as this task: not a pass.
    pending_check: Option<String>,
    pending_sent: bool,
    // Distinct search queries and fetched hosts, for the decline nudge.
    queries: std::collections::HashSet<String>,
    hosts: std::collections::HashSet<String>,
    // The tracker: seq of the last data-bearing call (read/webfetch/websearch), the last real exec
    // (bash/REPL run that ran something, or a read of a written path), and written paths.
    last_input: u32,
    last_exec: u32,
    last_compute: u32,
    written: Vec<String>,
    // mtime snapshot of the cwd at turn start (writes made through bash or the REPL).
    snapshot: Option<std::collections::HashMap<std::path::PathBuf, (std::time::SystemTime, u64)>>,
    shell_write_seq: u32,
    // The last webfetch whose output was cut: (url, seen chars, total chars). Cleared by a follow-up.
    cut_page: Option<(String, usize, usize)>,
    snapshot_dir: Option<std::path::PathBuf>,
    cut_spill: Option<String>,
    // The 50% budget stage was reached (iterate nudge window).
    half: bool,
    // Search/fetch tally for the snippet-only-answer nudge (sent once per turn).
    searches: u32,
    fetched: bool,
    snippet_sent: bool,
    // The 70% time-budget reminder was sent (set by the turn loop).
    late: bool,
    // A todo write marked an item completed as `verified`, and whether any non-todo tool ran.
    claims_verified: bool,
    inspected: bool,
    // A large non-code input sits in the cwd (headless runs), and how many non-todo calls ran.
    large_input: bool,
    analysis_calls: u32,
    measure_sent: bool,
    // Background tasks this run started that may be servers, and the ones found exited at a stop.
    watch: Vec<super::bg_guard::Watch>,
    service_exits: Vec<(String, String, String)>,
    service_sent: bool,
    // Task ids already given the stale-process note this turn.
    pub(super) stale_warned: std::collections::HashSet<String>,
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

/// The gate's command could not run at all: exit 127 or a missing-command / missing-module message.
fn no_runner(exit_code: i64, output: &str) -> bool {
    exit_code == 127 || output.contains("command not found") || output.contains("No module named")
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
        n.large_input = n.headless
            && self.session.parent_id.is_none()
            && cwd.as_deref().is_some_and(|d| !is_home(d) && has_large_input(d));
        if self.session.parent_id.is_none() && !self.in_goal() {
            let task = first_task_text(
                self.session
                    .messages
                    .iter()
                    .filter(|m| m.role == factr_message_types::Role::User)
                    .map(|m| m.content.as_slice()),
            );
            n.format_label = task.and_then(required_label);
            n.task_label = n.format_label.clone();
            n.notools_off = !factr_base::task_policy::gating_on();
            n.forbids_tools = task.is_some_and(|t| if n.notools_off { forbids_tools_legacy(t) } else { factr_base::task_policy::forbids_tools(t) });
        }
        let tool_ok = |name: &str| {
            self.allowed_tools.as_ref().is_none_or(|a| crate::tool::tool_name_is_allowed(a, name))
                && !crate::tool::tool_name_is_disabled(&self.disabled_tools, name)
        };
        n.compute_tool = tool_ok("bash") || (tool_ok("repl") && crate::tool::repl_available());
        if let Some(d) = cwd.as_deref().filter(|d| !is_home(d)) {
            n.cwd_files = std::fs::read_dir(d)
                .map(|rd| rd.flatten().map(|e| e.file_name().to_string_lossy().to_ascii_lowercase()).collect())
                .unwrap_or_default();
            if n.headless && self.session.parent_id.is_none() && guard_on("VERIFY") {
                n.snapshot = Some(fs_snapshot(d));
                n.snapshot_dir = Some(d.to_path_buf());
            }
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
        self.observe_full(tool, input, exit_code, None, None);
    }

    /// `observe` with the tool's result metadata and output text: a promoted (timed-out) or background
    /// check is recorded as pending, never as a pass; the output tells a run that ran nothing from one that did.
    pub(super) fn observe_full(&mut self, tool: &str, input: &serde_json::Value, exit_code: i64, meta: Option<&serde_json::Value>, output: Option<&str>) {
        self.used_tools = true;
        self.seq += 1;
        if tool == "todo" {
            self.claims_verified |= claims_verified(input);
        } else {
            self.progress += 1;
            self.inspected = true;
            if tool != "load_tools" {
                self.analysis_calls += 1;
            }
        }
        if exit_code == 0 && EDIT_TOOLS.contains(&tool) {
            self.last_write = self.seq;
            for p in edit_paths(input) {
                self.written.push(p);
            }
            if edits_code(input) {
                self.edited = true;
                self.last_edit = self.seq;
                self.passed = false;
            }
            if edit_paths(input).iter().any(|p| super::auto_verify::is_test_path(p)) {
                self.tests_touched = true;
            }
        }
        // The tracker: inputs read, real runs, and read-backs of what was written.
        if matches!(tool, "read" | "webfetch" | "websearch") {
            self.last_input = self.seq;
        }
        if tool == "read" && input["file_path"].as_str().or(input["path"].as_str()).is_some_and(|p| self.written.iter().any(|w| same_path(w, p))) {
            self.last_exec = self.seq;
        }
        if matches!(tool, "bash" | "repl") {
            let ran_nothing = output.is_some_and(|o| {
                let o = o.to_ascii_lowercase();
                ZERO_TESTS.iter().any(|m| o.contains(m))
            });
            self.last_compute = self.seq;
            if !ran_nothing {
                self.last_exec = self.seq;
            }
            if self.snapshot_changed() {
                self.shell_write_seq = self.seq;
            }
        }
        match tool {
            "websearch" => {
                self.searches += 1;
                if let Some(q) = input["query"].as_str() {
                    self.queries.insert(q.trim().to_ascii_lowercase());
                }
            }
            "webfetch" if exit_code == 0 => {
                self.fetched = true;
                if let Some(h) = input["url"].as_str().and_then(url_host) {
                    self.hosts.insert(h);
                }
                self.observe_fetch(input, output);
            }
            "read" if exit_code == 0 => {
                // A read of the spill path clears the cut-page record.
                if let (Some(sp), Some(p)) = (self.cut_spill.as_deref(), input["file_path"].as_str().or(input["path"].as_str())) {
                    if same_path(sp, p) {
                        self.cut_page = None;
                    }
                }
            }
            "bg" if matches!(input["action"].as_str(), Some("wait" | "output")) => self.pending_check = None,
            _ => {}
        }
        self.observe_bg(tool, input, meta, output);
        let promoted = meta.is_some_and(|m| m["timeout_promoted"] == true || m["background"] == true);
        let command = input["command"].as_str().filter(|_| tool == "bash");
        if command.is_some_and(looks_like_check) && promoted {
            self.pending_check = Some(meta.and_then(|m| m["task_id"].as_str()).unwrap_or("").to_string());
            return;
        }
        if command.is_some_and(looks_like_test_run) {
            self.last_test = self.seq;
            self.last_test_exit = exit_code;
        }
        if command.is_some_and(looks_like_check) {
            self.last_check = self.seq;
            self.last_check_exit = exit_code;
        }
    }

    /// Track background tasks that may be servers (see `bg_guard`).
    fn observe_bg(&mut self, tool: &str, input: &serde_json::Value, meta: Option<&serde_json::Value>, output: Option<&str>) {
        if tool == "bash" {
            let Some(id) = meta.filter(|m| m["background"] == true).and_then(|m| m["task_id"].as_str()) else { return };
            let command = input["command"].as_str().unwrap_or("");
            if meta.is_some_and(|m| m["timeout_promoted"] == true) && looks_like_check(command) {
                return;
            }
            let name = meta.and_then(|m| m["display_name"].as_str()).map(str::to_string).unwrap_or_else(|| command.chars().take(40).collect());
            self.watch.retain(|w| w.name != name);
            self.watch.push(super::bg_guard::Watch { id: id.to_string(), name, flagged: false });
        } else if tool == "bg" {
            let Some(id) = input["task_id"].as_str() else { return };
            let action = input["action"].as_str().unwrap_or("");
            let until = input["until"].as_str().is_some_and(|u| !u.is_empty());
            if action == "cancel" {
                self.watch.retain(|w| w.id != id);
                return;
            }
            let Some(i) = self.watch.iter().position(|w| w.id == id) else { return };
            if matches!(action, "wait" | "output" | "tail" | "status") {
                if (action == "wait" && until) || output.is_some_and(super::bg_guard::looks_ready) {
                    self.watch[i].flagged = true;
                } else if !self.watch[i].flagged {
                    // The model has seen how this task ended: nothing left to recheck.
                    self.watch.remove(i);
                }
            }
        }
    }

    pub(super) fn watched(&self) -> &[super::bg_guard::Watch] {
        &self.watch
    }

    /// Watched tasks found not running at this stop (queried by the turn loop before it asks).
    pub(super) fn set_service_exits(&mut self, exits: Vec<(String, String, String)>) {
        self.service_exits = exits;
    }

    /// Record or clear the cut-page state from a successful webfetch: its own window/offset note
    /// says whether the model saw all of the text.
    fn observe_fetch(&mut self, input: &serde_json::Value, output: Option<&str>) {
        let url = input["url"].as_str().unwrap_or("").to_string();
        let followup = input["find"].as_str().is_some_and(|f| !f.is_empty()) || input["offset"].is_number();
        let Some(out) = output else { return };
        match parse_fetch_cut(out) {
            Some((seen, total, spill)) if !followup => {
                self.cut_page = Some((url, seen, total));
                self.cut_spill = spill;
            }
            _ => {
                // A follow-up on the cut URL, or a fetch of another page, ends the obligation.
                if followup && self.cut_page.as_ref().is_some_and(|c| c.0 == url) || !followup && self.cut_page.as_ref().is_some_and(|c| c.0 != url) {
                    self.cut_page = None;
                    self.cut_spill = None;
                }
            }
        }
    }

    /// A deliverable was written (an edit tool, or a file changed in the cwd by bash or the REPL)
    /// and nothing ran or was read back after it.
    fn unexecuted_write(&mut self) -> bool {
        if self.shell_write_seq == 0 && self.last_compute > 0 && self.snapshot_changed() {
            self.shell_write_seq = self.last_compute;
        }
        let w = self.last_write.max(self.shell_write_seq);
        // Strictly after: a run that is itself the write (or came before it) does not exercise it.
        w > 0 && self.last_exec <= w
    }

    /// The cwd differs from the rolling snapshot in a file the edit tools did not write (those are
    /// tracked by seq); the snapshot is refreshed, so each shell write is seen once.
    fn snapshot_changed(&mut self) -> bool {
        let (Some(before), Some(dir)) = (self.snapshot.as_ref(), self.cwd_dir()) else { return false };
        let now = fs_snapshot(&dir);
        let written = &self.written;
        let changed = now
            .iter()
            .filter(|(p, v)| before.get(*p) != Some(*v))
            .chain(before.iter().filter(|(p, _)| !now.contains_key(*p)))
            .any(|(p, _)| !written.iter().any(|w| same_path(w, &p.to_string_lossy())));
        self.snapshot = Some(now);
        changed
    }

    fn cwd_dir(&self) -> Option<std::path::PathBuf> {
        self.cwd.clone().or_else(|| self.snapshot_dir.clone())
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
        let limit = std::time::Duration::from_secs(if slow_build { base.saturating_mul(3) } else { base });
        let mut r = run(&cmd, &dir, limit);
        let mut ran = cmd.clone();
        let mut ms = started.elapsed().as_millis() as u64;
        let mut skipped = if r.passed { super::auto_verify::skipped_count(&r.output) } else { 0 };
        let mut ignored_failed = false;
        let skip_follow = guard_on("SKIPPED");
        // A pass that skipped tests: cargo re-runs once including the ignored tests (no file edits).
        if skip_follow && r.passed && skipped > 0 && self.headless && name == "cargo" && !self.skipped_sent {
            self.skipped_sent = true;
            let again = format!("{cmd} -- --include-ignored");
            let second = run(&again, &dir, limit);
            ms = started.elapsed().as_millis() as u64;
            if second.passed {
                skipped = 0;
            } else {
                ignored_failed = true;
                ran = again;
            }
            r = second;
        }
        let reason = match (r.passed, r.exit_code) {
            (true, _) => "auto_verify_pass",
            (_, 124) => "auto_verify_timeout",
            _ => "auto_verify_fail",
        };
        let span = self.guard_span(session_id, reason, name, ms);
        // A pass that skipped tests is recorded.
        factr_base::obs_sink::emit(if r.passed { span.attr("skipped", skipped) } else { span });
        if r.passed {
            // The pass is now the latest test run; later edits re-arm the gate.
            self.seq += 1;
            self.last_test = self.seq;
            self.last_test_exit = 0;
            self.last_exec = self.seq;
            self.passed = true;
            if skip_follow && skipped > 0 && self.headless && !self.skipped_sent {
                // Other runners cannot include ignored tests natively: one message, then finish.
                self.skipped_sent = true;
                self.pending = Some(SKIPPED_NUDGE.to_string());
                self.nudges_total += 1;
                return Some(true);
            }
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
        let fallback = if no_runner(r.exit_code, &r.output) { format!("\n\n{NO_RUNNER_NOTE}") } else { String::new() };
        let ignored = if ignored_failed { "\n\nThese failures are in tests marked ignored: fix them if the task's requirements cover them, otherwise finish." } else { "" };
        self.pending = Some(format!(
            "<system-reminder>Auto-verify: `{ran}` failed (exit {}, round {}/{}). Last output:\n{}\nFix the failures, then finish.{}{ignored}{fallback}</system-reminder>",
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
        let lower = text.to_ascii_lowercase();
        // The task forbids tools: no nudge may tell the model to run, read, verify or call anything.
        let no_tools = self.no_tools();
        // The chain: max 2 nudges per turn; the second only after non-todo tool calls made progress.
        let chain_open = enabled() && self.sent < 2 && !(self.sent == 1 && self.progress == self.sent_progress);
        let mut picked: Option<(String, &str)> = None;
        let mut counts = true;
        let format_missing = self.format_label.as_deref().filter(|l| !has_label_line(text, l) && !text.trim().is_empty()).map(str::to_string);
        if chain_open {
            let routes = (self.queries.len() + self.hosts.len()) as u32;
            let fresh = self.headless && !self.late && !text.trim().is_empty();
            let wrote = self.last_write > 0 || self.shell_write_seq > 0;
            picked = if !no_tools && fresh && guard_on("VERIFY") && !self.verify_sent && !self.gate_ran && self.unexecuted_write() {
                self.verify_sent = true;
                Some((VERIFY_NUDGE.into(), "verify_nudge"))
            } else if !no_tools && !self.gate_ran && self.last_check > self.last_write && self.last_check_exit != 0 {
                Some((FAILED_CHECK_NUDGE.into(), "failed_check_nudge"))
            } else if !no_tools && guard_on("PENDING") && !self.passed && !self.pending_sent && self.pending_check.is_some() {
                self.pending_sent = true;
                let id = self.pending_check.clone().unwrap_or_default();
                let task = if id.is_empty() { "a background task".to_string() } else { format!("task {id}") };
                Some((format!("<system-reminder>Your check is still running as {task}: `bg` wait on it, then act on its result.</system-reminder>"), "pending_check_nudge"))
            } else if !no_tools && fresh && guard_on("SERVICE") && !self.service_sent && !self.late && !self.service_exits.is_empty() {
                self.service_sent = true;
                let (id, name, code) = self.service_exits[0].clone();
                Some((super::bg_guard::service_nudge(&id, &name, &code), "service_nudge"))
            } else if !no_tools && self.claims_verified && !self.inspected && !self.edited {
                self.claims_verified = false; // one per turn
                Some((UNVERIFIED_NUDGE.into(), "unverified_claim_nudge"))
            } else if !no_tools && self.headless && self.large_input && !self.edited && !self.measure_sent && self.analysis_calls <= 1 && !text.trim().is_empty() && !announces_action(&lower) {
                self.measure_sent = true;
                Some((MEASURE_NUDGE.into(), "measure_nudge"))
            } else if !no_tools && self.used_tools && announces_action(&lower) {
                Some((ACTION_NUDGE.into(), "action_nudge"))
            } else if self.headless && !self.edited && asks_or_offers(&lower) {
                Some((if no_tools { NO_TOOLS_QUESTION_NUDGE } else { QUESTION_NUDGE }.into(), "question_nudge"))
            } else if !no_tools && self.headless && !self.edited && self.snippet_only() {
                self.snippet_sent = true;
                Some((SNIPPET_NUDGE.into(), "snippet_nudge"))
            } else if self.headless && !self.half && !self.late && guard_on("SHORTFALL") && !self.shortfall_sent && !(self.last_test > self.last_write && self.last_test_exit == 0) && shortfall(text, &self.cwd_files) {
                self.shortfall_sent = true;
                let tpl = guard_on("TEMPLATE");
                Some(if self.forbids_tools {
                    ((if tpl { DECLINE_NO_TOOLS_NUDGE } else { DECLINE_NO_TOOLS_NUDGE_V1 }).into(), "shortfall_nudge")
                } else if wrote {
                    (ITERATE_NUDGE.into(), "shortfall_nudge")
                } else if routes < 2 {
                    ((if tpl { DECLINE_NUDGE } else { DECLINE_NUDGE_V1 }).into(), "decline_nudge")
                } else {
                    (DECLINE_MANY_ROUTES_NUDGE.into(), "decline_many_routes_nudge")
                })
            } else if !no_tools && fresh && self.compute_tool && !self.forbids_tools && guard_on("COMPUTE") && !self.compute_sent && self.compute_guard_applies(text) {
                self.compute_sent = true;
                Some((COMPUTE_GUARD_NUDGE.into(), "compute_guard"))
            } else if !no_tools && fresh && !self.edited && guard_on("UNREAD") && !self.unread_sent && self.cut_page.is_some() && !shortfall(text, &self.cwd_files) {
                self.unread_sent = true;
                self.cut_page.as_ref().map(|(url, seen, total)| {
                    let msg = if url.starts_with("http") {
                        format!("<system-reminder>The page you relied on was cut: you saw {seen} of {total} chars of {url}. Search it with webfetch find=<term> or read the rest before finishing.</system-reminder>")
                    } else {
                        format!("<system-reminder>The file you relied on was cut: you saw {seen} of {total} chars of {url}. Process it in code or read the rest before finishing.</system-reminder>")
                    };
                    (msg, "unread_remainder_nudge")
                })
            } else {
                None
            };
        }
        // The format nudge has its own once-per-turn allowance, outside the cap.
        if picked.is_none() && enabled() && !self.format_sent {
            if let Some(l) = format_missing {
                self.format_sent = true;
                self.format_label = None;
                counts = false;
                picked = Some((format_nudge(&l), "format_nudge"));
            }
        }
        if let Some((nudge, reason)) = picked {
            if counts {
                self.sent += 1;
                self.sent_progress = self.progress;
            }
            self.note_reply(text);
            self.queue(session_id, nudge, reason);
            return true;
        }
        false
    }

    /// The task forbids tools or code and the no-tools gating is on.
    pub(super) fn no_tools(&self) -> bool {
        self.forbids_tools && !self.notools_off
    }

    fn queue(&mut self, session_id: &str, nudge: String, reason: &str) {
        self.nudges_total += 1;
        self.pending = Some(nudge);
        factr_base::obs_sink::emit(
            factr_base::obs_sink::Span::new("loop.guard")
                .session(session_id)
                .attr("reason", reason)
                .attr("tests_touched", self.tests_touched),
        );
    }

    /// Remember a reply that is about to be followed by a nudge, and the answer value it carried.
    fn note_reply(&mut self, text: &str) {
        if !text.trim().is_empty() {
            self.replies.push(text.to_string());
        }
        if self.first_value.is_none() {
            self.first_value = answer_value(text, self.task_label.as_deref());
        }
    }

    /// The compute-before-final guard: the reply's answer is a bare number and no code ran on the
    /// data since it was read (or no tool ran at all).
    fn compute_guard_applies(&self, text: &str) -> bool {
        let Some(v) = answer_value(text, self.task_label.as_deref()) else { return false };
        if !is_computed_number(&v) {
            return false;
        }
        self.progress == 0 || (self.last_input > 0 && self.last_compute <= self.last_input)
    }

    /// The text the host receives as the turn's answer, whole and never rewritten: with a task label,
    /// the latest reply that has a label line; otherwise the last reply, falling back to earlier replies
    /// only when the last is empty or a bare stub.
    pub(super) fn final_text(&self, last: &str) -> String {
        if !guard_on("FINAL_TEXT") {
            return last.to_string();
        }
        let tpl = guard_on("TEMPLATE");
        let mut all: Vec<&str> = self.replies.iter().map(String::as_str).collect();
        all.push(last);
        if let Some(l) = self.task_label.as_deref() {
            if let Some(i) = all.iter().rposition(|r| has_label_line(r, l)) {
                // A stub carrying the label (possibly a wrongly detected one) never displaces an earlier
                // reply that ends in a template line of its own.
                if tpl && all[i].trim().chars().count() <= STUB_CHARS {
                    if let Some(e) = all[..i].iter().rev().find(|r| ends_with_template_line(r)) {
                        return (*e).to_string();
                    }
                }
                return all[i].to_string();
            }
        }
        if tpl && last.trim().chars().count() > STUB_CHARS && !ends_with_template_line(last) {
            if let Some(e) = self.replies.iter().rev().find(|r| ends_with_template_line(r)) {
                return e.clone();
            }
        }
        if last.trim().chars().count() > STUB_CHARS {
            return last.to_string();
        }
        // A bare value or a short decline is an answer of its own, not a confirmation.
        let own = is_computed_number(last.trim()) || shortfall(last, &[]) || last.contains('?');
        if last.trim().is_empty() || !own {
            if let Some(r) = self.replies.iter().rev().find(|r| !r.trim().is_empty()) {
                return r.clone();
            }
        }
        last.to_string()
    }

    /// `loop.guard reason=turn_end`: free instrumentation of how the turn ended.
    pub(super) fn emit_turn_end(&self, session_id: &str, final_text: &str, stop_reason: Option<&str>) {
        let mut span = factr_base::obs_sink::Span::new("loop.guard")
            .session(session_id)
            .attr("reason", "turn_end")
            .attr("nudges_sent", self.nudges_total)
            .attr("used_tools", self.used_tools)
            .attr("final_len", final_text.trim().chars().count())
            .attr("stop_reason", stop_reason.unwrap_or("none"));
        if let Some(before) = &self.first_value {
            let after = answer_value(final_text, self.task_label.as_deref());
            span = span.attr("answer_changed", after.as_deref() != Some(before.as_str()));
        }
        if guard_on("ANSWER_SHAPE") {
            span = span.attr("answer_shape", answer_shape(final_text, self.task_label.as_deref()));
        }
        factr_base::obs_sink::emit(span);
    }

    /// The turn loop reports whether the 70% deadline stage has been reached.
    pub(super) fn set_late(&mut self, late: bool) {
        self.late = late;
    }

    /// The turn loop reports whether the 50% stage has been reached (the iterate nudge window closes).
    pub(super) fn set_half(&mut self, half: bool) {
        self.half = half;
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

/// Literal label (all caps like `RESULT:` or Title-case like `Result:`) the task
/// says the reply must end with / contain / start with: quoted, or the first
/// unquoted label before a `[`/`<` placeholder, within a short window after a
/// formatting verb. Title-case words that merely introduce prose (`Note:`,
/// `Example:`) never count.
fn required_label(task: &str) -> Option<String> {
    if !guard_on("TEMPLATE") {
        return required_label_legacy(task);
    }
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| {
        regex::Regex::new(r#"(["'`\u{2018}\u{201C}])?\b([A-Z]{2,}(?: [A-Z]{2,}){0,3}|[A-Z][a-z]+(?: [A-Z][a-z]+){0,2}|[A-Za-z_][A-Za-z0-9_-]*)(\s*[:=])(\s*[\[<{][A-Za-z])?"#).unwrap()
    });
    for c in re.captures_iter(task) {
        let (Some(m), Some(d)) = (c.get(2), c.get(3)) else { continue };
        let delim = d.as_str().trim();
        // `==`, `=>`, `::` are operators, not a label delimiter.
        if task[d.end()..].starts_with(['=', '>']) || (delim == ":" && task[d.end()..].starts_with(':')) {
            continue;
        }
        if PROSE_LABELS.iter().any(|p| p.eq_ignore_ascii_case(&format!("{}:", m.as_str()))) {
            continue;
        }
        let cased = m.as_str().chars().next().is_some_and(char::is_uppercase);
        let quoted = c.get(1).is_some() && delim == ":" && cased;
        if c.get(4).is_none() && !quoted {
            continue;
        }
        let start = task[..m.start()].char_indices().rev().nth(120).map_or(0, |(i, _)| i);
        if mentions_format_verb(&task[start..m.start()]) {
            return Some(if delim == "=" { format!("{} =", m.as_str()) } else { format!("{}:", m.as_str()) });
        }
    }
    None
}

const PROSE_LABELS: [&str; 14] = ["Note:", "Example:", "Examples:", "Hint:", "Tip:", "Warning:", "Context:", "Question:", "Task:", "Step:", "Important:", "Remember:", "Format:", "Background:"];

/// A formatting verb as a whole word (`end`, `ends`, `ending`; never `endpoints`).
fn mentions_format_verb(before: &str) -> bool {
    const VERBS: [&str; 9] = ["end", "finish", "conclude", "report", "respond", "reply", "answer", "template", "format"];
    before.to_ascii_lowercase().split(|c: char| !c.is_ascii_alphabetic()).any(|w| {
        w.starts_with("finali")
            || VERBS.iter().any(|v| w.strip_prefix(v).is_some_and(|rest| ["", "s", "es", "ed", "d", "ing", "ted"].contains(&rest)))
    })
}

/// The reply's last non-empty line is a short `label: value` or `label = value` line (markdown ignored).
fn ends_with_template_line(text: &str) -> bool {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| regex::Regex::new(r"^[A-Za-z_][A-Za-z0-9_-]*(?: [A-Za-z][A-Za-z0-9_-]*){0,3}\s*[:=]\s*\S").unwrap());
    let Some(line) = text.lines().rev().find(|l| !l.trim().is_empty()) else { return false };
    let t = line.trim_start_matches(|c: char| c.is_whitespace() || matches!(c, '*' | '`' | '_' | '#' | '>' | '-'));
    t.chars().count() <= 60 && re.is_match(t) && !PROSE_LABELS.iter().any(|p| t.starts_with(p))
}

fn required_label_legacy(task: &str) -> Option<String> {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| regex::Regex::new(r#"(["'`\u{2018}\u{201C}])?\b([A-Z]{2,}(?: [A-Z]{2,}){0,3}:|[A-Z][a-z]+(?: [A-Z][a-z]+){0,2}:)(\s*[\[<])?"#).unwrap());
    const PROSE: [&str; 14] = ["Note:", "Example:", "Examples:", "Hint:", "Tip:", "Warning:", "Context:", "Question:", "Task:", "Step:", "Important:", "Remember:", "Format:", "Background:"];
    const VERBS: [&str; 10] = ["end", "finish", "conclude", "finali", "report", "respond", "reply", "answer", "template", "format"];
    for c in re.captures_iter(task) {
        let m = c.get(2)?;
        if (c.get(1).is_none() && c.get(3).is_none()) || PROSE.contains(&m.as_str()) {
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

/// A regular, non-code file of the cwd's top level is over the large-input threshold.
fn has_large_input(dir: &std::path::Path) -> bool {
    std::fs::read_dir(dir).is_ok_and(|rd| {
        rd.flatten().any(|e| {
            e.metadata().is_ok_and(|m| m.is_file() && m.len() > LARGE_INPUT_BYTES) && !is_code_path(&e.file_name().to_string_lossy())
        })
    })
}

/// A todo write marks an item completed with a self-declared `verified` confidence.
fn claims_verified(input: &serde_json::Value) -> bool {
    let items = match &input["todos"] {
        serde_json::Value::Array(a) => a.clone(),
        serde_json::Value::String(s) => serde_json::from_str(s).unwrap_or_default(),
        _ => return false,
    };
    items.iter().any(|t| {
        t["status"].as_str() == Some("completed")
            && [&t["completion_confidence"], &t["confidence"]].iter().any(|c| c.as_str() == Some("verified"))
    })
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

/// Shortfall detector: some sentence of the final paragraph pairs a negation or shortfall word with an
/// object word (or is a self-sufficient shortfall phrase), unless it blames an unreadable input.
fn shortfall(text: &str, cwd_files: &[String]) -> bool {
    // A typographic apostrophe must match like a plain one.
    let lower = text.to_ascii_lowercase().replace(['\u{2019}', '\u{2018}', '\u{02bc}'], "'");
    let body = lower.trim_end();
    let para = body.rsplit("\n\n").next().unwrap_or(body);
    let names_file = cwd_files.iter().any(|f| f.len() >= 3 && para.contains(f.as_str()));
    let unreadable = UNREADABLE_PHRASES.iter().any(|p| para.contains(p)) || (names_file && UNREADABLE_IF_FILE.iter().any(|p| para.contains(p)));
    if unreadable {
        return false;
    }
    para.split(['.', '!', '?', '\n', ';']).any(|sent| {
        let has = |list: &[&str]| list.iter().any(|w| contains_word(sent, w));
        SHORTFALL_PHRASES.iter().any(|p| sent.contains(p)) || (has(&SHORTFALL_NEG) && has(&OBJECT_WORDS))
    })
}

/// `needle` appears in `hay` starting at a word boundary (a stem like "test" matches "tests", "verify" "verified").
fn contains_word(hay: &str, needle: &str) -> bool {
    hay.match_indices(needle).any(|(i, _)| hay[..i].chars().next_back().is_none_or(|c| !c.is_alphanumeric()))
}

fn format_nudge(label: &str) -> String {
    format!("<system-reminder>Your reply must include the required line `{label} <value>` exactly as the task specifies (a single value unless it asks for a list). Give your result on that line now.</system-reminder>")
}

fn url_host(url: &str) -> Option<String> {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let host = rest.split(['/', '?', '#']).next()?.rsplit('@').next()?.split(':').next()?;
    (!host.is_empty()).then(|| host.trim_start_matches("www.").to_ascii_lowercase())
}

/// The v0.0.1 detector (ablation only; the default is `factr_base::task_policy::forbids_tools`).
fn forbids_tools_legacy(task: &str) -> bool {
    let t = task.to_ascii_lowercase().replace('\u{2019}', "'");
    TOOL_FORBIDS.iter().any(|p| t.contains(p))
}

fn same_path(a: &str, b: &str) -> bool {
    let n = |p: &str| p.trim().trim_start_matches("./").to_string();
    let (a, b) = (n(a), n(b));
    a == b || a.ends_with(&format!("/{b}")) || b.ends_with(&format!("/{a}"))
}

/// Window note of a cut webfetch ("showing head H + tail T of N chars; full text at PATH; ...") or of
/// a read overview of a large file: (chars seen, total chars, spill path).
fn parse_fetch_cut(out: &str) -> Option<(usize, usize, Option<String>)> {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| regex::Regex::new(r"showing head (\d+) \+ tail (\d+) of (\d+) chars(?:; full text at ([^;\n]+?);)?").unwrap());
    let c = re.captures(out)?;
    let n = |i: usize| c.get(i).and_then(|m| m.as_str().parse::<usize>().ok());
    Some((n(1)? + n(2)?, n(3)?, c.get(4).map(|m| m.as_str().to_string())))
}

/// The answer value of a reply: the text after the task's label on its latest label line, else the
/// last non-empty line, with markdown emphasis removed.
fn answer_value(text: &str, label: Option<&str>) -> Option<String> {
    let clean = |l: &str| l.trim().trim_matches(|c: char| matches!(c, '*' | '`' | '_' | '#' | '>')).trim().to_string();
    let line = match label {
        Some(l) => text
            .lines()
            .rev()
            .find(|x| has_label_line(x, l))
            .map(|x| {
                let t = x.trim_start_matches(|c: char| c.is_whitespace() || matches!(c, '*' | '`' | '_' | '#' | '>' | '-'));
                t.strip_prefix(l).unwrap_or(t).to_string()
            })
            .or_else(|| text.lines().rev().find(|x| !x.trim().is_empty()).map(str::to_string)),
        None => text.lines().rev().find(|x| !x.trim().is_empty()).map(str::to_string),
    }?;
    let v = clean(&line);
    (!v.is_empty()).then_some(v)
}

/// A bare computed number: optional markdown, currency, thousands separators, percent and one trailing
/// unit word stripped, then `-?\d+(\.\d+)?`; whole numbers 1000..=2100 (years) are exempt.
fn is_computed_number(value: &str) -> bool {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| regex::Regex::new(r"^-?\d+(\.\d+)?$").unwrap());
    let mut v = value.trim().trim_end_matches('.').trim().to_string();
    let mut parts: Vec<&str> = v.split_whitespace().collect();
    if parts.len() == 2 && parts[1].chars().all(|c| c.is_alphabetic() || matches!(c, '%' | '/' | '.')) {
        parts.truncate(1);
    }
    v = parts.join(" ");
    let v: String = v.chars().filter(|c| !matches!(c, '$' | '\u{20ac}' | '\u{a3}' | '\u{a5}' | '%' | ',' | '*' | '_' | '`')).collect();
    if !re.is_match(&v) {
        return false;
    }
    !(!v.contains('.') && v.parse::<i64>().is_ok_and(|n| (1000..=2100).contains(&n)))
}

/// Shape of the answer, for observation only: parenthetical, qualifier, multi-clause, list-count or plain.
fn answer_shape(text: &str, label: Option<&str>) -> &'static str {
    let Some(v) = answer_value(text, label) else { return "plain" };
    let lower = v.to_ascii_lowercase();
    let bullets = text.lines().filter(|l| { let t = l.trim_start(); t.starts_with("- ") || t.starts_with("* ") || t.chars().next().is_some_and(|c| c.is_ascii_digit()) && t.contains(". ") }).count();
    if bullets >= 2 || (v.matches(',').count() >= 2 && v.split(',').all(|p| p.split_whitespace().count() <= 4)) {
        "list-count"
    } else if v.contains('(') && v.contains(')') {
        "parenthetical"
    } else if ["approx", "about ", "around ", "roughly", "~", "at least", "at most", "more than", "less than", "up to", "between "].iter().any(|q| lower.contains(q)) {
        "qualifier"
    } else if v.contains(';') || v.contains(" but ") || v.split_whitespace().count() > 12 {
        "multi-clause"
    } else {
        "plain"
    }
}

/// mtime and size of the files under `dir` (depth <= 3, <= 5000 entries; VCS, dependency and venv
/// directories skipped): writes made through bash or the REPL show as differences between two snapshots.
fn fs_snapshot(dir: &std::path::Path) -> std::collections::HashMap<std::path::PathBuf, (std::time::SystemTime, u64)> {
    const SKIP: [&str; 7] = [".git", "node_modules", "target", "venv", ".venv", "__pycache__", ".tox"];
    let mut out = std::collections::HashMap::new();
    let mut stack = vec![(dir.to_path_buf(), 0u8)];
    while let Some((d, depth)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            if out.len() >= 5000 {
                return out;
            }
            let Ok(m) = e.metadata() else { continue };
            let name = e.file_name().to_string_lossy().into_owned();
            if m.is_dir() {
                if depth < 3 && !SKIP.contains(&name.as_str()) {
                    stack.push((e.path(), depth + 1));
                }
            } else if let Ok(t) = m.modified() {
                out.insert(e.path(), (t, m.len()));
            }
        }
    }
    out
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
        let title = Some("Result:".to_string());
        assert_eq!(l("Final answer must be in the form \"Result: <x>\""), title);
        assert_eq!(l("Reply with a final line: Result: [value]"), title);
        assert_eq!(l("Your answer must be given as 'Answer: <x>'."), Some("Answer:".to_string()));
        assert_eq!(l("Note: this is fine. Answer the question below."), None);
        assert_eq!(l("Answer briefly. Example: foo is a word."), None);
        assert_eq!(l("Report the totals. \"Example: <x>\" shows the style."), None);
        assert_eq!(l("Summary: the answer is long and the Total: 5 is fine."), None);
    }

    #[test]
    fn template_detection_table() {
        let cases: [(&str, Option<&str>); 12] = [
            // lowercase `=` template, then a Title-case data heading with a bracket
            ("Reply with one line, word = <value>.\nExample data:\nItems: [\n  1, 2\n]", Some("word =")),
            ("Finish with the line total = [number] and nothing else.", Some("total =")),
            ("Answer in the form `pick: <choice>`.", Some("pick:")),
            ("Report the winner on a last line: winner: {name}", Some("winner:")),
            ("End your reply with Verdict: [yes or no]", Some("Verdict:")),
            ("Conclude with 'RESULT: <value>'.", Some("RESULT:")),
            // no template at all
            ("Summarise the notes. Items: [1, 2, 3] are listed below.", None),
            // data brackets are never a placeholder
            ("Reply in this format. Rows: [[1, 2], [3, 4]] and Tags: [-1, 2] and Names: [\"a\"]", None),
            // `endpoints` is not the verb `end`
            ("List the endpoints. Handler = <fn> is the shape of the table.", None),
            ("Please answer. Note: <something>", None),
            ("Answer with x == <y> only", None),
            ("Ends with: Score: [n]", Some("Score:")),
        ];
        for (task, want) in cases {
            assert_eq!(required_label(task).as_deref(), want, "{task}");
        }
        assert_eq!(required_label_legacy("Reply with one line, word = <value>.\nItems: ["), Some("Items:".to_string()));
    }

    #[test]
    fn final_text_prefers_a_real_template_line_over_a_stub() {
        let mk = |label: Option<&str>, replies: &[&str]| StopNudge { task_label: label.map(str::to_string), replies: replies.iter().map(|r| r.to_string()).collect(), ..Default::default() };
        let good = "Worked through the steps in detail above.\nword = alpha";
        // wrong label recognised: the stub carrying it must not replace the earlier reply
        assert_eq!(mk(Some("Items:"), &[good]).final_text("Items: alpha"), good);
        // a long labelled reply still wins
        let long = "Items: alpha beta gamma delta, with enough words to be well over the stub limit.";
        assert_eq!(mk(Some("Items:"), &[good]).final_text(long), long);
        // no label: a last reply without a template line yields to an earlier one that ends in one
        let prose = "Let me explain why that earlier line is the one I would stand behind, in prose.";
        assert_eq!(mk(None, &[good]).final_text(prose), good);
        // no label: a long last reply is kept, a stub falls back
        assert_eq!(mk(None, &[prose]).final_text("Confirmed."), prose);
    }

    #[test]
    fn decline_nudges_ask_for_an_answer() {
        assert!(DECLINE_NUDGE.contains("best answer in the requested form"));
        assert!(DECLINE_NO_TOOLS_NUDGE.contains("best answer in the requested form"));
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
    fn decline_covers_common_phrasings_without_false_positives() {
        let mk = || {
            let mut n = StopNudge::default();
            n.headless = true;
            n
        };
        for t in ["I could not determine the total.", "There is insufficient information to answer.", "I am unable to provide a value.", "It is impossible to determine this."] {
            assert!(mk().on_text_only_stop("s", t), "{t}");
        }
        for t in [
            "I can\u{2019}t calculate the total from this.",
            "I can't calculate that.",
            "I cannot calculate the share.",
            "That figure cannot be verified.",
            "I can\u{2019}t verify the count.",
            "I could not determine the answer\u{2019}s basis, so I can\u{2019}t answer.",
        ] {
            assert!(mk().on_text_only_stop("s", t), "{t}");
        }
        for t in [
            "The total is 42. I can verify it against the table.",
            "I can calculate it: 42.",
            "I can\u{2019}t calculate by hand, so the code did it.\n\nAnswer: 42",
            "The total is 42, which the sum check confirms.",
        ] {
            assert!(!mk().on_text_only_stop("s", t), "{t}");
        }
        for t in ["The total is 42.", "Estimated total is about 40 (not enough information for more precision), best estimate 40.\n\nAnswer: 40"] {
            assert!(!mk().on_text_only_stop("s", t), "{t}");
        }
    }

    #[test]
    fn large_input_answer_after_one_call_gets_one_measure_nudge() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("data.txt"), "x\n".repeat(15_000)).unwrap();
        std::fs::write(d.path().join("tool.rs"), "x".repeat(30_000)).unwrap();
        assert!(has_large_input(d.path()));
        let small = tempfile::tempdir().unwrap();
        std::fs::write(small.path().join("a.txt"), "tiny").unwrap();
        std::fs::write(small.path().join("big.rs"), "x".repeat(30_000)).unwrap();
        assert!(!has_large_input(small.path()), "code files and small files do not count");
        let mk = || {
            let mut n = StopNudge::default();
            n.headless = true;
            n.large_input = true;
            n
        };
        let mut n = mk();
        n.observe("bash", &json!({"command": "wc -l data.txt"}), 0);
        assert!(n.on_text_only_stop("s", "The answer is 7."));
        assert_eq!(n.take_pending(), Some(MEASURE_NUDGE.to_string()));
        n.observe("bash", &json!({"command": "grep -c x data.txt"}), 0);
        assert!(!n.on_text_only_stop("s", "The answer is 7."), "once per turn");
        let mut many = mk();
        for _ in 0..2 {
            many.observe("bash", &json!({"command": "ls"}), 0);
        }
        assert!(!many.on_text_only_stop("s", "The answer is 7."), "more than one analysis call");
        let mut plain = mk();
        plain.large_input = false;
        assert!(!plain.on_text_only_stop("s", "The answer is 7."), "no large input");
    }

    fn no_tools_nudge() -> StopNudge {
        StopNudge { headless: true, large_input: true, forbids_tools: true, compute_tool: true, ..Default::default() }
    }

    #[test]
    fn a_task_that_forbids_tools_gets_no_measure_or_tool_nudge_even_with_zero_calls() {
        let mut n = no_tools_nudge();
        assert!(!n.on_text_only_stop("s", "The answer is 42."), "no MEASURE, no compute guard");
        assert!(n.take_pending().is_none());
        let mut q = no_tools_nudge();
        assert!(q.on_text_only_stop("s", "I could work it out. Want me to run it?"));
        assert_eq!(q.take_pending(), Some(NO_TOOLS_QUESTION_NUDGE.to_string()));
        // The same reply on a task that allows tools gets the tool-based nudges.
        let mut a = no_tools_nudge();
        a.forbids_tools = false;
        assert!(a.on_text_only_stop("s", "The answer is 42."));
        assert_eq!(a.take_pending(), Some(MEASURE_NUDGE.to_string()));
        let mut a = no_tools_nudge();
        a.forbids_tools = false;
        a.large_input = false;
        assert!(a.on_text_only_stop("s", "I could work it out. Want me to run it?"));
        assert_eq!(a.take_pending(), Some(QUESTION_NUDGE.to_string()));
        // Ablation: `notools_off` restores the ungated chain for a forbidding task.
        let mut o = no_tools_nudge();
        o.notools_off = true;
        assert!(o.on_text_only_stop("s", "The answer is 42."));
    }

    #[test]
    fn a_forbidding_task_with_earlier_tool_use_still_gets_no_tool_nudges() {
        let todo = json!({"todos": [{"content": "a", "status": "completed", "priority": "high", "id": "1", "confidence": "verified"}]});
        let mk = |forbid: bool| {
            let mut n = no_tools_nudge();
            n.forbids_tools = forbid;
            n.observe("todo", &todo, 0);
            n.observe("bash", &json!({"command": "ls"}), 0);
            n.observe("write", &json!({"file_path": "out.txt", "content": "x"}), 0);
            n.observe("bash", &json!({"command": "cargo test"}), 1);
            n
        };
        for text in ["Done, it is 42.", "Done. I'll run the tests now.", "Let me check the result next."] {
            let mut n = mk(true);
            assert!(!n.on_text_only_stop("s", text), "{text}");
            assert!(n.take_pending().is_none());
        }
        let mut n = mk(false);
        assert!(n.on_text_only_stop("s", "Done. I'll run the tests now."), "control: tools allowed queues a nudge");
        // A cut page and a pending check do not nudge either.
        let mut c = mk(true);
        c.observe_full("webfetch", &json!({"url": "https://a.example/p"}), 0, None, Some(CUT_OUT));
        assert!(!c.on_text_only_stop("s", "It is 7 apples."));
    }

    #[test]
    fn self_declared_verified_needs_an_inspecting_tool_call() {
        let todo = json!({"todos": [{"content": "a", "status": "completed", "priority": "high", "id": "1", "confidence": "verified"}]});
        let mut n = StopNudge::default();
        n.observe("todo", &todo, 0);
        assert!(n.on_text_only_stop("s", "Done, it is 42."));
        assert_eq!(n.take_pending(), Some(UNVERIFIED_NUDGE.to_string()));
        let mut n = StopNudge::default();
        n.observe("todo", &todo, 0);
        n.observe("bash", &json!({"command": "wc -l data.txt"}), 0);
        assert!(!n.on_text_only_stop("s", "Done, it is 42."), "a tool call inspected something");
        let plain = json!({"todos": [{"content": "a", "status": "completed", "priority": "high", "id": "1", "confidence": "plausible"}]});
        let mut n = StopNudge::default();
        n.observe("todo", &plain, 0);
        assert!(!n.on_text_only_stop("s", "Done."));
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
        let mut n = headless();
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
        n.headless = true;
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

    fn headless() -> StopNudge {
        StopNudge { headless: true, ..Default::default() }
    }

    #[test]
    fn no_nudge_text_carries_hedging_or_answer_shape_words() {
        let banned = ["final answer", "restate", "only", "format", "caveat", "estimate", "uncertain"];
        let mut all: Vec<String> = [
            VERIFY_NUDGE, STEER, QUESTION_NUDGE, NO_TOOLS_QUESTION_NUDGE, FAILED_CHECK_NUDGE, SNIPPET_NUDGE, DECLINE_NUDGE, DECLINE_MANY_ROUTES_NUDGE,
            DECLINE_NO_TOOLS_NUDGE, DECLINE_NUDGE_V1, DECLINE_NO_TOOLS_NUDGE_V1, ITERATE_NUDGE, COMPUTE_GUARD_NUDGE, SKIPPED_NUDGE, NO_RUNNER_NOTE, UNVERIFIED_NUDGE,
            MEASURE_NUDGE, ACTION_NUDGE,
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        all.push(format_nudge("RESULT:"));
        all.push(super::super::bg_guard::stale_note_text("t1"));
        all.push(super::super::bg_guard::service_nudge("t1", "srv", "1"));
        // The dynamic nudges, as the chain builds them.
        let mut n = headless();
        let meta = json!({"background": true, "task_id": "t7", "timeout_promoted": true});
        n.observe_full("bash", &json!({"command": "pytest"}), 0, Some(&meta), None);
        assert!(n.on_text_only_stop("s", "Looks fine."));
        all.push(n.take_pending().unwrap());
        let mut n = headless();
        n.observe_full("webfetch", &json!({"url": "https://a.example/p"}), 0, None, Some(CUT_OUT));
        assert!(n.on_text_only_stop("s", "It is 7 apples."));
        all.push(n.take_pending().unwrap());
        for t in &all {
            let l = t.to_ascii_lowercase();
            for b in banned {
                assert!(!l.contains(b), "`{b}` in {t}");
            }
        }
    }

    const CUT_OUT: &str = "Fetched https://a.example/p (50000 chars)\n(showing head 8000 + tail 4000 of 50000 chars; full text at /tmp/x/webfetch-1.txt; open it with read (it is outside the working directory); use find or offset)\n\nbody";

    #[test]
    fn final_text_is_one_whole_reply() {
        let long_a = "A: a long first reply with plenty of detail in it, well over forty characters.";
        let long_b = "B: a long unlabelled second reply with plenty of detail, over forty characters.";
        let mk = |label: Option<&str>, replies: &[&str]| StopNudge { task_label: label.map(str::to_string), replies: replies.iter().map(|r| r.to_string()).collect(), ..Default::default() };
        let labelled_a = format!("{long_a}\nRESULT: 7");
        // [labelled A, unlabelled long B] -> A (B is the last reply, passed in)
        assert_eq!(mk(Some("RESULT:"), &[&labelled_a]).final_text(long_b), labelled_a);
        // [A, labelled C] -> C
        let c = "Redone.\nRESULT: 8";
        assert_eq!(mk(Some("RESULT:"), &[long_a]).final_text(c), c);
        // [A, ''] -> A
        assert_eq!(mk(None, &[long_a]).final_text(""), long_a);
        assert_eq!(mk(None, &[long_a]).final_text("Confirmed."), long_a, "a bare stub falls back");
        // no label [A, B] -> B
        assert_eq!(mk(None, &[long_a]).final_text(long_b), long_b);
        // no concatenation, nothing appended
        assert!(!mk(None, &[long_a]).final_text(long_b).contains("A:"));
    }

    #[test]
    fn compute_guard_fires_once_on_an_unchecked_bare_number() {
        let mk = || StopNudge { headless: true, compute_tool: true, ..Default::default() };
        let mut n = mk();
        assert!(n.on_text_only_stop("s", "The count is high.\n\n42"));
        assert_eq!(n.take_pending(), Some(COMPUTE_GUARD_NUDGE.to_string()));
        assert!(!n.on_text_only_stop("s", "42"), "once");
        // read, answer from the read: fires; code ran after the read: quiet
        let mut r = mk();
        r.observe("read", &json!({"file_path": "d.csv"}), 0);
        assert!(r.on_text_only_stop("s", "Total:\n**$1,234.50**"));
        let mut c = mk();
        c.observe("read", &json!({"file_path": "d.csv"}), 0);
        c.observe("repl", &json!({}), 0);
        assert!(!c.on_text_only_stop("s", "1234.5"));
        // value forms, label value, years exempt, prose and forbids
        assert!(is_computed_number("42 km") && is_computed_number("12%") && is_computed_number("-3.5") && is_computed_number("$1,000,000"));
        assert!(!is_computed_number("2024") && !is_computed_number("about 42") && !is_computed_number("1.5.2"));
        let mut l = mk();
        l.task_label = Some("RESULT:".into());
        assert!(l.on_text_only_stop("s", "Reasoning 17.\n**RESULT: 51**"));
        let mut p = mk();
        assert!(!p.on_text_only_stop("s", "The answer is 42."), "prose line");
        let mut f = mk();
        f.forbids_tools = true;
        assert!(!f.on_text_only_stop("s", "51"));
        let mut i = mk();
        i.headless = false;
        assert!(!i.on_text_only_stop("s", "51"));
        let mut late = mk();
        late.set_late(true);
        assert!(!late.on_text_only_stop("s", "51"));
        let mut nt = mk();
        nt.compute_tool = false;
        assert!(!nt.on_text_only_stop("s", "51"));
    }

    #[test]
    fn unread_remainder_nudges_once_and_clears_on_search_or_spill_read() {
        let fetch = |n: &mut StopNudge, url: &str, extra: serde_json::Value| {
            let mut input = json!({"url": url});
            if let (Some(o), Some(e)) = (input.as_object_mut(), extra.as_object()) {
                o.extend(e.clone());
            }
            n.observe_full("webfetch", &input, 0, None, Some(if extra.is_null() { CUT_OUT } else { "Fetched (5 chars)\n(1 match(es) for \"x\" in 50000 chars)\n\nhit" }));
        };
        let mut n = headless();
        fetch(&mut n, "https://a.example/p", json!(null));
        assert_eq!(n.cut_page, Some(("https://a.example/p".to_string(), 12000, 50000)));
        assert!(n.on_text_only_stop("s", "It is 7 apples."));
        let msg = n.take_pending().unwrap();
        assert!(msg.contains("you saw 12000 of 50000 chars of https://a.example/p") && msg.contains("webfetch find=<term>"), "{msg}");
        n.observe("bash", &json!({"command": "ls"}), 0);
        assert!(!n.on_text_only_stop("s", "It is 7 apples."), "once");
        let mut f = headless();
        fetch(&mut f, "https://a.example/p", json!(null));
        fetch(&mut f, "https://a.example/p", json!({"find": "apples"}));
        assert!(f.cut_page.is_none() && !f.on_text_only_stop("s", "It is 7 apples."));
        let mut r = headless();
        fetch(&mut r, "https://a.example/p", json!(null));
        r.observe_full("read", &json!({"file_path": "/tmp/x/webfetch-1.txt"}), 0, None, Some("text"));
        assert!(r.cut_page.is_none());
        let mut o = headless();
        fetch(&mut o, "https://a.example/p", json!(null));
        o.observe_full("webfetch", &json!({"url": "https://b.example/q"}), 0, None, Some("Fetched https://b.example/q (10 chars)\n\nshort"));
        assert!(o.cut_page.is_none(), "another page was fetched in full");
        let mut d = headless();
        fetch(&mut d, "https://a.example/p", json!(null));
        assert!(d.on_text_only_stop("s", "I could not find it, the page was cut off before the end."), "a decline gets its own nudge");
        assert_ne!(d.take_pending().map(|m| m.contains("was cut")), Some(true));
    }

    #[test]
    fn verify_tracker_counts_exec_or_read_back_for_any_deliverable_and_ignores_zero_test_runs() {
        let mut n = headless();
        n.observe("write", &json!({"file_path": "out.md"}), 0);
        assert!(n.on_text_only_stop("s", "Done."), "a non-code deliverable with no run");
        assert_eq!(n.take_pending(), Some(VERIFY_NUDGE.to_string()));
        assert!(!n.on_text_only_stop("s", "Done."), "once");
        let mut r = headless();
        r.observe("write", &json!({"file_path": "out.md"}), 0);
        r.observe("read", &json!({"file_path": "./out.md"}), 0);
        assert!(!r.on_text_only_stop("s", "Done."), "read back");
        let mut x = headless();
        x.observe("write", &json!({"file_path": "out.md"}), 0);
        x.observe_full("bash", &json!({"command": "cat out.md | wc -l"}), 0, None, Some("12"));
        assert!(!x.on_text_only_stop("s", "Done."), "something ran");
        for zero in ["Ran 0 tests in 0.000s", "NO TESTS RAN", "collected 0 items", "running 0 tests", "0 passing"] {
            let mut z = headless();
            z.observe("write", &json!({"file_path": "a.py"}), 0);
            z.observe_full("bash", &json!({"command": "python3 -m unittest"}), 0, None, Some(zero));
            assert!(z.on_text_only_stop("s", "Done."), "{zero}");
        }
        let mut late = headless();
        late.observe("write", &json!({"file_path": "a.md"}), 0);
        late.set_late(true);
        assert!(!late.on_text_only_stop("s", "Done."));
        assert!(!{ let mut i = StopNudge::default(); i.observe("write", &json!({"file_path": "a.md"}), 0); i }.on_text_only_stop("s", "Done."), "interactive");
    }

    fn with_dir(d: &std::path::Path) -> StopNudge {
        let mut n = headless();
        n.snapshot = Some(fs_snapshot(d));
        n.snapshot_dir = Some(d.to_path_buf());
        n
    }

    #[test]
    fn verify_tracker_edit_then_run_is_quiet_and_unrun_writes_nudge() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("app.py");
        // edit, then a bash run that changes nothing else: no nudge (the edit's mtime is not a shell write).
        let mut n = with_dir(d.path());
        std::fs::write(&f, "x=1").unwrap();
        n.observe("edit", &json!({"file_path": f.to_string_lossy()}), 0);
        n.observe_full("bash", &json!({"command": "python3 app.py"}), 0, None, Some("1"));
        assert!(!n.on_text_only_stop("s", "Done."), "edit then run");
        // edit with no run after: nudge.
        let mut n = with_dir(d.path());
        n.observe("edit", &json!({"file_path": f.to_string_lossy()}), 0);
        assert!(n.on_text_only_stop("s", "Done."), "edit without a run");
        // bash-written file, nothing runs after it: nudge.
        let mut n = with_dir(d.path());
        std::fs::write(d.path().join("gen.txt"), "hi").unwrap();
        n.observe_full("bash", &json!({"command": "echo hi > gen.txt"}), 0, None, Some(""));
        assert!(n.on_text_only_stop("s", "Done."), "shell write, no later run");
        // bash-written file, then a later run: quiet.
        let mut n = with_dir(d.path());
        std::fs::write(d.path().join("gen2.txt"), "hi").unwrap();
        n.observe_full("bash", &json!({"command": "echo hi > gen2.txt"}), 0, None, Some(""));
        n.observe_full("bash", &json!({"command": "cat gen2.txt"}), 0, None, Some("hi"));
        assert!(!n.on_text_only_stop("s", "Done."), "shell write then a run");
        // a run that executed zero tests after the edit does not count.
        let mut n = with_dir(d.path());
        n.observe("edit", &json!({"file_path": f.to_string_lossy()}), 0);
        n.observe_full("bash", &json!({"command": "python3 -m unittest"}), 0, None, Some("Ran 0 tests in 0.000s"));
        assert!(n.on_text_only_stop("s", "Done."), "0-test run");
    }

    #[test]
    fn service_recheck_nudges_once_for_an_exited_watched_server_headless_only() {
        let started = |until: bool| {
            let mut n = headless();
            n.observe_full("bash", &json!({"command": "python3 -m http.server 8000", "run_in_background": true}), 0, Some(&json!({"background": true, "task_id": "t9", "display_name": "http server"})), None);
            if until {
                n.observe_full("bg", &json!({"action": "wait", "task_id": "t9", "until": "Serving"}), 0, None, Some("Output matched"));
            }
            n
        };
        let exited = vec![("t9".to_string(), "http server".to_string(), "1".to_string())];
        let mut n = started(true);
        assert!(!n.on_text_only_stop("s", "Done."), "nothing exited yet");
        n.set_service_exits(exited.clone());
        assert!(n.on_text_only_stop("s", "Done."));
        let t = n.take_pending().unwrap();
        assert_eq!(t, super::super::bg_guard::service_nudge("t9", "http server", "1"));
        for b in ["final answer", "restate", "only", "format", "caveat", "estimate", "uncertain"] {
            assert!(!t.to_ascii_lowercase().contains(b), "{b}");
        }
        n.observe("bash", &json!({"command": "ls"}), 0);
        assert!(!n.on_text_only_stop("s", "Done."), "once per turn");
        let mut i = started(true);
        i.headless = false;
        i.set_service_exits(exited.clone());
        assert!(!i.on_text_only_stop("s", "Done."), "interactive");
        let mut l = started(true);
        l.set_late(true);
        l.set_service_exits(exited.clone());
        assert!(!l.on_text_only_stop("s", "Done."), "late");
        // The model looked at how it ended, or cancelled it: nothing left to recheck.
        let mut seen = started(false);
        seen.observe_full("bg", &json!({"action": "output", "task_id": "t9"}), 0, None, Some("Traceback: boom"));
        assert!(seen.watched().is_empty());
        let mut c = started(true);
        c.observe("bg", &json!({"action": "cancel", "task_id": "t9"}), 0);
        assert!(c.watched().is_empty());
        // A ready line in the task's output flags it without until.
        let mut r = started(false);
        r.observe_full("bg", &json!({"action": "output", "task_id": "t9"}), 0, None, Some("Listening on 0.0.0.0:8000"));
        assert!(r.watched()[0].flagged);
    }

    #[test]
    fn a_file_written_by_a_shell_run_counts_as_a_write() {
        let d = tempfile::tempdir().unwrap();
        let mut n = headless();
        n.snapshot = Some(fs_snapshot(d.path()));
        n.snapshot_dir = Some(d.path().to_path_buf());
        n.observe_full("bash", &json!({"command": "echo hi > out.txt"}), 0, None, Some(""));
        std::fs::write(d.path().join("out.txt"), "hi").unwrap();
        assert!(n.on_text_only_stop("s", "Done."));
        let mut q = headless();
        q.snapshot = Some(fs_snapshot(d.path()));
        q.snapshot_dir = Some(d.path().to_path_buf());
        q.observe_full("bash", &json!({"command": "ls"}), 0, None, Some(""));
        assert!(!q.on_text_only_stop("s", "Done."), "nothing changed");
    }

    #[test]
    fn shortfall_with_files_written_iterates_without_a_format_ask_and_forbidding_tasks_get_no_tool_advice() {
        let mut n = headless();
        n.observe("write", &json!({"file_path": "a.md"}), 0);
        n.observe("bash", &json!({"command": "cat a.md"}), 0);
        assert!(n.on_text_only_stop("s", "Wrote it, but the score is below the target."));
        assert_eq!(n.take_pending(), Some(ITERATE_NUDGE.to_string()));
        let mut f = headless();
        f.forbids_tools = true;
        assert!(f.on_text_only_stop("s", "I was unable to determine the value."));
        assert_eq!(f.take_pending(), Some(DECLINE_NO_TOOLS_NUDGE.to_string()));
        let mut h = headless();
        h.set_half(true);
        assert!(!h.on_text_only_stop("s", "I could not find the value."), "past half the budget");
        let mut p = headless();
        p.observe("edit", &json!({"file_path": "a.rs"}), 0);
        p.observe("bash", &json!({"command": "cargo test"}), 0);
        assert!(!p.on_text_only_stop("s", "Fixed. I could not find any other caller."), "tests passed after the last write");
    }

    #[test]
    fn shortfall_detector_precision_and_recall_on_a_labelled_corpus() {
        let short = [
            "I could not find the value.", "I was unable to determine the total.", "I cannot reach the target score.",
            "The score is below the threshold, so the requirement is not met.", "Tests are not yet passing; the check fails to run.",
            "The result is short of the expected value.", "I couldn't verify the output against the requirement.",
            "Done, but the answer could not be confirmed.", "Wrote the file. The test was not re-tested after my last edit.",
            "I can't calculate that from the given data.", "Not possible to complete with these tools.",
            "It is not feasible to install the missing library here.", "numpy is not installed, so the check could not run.",
            "There is insufficient information to answer.", "I was unable to find the expected output.",
            "The solution fails to meet the target accuracy.", "I cannot complete the computation within the limit.",
            "The model score stays below the threshold of 0.8.", "Not verified: the value could not be reproduced.",
            "I couldn't find a source that gives the figure.", "I could not solve the last case.",
            "The accuracy is below the target; I have not been able to improve it.", "Unable to determine which row is expected.",
            "I cannot determine the answer from the file.", "The tests fail to pass, so the requirement is unmet.",
            "Result written, but the check could not be completed.", "I could not install the package, so the test did not run.",
            "That value is impossible to determine from what I have.", "I'm unable to verify the result.",
            "I can\u{2019}t calculate the share without the missing column.", "The output is not yet at the target; the score is below it.",
            "No way to determine the answer with the data given.", "I did not reach the expected score; it stays below.",
            "The checker could not be run, so the result is not verified.", "I was unable to access the value on the page.",
            "Partial: I could not complete the last requirement.", "It is not possible to find the answer on this site.",
            "I could not find the answer in the sources I checked.", "The test is still failing and I am unable to fix it.",
            "Not enough information to determine the result.", "The build is below the required size target.",
            "I cannot verify the answer.", "I couldn't determine the value.", "Unable to reach the threshold.",
            "The requirement could not be satisfied.", "I failed to find the expected value.",
            "Still short of the target; the score was not verified.", "I could not complete the check.",
            "Honestly, I cannot answer this.", "The result cannot be verified.",
        ];
        let done = [
            "The value is 42.", "Done. The tests pass and the file is written to out.md.", "All requirements are met and verified.",
            "The total is 1,234.50, taken from the sum over all rows.", "I wrote the report and checked each section against the task.",
            "RESULT: 7", "The answer is Paris.", "Fixed the bug in parse(); the suite passes.", "Summary of changes: renamed the helper and updated callers.",
            "The script prints 12 rows and exits with 0.", "The score is above the target at 0.91.", "I verified the output against the expected file and it matches.",
            "The function now handles empty input.", "Installed the package and ran the check; everything passes.", "The report is in report.md.",
            "The share is 12.5%.", "Both checks passed.", "The file contains 40 records; the count is 40.", "Yes.", "The capital is Ottawa.",
            "The page lists the figure as 3,400.", "I found the value in the second table: 17.", "The test suite passes with 25 tests.",
            "The answer is 3.", "Updated config and re-ran the check, which succeeded.", "The requirement is met: output sorted ascending.",
            "The result is 9, confirmed by two independent computations.", "I computed the mean as 4.2 and verified it in code.",
            "Created the three files requested.", "The code compiles and the example runs.", "The expected output matches exactly.",
            "The final value is 8.25.", "The difference is 14 days.", "The target was reached: accuracy 0.93.", "It works on all the inputs I tried.",
            "Checked: the total of the column equals 310.", "The threshold is 5 and the value 7 exceeds it.", "Here is the summary table.",
            "The command completed successfully and wrote the output.", "The answer: Smith.", "Everything is in place; the check script exits 0.",
            "The two sources agree on 1998.", "The solution passes all provided tests.", "I answered from the primary source.", "Sorted the list and saved it.",
            "The mean score is 82.", "The service is running and the endpoint answers.", "The value found is 0.5.", "No other callers exist; the rename is complete.",
            "Verified the result by rerunning the whole pipeline.", "The data has 3 columns and 10 rows.",
        ];
        let flagged_short = short.iter().filter(|t| shortfall(t, &[])).count();
        let flagged_done = done.iter().filter(|t| shortfall(t, &[])).count();
        let recall = flagged_short as f64 / short.len() as f64;
        let precision_done = 1.0 - flagged_done as f64 / done.len() as f64;
        assert!(precision_done >= 0.9, "complete messages kept quiet: {precision_done} ({flagged_done} flagged: {:?})", done.iter().filter(|t| shortfall(t, &[])).collect::<Vec<_>>());
        assert!(recall >= 0.8, "shortfall recall {recall}; missed {:?}", short.iter().filter(|t| !shortfall(t, &[])).collect::<Vec<_>>());
        assert!(short.len() + done.len() >= 100);
        assert!(!shortfall("I cannot determine this: the file is unreadable.", &[]));
        assert!(!shortfall("i was unable to access data.csv, so i could not determine it.", &["data.csv".to_string()]));
    }

    #[test]
    fn promoted_check_is_pending_not_a_pass_and_a_stop_gets_one_wait_nudge() {
        let meta = json!({"background": true, "task_id": "t7", "timeout_promoted": true});
        let mut m = headless();
        m.observe_full("bash", &json!({"command": "pytest"}), 0, Some(&meta), None);
        assert_eq!(m.last_test, 0, "a promoted run is not a test result");
        assert!(m.on_text_only_stop("s", "Looks fine."));
        let msg = m.take_pending().unwrap();
        assert!(msg.contains("still running as task t7") && msg.contains("`bg` wait"), "{msg}");
        assert!(!m.on_text_only_stop("s", "Looks fine."), "once");
        let mut waited = headless();
        waited.observe_full("bash", &json!({"command": "pytest"}), 0, Some(&meta), None);
        waited.observe("bg", &json!({"action": "wait", "task_id": "t7"}), 0);
        assert!(waited.pending_check.is_none());
    }

    #[test]
    fn decline_asks_for_distinct_routes_then_for_the_answer_once_routes_were_tried() {
        let mut few = headless();
        assert!(few.on_text_only_stop("s", "I could not find it."));
        assert!(few.take_pending().unwrap().contains("try two different routes"));
        let mut many = headless();
        many.observe("websearch", &json!({"query": "a"}), 0);
        many.observe("websearch", &json!({"query": "b"}), 0);
        many.observe("webfetch", &json!({"url": "https://www.example.org/x"}), 0);
        assert_eq!(many.hosts.len(), 1);
        assert!(many.on_text_only_stop("s", "I could not find it."));
        assert!(many.take_pending().unwrap().contains("tried several routes"));
    }

    #[test]
    fn todo_only_rounds_are_not_progress_for_the_second_nudge() {
        let mut n = StopNudge::default();
        n.observe("read", &json!({}), 0);
        assert!(n.on_text_only_stop("s", "I'll fix it."));
        n.observe("todo", &json!({"todos": []}), 0);
        assert!(!n.on_text_only_stop("s", "Next, I'll fix it."), "a todo write is not progress");
        n.observe("read", &json!({}), 0);
        assert!(n.on_text_only_stop("s", "Next, I'll fix it."));
    }

    #[test]
    fn format_nudge_has_its_own_allowance_outside_the_cap() {
        let mut n = StopNudge { format_label: Some("RESULT:".into()), task_label: Some("RESULT:".into()), ..Default::default() };
        n.observe("read", &json!({}), 0);
        assert!(n.on_text_only_stop("s", "I'll fix it."));
        n.observe("read", &json!({}), 0);
        assert!(n.on_text_only_stop("s", "Next, I'll fix it."));
        n.observe("read", &json!({}), 0);
        assert!(n.on_text_only_stop("s", "It is 7."), "cap is spent, the format nudge still fires");
        assert!(n.take_pending().unwrap().contains("`RESULT: <value>`"));
        assert!(!n.on_text_only_stop("s", "It is 7."), "once");
    }

    #[test]
    fn turn_end_span_reports_how_the_turn_ended() {
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = seen.clone();
        factr_base::obs_sink::install(move |s| sink.lock().unwrap().push(s));
        let mut n = headless();
        n.first_value = Some("5".into());
        n.emit_turn_end("turn-end-s", "RESULT: 5 (approx)", Some("end_turn"));
        let seen = seen.lock().unwrap();
        let span = seen.iter().find(|s| s.session_id.as_deref() == Some("turn-end-s") && s.attributes["reason"] == "turn_end").unwrap();
        assert_eq!(span.attributes["nudges_sent"], 0);
        assert_eq!(span.attributes["stop_reason"], "end_turn");
        assert_eq!(span.attributes["answer_changed"], true);
        assert_eq!(span.attributes["answer_shape"], "parenthetical");
    }

    #[test]
    fn answer_shapes_are_classified_for_observation() {
        assert_eq!(answer_shape("x\nabout 40 people", None), "qualifier");
        assert_eq!(answer_shape("x\nJohn, Mary, Paul", None), "list-count");
        assert_eq!(answer_shape("x\n42", None), "plain");
        assert_eq!(answer_shape("x\nthe value is 3 but the range is wide", None), "multi-clause");
    }

    #[test]
    fn ignored_tests_run_once_with_the_flag_for_cargo_and_one_message_for_other_runners() {
        let (mut n, _d) = gate_nudge(&[("Cargo.toml", "")]);
        n.headless = true;
        n.observe("edit", &json!({"file_path": "a.rs"}), 0);
        let cmds = RefCell::new(Vec::<String>::new());
        let run = |c: &str, _: &std::path::Path, _: std::time::Duration| {
            cmds.borrow_mut().push(c.to_string());
            GateResult { passed: true, exit_code: 0, output: "test result: ok. 3 passed; 0 failed; 2 ignored;".into() }
        };
        assert!(!n.on_text_only_stop_with("s", "Done.", &run));
        assert_eq!(*cmds.borrow(), ["cargo test", "cargo test -- --include-ignored"]);
        let (mut p, _d2) = gate_nudge(&[("go.mod", "")]);
        p.headless = true;
        p.observe("edit", &json!({"file_path": "a.go"}), 0);
        let run = |_: &str, _: &std::path::Path, _: std::time::Duration| GateResult { passed: true, exit_code: 0, output: "--- SKIP: TestA".into() };
        assert!(p.on_text_only_stop_with("s", "Done.", &run));
        assert_eq!(p.take_pending(), Some(SKIPPED_NUDGE.to_string()));
        assert!(!p.on_text_only_stop_with("s", "Done.", &run), "skipped message once");
    }

    #[test]
    fn missing_runner_feedback_names_generic_fallbacks() {
        let (mut n, _d) = gate_nudge(&[("test_a.py", "")]);
        n.observe("edit", &json!({"file_path": "a.py"}), 0);
        let run = |_: &str, _: &std::path::Path, _: std::time::Duration| GateResult { passed: false, exit_code: 127, output: "sh: pytest: command not found".into() };
        assert!(n.on_text_only_stop_with("s", "Done.", &run));
        let msg = n.take_pending().unwrap();
        assert!(msg.contains("python3 -m unittest") && msg.contains("node --test") && msg.contains("compile and run the test file"), "{msg}");
        assert!(no_runner(1, "ModuleNotFoundError: No module named 'x'\nNo module named x"));
        assert!(!no_runner(1, "1 failed"));
    }
}
