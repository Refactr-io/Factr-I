use anyhow::Result;

use crate::provider_catalog::{
    load_api_key, env_secret, normalize_api_base,
};

pub const ENDPOINT_ENV: &str = "AZURE_OPENAI_ENDPOINT";
pub const API_KEY_ENV: &str = "AZURE_OPENAI_API_KEY";
pub const MODEL_ENV: &str = "AZURE_OPENAI_MODEL";
pub const USE_ENTRA_ENV: &str = "AZURE_OPENAI_USE_ENTRA";
pub const COGNITIVE_SCOPE: &str = "https://cognitiveservices.azure.com/.default";

fn parse_bool(raw: &str) -> Option<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

pub fn normalize_endpoint(raw: &str) -> Option<String> {
    let mut endpoint = normalize_api_base(raw)?;
    if endpoint.ends_with("/openai/v1") {
        return Some(endpoint);
    }
    endpoint.push_str("/openai/v1");
    Some(endpoint)
}

pub fn load_endpoint() -> Option<String> {
    let raw = env_secret(ENDPOINT_ENV)?;
    normalize_endpoint(&raw)
}

pub fn load_model() -> Option<String> {
    env_secret(MODEL_ENV)
}

pub fn has_api_key() -> bool {
    load_api_key(API_KEY_ENV).is_some()
}

pub fn uses_entra_id() -> bool {
    env_secret(USE_ENTRA_ENV)
        .and_then(|value| parse_bool(&value))
        .unwrap_or(false)
}

pub fn has_configuration() -> bool {
    load_endpoint().is_some() && (has_api_key() || uses_entra_id())
}

pub fn method_detail() -> String {
    let mut parts = Vec::new();
    if has_api_key() {
        parts.push(format!("API key (`{API_KEY_ENV}`)"));
    }
    if uses_entra_id() {
        parts.push("Microsoft Entra ID (DefaultAzureCredential)".to_string());
    }
    if parts.is_empty() {
        "not configured".to_string()
    } else {
        parts.join(" + ")
    }
}

pub fn apply_runtime_env() -> Result<()> {
    let endpoint = load_endpoint().ok_or_else(|| {
        anyhow::anyhow!(
            "{} not found in the Factr keys or the environment",
            ENDPOINT_ENV
        )
    })?;

    crate::env::set_var("FACTR_OPENROUTER_API_BASE", endpoint);
    crate::env::set_var("FACTR_OPENROUTER_API_KEY_NAME", API_KEY_ENV);
    crate::env::set_var("FACTR_OPENROUTER_CACHE_NAMESPACE", "azure-openai");
    crate::env::set_var("FACTR_OPENROUTER_PROVIDER_FEATURES", "0");
    crate::env::set_var("FACTR_OPENROUTER_TRANSPORT_STATE", "direct-api-key");
    crate::env::set_var("FACTR_OPENROUTER_MODEL_CATALOG", "0");

    if uses_entra_id() {
        crate::env::set_var("FACTR_OPENROUTER_AUTH_HEADER", "authorization-bearer");
        crate::env::set_var("FACTR_OPENROUTER_DYNAMIC_BEARER_PROVIDER", "azure");
    } else {
        crate::env::set_var("FACTR_OPENROUTER_AUTH_HEADER", "api-key");
        crate::env::remove_var("FACTR_OPENROUTER_DYNAMIC_BEARER_PROVIDER");
    }

    Ok(())
}

pub async fn get_bearer_token() -> Result<String> {
    factr_azure_auth::get_bearer_token(COGNITIVE_SCOPE).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_endpoint_appends_openai_v1() {
        assert_eq!(
            normalize_endpoint("https://example.openai.azure.com"),
            Some("https://example.openai.azure.com/openai/v1".to_string())
        );
    }

    #[test]
    fn normalize_endpoint_preserves_existing_openai_v1() {
        assert_eq!(
            normalize_endpoint("https://example.openai.azure.com/openai/v1/"),
            Some("https://example.openai.azure.com/openai/v1".to_string())
        );
    }

    #[test]
    fn normalize_endpoint_rejects_insecure_remote_http() {
        assert_eq!(normalize_endpoint("http://example.com"), None);
    }
}
