//! Time-budget reminders. A runner sets `FACTR_TURN_DEADLINE_S` (seconds for
//! the whole turn) or `FACTR_HARD_DEADLINE_UNIX` (epoch seconds); at 70% and
//! 90% of the budget one reminder each is injected before the next request.
//! Off (no cost) when neither is set.

use std::time::{Duration, Instant};

const MSG_70: &str = "<system-reminder>About 30% of the time budget is left: stop exploring, write your best-effort result to the requested output now, then verify if time remains.</system-reminder>";
const MSG_70_NO_TOOLS: &str = "<system-reminder>About 30% of the time budget is left: stop exploring and give your best answer now, without tools.</system-reminder>";
const MSG_90: &str = "<system-reminder>Time is almost up: make sure the requested output files exist with your best result, then give a short final answer.</system-reminder>";

pub(super) struct TurnDeadline {
    start: Instant,
    total: Option<Duration>,
    stage: u8,
    no_tools: bool,
}

fn total_budget(turn_s: Option<&str>, hard_unix: Option<&str>, now_unix: u64) -> Option<Duration> {
    if let Some(s) = turn_s.and_then(|v| v.parse::<f64>().ok()).filter(|s| *s > 0.0) {
        return Some(Duration::from_secs_f64(s));
    }
    let d = hard_unix?.parse::<u64>().ok()?;
    (d > now_unix).then(|| Duration::from_secs(d - now_unix))
}

/// Reminder due at `elapsed` of `total` given the stages already sent.
fn due(elapsed: Duration, total: Duration, stage: u8, no_tools: bool) -> Option<(u8, &'static str, &'static str)> {
    let f = elapsed.as_secs_f64() / total.as_secs_f64();
    if f >= 0.9 && stage < 2 {
        Some((2, "deadline_90", MSG_90))
    } else if f >= 0.7 && stage < 1 {
        Some((1, "deadline_70", if no_tools { MSG_70_NO_TOOLS } else { MSG_70 }))
    } else {
        None
    }
}

impl TurnDeadline {
    pub(super) fn new() -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |t| t.as_secs());
        let total = total_budget(
            std::env::var("FACTR_TURN_DEADLINE_S").ok().as_deref(),
            std::env::var("FACTR_HARD_DEADLINE_UNIX").ok().as_deref(),
            now,
        );
        Self { start: Instant::now(), total, stage: 0, no_tools: false }
    }

    /// The task forbids tools: the reminders then never mention verifying.
    pub(super) fn set_no_tools(&mut self, no_tools: bool) {
        self.no_tools = no_tools;
    }

    /// Time left in the runner's budget; `None` when no deadline is set.
    pub(super) fn remaining(&self) -> Option<Duration> {
        Some(self.total?.saturating_sub(self.start.elapsed()))
    }

    /// The 70% reminder has been sent.
    pub(super) fn past_70(&self) -> bool {
        self.stage >= 1
    }

    /// Half of the runner's budget has elapsed (false when no deadline is set).
    pub(super) fn past_50(&self) -> bool {
        self.total.is_some_and(|t| self.start.elapsed().as_secs_f64() >= 0.5 * t.as_secs_f64())
    }

    /// The reminder to inject before the next model request, if one is due.
    pub(super) fn poll(&mut self, session_id: &str) -> Option<&'static str> {
        let (stage, reason, msg) = due(self.start.elapsed(), self.total?, self.stage, self.no_tools)?;
        self.stage = stage;
        factr_base::obs_sink::emit(
            factr_base::obs_sink::Span::new("loop.guard").session(session_id).attr("reason", reason),
        );
        Some(msg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_from_env_values() {
        assert_eq!(total_budget(Some("100"), None, 0), Some(Duration::from_secs(100)));
        assert_eq!(total_budget(None, Some("1100"), 1000), Some(Duration::from_secs(100)));
        assert_eq!(total_budget(None, Some("900"), 1000), None);
        assert_eq!(total_budget(None, None, 0), None);
    }

    #[test]
    fn one_reminder_per_threshold() {
        let t = Duration::from_secs(100);
        assert!(due(Duration::from_secs(69), t, 0, false).is_none());
        let (s, r, _) = due(Duration::from_secs(70), t, 0, false).unwrap();
        assert_eq!((s, r), (1, "deadline_70"));
        assert!(due(Duration::from_secs(80), t, 1, false).is_none());
        let (s, r, _) = due(Duration::from_secs(90), t, 1, false).unwrap();
        assert_eq!((s, r), (2, "deadline_90"));
        assert!(due(Duration::from_secs(99), t, 2, false).is_none());
        // Jumping straight past 90% sends only the 90% reminder.
        assert_eq!(due(Duration::from_secs(95), t, 0, false).unwrap().0, 2);
    }

    #[test]
    fn the_70_reminder_has_a_no_tools_variant_without_verify() {
        let t = Duration::from_secs(100);
        let (_, _, m) = due(Duration::from_secs(70), t, 0, true).unwrap();
        assert!(!m.contains("verify") && m.contains("without tools"));
        assert!(due(Duration::from_secs(70), t, 0, false).unwrap().2.contains("verify"));
    }

    #[test]
    fn off_without_env() {
        let mut d = TurnDeadline { start: Instant::now(), total: None, stage: 0, no_tools: false };
        assert!(d.poll("s").is_none());
    }
}
