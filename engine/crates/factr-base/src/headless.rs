//! Sessions driven with no user attached (`/api/agent/run`). The gateway marks them;
//! the agent loop reads the mark (same process only).

use std::collections::HashSet;
use std::sync::{LazyLock, Mutex};

static SESSIONS: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(Default::default);

pub fn mark(session_id: &str) {
    SESSIONS.lock().unwrap_or_else(|e| e.into_inner()).insert(session_id.to_string());
}

pub fn unmark(session_id: &str) {
    SESSIONS.lock().unwrap_or_else(|e| e.into_inner()).remove(session_id);
}

/// The whole process is headless (`FACTR_HEADLESS=1`, normalised onto `FACTR_HEADLESS`).
pub fn process() -> bool {
    std::env::var("FACTR_HEADLESS").is_ok_and(|v| v == "1")
}

/// Marked by the gateway, or the whole process is headless.
pub fn is(session_id: &str) -> bool {
    process()
        || SESSIONS.lock().unwrap_or_else(|e| e.into_inner()).contains(session_id)
}
