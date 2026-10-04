//! Deployment mode. Public (default) is Factr parity: logins and the credential store work as in
//! Factr. Private (`FACTR_DEPLOYMENT=private`, else `deployment = "private"` in the engine
//! `config.toml`) removes every login: keys come from the environment only.

pub use factr_base::factr_env::PRIVATE_MSG;

pub fn is_private() -> bool {
    factr_base::factr_env::deployment_private()
}

/// REST routes that start a login or write the credential store.
pub fn blocks_rest(method: &str, path: &str) -> bool {
    path.starts_with("/api/providers")
        || (path == "/api/env" && matches!(method, "PUT" | "DELETE" | "POST"))
}

/// Forwarded RPC methods that write credentials.
pub fn blocks_rpc(method: &str) -> bool {
    matches!(method, "model.save_key" | "model.disconnect")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_and_methods() {
        assert!(blocks_rest("POST", "/api/providers/oauth/openai-codex/start"));
        assert!(blocks_rest("GET", "/api/providers"));
        assert!(blocks_rest("PUT", "/api/env") && !blocks_rest("GET", "/api/env"));
        assert!(blocks_rpc("model.save_key") && !blocks_rpc("model.options"));
    }

    #[test]
    fn the_cached_mode_can_be_overridden_in_tests() {
        let _g = crate::factr_env::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        factr_base::factr_env::set_deployment_override_for_tests(Some(true));
        assert!(is_private());
        factr_base::factr_env::set_deployment_override_for_tests(Some(false));
        assert!(!is_private());
        factr_base::factr_env::set_deployment_override_for_tests(None);
    }
}
