//! Per-session request hints for unattended runs: settings the gateway picks for one run (not the process), read
//! where the request is built (same process only). Cleared when the run ends.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

static TEXT_VERBOSITY: LazyLock<Mutex<HashMap<String, String>>> = LazyLock::new(Default::default);

/// The default `text.verbosity` for `session_id`'s requests (`low`, `medium` or `high`); an explicit setting
/// (`FACTR_OPENAI_TEXT_VERBOSITY`, `agent.text_verbosity`) still wins.
pub fn set_text_verbosity(session_id: &str, verbosity: &str) {
    TEXT_VERBOSITY.lock().unwrap_or_else(|e| e.into_inner()).insert(session_id.to_string(), verbosity.to_string());
}

pub fn clear(session_id: &str) {
    TEXT_VERBOSITY.lock().unwrap_or_else(|e| e.into_inner()).remove(session_id);
}

/// The verbosity a request for `session_id` carries: the explicit setting, else this run's default.
pub fn text_verbosity(session_id: Option<&str>) -> Option<String> {
    crate::factr_config::text_verbosity().or_else(|| {
        let session = session_id?;
        TEXT_VERBOSITY.lock().unwrap_or_else(|e| e.into_inner()).get(session).cloned()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_runs_default_applies_to_its_session_only_until_cleared() {
        let _env = crate::storage::lock_test_env();
        crate::env::remove_var("FACTR_OPENAI_TEXT_VERBOSITY");
        set_text_verbosity("s-hint", "low");
        assert_eq!(text_verbosity(Some("s-hint")).as_deref(), Some("low"));
        assert_eq!(text_verbosity(Some("other")), None);
        assert_eq!(text_verbosity(None), None);
        crate::env::set_var("FACTR_OPENAI_TEXT_VERBOSITY", "high");
        assert_eq!(text_verbosity(Some("s-hint")).as_deref(), Some("high"), "an explicit setting wins");
        crate::env::remove_var("FACTR_OPENAI_TEXT_VERBOSITY");
        clear("s-hint");
        assert_eq!(text_verbosity(Some("s-hint")), None);
    }
}
