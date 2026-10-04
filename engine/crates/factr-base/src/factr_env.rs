//! Process-environment setup for Factr-I: the private-deployment switch and its
//! single-provider key shortcut.
//!
//! [`apply`] runs first thing in `main`, before any thread or child starts, so every
//! reader in the engine and the bundled Factr child sees one consistent environment.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

/// One-line refusal shown wherever a login or credential write is disabled.
pub const PRIVATE_MSG: &str = "private deployment: credentials come from the environment";

/// Private deployment: `FACTR_DEPLOYMENT=private`, else `deployment = "private"` in the engine
/// config (`config.toml`). The env var wins when set (any other value means public). Read once per
/// process: `apply` fixes the answer at startup (it even exports it for the Factr child), so the
/// per-request callers never re-read `config.toml`.
pub fn deployment_private() -> bool {
    match DEPLOYMENT_OVERRIDE.load(Ordering::Relaxed) {
        1 => return false,
        2 => return true,
        _ => {}
    }
    static CACHE: OnceLock<bool> = OnceLock::new();
    *CACHE.get_or_init(read_deployment_private)
}

/// 0 = none, 1 = public, 2 = private.
static DEPLOYMENT_OVERRIDE: AtomicU8 = AtomicU8::new(0);

/// Tests only: force the cached answer (`None` clears it).
#[doc(hidden)]
pub fn set_deployment_override_for_tests(private: Option<bool>) {
    DEPLOYMENT_OVERRIDE.store(private.map_or(0, |p| if p { 2 } else { 1 }), Ordering::Relaxed);
}

fn read_deployment_private() -> bool {
    if let Ok(v) = std::env::var("FACTR_DEPLOYMENT") {
        return v.trim().eq_ignore_ascii_case("private");
    }
    factr_storage::app_config_dir()
        .ok()
        .and_then(|dir| std::fs::read_to_string(dir.join("config.toml")).ok())
        .and_then(|text| text.parse::<toml::Table>().ok())
        .and_then(|t| t.get("deployment")?.as_str().map(|v| v.trim().eq_ignore_ascii_case("private")))
        .unwrap_or(false)
}

/// Single-provider shortcut for private mode: `FACTR_API_KEY` fills the provider env var the
/// engine and Factr already use for `FACTR_PROVIDER` (only when that variable is not set).
fn private_key_var(provider: &str) -> Option<&'static str> {
    Some(match provider.trim().to_ascii_lowercase().as_str() {
        "openai" | "openai-api" => "OPENAI_API_KEY",
        "anthropic" | "anthropic-api" | "claude" => "ANTHROPIC_API_KEY",
        "openrouter" => "OPENROUTER_API_KEY",
        _ => return None,
    })
}

/// Apply the private-deployment environment, once.
///
/// # Safety contract
/// Must run before any thread or child process is started (`std::env` is process-global).
pub fn apply() {
    if deployment_private() {
        factr_provider_env::set_env_only(true);
        // Bedrock/AWS: environment credentials only, never ~/.aws or the instance metadata service.
        for (k, v) in [("AWS_CONFIG_FILE", "/dev/null"), ("AWS_SHARED_CREDENTIALS_FILE", "/dev/null"), ("AWS_EC2_METADATA_DISABLED", "true")] {
            // SAFETY: documented contract above.
            unsafe { std::env::set_var(k, v) };
        }
        // SAFETY: documented contract above.
        unsafe { std::env::set_var("FACTR_DEPLOYMENT", "private") }; // the Factr child sees one answer
        if let (Ok(key), Ok(provider)) = (std::env::var("FACTR_API_KEY"), std::env::var("FACTR_PROVIDER"))
            && let Some(var) = private_key_var(&provider)
            && std::env::var_os(var).is_none()
        {
            // SAFETY: as above.
            unsafe { std::env::set_var(var, key) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_mode_env_and_key_shortcut() {
        assert_eq!(private_key_var("OpenAI"), Some("OPENAI_API_KEY"));
        assert_eq!(private_key_var("example"), None);
    }
}
