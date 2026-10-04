use std::path::{Path, PathBuf};
use std::sync::MutexGuard;

use crate::provider_catalog::openai_compatible_profiles;

pub struct AuthTestSandbox {
    _lock: MutexGuard<'static, ()>,
    temp: tempfile::TempDir,
    saved_env: Vec<(String, Option<String>)>,
}

impl AuthTestSandbox {
    pub fn new() -> anyhow::Result<Self> {
        let lock = crate::storage::lock_test_env();
        let temp = tempfile::Builder::new()
            .prefix("factr-auth-lifecycle-")
            .tempdir()?;
        let saved_env = tracked_env_vars()
            .into_iter()
            .map(|key| {
                let value = std::env::var(&key).ok();
                (key, value)
            })
            .collect::<Vec<_>>();

        for (key, _) in &saved_env {
            crate::env::remove_var(key);
        }

        std::fs::create_dir_all(temp.path().join("config").join("factr"))?;
        std::fs::create_dir_all(temp.path().join("external"))?;
        crate::env::set_var("FACTR_HOME", temp.path());
        crate::provider_catalog::force_apply_openai_compatible_profile_env(None);
        reset_global_auth_state();

        Ok(Self {
            _lock: lock,
            temp,
            saved_env,
        })
    }

    pub fn root(&self) -> &Path {
        self.temp.path()
    }

    pub fn config_dir(&self) -> PathBuf {
        self.root().join("config").join("factr")
    }

    pub fn external_dir(&self) -> PathBuf {
        self.root().join("external")
    }
}

impl Drop for AuthTestSandbox {
    fn drop(&mut self) {
        for (key, value) in self.saved_env.drain(..) {
            if let Some(value) = value {
                crate::env::set_var(&key, value);
            } else {
                crate::env::remove_var(&key);
            }
        }
        reset_global_auth_state();
    }
}

fn reset_global_auth_state() {
    crate::auth::AuthStatus::invalidate_cache();
    crate::provider::clear_all_provider_unavailability_for_account();
    crate::provider::clear_all_model_unavailability_for_account();
}

fn tracked_env_vars() -> Vec<String> {
    let mut keys = [
        "FACTR_HOME",
        "XDG_CONFIG_HOME",
        "FACTR_OPENROUTER_API_BASE",
        "FACTR_OPENROUTER_API_KEY_NAME",
        "FACTR_OPENROUTER_CACHE_NAMESPACE",
        "FACTR_OPENROUTER_PROVIDER_FEATURES",
        "FACTR_OPENROUTER_TRANSPORT_STATE",
        "FACTR_OPENROUTER_ALLOW_NO_AUTH",
        "FACTR_OPENROUTER_PROVIDER",
        "FACTR_OPENROUTER_NO_FALLBACK",
        "FACTR_OPENROUTER_MODEL",
        "FACTR_OPENROUTER_MODEL_CATALOG",
        "FACTR_OPENROUTER_STATIC_MODELS",
        "FACTR_OPENROUTER_AUTH_HEADER",
        "FACTR_OPENROUTER_AUTH_HEADER_NAME",
        "FACTR_OPENROUTER_DYNAMIC_BEARER_PROVIDER",
        "FACTR_OPENAI_COMPAT_API_BASE",
        "FACTR_OPENAI_COMPAT_API_KEY_NAME",
        "FACTR_OPENAI_COMPAT_SETUP_URL",
        "FACTR_OPENAI_COMPAT_DEFAULT_MODEL",
        "FACTR_OPENAI_COMPAT_LOCAL_ENABLED",
        "FACTR_NAMED_PROVIDER_PROFILE",
        "FACTR_PROVIDER_PROFILE_ACTIVE",
        "FACTR_PROVIDER_PROFILE_NAME",
        "FACTR_RUNTIME_PROVIDER",
        "FACTR_ACTIVE_PROVIDER",
        "FACTR_INITIAL_PROVIDER_EXPLICIT",
        "OPENAI_API_KEY",
        "OPENROUTER_API_KEY",
        "ANTHROPIC_API_KEY",
        "AZURE_OPENAI_ENDPOINT",
        "AZURE_OPENAI_MODEL",
        "AZURE_OPENAI_API_KEY",
        "AZURE_OPENAI_USE_ENTRA",
        "GOOGLE_API_KEY",
        "GEMINI_API_KEY",
        "CURSOR_API_KEY",
        "BEDROCK_API_KEY",
    ]
    .into_iter()
    .map(ToString::to_string)
    .collect::<std::collections::HashSet<_>>();

    for profile in openai_compatible_profiles() {
        keys.insert(profile.api_key_env.to_string());
    }

    let mut keys = keys.into_iter().collect::<Vec<_>>();
    keys.sort();
    keys
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sandbox_isolates_factr_home_and_config_dir() {
        let sandbox = AuthTestSandbox::new().expect("sandbox");

        assert_eq!(
            std::env::var("FACTR_HOME").ok().as_deref(),
            Some(sandbox.root().to_str().unwrap())
        );
        assert_eq!(
            crate::storage::app_config_dir().unwrap(),
            sandbox.config_dir()
        );
        assert!(sandbox.config_dir().starts_with(sandbox.root()));
        assert!(sandbox.external_dir().starts_with(sandbox.root()));
        assert!(sandbox.external_dir().exists());
    }
}
