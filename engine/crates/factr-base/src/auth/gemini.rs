//! Official Gemini Developer API key (Google AI Studio) resolution.
//!
//! An API key authenticates directly against `generativelanguage.googleapis.com`
//! and uses the key's own quota. There is no OAuth login for Gemini.

/// Environment variable names that hold an official Gemini Developer API key
/// (Google AI Studio). Checked in order; the first non-empty value wins.
pub const GEMINI_API_KEY_ENV_VARS: &[&str] = &["GEMINI_API_KEY", "GOOGLE_API_KEY"];

/// Resolve an official Gemini Developer API key from the credential store (Factr `.env`) or the
/// environment.
pub fn api_key() -> Option<String> {
    for env_key in GEMINI_API_KEY_ENV_VARS {
        if let Some(key) = crate::provider_catalog::load_api_key(env_key) {
            return Some(key);
        }
    }
    None
}

/// True when an official Gemini Developer API key is configured.
pub fn has_api_key() -> bool {
    api_key().is_some()
}
