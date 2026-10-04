//! Corrections and preferences the memory extractor stored this session, for the learning gate.
//!
//! Extraction is the one writer of what the user says about themselves and their corrections; the
//! gateway's learning gate reads this as one signal that a review is worth running (`pending`, then
//! `take` once the window has been judged). In-process
//! only, same as [`crate::headless`]: nothing is persisted, and a restart just forgets the hint.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

static HITS: LazyLock<Mutex<HashMap<String, usize>>> = LazyLock::new(Default::default);

/// The extractor stored a high-trust correction or preference from `session`.
pub fn note(session_id: &str) {
    *HITS.lock().unwrap_or_else(|e| e.into_inner()).entry(session_id.to_string()).or_default() += 1;
}

/// How many were stored since the last `take`, without forgetting them: a gate that is not due yet
/// must not lose the cue.
pub fn pending(session_id: &str) -> usize {
    HITS.lock().unwrap_or_else(|e| e.into_inner()).get(session_id).copied().unwrap_or_default()
}

/// How many were stored since the last `take`, and forget them.
pub fn take(session_id: &str) -> usize {
    HITS.lock().unwrap_or_else(|e| e.into_inner()).remove(session_id).unwrap_or_default()
}

/// Drop a closed session's count.
pub fn forget(session_id: &str) {
    take(session_id);
}
