//! Park a turn on a provider usage limit instead of failing it (factr-learn's `waitForUsage`).
//!
//! When a provider answers that the plan's quota is spent and says when it resets
//! (`factr_provider_core::usage_limit::reset_wait`), wait until then, keep the session cancellable
//! and keepalives flowing, and retry the same request. Bounded by `agents.wait_for_usage_max_s`
//! (env `FACTR_WAIT_FOR_USAGE_MAX_S`, default 2 h, 0 = off), by a few parks per turn, and never past
//! the runner's turn deadline (`FACTR_TURN_DEADLINE_S` / `FACTR_HARD_DEADLINE_UNIX`). A short
//! transient 429 never reaches here: the provider's own backoff handles it first.

use super::Agent;
use factr_agent_runtime::InterruptSignal;
use std::time::Duration;

const DEFAULT_MAX_WAIT_S: u64 = 7200;
const MAX_PARKS: u32 = 5;
/// Slack after the stated reset so the retry does not land a moment early.
const MARGIN: Duration = Duration::from_secs(2);

#[derive(Default)]
pub(super) struct UsageParks {
    used: u32,
}

fn max_wait(env: Option<&str>, config: Option<u64>) -> Duration {
    Duration::from_secs(env.and_then(|v| v.trim().parse::<u64>().ok()).or(config).unwrap_or(DEFAULT_MAX_WAIT_S))
}

/// How long to park for this error, or `None` to fail as before.
fn plan(max_wait: Duration, message: &str, now_unix: i64, parks: u32, deadline_left: Option<Duration>) -> Option<Duration> {
    if max_wait.is_zero() || parks >= MAX_PARKS {
        return None;
    }
    let wait = factr_provider_core::usage_limit::reset_wait(message, now_unix)?;
    if wait.is_zero() {
        return None;
    }
    let wait = wait + MARGIN;
    if wait > max_wait || deadline_left.is_some_and(|left| wait + MARGIN >= left) {
        return None;
    }
    Some(wait)
}

/// Sleep `wait`, calling `tick` every `every`; false if `cancel` fired first.
async fn sleep_or_cancel(wait: Duration, every: Duration, cancel: &InterruptSignal, mut tick: impl FnMut()) -> bool {
    let end = tokio::time::Instant::now() + wait;
    loop {
        let now = tokio::time::Instant::now();
        if now >= end {
            return !cancel.is_set();
        }
        tokio::select! {
            _ = cancel.notified() => return false,
            _ = tokio::time::sleep((end - now).min(every)) => tick(),
        }
    }
}

impl Agent {
    /// On a usage-limit error with a known reset: park until the reset. True means retry the request;
    /// false means fail as before (not parkable, over the bound, past the deadline, or cancelled while parked).
    pub(super) async fn park_for_usage_limit(
        &self,
        parks: &mut UsageParks,
        message: &str,
        deadline: &super::turn_deadline::TurnDeadline,
        mut keepalive: impl FnMut(),
    ) -> bool {
        let now_unix = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
        let configured = crate::config::config().agents.wait_for_usage_max_s;
        let max = max_wait(std::env::var("FACTR_WAIT_FOR_USAGE_MAX_S").ok().as_deref(), configured);
        let Some(wait) = plan(max, message, now_unix, parks.used, deadline.remaining()) else {
            return false;
        };
        parks.used += 1;
        let provider = self.provider.name().to_string();
        factr_base::obs_sink::emit(
            factr_base::obs_sink::Span::new("loop.guard")
                .session(&self.session.id)
                .attr("reason", "usage_wait")
                .attr("provider", provider.clone())
                .attr("seconds", wait.as_secs()),
        );
        crate::logging::warn(&format!("{provider} usage limit: parking the turn for {}s until the reset", wait.as_secs()));
        sleep_or_cancel(wait, Duration::from_secs(20), &self.graceful_shutdown, &mut keepalive).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const LIMIT: &str = "Rate limited: The usage limit has been reached. Resets in 10m.";

    #[test]
    fn parks_until_the_reset_within_bounds() {
        let h2 = Duration::from_secs(7200);
        assert_eq!(plan(h2, LIMIT, 0, 0, None), Some(Duration::from_secs(602)));
        // Over the bound, off, or too many parks: fail as before.
        assert_eq!(plan(Duration::from_secs(300), LIMIT, 0, 0, None), None);
        assert_eq!(plan(Duration::ZERO, LIMIT, 0, 0, None), None);
        assert_eq!(plan(h2, LIMIT, 0, MAX_PARKS, None), None);
        // A short 429 with no reset time, or a zero wait, is the provider backoff's business.
        assert_eq!(plan(h2, "429 Too Many Requests", 0, 0, None), None);
        assert_eq!(plan(h2, "RESOURCE_EXHAUSTED quota will reset after 0s", 0, 0, None), None);
    }

    #[test]
    fn never_waits_past_the_deadline() {
        let h2 = Duration::from_secs(7200);
        assert_eq!(plan(h2, LIMIT, 0, 0, Some(Duration::from_secs(300))), None);
        assert_eq!(plan(h2, LIMIT, 0, 0, Some(Duration::from_secs(3600))), Some(Duration::from_secs(602)));
    }

    #[test]
    fn bound_from_env_then_config_then_default() {
        assert_eq!(max_wait(Some("30"), Some(5)), Duration::from_secs(30));
        assert_eq!(max_wait(None, Some(5)), Duration::from_secs(5));
        assert_eq!(max_wait(Some("bad"), None), Duration::from_secs(7200));
        assert_eq!(max_wait(Some("0"), None), Duration::ZERO);
    }

    #[tokio::test]
    async fn sleep_completes_ticks_and_is_cancellable() {
        let signal = InterruptSignal::new();
        let mut ticks = 0;
        assert!(sleep_or_cancel(Duration::from_millis(60), Duration::from_millis(20), &signal, || ticks += 1).await);
        assert!(ticks >= 2, "keepalive ticks while parked: {ticks}");
        let s2 = signal.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(30)).await;
            s2.fire();
        });
        let started = std::time::Instant::now();
        assert!(!sleep_or_cancel(Duration::from_secs(30), Duration::from_secs(10), &signal, || {}).await);
        assert!(started.elapsed() < Duration::from_secs(5), "cancel ends the park at once");
    }
}
