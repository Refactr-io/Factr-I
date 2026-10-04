//! Factr `fallback_providers`: when the active provider fails a request for good, move the turn to
//! the next configured provider/model and come back to the primary after a cooldown.
//!
//! "For good" means the provider's own retries and cross-provider failover are spent (the error reached
//! the turn loop), it is not a context-limit error (compaction recovers those), and it is not a usage
//! limit with a known reset (`usage_wait` parks the turn instead). A fallback is a fork of the active
//! provider switched with `set_model("provider:model")`, which refuses an entry whose credentials do
//! not exist (and, in private mode, any that are not environment keys): that entry is skipped.
//!
//! Limits: one pass over the list per turn, never back to an entry already tried this turn. The
//! cooldown follows `agent/fallback_cooldown.py`: a rate-limit/billing failure keeps the turn chain on
//! the fallback for 60 s, 2 min, 4 min ... up to 4 h (doubling for consecutive failures); any other failure
//! returns to the primary at the next turn.

use super::Agent;
use std::sync::Arc;
use std::time::{Duration, Instant};

const BASE_COOLDOWN: Duration = Duration::from_secs(60);
const MAX_COOLDOWN: Duration = Duration::from_secs(4 * 60 * 60);
/// A switch this long after the previous one starts the backoff over.
const BACKOFF_FORGET: Duration = Duration::from_secs(30 * 60);

/// Per-agent state: the provider to return to and when.
#[derive(Default)]
pub(super) struct FallbackState {
    primary: Option<Arc<dyn crate::provider::Provider>>,
    pub(super) until: Option<Instant>,
    backoff: u32,
    last_switch: Option<Instant>,
}

/// Per-turn walk over the list: the next entry to try, so a request passes through it once.
#[derive(Default)]
pub(super) struct FallbackWalk {
    next: usize,
}

/// Cooldown for the `backoff`-th consecutive rate-limit failure: 60 s doubling to 4 h.
fn cooldown(backoff: u32) -> Duration {
    BASE_COOLDOWN.saturating_mul(1u32.checked_shl(backoff.min(12)).unwrap_or(u32::MAX)).min(MAX_COOLDOWN)
}

/// A failure of the quota/billing/rate-limit kind (Factr arms the cooldown only for these).
fn rate_limited(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    ["rate limit", "rate-limit", "too many requests", "quota", "billing", "payment required", "credit", "usage limit", "429", "402"]
        .iter()
        .any(|needle| lower.contains(needle))
}

/// Whether `entry_provider`/`entry_model` is what `active_name`/`active_model` already is.
fn same_route(entry: &factr_base::factr_config::Fallback, active_name: &str, active_model: &str) -> bool {
    let (a, p) = (active_name.to_ascii_lowercase(), entry.provider.to_ascii_lowercase());
    entry.model == active_model && (a.contains(&p) || p.contains(&a))
}

impl Agent {
    /// At the start of a turn: return to the primary provider once its cooldown has run out.
    pub(super) fn restore_primary_provider(&mut self) {
        let due = self.fallback.until.is_some_and(|until| Instant::now() >= until);
        if !due {
            return;
        }
        let Some(primary) = self.fallback.primary.take() else { return };
        self.fallback.until = None;
        crate::logging::info(&format!("provider fallback: back on {} after the cooldown", primary.name()));
        self.provider = primary;
        self.after_provider_swap();
    }

    fn after_provider_swap(&mut self) {
        self.refresh_compaction_budget();
        // Provider-side resume ids, cache signatures and tool locks belong to the old provider.
        self.cache_tracker.reset();
        self.locked_tools = None;
        self.provider_session_id = None;
        self.session.provider_session_id = None;
    }

    /// After a request failed for good: switch to the next usable fallback entry. True means retry the
    /// request on the new provider; false means fail as before (nothing configured, nothing usable
    /// left this turn, a context-limit error, or the turn was cancelled).
    pub(super) fn try_provider_fallback(&mut self, walk: &mut FallbackWalk, error: &str) -> bool {
        if self.is_graceful_shutdown() || Self::is_context_limit_error(error) {
            return false;
        }
        let config = factr_base::factr_config::current();
        while let Some(entry) = config.fallbacks.get(walk.next) {
            walk.next += 1;
            if same_route(entry, self.provider.name(), &self.provider.model()) {
                continue;
            }
            let fork = self.provider.fork();
            if let Err(refusal) = fork.set_model(&entry.spec()) {
                crate::logging::warn(&format!("provider fallback: {} skipped: {refusal}", entry.provider));
                continue;
            }
            let from = self.provider.name().to_string();
            let to = fork.name().to_string();
            let now = Instant::now();
            if self.fallback.primary.is_none() {
                self.fallback.primary = Some(Arc::clone(&self.provider));
                if self.fallback.last_switch.is_some_and(|at| now.duration_since(at) > BACKOFF_FORGET) {
                    self.fallback.backoff = 0;
                }
                self.fallback.until = Some(if rate_limited(error) {
                    let wait = cooldown(self.fallback.backoff);
                    self.fallback.backoff += 1;
                    now + wait
                } else {
                    now
                });
            }
            self.fallback.last_switch = Some(now);
            factr_base::obs_sink::emit(
                factr_base::obs_sink::Span::new("loop.guard")
                    .session(&self.session.id)
                    .attr("reason", "provider_fallback")
                    .attr("from", from.clone())
                    .attr("to", to.clone()),
            );
            crate::logging::warn(&format!("provider fallback: {from} failed, continuing the turn on {to}"));
            self.provider = fork;
            self.after_provider_swap();
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cooldown_doubles_from_a_minute_to_four_hours() {
        let secs: Vec<u64> = (0..9).map(|n| cooldown(n).as_secs()).collect();
        assert_eq!(secs, [60, 120, 240, 480, 960, 1920, 3840, 7680, 14400]);
        assert_eq!(cooldown(40), MAX_COOLDOWN);
    }

    #[test]
    fn only_quota_and_rate_failures_arm_the_cooldown() {
        assert!(rate_limited("429 Too Many Requests"));
        assert!(rate_limited("402 payment required"));
        assert!(!rate_limited("500 internal server error"));
        assert!(!rate_limited("connection reset"));
    }

    #[test]
    fn an_entry_equal_to_the_failing_route_is_skipped() {
        let e = factr_base::factr_config::Fallback { provider: "openrouter".into(), model: "a/b".into() };
        assert!(same_route(&e, "OpenRouter", "a/b"));
        assert!(!same_route(&e, "OpenRouter", "other"));
        assert!(!same_route(&e, "Claude", "a/b"));
    }
}
