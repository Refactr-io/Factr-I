use std::sync::Arc;
use tokio::sync::OnceCell;

/// Test mode: `FACTR_DEBUG_CONTROL` truthy binds the introspection socket and keeps the idle-timeout
/// monitor off. The integration tests set it; a normal run does neither.
pub(crate) fn debug_control_allowed() -> bool {
    factr_base::factr_config::env_bool("FACTR_DEBUG_CONTROL").unwrap_or(false)
}

pub(crate) async fn get_shared_mcp_pool(
    cell: &OnceCell<Arc<crate::mcp::SharedMcpPool>>,
) -> Arc<crate::mcp::SharedMcpPool> {
    cell.get_or_init(|| async { Arc::new(crate::mcp::SharedMcpPool::from_default_config()) })
        .await
        .clone()
}

/// Return the swarm identity for an independently-created root session.
///
/// Swarm plans are keyed by swarm id. Deriving that id from the working
/// directory made every session opened in one repository share one plan, even
/// when those sessions were unrelated. Root sessions therefore own a swarm by
/// default. `FACTR_SWARM_ID` remains an explicit opt-in to a shared swarm.
pub(crate) fn swarm_id_for_session(session_id: &str) -> Option<String> {
    if let Ok(sw_id) = std::env::var("FACTR_SWARM_ID") {
        let trimmed = sw_id.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    default_swarm_id_for_session(session_id)
}

fn default_swarm_id_for_session(session_id: &str) -> Option<String> {
    if session_id.trim().is_empty() {
        None
    } else {
        Some(format!("session:{session_id}"))
    }
}

#[cfg(test)]
mod swarm_identity_tests {
    use super::default_swarm_id_for_session;

    #[test]
    fn independent_root_sessions_have_distinct_swarm_ids() {
        assert_eq!(
            default_swarm_id_for_session("session-one").as_deref(),
            Some("session:session-one")
        );
        assert_eq!(
            default_swarm_id_for_session("session-two").as_deref(),
            Some("session:session-two")
        );
        assert_ne!(
            default_swarm_id_for_session("session-one"),
            default_swarm_id_for_session("session-two")
        );
    }

    #[test]
    fn empty_session_cannot_own_a_swarm() {
        assert_eq!(default_swarm_id_for_session("  "), None);
    }
}

/// Decide whether any reload candidate is *provably* newer than the running
/// server binary.
///
/// This is intentionally conservative. An earlier version reported "update
/// available" whenever the mtime comparison was inconclusive (e.g. a metadata
/// read failed) as long as the candidate path differed from the running exe.
/// On some systems that fallback fired permanently, so the client would
/// auto-reload the server, the server would exec into the candidate, and the
/// freshly-exec'd server would again report an update -> an infinite reload
/// loop that flickers the terminal (see issue #277).
///
/// We now only report an update when we can read both mtimes and the candidate
/// is strictly newer than the running binary. Any uncertainty suppresses the
/// auto-reload signal so it can never wedge the client into a loop.
/// Server identity for multi-server support
#[derive(Debug, Clone)]
pub struct ServerIdentity {
    /// Full server ID (e.g., "server_blazing_1705012345678")
    pub id: String,
    /// Short name (e.g., "blazing")
    pub name: String,
    /// Icon for display (e.g., "🔥")
    pub icon: String,
    /// Git hash of the binary
    pub git_hash: String,
    /// Version string (e.g., "v0.1.123")
    pub version: String,
}

impl ServerIdentity {
    /// Display name with icon (e.g., "🔥 blazing")
    pub fn display_name(&self) -> String {
        format!("{} {}", self.icon, self.name)
    }
}

pub(crate) fn startup_headless_recovery_test_delay() -> Option<std::time::Duration> {
    let raw = std::env::var("FACTR_TEST_HEADLESS_STARTUP_RECOVERY_DELAY_MS").ok()?;
    let delay_ms = raw.trim().parse::<u64>().ok()?;
    (delay_ms > 0).then(|| std::time::Duration::from_millis(delay_ms))
}

