//! Persistence for unattended runs (`persistence.enabled`, on unless disabled).
//!
//! A one-shot `/api/agent/run` whose task names its own check ("test with ./eval") is given an implicit,
//! unattended goal with that check as its quality gate. Everything else is the goal machinery that already
//! exists: `driver::turn_done` runs the gate after each turn (the host's own run, not the model's claim),
//! records the ratchet and attempt log, and injects the next prompt with the failed output; the run ends
//! when the gate passes, its retries are spent, the wall budget is used, or the run is interrupted. This
//! module only creates the goal, decides when the run is over, and cleans up.

use factr_learn::agent_loop::{ControlStore, GoalStatus, QualityGate, SessionGoal};
use factr_learn::task_check::task_check_commands;
use serde_json::{Value, json};
use std::time::Duration;

const DEFAULT_RETRIES: i64 = 3;
const STALE_AFTER: Duration = Duration::from_secs(3600);
/// A persisting run stops retrying after this long unless `persistence.max_seconds` says otherwise: the best
/// pass-per-second point of the measured pass-vs-time curve.
const DEFAULT_MAX_SECONDS: u64 = 400;

/// `FACTR_PERSIST=0|1` overrides `persistence.enabled` (default on).
pub(crate) fn enabled() -> bool {
    match std::env::var("FACTR_PERSIST").ok().as_deref() {
        Some("0") => false,
        Some("1") => true,
        _ => factr_base::factr_config::current().persistence.enabled.unwrap_or(true),
    }
}

/// A positive integer from the environment (`FACTR_PERSIST_MAX_RETRIES`, `..._MAX_TOKENS`, `..._MAX_SECONDS`): the
/// top-priority override of the matching `persistence.*` setting, for runners that set only the environment.
fn env_number(name: &str) -> Option<u64> {
    factr_base::factr_config::env_text(name)?.parse::<u64>().ok().filter(|n| *n > 0)
}

/// Create the run's goal when the task names a check: the check commands when the run is persisting.
pub(crate) fn start(store: &ControlStore, session_id: &str, prompt: &str, cwd: Option<&str>, timeout: Duration) -> Option<Vec<String>> {
    let checks = task_check_commands(prompt);
    if checks.is_empty() {
        return None;
    }
    // Rows of runs that died or lost the cleanup race (a gate finishing after its run timed out).
    let _ = store.purge_stale_unattended(STALE_AFTER.as_millis() as i64);
    let settings = factr_base::factr_config::current().persistence.clone();
    let retries = env_number("FACTR_PERSIST_MAX_RETRIES").map(|n| n.min(10)).or(settings.max_retries.map(u64::from)).map_or(DEFAULT_RETRIES, |n| n as i64);
    // One check may use up to a third of the run (at least a minute); the wall budget leaves room for one more.
    let gate_secs = (timeout.as_secs() / 3).clamp(60, 900) as i64;
    let mut goal = SessionGoal::new(format!("the task's check: {}", checks.join(", ")));
    goal.unattended = true;
    goal.max_turns = 0;
    // A fixed ceiling of fresh tokens, or (0) the automatic one set after the first turn.
    goal.token_budget = env_number("FACTR_PERSIST_MAX_TOKENS").or(settings.max_tokens).map(|n| n as i64);
    goal.wall_budget_ms = Some(wall_budget_ms(timeout.as_millis() as i64, gate_secs, env_number("FACTR_PERSIST_MAX_SECONDS").or(settings.max_seconds).or(Some(DEFAULT_MAX_SECONDS))));
    goal.gates = checks
        .iter()
        .cloned()
        .map(|command| {
            let mut gate = QualityGate::new(command);
            gate.max_retries = retries;
            gate.timeout_seconds = gate_secs;
            gate
        })
        .collect();
    // Terser turns (a faster run) unless the user chose a verbosity.
    factr_base::run_hints::set_text_verbosity(session_id, "low");
    let gates: Vec<(String, u64)> = goal.gates.iter().map(|g| (g.command.clone(), g.timeout_seconds.max(1) as u64)).collect();
    factr_learn::check_memo::register(session_id, &gates, cwd.map(std::path::Path::new));
    store.set_goal(session_id, Some(&goal)).is_ok().then_some(checks)
}

/// A one-off system reminder for a persisting run, from the flags that are on (all default off):
/// `FACTR_PERSIST_AUTOCHECK=1` tells the model the engine runs the check after every change and appends the result;
/// `FACTR_PERSIST_ORIENT=1` lists the working directory and shows the start of the check script, so the model need
/// not spend calls on `ls` and `read`.
pub(crate) fn reminder(existing: Option<&str>, checks: &[String], cwd: Option<&str>) -> Option<String> {
    let mut parts: Vec<String> = existing.map(str::to_string).into_iter().collect();
    let list = checks.iter().map(|c| format!("`{c}`")).collect::<Vec<_>>().join(", ");
    if factr_learn::check_memo::autocheck_enabled() {
        parts.push(format!("The engine runs {list} itself after every file change you make and appends the result to that change's output, and again when you stop. Do not run it yourself and do not re-read a file you just wrote: read the appended result, then fix what it points to or stop."));
    }
    if factr_base::factr_config::env_bool("FACTR_PERSIST_ORIENT").unwrap_or(false) {
        if let Some(dir) = cwd.map(std::path::Path::new) {
            parts.push(orientation(dir, checks));
        }
    }
    (!parts.is_empty()).then(|| parts.join("\n\n"))
}

/// The working directory's files and the start of the first check that is a small text file in it.
fn orientation(dir: &std::path::Path, checks: &[String]) -> String {
    let mut files: Vec<String> = std::fs::read_dir(dir)
        .map(|rd| rd.flatten().filter_map(|e| Some(format!("{} ({} B)", e.file_name().to_str()?, e.metadata().ok()?.len()))).collect())
        .unwrap_or_default();
    files.sort();
    files.truncate(40);
    let mut text = format!("Working directory now: {}.", if files.is_empty() { "empty".to_string() } else { files.join(", ") });
    let script = checks.iter().find_map(|c| {
        let first = c.split_whitespace().next()?.trim_start_matches("./");
        let path = dir.join(first);
        let meta = std::fs::metadata(&path).ok().filter(|m| m.is_file() && m.len() <= 4096)?;
        let body = std::fs::read_to_string(&path).ok()?;
        Some((first.to_string(), body, meta.len()))
    });
    if let Some((name, body, _)) = script {
        let head: String = body.chars().take(1200).collect();
        text.push_str(&format!(" The start of `{name}`:\n```\n{head}\n```"));
    }
    text
}

/// The run's wall budget for persisting: its time limit minus one more check (at least half of it), which
/// `persistence.max_seconds` can only shorten.
fn wall_budget_ms(timeout_ms: i64, gate_secs: i64, cap_secs: Option<u64>) -> i64 {
    let run_budget = (timeout_ms - gate_secs * 1000 - 5_000).max(timeout_ms / 2);
    cap_secs.map_or(run_budget, |c| (c as i64).saturating_mul(1000).min(run_budget))
}

/// The tools a write-and-check task needs. With `FACTR_PERSIST_TOOLS=minimal` a persisting run's allowed tools are cut
/// to these (fewer schemas in every request); anything else, or any other value, leaves them alone.
const MINIMAL_TOOLS: [&str; 5] = ["bash", "read", "write", "edit", "ls"];

pub(crate) fn minimal_tools(allowed: Vec<String>) -> Vec<String> {
    if factr_base::factr_config::env_text("FACTR_PERSIST_TOOLS").as_deref() != Some("minimal") {
        return allowed;
    }
    let kept: Vec<String> = allowed.iter().filter(|t| MINIMAL_TOOLS.contains(&t.as_str())).cloned().collect();
    // A policy with none of them is left as it is rather than emptied.
    if kept.is_empty() { allowed } else { kept }
}

/// Whether a `session.control.update` payload still shows an active goal (the run goes on).
pub(crate) fn goal_active(control: &Value) -> bool {
    control["goal"]["status"].as_str() == Some("active")
}

/// Token usage summed over every turn of the run (`payload["usage"]` of each `message.complete`).
#[derive(Default)]
pub(crate) struct Usage {
    input: u64,
    output: u64,
    cache_read: u64,
    seen: bool,
}

impl Usage {
    pub(crate) fn add(&mut self, usage: &Value) {
        let n = |k: &str| usage[k].as_u64().unwrap_or(0);
        (self.input, self.output, self.cache_read) = (self.input + n("input"), self.output + n("output"), self.cache_read + n("cache_read"));
        self.seen |= !usage.is_null();
    }

    /// Replace the final turn's usage with the run's total.
    pub(crate) fn apply(&self, payload: &mut Value) {
        if self.seen {
            payload["usage"] = json!({ "input": self.input, "output": self.output, "cache_read": self.cache_read });
        }
    }
}

/// End of the run: what the check did, then the goal row and its checkpoint pins are removed. A gate still
/// running at a timeout may write its row back afterwards; `start` purges such leftovers, and the driver
/// never reads an unattended goal.
pub(crate) fn finish(store: &ControlStore, session_id: &str) -> Value {
    let goal = store.get_goal(session_id).ok().flatten();
    let report = goal.as_ref().map_or(Value::Null, |g| {
        json!({
            "check_passed": g.status == GoalStatus::Done,
            "turns": g.turns_used,
            "stopped": g.paused_reason.clone().or_else(|| (g.status == GoalStatus::Active).then(|| "the run ended while the check was still failing".to_string())),
            "attempts": g.attempt_log,
        })
    });
    if let Some(g) = &goal {
        g.release_checkpoints(session_id);
    }
    let _ = store.forget_session(session_id);
    factr_base::run_hints::clear(session_id);
    factr_learn::check_memo::forget(session_id);
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_run_persists_only_when_its_task_names_a_check_and_the_goal_is_unattended() {
        let dir = std::env::temp_dir().join(format!("persist-run-{}-1", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = ControlStore::open(&dir).unwrap();
        assert!(start(&store, "none", "Fix the bug in this directory.", None, Duration::from_secs(600)).is_none());
        assert!(store.get_goal("none").unwrap().is_none());
        assert_eq!(start(&store, "s", "Fix it. Test with ./run_check.sh and make it pass.", None, Duration::from_secs(900)), Some(vec!["./run_check.sh".to_string()]));
        let goal = store.get_goal("s").unwrap().unwrap();
        assert!(goal.unattended && goal.max_turns == 0);
        assert_eq!(goal.gates.len(), 1);
        assert_eq!((goal.gates[0].command.as_str(), goal.gates[0].max_retries, goal.gates[0].timeout_seconds), ("./run_check.sh", 3, 300));
        assert_eq!(goal.wall_budget_ms, Some(400_000), "the default time cap, and room is left for the last check");
        assert_eq!(factr_base::run_hints::text_verbosity(Some("s")).as_deref(), Some("low"), "terser turns for this run");
        finish(&store, "s");
        assert_eq!(factr_base::run_hints::text_verbosity(Some("s")), None, "the hint ends with the run");
        assert!(store.active_sessions().unwrap().is_empty(), "the driver never picks up a run's goal");
    }

    #[test]
    fn the_reminder_says_what_the_flags_say_and_nothing_when_they_are_off() {
        let _env = factr_base::storage::lock_test_env();
        let dir = std::env::temp_dir().join(format!("persist-reminder-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("eval"), "#!/bin/sh\necho check-body\n").unwrap();
        std::fs::write(dir.join("solution.py"), "x").unwrap();
        let checks = vec!["./eval".to_string()];
        let cwd = dir.to_str();
        factr_base::env::remove_var("FACTR_PERSIST_AUTOCHECK");
        factr_base::env::remove_var("FACTR_PERSIST_ORIENT");
        assert_eq!(reminder(None, &checks, cwd), None);
        assert_eq!(reminder(Some("keep"), &checks, cwd).as_deref(), Some("keep"));
        factr_base::env::set_var("FACTR_PERSIST_ORIENT", "1");
        let oriented = reminder(None, &checks, cwd).unwrap();
        assert!(oriented.contains("eval (") && oriented.contains("solution.py (1 B)") && oriented.contains("check-body"), "{oriented}");
        factr_base::env::set_var("FACTR_PERSIST_AUTOCHECK", "1");
        let both = reminder(Some("keep"), &checks, cwd).unwrap();
        assert!(both.starts_with("keep") && both.contains("runs `./eval` itself") || both.contains("runs `./eval`"), "{both}");
        factr_base::env::remove_var("FACTR_PERSIST_AUTOCHECK");
        factr_base::env::remove_var("FACTR_PERSIST_ORIENT");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn minimal_tools_cut_the_surface_only_when_asked() {
        let _env = factr_base::storage::lock_test_env();
        let all: Vec<String> = ["agentgrep", "bash", "batch", "bg", "edit", "ls", "read", "repl", "todo", "write"].map(String::from).to_vec();
        factr_base::env::remove_var("FACTR_PERSIST_TOOLS");
        assert_eq!(minimal_tools(all.clone()), all);
        factr_base::env::set_var("FACTR_PERSIST_TOOLS", "minimal");
        assert_eq!(minimal_tools(all), ["bash", "edit", "ls", "read", "write"]);
        assert_eq!(minimal_tools(vec!["repl".into()]), ["repl"], "never emptied");
        factr_base::env::remove_var("FACTR_PERSIST_TOOLS");
    }

    #[test]
    fn the_time_cap_only_shortens_the_wall_budget() {
        assert_eq!(wall_budget_ms(1_200_000, 300, None), 895_000);
        assert_eq!(wall_budget_ms(1_200_000, 300, Some(DEFAULT_MAX_SECONDS)), 400_000, "the default cap");
        assert_eq!(wall_budget_ms(1_200_000, 300, Some(400)), 400_000);
        assert_eq!(wall_budget_ms(1_200_000, 300, Some(5_000)), 895_000, "never beyond the run's own limit");
        assert_eq!(wall_budget_ms(120_000, 60, None), 60_000, "at least half of a short run");
    }

    #[test]
    fn the_run_ends_when_the_goal_is_no_longer_active_and_usage_is_summed() {
        assert!(goal_active(&json!({ "goal": { "status": "active" } })));
        for over in [json!({ "goal": { "status": "done" } }), json!({ "goal": { "status": "paused" } }), json!({ "goal": null }), json!({})] {
            assert!(!goal_active(&over), "{over}");
        }
        let mut total = Usage::default();
        total.add(&json!({ "input": 10, "output": 2, "cache_read": 4 }));
        total.add(&json!({ "input": 5, "output": 1 }));
        let mut payload = json!({ "usage": { "input": 5 } });
        total.apply(&mut payload);
        assert_eq!(payload["usage"], json!({ "input": 15, "output": 3, "cache_read": 4 }));
        let mut none = json!({});
        Usage::default().apply(&mut none);
        assert!(none["usage"].is_null());
    }

    #[test]
    fn finishing_reports_the_outcome_and_removes_the_goal() {
        let dir = std::env::temp_dir().join(format!("persist-run-{}-2", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = ControlStore::open(&dir).unwrap();
        start(&store, "s", "Test with ./check.sh", None, Duration::from_secs(600));
        let mut goal = store.get_goal("s").unwrap().unwrap();
        goal.status = GoalStatus::Done;
        goal.turns_used = 2;
        store.set_goal("s", Some(&goal)).unwrap();
        let report = finish(&store, "s");
        assert_eq!((report["check_passed"].clone(), report["turns"].clone()), (json!(true), json!(2)));
        assert!(store.get_goal("s").unwrap().is_none());
        assert!(finish(&store, "s").is_null(), "nothing left to report");
    }
}
