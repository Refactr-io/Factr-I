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

/// `FACTR_PERSIST=0|1` overrides `persistence.enabled` (default on).
pub(crate) fn enabled() -> bool {
    match std::env::var("FACTR_PERSIST").ok().as_deref() {
        Some("0") => false,
        Some("1") => true,
        _ => factr_base::factr_config::current().persistence.enabled.unwrap_or(true),
    }
}

/// Create the run's goal when the task names a check. `true` when the run is persisting.
pub(crate) fn start(store: &ControlStore, session_id: &str, prompt: &str, timeout: Duration) -> bool {
    let checks = task_check_commands(prompt);
    if checks.is_empty() {
        return false;
    }
    // Rows of runs that died or lost the cleanup race (a gate finishing after its run timed out).
    let _ = store.purge_stale_unattended(STALE_AFTER.as_millis() as i64);
    let retries = factr_base::factr_config::current().persistence.max_retries.map_or(DEFAULT_RETRIES, i64::from);
    // One check may use up to a third of the run (at least a minute); the wall budget leaves room for one more.
    let gate_secs = (timeout.as_secs() / 3).clamp(60, 900) as i64;
    let mut goal = SessionGoal::new(format!("the task's check: {}", checks.join(", ")));
    goal.unattended = true;
    goal.max_turns = 0;
    // A fixed ceiling of fresh tokens, or (0) the automatic one set after the first turn.
    goal.token_budget = factr_base::factr_config::current().persistence.max_tokens.map(|n| n as i64);
    goal.wall_budget_ms = Some(wall_budget_ms(timeout.as_millis() as i64, gate_secs, factr_base::factr_config::current().persistence.max_seconds));
    goal.gates = checks
        .into_iter()
        .map(|command| {
            let mut gate = QualityGate::new(command);
            gate.max_retries = retries;
            gate.timeout_seconds = gate_secs;
            gate
        })
        .collect();
    store.set_goal(session_id, Some(&goal)).is_ok()
}

/// The run's wall budget for persisting: its time limit minus one more check (at least half of it), which
/// `persistence.max_seconds` can only shorten.
fn wall_budget_ms(timeout_ms: i64, gate_secs: i64, cap_secs: Option<u64>) -> i64 {
    let run_budget = (timeout_ms - gate_secs * 1000 - 5_000).max(timeout_ms / 2);
    cap_secs.map_or(run_budget, |c| (c as i64).saturating_mul(1000).min(run_budget))
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
        assert!(!start(&store, "none", "Fix the bug in this directory.", Duration::from_secs(600)));
        assert!(store.get_goal("none").unwrap().is_none());
        assert!(start(&store, "s", "Fix it. Test with ./run_check.sh and make it pass.", Duration::from_secs(900)));
        let goal = store.get_goal("s").unwrap().unwrap();
        assert!(goal.unattended && goal.max_turns == 0);
        assert_eq!(goal.gates.len(), 1);
        assert_eq!((goal.gates[0].command.as_str(), goal.gates[0].max_retries, goal.gates[0].timeout_seconds), ("./run_check.sh", 3, 300));
        assert!(goal.wall_budget_ms.unwrap() < 900_000, "room is left for the last check");
        assert!(store.active_sessions().unwrap().is_empty(), "the driver never picks up a run's goal");
    }

    #[test]
    fn the_time_cap_only_shortens_the_wall_budget() {
        assert_eq!(wall_budget_ms(1_200_000, 300, None), 895_000);
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
        start(&store, "s", "Test with ./check.sh", Duration::from_secs(600));
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
