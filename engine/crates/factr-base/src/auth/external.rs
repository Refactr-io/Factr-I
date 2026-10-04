use anyhow::{Context, Result};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;

pub const OPENCODE_AUTH_JSON_SOURCE_ID: &str = "opencode_auth_json";
pub const PI_AUTH_JSON_SOURCE_ID: &str = "pi_auth_json";
pub const OPENCLAW_AUTH_JSON_SOURCE_ID: &str = "openclaw_auth_json";
pub const FACTR_AUTH_JSON_SOURCE_ID: &str = "factr_auth_json";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalAuthSource {
    OpenCode,
    Pi,
    OpenClaw,
    Factr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalOAuthTokens {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: i64,
}

impl ExternalAuthSource {
    pub fn source_id(self) -> &'static str {
        match self {
            Self::OpenCode => OPENCODE_AUTH_JSON_SOURCE_ID,
            Self::Pi => PI_AUTH_JSON_SOURCE_ID,
            Self::OpenClaw => OPENCLAW_AUTH_JSON_SOURCE_ID,
            Self::Factr => FACTR_AUTH_JSON_SOURCE_ID,
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::OpenCode => "OpenCode auth.json",
            Self::Pi => "pi auth.json",
            Self::OpenClaw => "OpenClaw auth.json",
            Self::Factr => "Factr auth.json",
        }
    }

    pub fn path(self) -> Result<PathBuf> {
        match self {
            Self::OpenCode => crate::storage::user_home_path(".local/share/opencode/auth.json"),
            Self::Pi => crate::storage::user_home_path(".pi/agent/auth.json"),
            Self::OpenClaw => openclaw_auth_path(),
            Self::Factr => match crate::factr_config::home() {
                Some(home) => Ok(home.join("auth.json")),
                None => crate::storage::user_home_path(".factr/auth.json"),
            },
        }
    }
}

/// Resolve OpenClaw's credential file. OpenClaw has moved its auth store over
/// time, so probe the known locations and return the first that exists:
///
///   1. `~/.openclaw/agent/auth.json` - the original pi-fork layout.
///   2. `~/.openclaw/agents/<agentId>/agent/auth-profiles.json` - the current
///      per-agent profile store (`main` is checked first, then any agent).
///   3. `~/.openclaw/agents/<agentId>/agent/auth.json` - per-agent legacy file.
///   4. `~/.openclaw/credentials/oauth.json` - legacy import-only OAuth file.
///
/// Falls back to the pi-fork path when nothing exists (so consent bookkeeping
/// always has a stable path to record).
fn openclaw_auth_path() -> Result<PathBuf> {
    let legacy = crate::storage::user_home_path(".openclaw/agent/auth.json")?;
    if legacy.is_file() {
        return Ok(legacy);
    }

    let agents_root = crate::storage::user_home_path(".openclaw/agents")?;
    let agent_dirs = || -> Vec<PathBuf> {
        let mut dirs = Vec::new();
        // Check the default agent first so multi-agent installs resolve
        // deterministically to the main store.
        dirs.push(agents_root.join("main"));
        if let Ok(entries) = std::fs::read_dir(&agents_root) {
            let mut rest: Vec<PathBuf> = entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir() && p.file_name().is_some_and(|n| n != "main"))
                .collect();
            rest.sort();
            dirs.extend(rest);
        }
        dirs
    };
    for dir in agent_dirs() {
        let profiles = dir.join("agent/auth-profiles.json");
        if profiles.is_file() {
            return Ok(profiles);
        }
        let auth = dir.join("agent/auth.json");
        if auth.is_file() {
            return Ok(auth);
        }
    }

    let credentials = crate::storage::user_home_path(".openclaw/credentials/oauth.json")?;
    if credentials.is_file() {
        return Ok(credentials);
    }

    Ok(legacy)
}

const SOURCES: [ExternalAuthSource; 4] = [
    ExternalAuthSource::OpenCode,
    ExternalAuthSource::Pi,
    ExternalAuthSource::OpenClaw,
    ExternalAuthSource::Factr,
];

pub fn trust_external_auth_source(source: ExternalAuthSource) -> Result<()> {
    crate::config::Config::allow_external_auth_source_for_path(
        source.source_id(),
        &source.path()?,
    )?;
    super::AuthStatus::invalidate_cache();
    Ok(())
}

pub fn has_any_unconsented_external_auth() -> bool {
    SOURCES
        .into_iter()
        .filter(|source| source.path().map(|path| path.exists()).unwrap_or(false))
        .any(|source| !source_allowed(source) && source_has_supported_auth(source))
}

pub fn unconsented_sources() -> Vec<ExternalAuthSource> {
    SOURCES
        .into_iter()
        .filter(|source| source.path().map(|path| path.exists()).unwrap_or(false))
        .filter(|source| !source_allowed(*source) && source_has_supported_auth(*source))
        .collect()
}

pub fn source_provider_labels(source: ExternalAuthSource) -> Vec<&'static str> {
    let mut labels = Vec::new();
    if source_contains_oauth_provider(source, &["openai-codex", "openai_codex", "openai"])
        .unwrap_or(false)
    {
        labels.push("OpenAI/Codex");
    }
    if source_contains_oauth_provider(source, &["anthropic", "claude"]).unwrap_or(false) {
        labels.push("Claude");
    }
    if source_contains_oauth_provider(source, &["github-copilot", "copilot"]).unwrap_or(false) {
        labels.push("GitHub Copilot");
    }
    if source_contains_supported_api_key(source).unwrap_or(false) {
        labels.push("OpenRouter/API-key providers");
    }
    labels
}

pub fn preferred_unconsented_api_key_source() -> Option<ExternalAuthSource> {
    SOURCES
        .into_iter()
        .filter(|source| source.path().map(|path| path.exists()).unwrap_or(false))
        .find(|source| {
            !source_allowed(*source) && source_contains_supported_api_key(*source).unwrap_or(false)
        })
}

pub fn preferred_unconsented_api_key_source_for_env(env_key: &str) -> Option<ExternalAuthSource> {
    SOURCES
        .into_iter()
        .filter(|source| source.path().map(|path| path.exists()).unwrap_or(false))
        .find(|source| {
            !source_allowed(*source)
                && load_api_key_from_source(*source, env_key)
                    .map(|key| !key.trim().is_empty())
                    .unwrap_or(false)
        })
}

pub fn preferred_unconsented_openai_oauth_source() -> Option<ExternalAuthSource> {
    preferred_unconsented_oauth_source_for_candidates(&["openai-codex", "openai_codex", "openai"])
}

pub fn preferred_unconsented_anthropic_oauth_source() -> Option<ExternalAuthSource> {
    preferred_unconsented_oauth_source_for_candidates(&["anthropic", "claude"])
}

pub fn load_api_key_for_env(env_key: &str) -> Option<String> {
    for source in SOURCES {
        if !source_allowed(source) {
            continue;
        }
        if let Some(key) = load_api_key_from_source(source, env_key) {
            return Some(key);
        }
    }
    None
}

pub fn load_openai_oauth_tokens() -> Option<ExternalOAuthTokens> {
    load_oauth_tokens_for_candidates(&["openai-codex", "openai_codex", "openai"])
}

pub fn load_copilot_oauth_token() -> Option<String> {
    load_oauth_tokens_for_candidates(&["github-copilot", "copilot"])
        .map(|tokens| tokens.access_token)
}

pub fn source_has_copilot_oauth(source: ExternalAuthSource) -> bool {
    source_contains_oauth_provider(source, &["github-copilot", "copilot"]).unwrap_or(false)
}

pub fn load_anthropic_oauth_tokens() -> Option<ExternalOAuthTokens> {
    load_oauth_tokens_for_candidates(&["anthropic", "claude"])
}

const FACTR_EXPIRY_SKEW_MS: i64 = 5 * 60 * 1000;
const FACTR_REFRESH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
static FACTR_REFRESH_CMD: std::sync::Mutex<Option<Vec<String>>> = std::sync::Mutex::new(None);
static FACTR_REFRESH_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
static FACTR_CODEX_EXPIRED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The bundled Factr command (program + leading args) used to refresh the Codex grant.
pub fn set_factr_refresh_command(cmd: Vec<String>) {
    *FACTR_REFRESH_CMD.lock().unwrap_or_else(|e| e.into_inner()) = Some(cmd);
}

/// True when the last look at the Factr Codex grant found it expired and Factr could not refresh it.
pub fn factr_codex_expired() -> bool {
    FACTR_CODEX_EXPIRED.load(std::sync::atomic::Ordering::Relaxed)
}

/// Ask Factr (the one refresher) for a new Codex access token after the server rejected
/// `rejected_access_token`, then re-read the store. Blocking: runs the Factr command (30 s cap).
/// The returned tokens carry no refresh token.
pub fn refresh_factr_codex_after_rejection(rejected_access_token: &str) -> Option<ExternalOAuthTokens> {
    let now_ms = chrono::Utc::now().timestamp_millis();
    let mut tokens = refresh_factr_codex(ExternalAuthSource::Factr, now_ms, Some(rejected_access_token))?;
    tokens.refresh_token.clear();
    Some(tokens)
}

pub const CODEX_EXPIRED_MSG: &str = "Codex login expired: run factr auth add openai-codex or reconnect in Settings";

/// Run `factr auth refresh openai-codex` (30 s cap), one at a time, and re-read the store.
/// Callers that waited on the lock find a fresh grant and do not spawn again. Never logs tokens.
///
/// `rejected_access_token` is the bearer the server just refused: a re-read that still returns it
/// is not a refresh, so Factr is asked to mint a new one.
fn refresh_factr_codex(
    source: ExternalAuthSource,
    now_ms: i64,
    rejected_access_token: Option<&str>,
) -> Option<ExternalOAuthTokens> {
    let reread = || {
        let entry = load_auth_map(source).ok()?.get("openai-codex")?.clone();
        extract_oauth_tokens(source, &entry)
            .filter(|t| t.expires_at > now_ms + FACTR_EXPIRY_SKEW_MS)
            .filter(|t| Some(t.access_token.as_str()) != rejected_access_token)
    };
    let _one_at_a_time = FACTR_REFRESH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(fresh) = reread() {
        return Some(fresh);
    }
    let cmd = FACTR_REFRESH_CMD.lock().unwrap_or_else(|e| e.into_inner()).clone()?;
    let (program, args) = cmd.split_first()?;
    let mut command = std::process::Command::new(program);
    command.args(args).args(["auth", "refresh", "openai-codex"])
        .stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    if let Some(home) = source.path().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())) {
        command.env("FACTR_CONFIG_HOME", home);
    }
    let mut child = command.spawn().ok()?;
    let started = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() < FACTR_REFRESH_TIMEOUT => std::thread::sleep(std::time::Duration::from_millis(25)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let fresh = reread();
    if fresh.is_some() {
        FACTR_CODEX_EXPIRED.store(false, std::sync::atomic::Ordering::Relaxed);
    }
    fresh
}

static FACTR_STORE_OWN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The engine running inside the Factr-I bundle owns the Factr store (the UI and `factr login`
/// write it), so it needs no per-file trust prompt. Never set in private deployment.
pub fn treat_factr_store_as_own() {
    FACTR_STORE_OWN.store(true, std::sync::atomic::Ordering::Relaxed);
}

pub fn source_allowed(source: ExternalAuthSource) -> bool {
    if factr_provider_env::env_only() {
        return false;
    }
    if source == ExternalAuthSource::Factr && FACTR_STORE_OWN.load(std::sync::atomic::Ordering::Relaxed) {
        return true;
    }
    let Ok(path) = source.path() else {
        return false;
    };

    if crate::config::Config::external_auth_source_allowed_for_path(source.source_id(), &path) {
        return true;
    }

    match source {
        ExternalAuthSource::OpenCode => {
            crate::config::Config::external_auth_source_allowed_for_path(
                crate::auth::claude::OPENCODE_AUTH_SOURCE_ID,
                &path,
            )
        }
        ExternalAuthSource::Pi | ExternalAuthSource::OpenClaw | ExternalAuthSource::Factr => false,
    }
}

fn load_oauth_tokens_for_candidates(provider_keys: &[&str]) -> Option<ExternalOAuthTokens> {
    let now_ms = chrono::Utc::now().timestamp_millis();
    let mut expired: Option<ExternalOAuthTokens> = None;

    for source in SOURCES {
        if !source_allowed(source) {
            continue;
        }

        let Ok(auth_map) = load_auth_map(source) else {
            continue;
        };
        for key in provider_keys {
            if let Some(entry) = auth_map.get(*key)
                && let Some(mut tokens) = extract_oauth_tokens(source, entry)
            {
                // Factr's Codex grant: Factr's Python backend is the ONE refresher.
                if source == ExternalAuthSource::Factr
                    && *key == "openai-codex"
                    && tokens.expires_at <= now_ms + FACTR_EXPIRY_SKEW_MS
                    && let Some(fresh) = refresh_factr_codex(source, now_ms, None)
                {
                    tokens = fresh;
                }
                // Codex refresh tokens are single use and Factr is their only spender: the
                // engine never holds the Factr one, so nothing in-process can burn it.
                if source == ExternalAuthSource::Factr && provider_keys.contains(&"openai-codex") {
                    tokens.refresh_token.clear();
                }
                if tokens.expires_at > now_ms {
                    return Some(tokens);
                }
                // Codex refresh tokens are single use: only Factr's Python backend may spend one,
                // so an expired Factr grant is never handed to the engine's own refresher.
                if source == ExternalAuthSource::Factr {
                    if *key == "openai-codex" {
                        FACTR_CODEX_EXPIRED.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                    continue;
                }
                if expired.is_none() {
                    expired = Some(tokens);
                }
            }
        }
    }

    expired
}

fn preferred_unconsented_oauth_source_for_candidates(
    provider_keys: &[&str],
) -> Option<ExternalAuthSource> {
    SOURCES
        .into_iter()
        .filter(|source| source.path().map(|path| path.exists()).unwrap_or(false))
        .find(|source| {
            !source_allowed(*source)
                && source_contains_oauth_provider(*source, provider_keys).unwrap_or(false)
        })
}

fn source_has_supported_auth(source: ExternalAuthSource) -> bool {
    source_contains_supported_api_key(source).unwrap_or(false)
        || source_contains_oauth_provider(
            source,
            &[
                "openai-codex",
                "openai_codex",
                "openai",
                "anthropic",
                "claude",
                "github-copilot",
                "copilot",
            ],
        )
        .unwrap_or(false)
}

fn source_contains_supported_api_key(source: ExternalAuthSource) -> Result<bool> {
    let auth = load_auth_map(source)?;
    Ok(auth
        .values()
        .any(|entry| extract_api_key(source, entry).is_some()))
}

fn source_contains_oauth_provider(
    source: ExternalAuthSource,
    provider_keys: &[&str],
) -> Result<bool> {
    let auth = load_auth_map(source)?;
    Ok(provider_keys.iter().any(|provider_key| {
        auth.get(*provider_key)
            .and_then(|entry| extract_oauth_tokens(source, entry))
            .is_some()
    }))
}

fn load_api_key_from_source(source: ExternalAuthSource, env_key: &str) -> Option<String> {
    let auth = load_auth_map(source).ok()?;
    for &provider_key in provider_keys_for_env(env_key) {
        if let Some(entry) = auth.get(provider_key)
            && let Some(key) = extract_api_key(source, entry)
            && !key.trim().is_empty()
        {
            return Some(key);
        }
    }
    None
}

fn load_auth_map(source: ExternalAuthSource) -> Result<HashMap<String, Value>> {
    let path = crate::storage::validate_external_auth_file(&source.path()?)?;
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read {}", path.display()))?;
    let value: Value = serde_json::from_str(&raw)
        .with_context(|| format!("Failed to parse {}", path.display()))?;
    match source {
        ExternalAuthSource::OpenCode | ExternalAuthSource::Pi => {
            // Flat `provider -> credential` maps.
            Ok(value
                .as_object()
                .map(|object| {
                    object
                        .iter()
                        .map(|(key, value)| (key.clone(), value.clone()))
                        .collect()
                })
                .unwrap_or_default())
        }
        ExternalAuthSource::OpenClaw => Ok(flatten_openclaw_auth_store(&value)),
        ExternalAuthSource::Factr => Ok(flatten_factr_auth_store(&value)),
    }
}

/// OpenClaw historically used the flat pi-style `provider -> credential` map,
/// but its current store is `auth-profiles.json`:
///
/// ```json
/// {
///   "version": 1,
///   "profiles": {
///     "openai:default": { "type": "oauth", "provider": "openai", "access": ..., "refresh": ..., "expires": ... },
///     "openrouter:default": { "type": "api_key", "provider": "openrouter", "key": "..." }
///   }
/// }
/// ```
///
/// Normalize both shapes to a flat `provider -> credential` map. Profile
/// entries keep the pi-style credential fields, so the shared extractors work
/// unchanged. When several profiles exist for one provider, the `<provider>:default`
/// profile wins; otherwise the first seen is kept.
fn flatten_openclaw_auth_store(value: &Value) -> HashMap<String, Value> {
    let Some(object) = value.as_object() else {
        return HashMap::new();
    };
    let Some(profiles) = object.get("profiles").and_then(Value::as_object) else {
        // Legacy flat map.
        return object
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
    };

    let mut map: HashMap<String, Value> = HashMap::new();
    for (profile_id, entry) in profiles {
        let provider = entry
            .get("provider")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .or_else(|| profile_id.split(':').next().map(ToOwned::to_owned));
        let Some(provider) = provider else { continue };
        let is_default = profile_id.ends_with(":default") || !profile_id.contains(':');
        if is_default || !map.contains_key(&provider) {
            map.insert(provider, entry.clone());
        }
    }
    map
}

/// Factr persists credentials in a nested store:
///
/// ```json
/// {
///   "version": 1,
///   "active_provider": "anthropic",
///   "credential_pool": { "<provider>": [ { "auth_type": ..., "access_token": ... }, ... ] },
///   "providers": { "<provider>": { ...singleton state... } }
/// }
/// ```
///
/// Normalize it to a flat `provider -> representative credential` map so the
/// shared extraction logic can treat it like the other sources. The highest
/// priority (first) credential-pool entry wins; legacy `providers.<id>` blocks
/// are used only when the pool has no entry for that provider.
fn flatten_factr_auth_store(value: &Value) -> HashMap<String, Value> {
    let mut map: HashMap<String, Value> = HashMap::new();

    if let Some(providers) = value.get("providers").and_then(Value::as_object) {
        for (provider, state) in providers {
            map.insert(provider.clone(), state.clone());
        }
    }

    if let Some(pool) = value.get("credential_pool").and_then(Value::as_object) {
        for (provider, entries) in pool {
            if let Some(first) = entries.as_array().and_then(|entries| entries.first()) {
                // Credential-pool entries are authoritative over legacy blocks.
                map.insert(provider.clone(), first.clone());
            }
        }
    }

    map
}

fn extract_api_key(source: ExternalAuthSource, entry: &Value) -> Option<String> {
    let object = entry.as_object()?;
    match source {
        ExternalAuthSource::OpenCode => {
            if object.get("type")?.as_str()? != "api" {
                return None;
            }
            object
                .get("key")?
                .as_str()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
        }
        ExternalAuthSource::Pi | ExternalAuthSource::OpenClaw => {
            if object.get("type")?.as_str()? != "api_key" {
                return None;
            }
            resolve_pi_api_key_value(object.get("key")?.as_str()?)
        }
        ExternalAuthSource::Factr => {
            // Factr stores API keys as credential-pool entries whose
            // `auth_type` is `api_key` and whose literal key lives in
            // `access_token`.
            if object.get("auth_type")?.as_str()? != "api_key" {
                return None;
            }
            object
                .get("access_token")?
                .as_str()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
        }
    }
}

fn resolve_pi_api_key_value(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() || raw.starts_with('!') {
        return None;
    }

    if let Ok(value) = std::env::var(raw) {
        let value = value.trim();
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }

    Some(raw.to_string())
}

fn extract_oauth_tokens(source: ExternalAuthSource, entry: &Value) -> Option<ExternalOAuthTokens> {
    match source {
        ExternalAuthSource::OpenCode | ExternalAuthSource::Pi | ExternalAuthSource::OpenClaw => {
            extract_oauth_tokens_pi_style(entry)
        }
        ExternalAuthSource::Factr => extract_oauth_tokens_factr_style(entry),
    }
}

/// OpenCode / pi / OpenClaw share the `{ type: "oauth", access, refresh,
/// expires }` shape (epoch milliseconds in `expires`).
fn extract_oauth_tokens_pi_style(entry: &Value) -> Option<ExternalOAuthTokens> {
    let object = entry.as_object()?;
    let token_type = object.get("type").and_then(Value::as_str);
    if let Some(token_type) = token_type
        && token_type != "oauth"
    {
        return None;
    }

    let access_token = object.get("access")?.as_str()?.trim().to_string();
    let refresh_token = object.get("refresh")?.as_str()?.trim().to_string();
    let expires_at = object.get("expires")?.as_i64()?;

    if access_token.is_empty() || refresh_token.is_empty() {
        return None;
    }

    Some(ExternalOAuthTokens {
        access_token,
        refresh_token,
        expires_at,
    })
}

/// Factr credential-pool entries use `access_token` / `refresh_token` and
/// store the expiry either as `expires_at_ms` (epoch milliseconds) or
/// `expires_at` (RFC 3339 string). `auth_type` distinguishes OAuth entries
/// (`oauth_device_code`, `oauth_external`, `oauth_minimax`) from API keys.
fn extract_oauth_tokens_factr_style(entry: &Value) -> Option<ExternalOAuthTokens> {
    let object = entry.as_object()?;
    if let Some(auth_type) = object.get("auth_type").and_then(Value::as_str)
        && !auth_type.starts_with("oauth")
    {
        return None;
    }

    let access_token = object.get("access_token")?.as_str()?.trim().to_string();
    let refresh_token = object
        .get("refresh_token")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_string();
    let expires_at = factr_expires_at_ms(object)?;

    if access_token.is_empty() || refresh_token.is_empty() {
        return None;
    }

    Some(ExternalOAuthTokens {
        access_token,
        refresh_token,
        expires_at,
    })
}

fn factr_expires_at_ms(object: &serde_json::Map<String, Value>) -> Option<i64> {
    if let Some(ms) = object.get("expires_at_ms").and_then(Value::as_i64) {
        return Some(ms);
    }
    if let Some(text) = object.get("expires_at").and_then(Value::as_str)
        && let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(text.trim())
    {
        return Some(parsed.timestamp_millis());
    }
    // Factr's own Codex login (`factr auth add openai-codex`) stores no expiry field: the access
    // token is a JWT and its `exp` claim is the expiry.
    object.get("access_token").and_then(Value::as_str).and_then(jwt_expiry_ms)
}

/// The `exp` claim (epoch seconds) of a JWT as epoch milliseconds; `None` when it is not a JWT.
fn jwt_expiry_ms(token: &str) -> Option<i64> {
    use base64::Engine;
    let payload = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?;
    let claims: Value = serde_json::from_slice(&bytes).ok()?;
    claims.get("exp").and_then(Value::as_i64).map(|secs| secs * 1000)
}

fn provider_keys_for_env(env_key: &str) -> &'static [&'static str] {
    match env_key {
        "ANTHROPIC_API_KEY" => &["anthropic", "claude"],
        "AZURE_OPENAI_API_KEY" => &["azure-openai-responses", "azure", "azure-openai"],
        "OPENAI_API_KEY" => &["openai", "openai-api"],
        "GEMINI_API_KEY" => &["google", "gemini"],
        "MISTRAL_API_KEY" => &["mistral"],
        "GROQ_API_KEY" => &["groq"],
        "CEREBRAS_API_KEY" => &["cerebras"],
        "BELVEDIR_API_KEY" => &["belvedir"],
        "XAI_API_KEY" => &["xai"],
        "OPENROUTER_API_KEY" => &["openrouter"],
        "CONIFER_API_KEY" => &["conifer"],
        "AI_GATEWAY_API_KEY" => &["vercel-ai-gateway"],
        "ZHIPU_API_KEY" | "ZAI_API_KEY" => &["zai"],
        "OPENCODE_API_KEY" => &["opencode"],
        "OPENCODE_GO_API_KEY" => &["opencode-go", "opencode"],
        "HF_TOKEN" => &["huggingface"],
        "KIMI_API_KEY" => &["kimi-coding", "kimi", "moonshot"],
        "MINIMAX_API_KEY" => &["minimax"],
        "MINIMAX_CN_API_KEY" => &["minimax-cn"],
        "NEBIUS_API_KEY" => &["nebius"],
        "SCALEWAY_API_KEY" => &["scaleway"],
        "STACKIT_API_KEY" => &["stackit"],
        "TOGETHER_API_KEY" => &["togetherai", "together-ai", "together"],
        "DEEPINFRA_API_KEY" => &["deepinfra"],
        "FIREWORKS_API_KEY" => &["fireworks"],
        "NOVITA_API_KEY" => &["novita", "novita-ai", "novita.ai"],
        "CHUTES_API_KEY" => &["chutes"],
        "BASETEN_API_KEY" => &["baseten"],
        "CORTECS_API_KEY" => &["cortecs"],
        "COMTEGRA_API_KEY" => &["comtegra", "cgc"],
        "DEEPSEEK_API_KEY" => &["deepseek"],
        "FIRMWARE_API_KEY" => &["firmware"],
        "MOONSHOT_API_KEY" => &["moonshotai", "moonshot"],
        "PERPLEXITY_API_KEY" => &["perplexity"],
        "BAILIAN_CODING_PLAN_API_KEY" => &["alibaba-coding-plan", "bailian"],
        _ => &[],
    }
}

#[cfg(test)]
#[path = "external_tests.rs"]
mod external_tests;
