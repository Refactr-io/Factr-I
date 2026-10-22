//! Persist after a failed check (`persistence.enabled`, off by default).
//!
//! A headless turn that is about to end while its latest verification failed (or whose last reply admits
//! the work is not done) is not ended: it gets an escalating, bounded nudge instead. Stops when the check
//! passes, after the nudge cap, near the end of the time budget, or when two attempts in a row change
//! nothing. General behaviour: no task, benchmark or tool specific text anywhere.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::OnceLock;

use regex::Regex;

pub(super) const DEFAULT_MAX_NUDGES: u32 = 6;
pub(super) const DEFAULT_BUDGET_STOP: f64 = 0.85;
const MAX_LOG: usize = 12;
const SHOWN_LOG: usize = 6;
const MAX_TASK_CHECKS: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Verdict {
    Pass,
    Fail,
    /// The check could not run at all (missing tool, no GPU, timeout): not a verdict on the work.
    Unavailable,
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

fn normalized(command: &str) -> String {
    // `cd dir && ./eval x` -> `./eval x`; collapse whitespace.
    let c = command.rsplit("&&").next().unwrap_or(command);
    c.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(super) fn is_named_check(task_checks: &[String], command: &str) -> bool {
    let c = normalized(command);
    task_checks.iter().any(|t| c.contains(t.as_str()))
}

// ---------------------------------------------------------------- the pass/fail classifier

fn kv_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#""([A-Za-z0-9_\-\. ]{1,40})"\s*:\s*(true|false|-?[0-9]+(?:\.[0-9]+)?(?:[eE][-+]?[0-9]+)?)"#).expect("static"))
}

const OK_KEYS: [&str; 10] = ["acc", "pass", "ok", "correct", "success", "valid", "match", "equal", "allclose", "verified"];
const COUNT_FAIL_KEYS: [&str; 7] = ["failed", "failures", "failure", "errors", "mismatch", "mismatches", "num_failed"];

/// (failing signals, passing signals) from `"key": value` pairs in a JSON-looking report.
fn json_signals(output: &str) -> (u32, u32) {
    let (mut fails, mut passes) = (0u32, 0u32);
    for cap in kv_re().captures_iter(output) {
        let key = cap[1].to_ascii_lowercase();
        let val = &cap[2];
        if COUNT_FAIL_KEYS.iter().any(|k| key == *k || key.ends_with(&format!("_{k}")) || key.starts_with(&format!("{k}_"))) {
            match val.parse::<f64>() {
                Ok(n) if n > 0.0 => fails += 1,
                Ok(_) => passes += 1,
                Err(_) => {}
            }
            continue;
        }
        if !OK_KEYS.iter().any(|k| key.contains(k)) {
            continue;
        }
        match val {
            "false" => fails += 1,
            "true" => passes += 1,
            n => match n.parse::<f64>() {
                Ok(x) if x >= 1.0 => passes += 1,
                Ok(_) => fails += 1,
                Err(_) => {}
            },
        }
    }
    (fails, passes)
}

fn has_nonzero_count(out: &str, word: &str) -> bool {
    let words: Vec<&str> = out.split(|c: char| c.is_whitespace() || matches!(c, ',' | ';' | '(' | ')' | '=' | ':')).filter(|w| !w.is_empty()).collect();
    words.iter().enumerate().any(|(i, w)| {
        if !w.eq_ignore_ascii_case(word) {
            return false;
        }
        // "3 failed" counts by the number before it; "failed: 3" / "failures=3" by the one after.
        match i.checked_sub(1).and_then(|p| words[p].parse::<u64>().ok()) {
            Some(n) => n > 0,
            None => words.get(i + 1).and_then(|n| n.parse::<u64>().ok()).is_some_and(|n| n > 0),
        }
    })
}

const TEXT_FAIL: [&str; 9] = [
    "assertionerror", "does not match", "do not match", "mismatch", "not equal", "not close", "traceback (most recent call last)",
    "result: fail", "status: fail",
];

fn text_signals(output: &str) -> (bool, bool) {
    let lower = output.to_ascii_lowercase();
    let fail = TEXT_FAIL.iter().any(|m| lower.contains(m))
        || output.contains("FAILED")
        || output.lines().any(|l| l.trim_start().starts_with("--- FAIL") || l.trim_start().starts_with("FAIL:") || l.trim_start().starts_with("✗"))
        || has_nonzero_count(output, "failed")
        || has_nonzero_count(output, "failures");
    let pass = lower.contains("all tests passed") || lower.contains("result: pass") || lower.contains("status: pass") || output.contains("PASSED") || has_nonzero_count(output, "passed");
    (fail, pass)
}

/// Verdict of a check run from its exit status and output, and whether the output is a recognisable
/// pass/fail report (which makes any command that printed it count as a check).
const UNAVAILABLE: [&str; 9] = [
    "command not found", "no such file or directory", "permission denied", "no cuda", "cuda error", "cuda is not available",
    "no gpu", "modulenotfounderror", "no module named",
];
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

/// Whether the output carries structured pass/fail fields (`"exec_acc": false`, `"failed": 2`, ...).
pub(super) fn json_recognised(output: &str) -> bool {
    let (f, p) = json_signals(tail(output));
    f + p > 0
}

pub(super) fn classify(exit_code: i64, output: &str) -> (Verdict, bool) {
    let output = tail(output);
    let lower = output.to_ascii_lowercase();
    if exit_code == 127 || exit_code == 124 || (exit_code != 0 && UNAVAILABLE.iter().any(|m| lower.contains(m))) {
        return (Verdict::Unavailable, false);
    }
    let (jf, jp) = json_signals(output);
    let (tf, tp) = text_signals(output);
    let recognised = jf + jp > 0 || tf || tp;
    let failing = jf > 0 || tf || exit_code != 0;
    (if failing { Verdict::Fail } else { Verdict::Pass }, recognised)
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

// ---------------------------------------------------------------- the reply admits it is not done

const ADMIT_NEG: [&str; 22] = [
    "wasn't able", "was not able", "wasn't successful", "unable to", "could not", "couldn't", "cannot", "can't", "did not",
    "didn't", "does not", "doesn't", "do not", "not passing", "not validated", "not verified", "unverified", "not fully",
    "not yet", "no longer", "remaining", "still",
];
const ADMIT_OBJ: [&str; 22] = [
    "check", "test", "eval", "result", "output", "pass", "match", "correct", "accuracy", "tolerance", "mismatch", "differ",
    "fail", "solution", "kernel", "requirement", "expected", "validate", "verify", "diagnos", "feedback", "numeric",
];
// Self-sufficient phrases (negation and object in one).
const ADMIT_PHRASES: [&str; 14] = [
    "still differs", "still differ", "still fails", "still failing", "still mismatch", "does not pass", "doesn't pass",
    "did not pass", "didn't pass", "not passing", "not validated", "no per-element", "no further information", "out of ideas",
];
const HONEST_LIMIT: [&str; 6] = ["unreadable", "inaccessible", "cannot read", "can't read", "could not read", "couldn't read"];

/// The last paragraph of `text` says the work is not done or does not pass. Broader than the existing
/// shortfall detector, and only used for the persist loop.
pub(super) fn admits_failure(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    let last = lower.trim().rsplit("\n\n").next().unwrap_or("");
    if HONEST_LIMIT.iter().any(|p| last.contains(p)) {
        return false;
    }
    if ADMIT_PHRASES.iter().any(|p| last.contains(p)) {
        return true;
    }
    last.split(['.', '!', '?', '\n']).any(|s| ADMIT_NEG.iter().any(|n| s.contains(n)) && ADMIT_OBJ.iter().any(|o| s.contains(o)))
}

// ---------------------------------------------------------------- state

#[derive(Debug)]
struct Attempt {
    edits: Vec<String>,
    verdict: Verdict,
    summary: String,
}

#[derive(Default)]
pub(super) struct Persist {
    pub(super) enabled: bool,
    cap: u32,
    budget_stop: f64,
    pub(super) task_checks: Vec<String>,
    /// Verdict of the latest check run and the failure output's hash.
    last: Option<(Verdict, u64)>,
    edits_since_check: Vec<String>,
    log: Vec<Attempt>,
    nudges: u32,
    last_fingerprint: Option<u64>,
    stalled: u32,
    budget_used: Option<f64>,
    done: bool,
    started: Option<std::time::Instant>,
    max_seconds: Option<f64>,
    seen: std::collections::HashSet<u64>,
    last_cmd: String,
    admit_nudges: u32,
    /// `StopNudge::progress` at the last nudge: a nudge answered with no tool call is not repeated.
    progress_at_nudge: Option<u32>,
}

pub(super) fn hash_text(s: &str) -> u64 {
    let mut h = DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

impl Persist {
    pub(super) fn new(enabled: bool, cap: u32, budget_stop: f64, max_seconds: Option<f64>, task: Option<&str>) -> Self {
        Self {
            enabled,
            started: Some(std::time::Instant::now()),
            max_seconds,
            cap: cap.max(1),
            budget_stop: budget_stop.clamp(0.1, 0.99),
            task_checks: if enabled { task.map(task_check_commands).unwrap_or_default() } else { Vec::new() },
            ..Self::default()
        }
    }

    pub(super) fn set_budget_used(&mut self, fraction: Option<f64>) {
        self.budget_used = fraction;
    }

    /// An edit tool changed `path` since the last check.
    pub(super) fn note_edit(&mut self, path: &str) {
        if !self.edits_since_check.iter().any(|p| p == path) && self.edits_since_check.len() < 3 {
            self.edits_since_check.push(path.to_string());
        }
    }

    /// A check ran: record its verdict and attempt.
    pub(super) fn record_check(&mut self, command: &str, exit_code: i64, output: &str) {
        let (verdict, _) = classify(exit_code, output);
        let summary = match verdict {
            Verdict::Fail => failure_summary(tail(output)),
            Verdict::Pass => "passed".to_string(),
            Verdict::Unavailable => "could not run".to_string(),
        };
        self.last_cmd = command.chars().take(80).collect();
        // The failure text with timings, floats and hex ids dropped (they vary run to run).
        self.last = Some((verdict, if verdict == Verdict::Pass { 0 } else { super::stop_nudge::hash_str(tail(output)) }));
        if self.log.len() >= MAX_LOG {
            self.log.remove(0);
        }
        self.log.push(Attempt { edits: std::mem::take(&mut self.edits_since_check), verdict, summary });
        self.done = verdict == Verdict::Pass;
    }

    /// The latest recorded check failed.
    pub(super) fn last_failed(&self) -> bool {
        matches!(self.last, Some((Verdict::Fail, _)))
    }

    pub(super) fn passed(&self) -> bool {
        matches!(self.last, Some((Verdict::Pass, _)))
    }

    /// Decide whether to keep going at a text-only stop. `fingerprint` is a hash of the work tree state
    /// (it is only computed when asked for, hence the closure). Returns the nudge to send.
    pub(super) fn decide(&mut self, reply: &str, progress: u32, edited: bool, fingerprint: impl FnOnce() -> u64) -> Option<String> {
        if !self.enabled || self.nudges >= self.cap {
            return None;
        }
        if self.budget_used.is_some_and(|f| f >= self.budget_stop) {
            return None;
        }
        if self.max_seconds.zip(self.started).is_some_and(|(max, t)| t.elapsed().as_secs_f64() >= max) {
            return None;
        }
        // A nudge that was answered with no tool call at all is not repeated.
        if self.progress_at_nudge == Some(progress) {
            return None;
        }
        // A pass after the last edit ends it, whatever the reply says.
        if self.passed() && self.edits_since_check.is_empty() {
            return None;
        }
        let unavailable = matches!(self.last, Some((Verdict::Unavailable, _)));
        // Without a failing check the reply's own admission counts, but only after edits and at most twice.
        let admitted = !self.last_failed() && !unavailable && !self.passed() && edited && self.admit_nudges < 2 && admits_failure(reply);
        if !(self.last_failed() || unavailable || admitted) {
            return None;
        }
        // The check could not run: one nudge to fix what blocks it or to build its own check, no more.
        if unavailable && self.nudges >= 1 {
            return None;
        }
        // The same work tree and the same failure as an earlier attempt (also after a revert) is no change.
        let fp = fingerprint() ^ self.last.map_or(0, |(_, h)| h);
        if self.seen.contains(&fp) {
            self.stalled += 1;
        } else {
            self.stalled = 0;
            if self.seen.len() < 64 {
                self.seen.insert(fp);
            }
        }
        // Two consecutive attempts that changed nothing: it is not going anywhere.
        if self.stalled >= 2 {
            return None;
        }
        self.progress_at_nudge = Some(progress);
        self.nudges += 1;
        if admitted {
            self.admit_nudges += 1;
        }
        Some(self.nudge_text(unavailable))
    }

    pub(super) fn nudges_sent(&self) -> u32 {
        self.nudges
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

    fn nudge_text(&self, unavailable: bool) -> String {
        let log = self.log_lines();
        let log = if log.is_empty() { String::new() } else { format!("\nAttempts so far:\n{log}") };
        let cmd = if self.last_cmd.is_empty() { "your check".to_string() } else { format!("`{}`", self.last_cmd) };
        let left = self.budget_used.map_or(String::new(), |f| format!(" and about {}% of the time budget remains, so continue", ((1.0 - f) * 100.0).round().clamp(0.0, 100.0)));
        let body = if unavailable {
            format!("{cmd} could not run. Find out what blocks it and fix that, or write your own small check of the requirements and run it, then fix what it shows. Do not finish without a check that ran.")
        } else {
            match (self.nudges, self.nudges >= self.cap) {
                (1, _) => format!("Your check {cmd} still fails{left}. Do not finish. Re-read its output for anything specific (which case, value, line). Write 2-3 hypotheses, change ONE thing, re-run the check.{log}"),
                (n, _) if n == 2 => format!("Still failing after {n} attempts. Do not repeat a logged attempt. A check that gives too little detail is not a blocker: build your own diagnostic. Run on a tiny input, print intermediate values or compare element by element against a simple reference, and find where it first diverges. Fix that, re-run.{log}"),
                (_, true) => format!("Final attempt: apply the most promising untried idea and re-run {cmd}. Make sure the saved version is the one that came closest, then report plainly what passes and what fails.{log}"),
                _ => format!("List the small discrete choices your solution depends on (orderings, layouts, index or offset conventions, types, signs, boundaries). Try untried combinations one at a time, cheapest first, logging each result. Re-read the task for a misread requirement. Keep the closest version in place.{log}"),
            }
        };
        format!("<system-reminder>{body}</system-reminder>")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_named_checks_are_extracted_conservatively() {
        assert_eq!(task_check_commands("Write the code. Test with ./eval before you finish."), vec!["./eval"]);
        assert_eq!(task_check_commands("Check it with `make check` please"), vec!["make check"]);
        assert_eq!(task_check_commands("verify with python3 run_checks.py, then stop"), vec!["python3 run_checks.py"]);
        assert_eq!(task_check_commands("Run the tests with scripts/test.sh."), vec!["scripts/test.sh"]);
        assert!(task_check_commands("test with care and check with your eyes").is_empty());
        assert!(task_check_commands("").is_empty());
    }

    #[test]
    fn named_checks_match_through_cd_and_arguments() {
        let t = vec!["./eval".to_string()];
        assert!(is_named_check(&t, "cd /work/task && ./eval --fast"));
        assert!(is_named_check(&t, "./eval"));
        assert!(!is_named_check(&t, "ls -la"));
    }

    #[test]
    fn classifier_reads_json_reports_runner_output_and_exit_codes() {
        assert_eq!(classify(0, r#"{"call_acc": false, "exec_acc": true}"#), (Verdict::Fail, true));
        assert_eq!(classify(0, r#"{"call_acc": true, "exec_acc": true}"#), (Verdict::Pass, true));
        assert_eq!(classify(0, r#"{"pass_rate": 0.0}"#), (Verdict::Fail, true));
        assert_eq!(classify(0, r#"{"failed": 3, "passed": 1}"#), (Verdict::Fail, true));
        assert_eq!(classify(0, r#"{"failed": 0, "passed": 4}"#), (Verdict::Pass, true));
        assert_eq!(classify(1, "plain output"), (Verdict::Fail, false));
        assert_eq!(classify(0, "done"), (Verdict::Pass, false));
        assert_eq!(classify(0, "2 failed, 8 passed in 0.3s"), (Verdict::Fail, true));
        assert_eq!(classify(0, "0 failed, 8 passed"), (Verdict::Pass, true));
        assert_eq!(classify(0, "AssertionError: tensors differ"), (Verdict::Fail, true));
        assert_eq!(classify(0, "max error 3e-3 mismatch at index 4"), (Verdict::Fail, true));
    }

    #[test]
    fn failure_summary_picks_the_telling_line_and_is_bounded() {
        assert_eq!(failure_summary("setup ok\nAssertionError: x != y\nbye"), "AssertionError: x != y");
        assert!(failure_summary(&"e".repeat(500)).ends_with("..."));
        assert_eq!(failure_summary(""), "no output");
    }

    #[test]
    fn admissions_cover_the_missing_phrasings_and_skip_honest_limits() {
        for t in [
            "I wasn't able to resolve the mismatch with the available evaluator feedback.",
            "The result still differs from the reference.",
            "The kernel does not pass the check yet.",
            "This is not validated against the reference.",
            "I tried both orders but the evaluator provides no per-element details to guide further fixes.",
            "Could not get the test to pass.",
        ] {
            assert!(admits_failure(t), "{t}");
        }
        for t in [
            "All checks pass and the output matches the reference.",
            "Done. The solution is in solution.py.",
            "I could not read the unreadable input file, so I used the sample.",
            "",
        ] {
            assert!(!admits_failure(t), "{t}");
        }
    }

    /// `decide` with progress that advances (a tool call ran) since the previous nudge.
    fn d(p: &mut Persist, reply: &str, fp: u64) -> Option<String> {
        let progress = p.progress_at_nudge.map_or(1, |x| x + 1);
        p.decide(reply, progress, true, || fp)
    }

    fn on() -> Persist {
        Persist::new(true, 6, 0.85, None, Some("test with ./eval"))
    }

    #[test]
    fn off_never_nudges() {
        let mut p = Persist::new(false, 6, 0.85, None, None);
        p.record_check("./eval", 1, "FAILED");
        assert_eq!(d(&mut p, "I could not make the test pass", 1), None);
    }

    #[test]
    fn a_failed_check_is_nudged_with_an_attempt_log_and_a_pass_stops_it() {
        let mut p = on();
        p.note_edit("kernel.py");
        p.record_check("./eval", 0, r#"{"exec_acc": false}"#);
        let n1 = d(&mut p, "done", 1).expect("nudge");
        assert!(n1.contains("kernel.py") && n1.contains("FAIL") && n1.contains("Do not finish"));
        p.note_edit("kernel.py");
        p.record_check("./eval", 0, r#"{"exec_acc": false, "x": 1}"#);
        let n2 = d(&mut p, "done", 2).expect("second nudge");
        assert!(n2.contains("diagnostic"));
        p.note_edit("kernel.py");
        p.record_check("./eval", 0, r#"{"exec_acc": true}"#);
        assert_eq!(d(&mut p, "I could not make it pass", 3), None, "a pass ends the loop");
        assert_eq!(p.nudges_sent(), 2);
    }

    #[test]
    fn the_cap_stops_it_and_no_check_passing_never_gets_extra_nudges() {
        let mut p = Persist::new(true, 3, 0.85, None, None);
        p.record_check("./eval", 1, "FAILED");
        for i in 0..3 {
            assert!(d(&mut p, "", i as u64).is_some(), "{i}");
        }
        assert_eq!(d(&mut p, "", 99), None, "cap reached");
    }

    #[test]
    fn two_attempts_that_change_nothing_end_it() {
        let mut p = on();
        p.record_check("./eval", 1, "FAILED same");
        assert!(d(&mut p, "", 7).is_some());
        assert!(d(&mut p, "", 7).is_some(), "first repeat is allowed");
        assert_eq!(d(&mut p, "", 7), None, "second repeat stops");
    }

    #[test]
    fn a_change_resets_the_no_change_count() {
        let mut p = on();
        p.record_check("./eval", 1, "FAILED a");
        assert!(d(&mut p, "", 1).is_some());
        assert!(d(&mut p, "", 1).is_some());
        assert!(d(&mut p, "", 2).is_some(), "work tree changed");
        assert!(d(&mut p, "", 2).is_some());
        assert_eq!(d(&mut p, "", 2), None);
    }

    #[test]
    fn a_nudge_answered_without_any_tool_call_is_not_repeated() {
        let mut p = on();
        p.record_check("./eval", 1, "FAILED");
        assert!(p.decide("", 5, true, || 1).is_some());
        assert_eq!(p.decide("", 5, true, || 2), None, "no tool ran since the nudge");
        assert!(p.decide("", 6, true, || 3).is_some());
    }

    #[test]
    fn a_check_that_cannot_run_gets_one_nudge_and_a_revert_counts_as_no_change() {
        let mut p = on();
        p.record_check("./eval", 127, "bash: ./eval: command not found");
        let n = d(&mut p, "", 1).expect("one nudge");
        assert!(n.contains("could not run"));
        assert_eq!(d(&mut p, "", 2), None);
        let mut q = on();
        q.record_check("./eval", 1, "FAILED a");
        assert!(d(&mut q, "", 1).is_some()); // state A
        assert!(d(&mut q, "", 2).is_some()); // state B
        assert!(d(&mut q, "", 1).is_some(), "back to A: one no-change");
        assert_eq!(d(&mut q, "", 1), None, "second no-change in a row");
    }

    #[test]
    fn admission_only_triggers_need_edits_and_stop_after_two() {
        let mut p = on();
        assert_eq!(p.decide("The result still differs.", 1, false, || 1), None, "no edits made");
        assert!(p.decide("The result still differs.", 2, true, || 2).is_some());
        assert!(p.decide("The result still differs.", 3, true, || 3).is_some());
        assert_eq!(p.decide("The result still differs.", 4, true, || 4), None, "admission cap");
    }

    #[test]
    fn the_time_cap_stops_it() {
        let mut p = Persist::new(true, 6, 0.85, Some(0.0), None);
        p.record_check("./eval", 1, "FAILED");
        assert_eq!(p.decide("", 1, true, || 1), None);
    }

    #[test]
    fn the_budget_stop_wins_over_everything() {
        let mut p = on();
        p.record_check("./eval", 1, "FAILED");
        p.set_budget_used(Some(0.9));
        assert_eq!(d(&mut p, "", 1), None);
        p.set_budget_used(Some(0.5));
        assert!(d(&mut p, "", 1).is_some());
    }

    #[test]
    fn an_honest_admission_without_any_check_is_nudged_but_a_confident_reply_is_not() {
        let mut p = on();
        assert_eq!(d(&mut p, "All done, the solution handles every case.", 1), None);
        assert!(d(&mut p, "The result still differs from the reference.", 2).is_some());
    }

    #[test]
    fn the_attempt_log_is_bounded() {
        let mut p = on();
        for i in 0..40 {
            p.note_edit(&format!("f{i}.py"));
            p.record_check("./eval", 1, &format!("FAILED {i}"));
        }
        assert!(p.log.len() <= MAX_LOG);
        let text = p.nudge_text(false);
        assert!(text.matches("FAIL:").count() <= SHOWN_LOG);
    }
}
