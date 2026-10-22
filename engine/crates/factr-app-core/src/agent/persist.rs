//! Persist after a failed check (`persistence.enabled`, off by default).
//!
//! A headless turn that is about to end while the check it is working against still fails is not ended:
//! it gets a bounded, escalating nudge instead. The loop is an explicit state machine over one chased
//! check; everything that is observed (a check ran, a file was edited, the model tried to stop) is an event.
//!
//! ```text
//!   Idle ----check Fail/Unavailable----> Failed ----stop attempt (guards ok)----> nudge, stays Failed
//!   Idle ----check Pass----------------> Passed          (the nudge is outstanding until a check runs)
//!   Passed ---edit----------------------> Unverified ---stop attempt---> one "re-run it" nudge
//!   Failed/Unverified ---check Pass-----> Passed
//!   any ---cap | budget | no change | ignored | no re-run | unavailable twice---> Exhausted(reason)
//! ```
//!
//! Invariants: `Exhausted` is a latch, never left; a pass of a different command never clears the failure
//! of the chased one; the nudge count only bounds the loop, the wording follows what the evidence shows
//! (progress, same failure, unavailable, not re-run); a check that ran is judged by its exit code, and
//! its output only refines the failure's signature. General behaviour: no task, benchmark or tool text.

use std::collections::HashSet;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::OnceLock;

use regex::Regex;

pub(super) const DEFAULT_MAX_NUDGES: u32 = 6;
pub(super) const DEFAULT_BUDGET_STOP: f64 = 0.85;
const MAX_LOG: usize = 12;
const SHOWN_LOG: usize = 6;
const MAX_TASK_CHECKS: usize = 3;
/// Two consecutive attempts that leave the work and the failure as they were end the loop.
const MAX_STALLED: u32 = 2;
/// Files the no-change fingerprint reads (and how much of each).
const FINGERPRINT_FILES: usize = 400;
const FINGERPRINT_FILE_BYTES: u64 = 512 * 1024;
const SOURCE_EXTS: [&str; 40] = [
    "rs", "py", "js", "jsx", "ts", "tsx", "mjs", "cjs", "go", "java", "kt", "swift", "c", "h", "cc", "cpp", "hpp", "cs", "rb",
    "php", "scala", "sh", "lua", "dart", "zig", "ex", "cu", "cuh", "cl", "metal", "wgsl", "glsl", "hlsl", "jl", "r", "sql",
    "toml", "yaml", "yml", "cfg",
];
const SKIP_DIRS: [&str; 9] = [".git", "node_modules", "target", "venv", ".venv", "__pycache__", ".tox", ".pytest_cache", "build"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Verdict {
    Pass,
    Fail,
    /// The check could not run at all (missing tool or runner, no GPU): not a verdict on the work.
    Unavailable,
}

/// How a command came to count as a check; it decides how its result is read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Source {
    /// The task text names it, or the project gate ran it: the exit code decides, the output refines it.
    Named,
    /// A test runner command: the exit code decides.
    Runner,
    /// A local script that printed a structured pass/fail report: only the report decides.
    Report,
}

// ---------------------------------------------------------------- check commands named by the task

const CHECK_TRIGGERS: [&str; 16] = [
    "test with", "tests with", "run the tests with", "run tests with", "check with", "check it with", "verify with",
    "verify it with", "validate with", "evaluate with", "evaluate using", "test using", "check using", "verify using",
    "run the evaluator", "evaluator is",
];
const INTERPRETERS: [&str; 8] = ["python", "python3", "node", "bash", "sh", "ruby", "perl", "make"];

fn clean_token(t: &str) -> &str {
    t.trim_start_matches(|c: char| matches!(c, '"' | '\'' | '`' | '('))
        .trim_end_matches(|c: char| matches!(c, '.' | ',' | ';' | ':' | '"' | '\'' | '`' | ')'))
}

fn looks_like_command_token(t: &str) -> bool {
    !t.is_empty()
        && (t.starts_with("./")
            || t.starts_with('/')
            || t.contains('/')
            || [".py", ".sh", ".js", ".rb", ".pl"].iter().any(|e| t.ends_with(e))
            || INTERPRETERS.contains(&t))
}

/// Commands the task text names for checking the work ("test with ./eval", "check it with `make check`").
/// Conservative: only what follows an explicit trigger phrase, at most three, each at most 200 chars.
pub(super) fn task_check_commands(task: &str) -> Vec<String> {
    let lower = task.to_ascii_lowercase();
    let mut out: Vec<String> = Vec::new();
    for trigger in CHECK_TRIGGERS {
        let mut from = 0;
        while let Some(at) = lower[from..].find(trigger) {
            let start = from + at + trigger.len();
            from = start;
            let rest = task[start..].trim_start();
            let cmd = if let Some(inner) = rest.strip_prefix('`') {
                inner.split('`').next().unwrap_or("").trim().to_string()
            } else {
                let mut toks = rest.split_whitespace();
                let first = clean_token(toks.next().unwrap_or(""));
                if !looks_like_command_token(first) {
                    continue;
                }
                let mut cmd = first.to_string();
                if INTERPRETERS.contains(&first) {
                    if let Some(next) = toks.next().map(clean_token).filter(|t| !t.is_empty()) {
                        cmd = format!("{first} {next}");
                    }
                }
                cmd
            };
            if cmd.len() >= 3 && cmd.len() <= 200 && !out.contains(&cmd) {
                out.push(cmd);
                if out.len() >= MAX_TASK_CHECKS {
                    return out;
                }
            }
        }
    }
    out
}

/// Whether `command` is one the task named: any `&&`/`;` segment starts with the named command, word for word
/// (`./eval` matches `cd w && ./eval --fast`, not `./evaluate`).
pub(super) fn is_named_check(task_checks: &[String], command: &str) -> bool {
    command.split("&&").flat_map(|s| s.split(';')).any(|segment| {
        let words: Vec<&str> = segment.split_whitespace().collect();
        task_checks.iter().any(|t| {
            let named: Vec<&str> = t.split_whitespace().collect();
            !named.is_empty() && words.len() >= named.len() && words[..named.len()] == named[..]
        })
    })
}

/// Whether `command` runs a local program or script (`./x`, `scripts/x.sh`, an interpreter and a file), as
/// opposed to a viewer, a network call or a package tool. Only such a command can be a check by its report.
pub(super) fn runs_local_program(command: &str) -> bool {
    let segment = command.rsplit("&&").next().unwrap_or(command);
    let mut words = segment.split_whitespace();
    let first = words.next().unwrap_or("");
    if first.starts_with("./") || first.starts_with('/') || first.contains('/') {
        return true;
    }
    INTERPRETERS.contains(&first) && words.find(|w| !w.starts_with('-')).is_some_and(|f| [".py", ".sh", ".js", ".rb", ".pl"].iter().any(|e| f.ends_with(e)))
}

// ---------------------------------------------------------------- reading a check's result

fn kv_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#""([A-Za-z0-9_\-\. ]{1,40})"\s*:\s*(true|false|[0-9]+)"#).expect("static"))
}

/// A boolean field with one of these words in its name is a pass/fail verdict (`"exec_acc": false`).
const VERDICT_KEYS: [&str; 10] = ["acc", "pass", "ok", "correct", "success", "valid", "match", "equal", "allclose", "verified"];
/// An integer field named like this counts failures (`"failed": 2`).
const FAIL_COUNT_KEYS: [&str; 7] = ["failed", "failures", "failure", "errors", "mismatch", "mismatches", "num_failed"];
/// Output of a check that failed even though it exited 0 (only trusted for a named check).
const FAIL_MARKERS: [&str; 4] = ["assertionerror", "traceback (most recent call last)", "result: fail", "status: fail"];
/// The runner or interpreter itself is missing, or the machine lacks the device the check needs.
const UNAVAILABLE: [&str; 5] = ["command not found", "no cuda", "cuda is not available", "no gpu", "no module named pytest"];
const TAIL_CHARS: usize = 6000;

/// The last `TAIL_CHARS` of `output` (history copies keep head and tail, so the verdict lives in the tail).
fn tail(output: &str) -> &str {
    if output.len() <= TAIL_CHARS {
        return output;
    }
    let mut from = output.len() - TAIL_CHARS;
    while !output.is_char_boundary(from) {
        from += 1;
    }
    &output[from..]
}

/// What a report says: failing keys, passing keys, and failures counted by `"failed": N` style fields.
#[derive(Default)]
struct Report {
    failing: Vec<String>,
    passing: u32,
    counted: u32,
}

fn read_report(output: &str) -> Report {
    let mut r = Report::default();
    for cap in kv_re().captures_iter(output) {
        let key = cap[1].to_ascii_lowercase();
        let val = &cap[2];
        if FAIL_COUNT_KEYS.iter().any(|k| key == *k || key.ends_with(&format!("_{k}")) || key.starts_with(&format!("{k}_"))) {
            match val.parse::<u32>() {
                Ok(n) if n > 0 => {
                    r.counted += n;
                    r.failing.push(key);
                }
                Ok(_) => r.passing += 1,
                Err(_) => {}
            }
        } else if VERDICT_KEYS.iter().any(|k| key.contains(k)) {
            match val {
                "false" => r.failing.push(key),
                "true" => r.passing += 1,
                _ => {} // a number is a score, not a verdict
            }
        }
    }
    r
}

/// The number before (`3 failed`) or after (`failed: 3`) a runner's failure word.
fn runner_fail_count(out: &str) -> u32 {
    let words: Vec<&str> = out.split(|c: char| c.is_whitespace() || matches!(c, ',' | ';' | '(' | ')' | '=' | ':')).filter(|w| !w.is_empty()).collect();
    let mut total = 0;
    for (i, w) in words.iter().enumerate() {
        if ["failed", "failures", "errors"].iter().any(|k| w.eq_ignore_ascii_case(k)) {
            let before = i.checked_sub(1).and_then(|p| words[p].parse::<u32>().ok());
            total += before.or_else(|| words.get(i + 1).and_then(|n| n.parse().ok())).unwrap_or(0);
        }
    }
    total + out.lines().filter(|l| l.trim_start().starts_with("--- FAIL") || l.trim_start().starts_with("FAILED ")).count() as u32
}

/// One check's result as the loop needs it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Outcome {
    pub(super) verdict: Verdict,
    /// How many things failed, when the output says; a smaller number than before is progress.
    pub(super) fail_count: Option<u32>,
    /// Identifies the failure itself: which items failed, else the output with its numbers masked.
    pub(super) signature: u64,
    pub(super) summary: String,
}

fn failure_signature(output: &str, report: &Report) -> u64 {
    let mut parts: Vec<String> = report.failing.clone();
    parts.extend(
        output.lines().map(str::trim).filter(|l| l.starts_with("FAILED ") || l.starts_with("--- FAIL") || l.starts_with("FAIL:")).map(str::to_string),
    );
    if parts.is_empty() {
        // Nothing parses: the output with every digit run masked, so timings and addresses do not matter.
        let masked: String = output.chars().fold(String::new(), |mut acc, c| {
            if c.is_ascii_digit() {
                if !acc.ends_with('#') {
                    acc.push('#');
                }
            } else {
                acc.push(c);
            }
            acc
        });
        return hash_text(&masked);
    }
    parts.sort();
    parts.dedup();
    hash_text(&parts.join("\n"))
}

/// Judge one run of a check. `None` when the run says nothing about the work: a report-only command that
/// printed no report. Precedence: a missing runner or exit 126/127 is `Unavailable`; for a named check or a
/// runner the exit code decides (a named check that exits 0 can still print a failing report); a
/// report-only command is judged by its report alone. A timeout (124) is a failure like any other.
pub(super) fn classify(source: Source, exit_code: i64, output: &str) -> Option<Outcome> {
    let output = tail(output);
    let report = read_report(output);
    let reported = !report.failing.is_empty() || report.passing > 0;
    let lower = output.to_ascii_lowercase();
    let outcome = |verdict, fail_count: Option<u32>, summary: String| Outcome { verdict, fail_count, signature: failure_signature(output, &report), summary };
    if exit_code == 126 || exit_code == 127 || (exit_code != 0 && !reported && UNAVAILABLE.iter().any(|m| lower.contains(m))) {
        return Some(outcome(Verdict::Unavailable, None, "could not run".into()));
    }
    let counted = report.counted + runner_fail_count(output);
    let fail_count = (counted > 0 || !report.failing.is_empty()).then(|| counted.max(report.failing.len() as u32));
    let printed_failure = !report.failing.is_empty()
        || (source == Source::Named && (FAIL_MARKERS.iter().any(|m| lower.contains(m)) || output.lines().any(|l| l.trim_start().starts_with("--- FAIL"))));
    let failed = match source {
        Source::Report => {
            if !reported {
                return None;
            }
            !report.failing.is_empty()
        }
        Source::Runner => exit_code != 0,
        Source::Named => exit_code != 0 || printed_failure,
    };
    let (verdict, summary) = if failed { (Verdict::Fail, failure_summary(output)) } else { (Verdict::Pass, "passed".to_string()) };
    Some(outcome(verdict, fail_count, summary))
}

/// One line saying what failed, for the attempt log.
pub(super) fn failure_summary(output: &str) -> String {
    let pick = output
        .lines()
        .map(str::trim)
        .find(|l| {
            let lo = l.to_ascii_lowercase();
            !l.is_empty() && (lo.contains("fail") || lo.contains("error") || lo.contains("mismatch") || lo.contains("false") || lo.contains("assert"))
        })
        .or_else(|| output.lines().map(str::trim).rfind(|l| !l.is_empty()))
        .unwrap_or("no output");
    let mut s: String = pick.chars().take(140).collect();
    if pick.chars().count() > 140 {
        s.push_str("...");
    }
    s
}

pub(super) fn hash_text(s: &str) -> u64 {
    let mut h = DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

/// Hash of the contents of the source files under `dir` (depth <= 3, bounded): what the model could have
/// changed, not what a check writes (caches, reports, build output).
pub(super) fn source_fingerprint(dir: &std::path::Path) -> u64 {
    let mut files = Vec::new();
    let mut stack = vec![(dir.to_path_buf(), 0u8)];
    while let Some((d, depth)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let path = e.path();
            let Ok(meta) = e.metadata() else { continue };
            let name = e.file_name().to_string_lossy().into_owned();
            if meta.is_dir() {
                if depth < 3 && !SKIP_DIRS.contains(&name.as_str()) {
                    stack.push((path, depth + 1));
                }
            } else if meta.len() <= FINGERPRINT_FILE_BYTES
                && path.extension().and_then(|x| x.to_str()).is_some_and(|x| SOURCE_EXTS.contains(&x.to_ascii_lowercase().as_str()))
            {
                files.push(path);
            }
        }
    }
    files.sort();
    files.truncate(FINGERPRINT_FILES);
    let mut h = DefaultHasher::new();
    for f in files {
        f.hash(&mut h);
        std::fs::read(&f).unwrap_or_default().hash(&mut h);
    }
    h.finish()
}

// ---------------------------------------------------------------- the state machine

/// Why the loop gave up; once set it never clears.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Stop {
    /// The nudge cap was reached.
    Cap,
    /// The time budget is nearly used up.
    Budget,
    /// Two attempts in a row left the sources and the failure as they were.
    NoChange,
    /// A nudge was sent for this very check result and the model did nothing with it.
    Ignored,
    /// The model was asked once to re-run the check after editing and did not.
    NoRerun,
    /// The check could not run, was nudged about once, and still cannot.
    Unavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Phase {
    /// No check has run yet.
    Idle,
    /// The chased check failed or could not run.
    Failed,
    /// The chased check passed, then files were edited: the pass no longer covers the work.
    Unverified,
    Passed,
    Exhausted(Stop),
}

/// The check being chased: its identity, latest result, and the state of the work when it ran.
struct Chased {
    key: String,
    outcome: Outcome,
    /// `source_fingerprint` when the check ran.
    tree: u64,
    /// Nudges sent before this result arrived: a later nudge was sent for *this* result.
    nudges_before: u32,
    /// Fewer things fail than in the previous run of the same check.
    improved: bool,
}

#[derive(Debug)]
struct Attempt {
    edits: Vec<String>,
    verdict: Verdict,
    summary: String,
}

/// The wording of a nudge follows the evidence; the count only bounds the loop.
enum Nudge {
    Hypotheses,
    Progress,
    Diagnose,
    Search,
    Final,
    Rerun,
    Unblock,
}

#[derive(Default)]
pub(super) struct Persist {
    enabled: bool,
    cap: u32,
    budget_stop: f64,
    task_checks: Vec<String>,
    chased: Option<Chased>,
    exhausted: Option<Stop>,
    log: Vec<Attempt>,
    /// Files edited since the last check ran.
    edits: Vec<String>,
    nudges: u32,
    rerun_asks: u32,
    stalled: u32,
    seen: HashSet<(u64, u64)>,
    budget_used: Option<f64>,
    last_cmd: String,
}

fn command_key(command: &str) -> String {
    command.rsplit("&&").next().unwrap_or(command).split_whitespace().collect::<Vec<_>>().join(" ")
}

impl Persist {
    pub(super) fn new(enabled: bool, cap: u32, budget_stop: f64, task: Option<&str>) -> Self {
        Self {
            enabled,
            cap: cap.max(1),
            budget_stop: budget_stop.clamp(0.1, 0.99),
            task_checks: if enabled { task.map(task_check_commands).unwrap_or_default() } else { Vec::new() },
            ..Self::default()
        }
    }

    pub(super) fn enabled(&self) -> bool {
        self.enabled
    }

    /// The task named `command` as its check.
    pub(super) fn is_named(&self, command: &str) -> bool {
        is_named_check(&self.task_checks, command)
    }

    /// Persist owns the failed-check follow-up for this turn, so the older single nudge stays quiet.
    pub(super) fn owns_failed_check(&self) -> bool {
        self.enabled && self.chased.is_some()
    }

    pub(super) fn set_budget_used(&mut self, fraction: Option<f64>) {
        self.budget_used = fraction;
    }

    pub(super) fn phase(&self) -> Phase {
        if let Some(stop) = self.exhausted {
            return Phase::Exhausted(stop);
        }
        match &self.chased {
            None => Phase::Idle,
            Some(c) if c.outcome.verdict == Verdict::Pass => if self.edits.is_empty() { Phase::Passed } else { Phase::Unverified },
            Some(_) => Phase::Failed,
        }
    }

    /// Event: an edit tool changed `path`.
    pub(super) fn note_edit(&mut self, path: &str) {
        if !self.edits.iter().any(|p| p == path) && self.edits.len() < 3 {
            self.edits.push(path.to_string());
        }
    }

    /// Event: a check ran. `tree` is the source fingerprint of the work it ran against.
    pub(super) fn check_ran(&mut self, command: &str, source: Source, exit_code: i64, output: &str, tree: u64) {
        if !self.enabled {
            return;
        }
        let Some(outcome) = classify(source, exit_code, output) else { return };
        let key = command_key(command);
        // A pass of some other command does not clear the failure being chased.
        if let Some(c) = &self.chased {
            if c.outcome.verdict != Verdict::Pass && outcome.verdict == Verdict::Pass && c.key != key {
                return;
            }
        }
        let improved = self.chased.as_ref().filter(|c| c.key == key).is_some_and(|c| match (c.outcome.fail_count, outcome.fail_count) {
            (Some(before), Some(now)) => now < before,
            (Some(_), None) => outcome.verdict == Verdict::Pass,
            _ => false,
        });
        self.last_cmd = key.chars().take(80).collect();
        if self.log.len() >= MAX_LOG {
            self.log.remove(0);
        }
        self.log.push(Attempt { edits: std::mem::take(&mut self.edits), verdict: outcome.verdict, summary: outcome.summary.clone() });
        self.rerun_asks = 0;
        self.chased = Some(Chased { key, outcome, tree, nudges_before: self.nudges, improved });
    }

    /// Event: the model is about to end its turn with text. The nudge that keeps it going, or `None`.
    /// `tree_now` computes the source fingerprint (only asked for when a check is being chased).
    pub(super) fn stop_attempt(&mut self, tree_now: impl FnOnce() -> u64) -> Option<String> {
        let phase = self.phase();
        if !self.enabled || matches!(phase, Phase::Idle | Phase::Passed | Phase::Exhausted(_)) {
            return None;
        }
        if self.budget_used.is_some_and(|f| f >= self.budget_stop) {
            return self.exhaust(Stop::Budget);
        }
        if self.nudges >= self.cap {
            return self.exhaust(Stop::Cap);
        }
        if phase == Phase::Unverified {
            return self.ask_rerun();
        }
        let chased = self.chased.as_ref()?;
        if chased.outcome.verdict == Verdict::Unavailable {
            return if self.nudges >= 1 { self.exhaust(Stop::Unavailable) } else { self.send(Nudge::Unblock) };
        }
        let (tree, signature, improved, nudged_for_this) = (chased.tree, chased.outcome.signature, chased.improved, self.nudges > chased.nudges_before);
        if !self.edits.is_empty() || tree_now() != tree {
            return self.ask_rerun();
        }
        if nudged_for_this {
            return self.exhaust(Stop::Ignored);
        }
        if !self.seen.insert((tree, signature)) {
            self.stalled += 1;
        } else {
            self.stalled = 0;
        }
        if improved {
            self.stalled = 0;
        }
        if self.stalled >= MAX_STALLED {
            return self.exhaust(Stop::NoChange);
        }
        let kind = if self.nudges + 1 >= self.cap {
            Nudge::Final
        } else if improved {
            Nudge::Progress
        } else if self.nudges == 0 {
            Nudge::Hypotheses
        } else if self.nudges == 1 || self.stalled >= 1 {
            Nudge::Diagnose
        } else {
            Nudge::Search
        };
        self.send(kind)
    }

    fn exhaust(&mut self, stop: Stop) -> Option<String> {
        self.exhausted = Some(stop);
        None
    }

    /// Files changed since the last check ran: ask for the re-run once.
    fn ask_rerun(&mut self) -> Option<String> {
        if self.rerun_asks >= 1 {
            return self.exhaust(Stop::NoRerun);
        }
        self.rerun_asks += 1;
        self.send(Nudge::Rerun)
    }

    fn send(&mut self, kind: Nudge) -> Option<String> {
        self.nudges += 1;
        Some(self.text(kind))
    }

    fn log_lines(&self) -> String {
        let shown = self.log.len().saturating_sub(SHOWN_LOG);
        self.log[shown..]
            .iter()
            .enumerate()
            .map(|(i, a)| {
                let edits = if a.edits.is_empty() { "no file edits".to_string() } else { format!("edited {}", a.edits.join(", ")) };
                let v = if a.verdict == Verdict::Pass { "PASS" } else { "FAIL" };
                format!("{}. {edits} -> {v}: {}", shown + i + 1, a.summary)
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn text(&self, kind: Nudge) -> String {
        let log = self.log_lines();
        let log = if log.is_empty() { String::new() } else { format!("\nAttempts so far:\n{log}") };
        let cmd = if self.last_cmd.is_empty() { "your check".to_string() } else { format!("`{}`", self.last_cmd) };
        let left = self.budget_used.map_or(String::new(), |f| format!(" About {}% of the time budget remains.", ((1.0 - f) * 100.0).round().clamp(0.0, 100.0)));
        let body = match kind {
            Nudge::Hypotheses => format!("Your check {cmd} still fails.{left} Do not finish. Re-read its output for anything specific (which case, value, line). Write 2-3 hypotheses, change ONE thing, re-run the check.{log}"),
            Nudge::Progress => format!("{cmd} has fewer failures than the run before, so the last change helped.{left} Keep going the same way: find what still fails, change one thing, re-run.{log}"),
            Nudge::Diagnose => format!("{cmd} still fails and the last attempt did not change why. Do not repeat a logged attempt. A check that gives too little detail is not a blocker: build your own diagnostic. Run on a tiny input, print intermediate values or compare element by element against a simple reference, and find where it first diverges. Fix that, re-run.{log}"),
            Nudge::Search => format!("List the small discrete choices your solution depends on (orderings, layouts, index or offset conventions, types, signs, boundaries). Try untried combinations one at a time, cheapest first, logging each result. Re-read the task for a misread requirement. Keep the closest version in place.{log}"),
            Nudge::Final => format!("Final attempt: apply the most promising untried idea and re-run {cmd}. Make sure the saved version is the one that came closest, then report plainly what passes and what fails.{log}"),
            Nudge::Rerun => format!("You changed files after the last run of {cmd}. Run it again and act on its result before you finish.{log}"),
            Nudge::Unblock => format!("{cmd} could not run. Find out what blocks it and fix that, or write your own small check of the requirements and run it, then fix what it shows. Do not finish without a check that ran.{log}"),
        };
        format!("<system-reminder>{body}</system-reminder>")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FAIL: &str = r#"{"call_acc": true, "exec_acc": false}"#;
    const OK: &str = r#"{"call_acc": true, "exec_acc": true}"#;

    fn machine() -> Persist {
        Persist::new(true, 6, 0.85, Some("Fix it. Test with ./eval before you finish."))
    }

    fn run(p: &mut Persist, out: &str, tree: u64) {
        p.check_ran("./eval", Source::Named, 0, out, tree);
    }

    fn stop(p: &mut Persist, tree: u64) -> Option<String> {
        p.stop_attempt(|| tree)
    }

    // ---- recognising the check

    #[test]
    fn task_named_checks_are_extracted_conservatively() {
        assert_eq!(task_check_commands("Write the code. Test with ./eval before you finish."), vec!["./eval"]);
        assert_eq!(task_check_commands("Check it with `make check` please"), vec!["make check"]);
        assert_eq!(task_check_commands("verify with python3 run_checks.py, then stop"), vec!["python3 run_checks.py"]);
        assert_eq!(task_check_commands("Run the tests with scripts/test.sh."), vec!["scripts/test.sh"]);
        for none in ["test with care and check with your eyes", "verify with the user", ""] {
            assert!(task_check_commands(none).is_empty(), "{none}");
        }
    }

    #[test]
    fn a_named_check_matches_word_for_word_through_cd_and_arguments() {
        let t = vec!["./eval".to_string()];
        assert!(is_named_check(&t, "cd /work/task && ./eval --fast"));
        assert!(is_named_check(&t, "./eval"));
        assert!(!is_named_check(&t, "./evaluate-other"));
        assert!(!is_named_check(&t, "ls -la"));
        assert!(is_named_check(&["make check".to_string()], "make check"));
        assert!(!is_named_check(&["make check".to_string()], "make build"));
    }

    #[test]
    fn only_local_programs_can_be_a_check_by_their_report() {
        for yes in ["./eval", "scripts/run.sh --x", "python3 bench.py --n 3", "cd w && ./eval"] {
            assert!(runs_local_program(yes), "{yes}");
        }
        for no in ["curl -s https://x/y", "pip install foo", "cat results.json", "python3 -c 'print(1)'", "ls"] {
            assert!(!runs_local_program(no), "{no}");
        }
    }

    // ---- reading a result (the precedence table)

    fn verdict(source: Source, exit: i64, out: &str) -> Option<Verdict> {
        classify(source, exit, out).map(|o| o.verdict)
    }

    #[test]
    fn a_named_check_is_judged_by_exit_code_and_its_printed_report() {
        assert_eq!(verdict(Source::Named, 1, "plain output"), Some(Verdict::Fail));
        assert_eq!(verdict(Source::Named, 0, "done"), Some(Verdict::Pass));
        assert_eq!(verdict(Source::Named, 0, FAIL), Some(Verdict::Fail), "exits 0 but reports a failure");
        assert_eq!(verdict(Source::Named, 0, OK), Some(Verdict::Pass));
        assert_eq!(verdict(Source::Named, 0, "AssertionError: tensors differ"), Some(Verdict::Fail));
        assert_eq!(verdict(Source::Named, 0, r#"{"failed": 3, "passed": 1}"#), Some(Verdict::Fail));
        assert_eq!(verdict(Source::Named, 0, r#"{"failed": 0, "passed": 4}"#), Some(Verdict::Pass));
    }

    #[test]
    fn a_runner_is_judged_by_its_exit_code_alone() {
        assert_eq!(verdict(Source::Runner, 0, "warning: FAILED to load plugin, mismatch ignored"), Some(Verdict::Pass));
        assert_eq!(verdict(Source::Runner, 1, "2 failed, 8 passed"), Some(Verdict::Fail));
        assert_eq!(verdict(Source::Runner, 124, "no output"), Some(Verdict::Fail), "a timeout is a failure");
    }

    #[test]
    fn a_report_only_command_is_judged_by_booleans_and_counts_never_scores() {
        assert_eq!(verdict(Source::Report, 0, FAIL), Some(Verdict::Fail));
        assert_eq!(verdict(Source::Report, 0, OK), Some(Verdict::Pass));
        assert_eq!(verdict(Source::Report, 0, r#"{"accuracy": 0.95, "success_rate": 0.9}"#), None, "a score is not a verdict");
        assert_eq!(verdict(Source::Report, 1, "boom"), None, "no report: says nothing about the work");
    }

    #[test]
    fn a_missing_runner_is_unavailable_but_a_failure_that_mentions_a_file_is_not() {
        assert_eq!(verdict(Source::Runner, 127, ""), Some(Verdict::Unavailable));
        assert_eq!(verdict(Source::Named, 126, ""), Some(Verdict::Unavailable));
        assert_eq!(verdict(Source::Named, 1, "sh: pytest: command not found"), Some(Verdict::Unavailable));
        assert_eq!(verdict(Source::Named, 1, "CUDA is not available"), Some(Verdict::Unavailable));
        assert_eq!(verdict(Source::Named, 1, "FileNotFoundError: no such file or directory: 'in.txt'"), Some(Verdict::Fail));
        assert_eq!(verdict(Source::Named, 1, r#"{"exec_acc": false} permission denied"#), Some(Verdict::Fail));
    }

    #[test]
    fn the_failure_signature_ignores_numbers_and_follows_what_failed() {
        let sig = |s, e, o: &str| classify(s, e, o).unwrap().signature;
        assert_eq!(sig(Source::Runner, 1, "error at 0.31 after 12ms"), sig(Source::Runner, 1, "error at 0.002 after 97ms"));
        assert_ne!(sig(Source::Runner, 1, "error at line"), sig(Source::Runner, 1, "other problem"));
        assert_eq!(sig(Source::Named, 0, r#"{"a_acc": false, "t": 1}"#), sig(Source::Named, 0, r#"{"a_acc": false, "t": 9}"#));
        assert_ne!(sig(Source::Named, 0, r#"{"a_acc": false}"#), sig(Source::Named, 0, r#"{"b_acc": false}"#));
        assert_eq!(classify(Source::Runner, 1, "3 failed, 1 passed").unwrap().fail_count, Some(3));
    }

    #[test]
    fn failure_summary_picks_the_telling_line_and_is_bounded() {
        assert_eq!(failure_summary("setup ok\nAssertionError: x != y\nbye"), "AssertionError: x != y");
        assert!(failure_summary(&"e".repeat(500)).ends_with("..."));
        assert_eq!(failure_summary(""), "no output");
    }

    #[test]
    fn the_source_fingerprint_follows_source_contents_not_outputs() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("k.py"), "a").unwrap();
        let before = source_fingerprint(d.path());
        std::fs::write(d.path().join("results.json"), "x").unwrap();
        std::fs::create_dir(d.path().join("__pycache__")).unwrap();
        std::fs::write(d.path().join("__pycache__/k.pyc"), "x").unwrap();
        assert_eq!(source_fingerprint(d.path()), before, "outputs and caches do not count");
        std::fs::write(d.path().join("k.py"), "b").unwrap();
        assert_ne!(source_fingerprint(d.path()), before);
        std::fs::write(d.path().join("k.py"), "a").unwrap();
        assert_eq!(source_fingerprint(d.path()), before, "a revert is the same state");
    }

    // ---- the transitions

    #[test]
    fn idle_until_a_check_runs_then_the_result_decides() {
        let mut p = machine();
        assert_eq!(p.phase(), Phase::Idle);
        assert_eq!(stop(&mut p, 1), None, "no check, nothing to persist");
        run(&mut p, FAIL, 1);
        assert_eq!(p.phase(), Phase::Failed);
        let mut q = machine();
        run(&mut q, OK, 1);
        assert_eq!(q.phase(), Phase::Passed);
        assert_eq!(stop(&mut q, 1), None);
    }

    #[test]
    fn a_failed_check_is_nudged_with_a_log_and_stays_failed_until_it_runs_again() {
        let mut p = machine();
        run(&mut p, FAIL, 1);
        let n = stop(&mut p, 1).expect("nudge");
        assert!(n.contains("still fails") && n.contains("hypotheses") && n.contains("Attempts so far"), "{n}");
        assert_eq!(p.phase(), Phase::Failed);
    }

    #[test]
    fn failed_then_pass_is_passed_and_ends_the_loop() {
        let mut p = machine();
        run(&mut p, FAIL, 1);
        stop(&mut p, 1).unwrap();
        p.note_edit("k.py");
        run(&mut p, OK, 2);
        assert_eq!(p.phase(), Phase::Passed);
        assert_eq!(stop(&mut p, 2), None);
    }

    #[test]
    fn a_pass_of_a_different_command_does_not_clear_the_failure() {
        let mut p = machine();
        run(&mut p, FAIL, 1);
        p.check_ran("python -m pytest tests/one.py", Source::Runner, 0, "1 passed", 1);
        assert_eq!(p.phase(), Phase::Failed);
        assert!(stop(&mut p, 1).is_some());
    }

    #[test]
    fn edits_after_a_pass_are_unverified_and_get_one_rerun_request() {
        let mut p = machine();
        run(&mut p, OK, 1);
        p.note_edit("k.py");
        assert_eq!(p.phase(), Phase::Unverified);
        assert!(stop(&mut p, 2).expect("rerun").contains("Run it again"));
        assert_eq!(stop(&mut p, 2), None);
        assert_eq!(p.phase(), Phase::Exhausted(Stop::NoRerun));
    }

    #[test]
    fn edits_after_a_failure_get_one_rerun_request_not_another_still_fails() {
        let mut p = machine();
        run(&mut p, FAIL, 1);
        p.note_edit("k.py");
        assert!(stop(&mut p, 2).expect("rerun").contains("Run it again"));
        assert_eq!(stop(&mut p, 2), None);
        assert_eq!(p.phase(), Phase::Exhausted(Stop::NoRerun));
    }

    #[test]
    fn a_change_made_through_a_shell_is_seen_in_the_fingerprint() {
        let mut p = machine();
        run(&mut p, FAIL, 1);
        assert!(stop(&mut p, 2).expect("rerun").contains("Run it again"), "the sources changed without an edit tool");
    }

    #[test]
    fn a_nudge_that_changes_nothing_is_ignored_once_not_repeated() {
        let mut p = machine();
        run(&mut p, FAIL, 1);
        stop(&mut p, 1).unwrap();
        assert_eq!(stop(&mut p, 1), None);
        assert_eq!(p.phase(), Phase::Exhausted(Stop::Ignored));
    }

    #[test]
    fn the_same_work_and_the_same_failure_twice_is_no_change() {
        let mut p = machine();
        run(&mut p, FAIL, 1);
        assert!(stop(&mut p, 1).is_some());
        run(&mut p, FAIL, 1);
        assert!(stop(&mut p, 1).is_some(), "first repeat is allowed");
        run(&mut p, FAIL, 1);
        assert_eq!(stop(&mut p, 1), None);
        assert_eq!(p.phase(), Phase::Exhausted(Stop::NoChange));
    }

    #[test]
    fn a_revert_to_an_earlier_state_with_the_same_failure_is_no_change() {
        let mut p = machine();
        run(&mut p, FAIL, 1);
        assert!(stop(&mut p, 1).is_some());
        p.note_edit("k.py");
        run(&mut p, FAIL, 2);
        assert!(stop(&mut p, 2).is_some());
        p.note_edit("k.py");
        run(&mut p, FAIL, 1);
        assert!(stop(&mut p, 1).is_some(), "stalled once");
        p.note_edit("k.py");
        run(&mut p, FAIL, 2);
        assert_eq!(stop(&mut p, 2), None);
        assert_eq!(p.phase(), Phase::Exhausted(Stop::NoChange));
    }

    #[test]
    fn fewer_failures_is_progress_and_resets_the_stall_count() {
        let mut p = machine();
        p.check_ran("./eval", Source::Runner, 1, "4 failed, 2 passed", 1);
        assert!(stop(&mut p, 1).is_some());
        p.note_edit("k.py");
        p.check_ran("./eval", Source::Runner, 1, "2 failed, 4 passed", 2);
        let n = stop(&mut p, 2).expect("nudge");
        assert!(n.contains("fewer failures"), "{n}");
    }

    #[test]
    fn the_cap_ends_the_loop_and_the_last_nudge_is_the_final_attempt() {
        let mut p = Persist::new(true, 3, 0.85, Some("Test with ./eval"));
        let mut texts = Vec::new();
        for t in 1..=3u64 {
            p.check_ran("./eval", Source::Named, 0, &format!(r#"{{"k{t}_acc": false}}"#), t);
            texts.push(stop(&mut p, t).expect("nudge"));
        }
        assert!(texts[2].contains("Final attempt"), "{}", texts[2]);
        p.check_ran("./eval", Source::Named, 0, FAIL, 9);
        assert_eq!(stop(&mut p, 9), None);
        assert_eq!(p.phase(), Phase::Exhausted(Stop::Cap));
    }

    #[test]
    fn the_wording_follows_the_evidence_not_the_count() {
        let mut p = Persist::new(true, 8, 0.85, Some("Test with ./eval"));
        let mut texts = Vec::new();
        for t in 1..=4u64 {
            p.check_ran("./eval", Source::Named, 0, &format!(r#"{{"k{t}_acc": false}}"#), t);
            texts.push(stop(&mut p, t).expect("nudge"));
        }
        assert!(texts[0].contains("hypotheses"));
        assert!(texts[1].contains("diagnostic"));
        assert!(texts[2].contains("discrete choices") && texts[3].contains("discrete choices"));
    }

    #[test]
    fn a_check_that_cannot_run_gets_one_nudge_then_gives_up() {
        let mut p = machine();
        p.check_ran("./eval", Source::Named, 127, "", 1);
        assert!(stop(&mut p, 1).expect("nudge").contains("could not run"));
        p.check_ran("./eval", Source::Named, 127, "", 1);
        assert_eq!(stop(&mut p, 1), None);
        assert_eq!(p.phase(), Phase::Exhausted(Stop::Unavailable));
    }

    #[test]
    fn the_time_budget_ends_it_and_a_pass_still_reads_as_passed_elsewhere() {
        let mut p = machine();
        run(&mut p, FAIL, 1);
        p.set_budget_used(Some(0.9));
        assert_eq!(stop(&mut p, 1), None);
        assert_eq!(p.phase(), Phase::Exhausted(Stop::Budget));
        assert!(p.owns_failed_check(), "the older nudge stays quiet after exhaustion");
        let mut q = machine();
        q.set_budget_used(Some(0.9));
        run(&mut q, OK, 1);
        assert_eq!(stop(&mut q, 1), None);
        assert_eq!(q.phase(), Phase::Passed, "a pass is never turned into an exhaustion");
    }

    #[test]
    fn exhaustion_is_a_latch() {
        let mut p = machine();
        run(&mut p, FAIL, 1);
        p.set_budget_used(Some(0.95));
        assert_eq!(stop(&mut p, 1), None);
        p.set_budget_used(Some(0.0));
        p.note_edit("k.py");
        run(&mut p, FAIL, 2);
        assert_eq!(stop(&mut p, 2), None);
        assert_eq!(p.phase(), Phase::Exhausted(Stop::Budget));
    }

    #[test]
    fn disabled_never_acts() {
        let mut p = Persist::new(false, 6, 0.85, Some("Test with ./eval"));
        run(&mut p, FAIL, 1);
        assert_eq!(p.phase(), Phase::Idle);
        assert_eq!(stop(&mut p, 1), None);
        assert!(!p.owns_failed_check());
    }

    #[test]
    fn the_attempt_log_is_bounded_and_reports_edits() {
        let mut p = Persist::new(true, 50, 0.85, Some("Test with ./eval"));
        for t in 0..30u64 {
            p.note_edit("k.py");
            p.check_ran("./eval", Source::Named, 0, &format!(r#"{{"k{t}_acc": false}}"#), t);
        }
        assert_eq!(p.log.len(), MAX_LOG);
        let n = stop(&mut p, 29).expect("nudge");
        assert_eq!(n.lines().filter(|l| l.contains("-> FAIL")).count(), SHOWN_LOG);
        assert!(n.contains("edited k.py"));
    }
}
