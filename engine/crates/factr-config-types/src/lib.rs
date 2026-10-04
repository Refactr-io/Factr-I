use serde::{Deserialize, Serialize};

mod display;
mod serde_lenient;
pub use display::DisplayConfig;
/// Compaction mode
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "lowercase")]
pub enum CompactionMode {
    /// Compact when context hits a fixed threshold (default)
    #[default]
    Reactive,
    /// Compact early based on predicted token growth rate. Also accepts the
    /// removed `semantic` value from older configs.
    #[serde(alias = "semantic")]
    Proactive,
}

impl CompactionMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Reactive => "reactive",
            Self::Proactive => "proactive",
        }
    }

    pub fn parse(input: &str) -> Option<Self> {
        match input.trim().to_ascii_lowercase().as_str() {
            "reactive" => Some(Self::Reactive),
            "proactive" | "semantic" => Some(Self::Proactive),
            _ => None,
        }
    }
}



/// How reasoning/thinking content is rendered into history.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningDisplayMode {
    /// Never display reasoning content.
    #[default]
    Off,
    /// Keep every reasoning trace in the transcript.
    Full,
    /// Show only the *current* reasoning live; collapse it once the model
    /// commits an assistant message or tool call, then show the next one.
    Current,
}

impl ReasoningDisplayMode {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_lowercase().as_str() {
            "off" | "none" | "false" | "0" | "no" => Some(Self::Off),
            "full" | "all" | "true" | "1" | "yes" | "on" => Some(Self::Full),
            "current" | "live" | "ephemeral" | "collapse" => Some(Self::Current),
            _ => None,
        }
    }
}

/// Update channel: how aggressively to receive updates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum UpdateChannel {
    /// Only update from tagged GitHub Releases (default).
    #[default]
    Stable,
    /// Update from latest commit on main branch (bleeding edge).
    Main,
}

impl UpdateChannel {
    /// Parse a channel name, returning `None` for unknown values.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "stable" | "release" => Some(Self::Stable),
            "main" | "nightly" | "edge" => Some(Self::Main),
            _ => None,
        }
    }
}

/// Config deserialization is deliberately lenient: an unknown or removed
/// channel name (e.g. a stale `update_channel = "manual"` left in
/// config.toml) falls back to the default channel instead of failing the
/// entire config parse. A strict enum here once made the freshly exec'd
/// server die during the reload handoff, leaving the handoff marker stuck
/// in `starting` and clients re-requesting the reload forever (issue #349).
impl<'de> Deserialize<'de> for UpdateChannel {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Ok(Self::parse(&value).unwrap_or_default())
    }
}

impl std::fmt::Display for UpdateChannel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Stable => write!(f, "stable"),
            Self::Main => write!(f, "main"),
        }
    }
}

/// Cross-provider failover behavior when the same input would be resent elsewhere.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum CrossProviderFailoverMode {
    /// Show a 3-second cancelable countdown, then resend on another provider.
    #[default]
    Countdown,
    /// Do not resend the prompt to another provider automatically.
    #[serde(alias = "off", alias = "false", alias = "disabled", alias = "none")]
    Manual,
}

impl CrossProviderFailoverMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Countdown => "countdown",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "manual" | "off" | "false" | "disabled" | "none" => Some(Self::Manual),
            "countdown" | "auto" | "automatic" => Some(Self::Countdown),
            _ => None,
        }
    }
}

/// Compaction configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CompactionConfig {
    /// Compaction mode: reactive (default) or proactive
    pub mode: CompactionMode,

    /// [proactive] Number of turns to look ahead when projecting token growth
    pub lookahead_turns: usize,

    /// [proactive] EWMA alpha for token growth smoothing (0.0-1.0, higher = more recency bias)
    pub ewma_alpha: f32,

    /// [proactive] Minimum context fill level before any proactive check fires (0.0-1.0)
    pub proactive_floor: f32,

    /// [proactive] Minimum number of token snapshots needed before proactive check
    pub min_samples: usize,

    /// [proactive] Number of stable turns (no growth) before suppressing proactive compact
    pub stall_window: usize,

    /// [proactive] Minimum turns between two compactions (cooldown)
    pub min_turns_between_compactions: usize,
}

impl Default for CompactionConfig {
    fn default() -> Self {
        Self {
            mode: CompactionMode::Reactive,
            lookahead_turns: 15,
            ewma_alpha: 0.3,
            proactive_floor: 0.40,
            min_samples: 3,
            stall_window: 5,
            min_turns_between_compactions: 10,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum NamedProviderType {
    #[serde(alias = "openai-compatible", alias = "openai_compatible")]
    #[default]
    OpenAiCompatible,
    #[serde(alias = "anthropic-compatible", alias = "anthropic_compatible")]
    AnthropicCompatible,
    OpenRouter,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum NamedProviderAuth {
    #[serde(alias = "Bearer", alias = "BEARER")]
    #[default]
    Bearer,
    #[serde(alias = "Header", alias = "HEADER")]
    Header,
    #[serde(alias = "None", alias = "NONE")]
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(default)]
pub struct NamedProviderModelConfig {
    pub id: String,
    /// Explicitly enable or disable `/effort` for this model. When omitted,
    /// the provider-level setting and built-in model-family detection apply.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<bool>,
    /// Reasoning effort selected when this model becomes active. This overrides
    /// `[provider].openai_reasoning_effort` for this model only.
    #[serde(
        default,
        alias = "reasoning-effort",
        skip_serializing_if = "Option::is_none"
    )]
    pub reasoning_effort: Option<String>,
    #[serde(
        default,
        alias = "context_limit",
        alias = "context-length",
        alias = "context-window",
        alias = "context_length",
        skip_serializing_if = "Option::is_none"
    )]
    pub context_window: Option<usize>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub input: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct NamedProviderConfig {
    #[serde(rename = "type")]
    pub provider_type: NamedProviderType,
    pub base_url: String,
    pub api: Option<String>,
    pub auth: NamedProviderAuth,
    pub auth_header: Option<String>,
    /// Extra HTTP headers sent with every request to this provider.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub headers: std::collections::BTreeMap<String, String>,
    pub api_key_env: Option<String>,
    pub api_key: Option<String>,
    pub default_model: Option<String>,
    pub requires_api_key: Option<bool>,
    #[serde(default)]
    pub provider_routing: bool,
    #[serde(default)]
    pub model_catalog: bool,
    #[serde(default)]
    pub allow_provider_pinning: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<NamedProviderModelConfig>,
    /// Extra top-level JSON fields merged into every chat/completions request
    /// body sent to this provider. Lets users inject non-standard parameters
    /// some OpenAI-compatible backends require (e.g. NVIDIA NIM DeepSeek-V4
    /// needs `chat_template_kwargs = { thinking = true, reasoning_effort = "high" }`).
    /// Must be a JSON object; keys here override factr-generated body fields.
    #[serde(default, alias = "extra-body", skip_serializing_if = "Option::is_none")]
    pub extra_body: Option<serde_json::Value>,
    /// Whether this endpoint accepts the DeepSeek-style top-level
    /// `reasoning_effort` request field (`/effort` support). When unset, factr
    /// auto-detects it from the active model id (DeepSeek-family models
    /// support it regardless of which gateway serves them). Set `false` to
    /// suppress auto-detection for strict-schema endpoints.
    #[serde(
        default,
        alias = "supports-reasoning-effort",
        alias = "reasoning_effort",
        skip_serializing_if = "Option::is_none"
    )]
    pub supports_reasoning_effort: Option<bool>,
    /// Disable model-name based reasoning detection for this profile. Explicit
    /// provider/model capability settings continue to work.
    #[serde(default, alias = "disable-reasoning-heuristics")]
    pub disable_reasoning_heuristics: bool,
}

impl Default for NamedProviderConfig {
    fn default() -> Self {
        Self {
            provider_type: NamedProviderType::OpenAiCompatible,
            base_url: String::new(),
            api: None,
            auth: NamedProviderAuth::Bearer,
            auth_header: None,
            headers: std::collections::BTreeMap::new(),
            api_key_env: None,
            api_key: None,
            default_model: None,
            requires_api_key: None,
            provider_routing: false,
            model_catalog: false,
            allow_provider_pinning: false,
            models: Vec::new(),
            extra_body: None,
            supports_reasoning_effort: None,
            disable_reasoning_heuristics: false,
        }
    }
}

/// Remembered trust decisions for external auth sources managed by other tools.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct AuthConfig {
    /// External auth source ids that the user has approved factr to read/use.
    pub trusted_external_sources: Vec<String>,
    /// Path-bound approvals for external auth sources managed by other tools.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trusted_external_source_paths: Vec<String>,
}

/// Agent-specific model defaults.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentsConfig {
    /// Longest a turn parks when a provider reports a usage limit with a reset time, in seconds
    /// (default 7200; 0 turns parking off). Env: `FACTR_WAIT_FOR_USAGE_MAX_S`.
    pub wait_for_usage_max_s: Option<u64>,
    /// Optional default reasoning effort for spawned swarm/subagent sessions
    /// (`"low"`, `"medium"`, `"high"`, ...). Applied when a `swarm spawn`
    /// call does not pass an explicit `effort`. Leave unset to let workers
    /// inherit the provider-wide reasoning effort.
    pub swarm_effort: Option<String>,
    /// Root reasoning effort in light swarm mode. Unset or invalid means `max`.
    /// This does not change worker effort (`swarm_effort`).
    pub swarm_root_effort: Option<String>,
    /// Root reasoning effort in deep swarm mode. Unset or invalid means `max`.
    pub swarm_deep_root_effort: Option<String>,
    /// Default terminal mode for swarm-created agents.
    pub swarm_spawn_mode: SwarmSpawnMode,
    /// Maximum percentage (1-90) of the chat column height the inline swarm
    /// gallery band may occupy. Leave unset to use the built-in default (40%).
    /// Lower values keep more of the transcript visible; set near the minimum
    /// to effectively collapse the gallery to a thin strip.
    pub swarm_gallery_max_pct: Option<u8>,
    /// Maximum number of live swarm worker agents in one swarm. This is the RAM
    /// safety budget for both recursive ad hoc spawning and deep-mode `run_plan`
    /// parallelism. Completed/stopped workers do not consume slots. Light mode
    /// still uses a smaller fixed fan-out. `0` disables this configurable guard,
    /// leaving only the absolute `MAX_SWARM_MEMBERS` hard cap.
    /// Env override: `FACTR_SWARM_MAX_CONCURRENT_AGENTS`.
    #[serde(default = "default_swarm_max_concurrent_agents")]
    pub swarm_max_concurrent_agents: usize,
    /// Nudge once when a turn ends after code edits with no test run, or with a
    /// text-only "I will ..." reply. Env: `FACTR_VERIFY_ON_STOP=0` disables.
    #[serde(default = "default_verify_on_stop")]
    pub verify_on_stop: bool,
    /// After code edits, run the project's detected test command when a turn
    /// ends and feed failures back (plain mode only). Env: `FACTR_AUTO_VERIFY=0`.
    #[serde(default = "default_verify_on_stop")]
    pub auto_verify: bool,
    /// Per-run timeout of the auto-verify test command.
    #[serde(default = "default_auto_verify_timeout_s")]
    pub auto_verify_timeout_s: u64,
    /// Maximum auto-verify gate runs per turn.
    #[serde(default = "default_auto_verify_rounds")]
    pub auto_verify_rounds: u32,
    /// Add a one-line environment snapshot (cwd, tools on PATH, files) to a
    /// session's first user message. Env: `FACTR_ENV_SNAPSHOT=0` disables.
    #[serde(default = "default_verify_on_stop")]
    pub environment_snapshot: bool,
    /// Offer the deferred `repl` tool (Python in a sandbox; uses Factr's
    /// interpreter or the system `python3`). Env: `FACTR_REPL=0` disables.
    #[serde(default = "default_verify_on_stop")]
    pub repl: bool,
    /// Optional model for `llm_query` / `llm_query_batch` sub-calls
    /// (default: the active model).
    #[serde(default)]
    pub repl_sub_model: Option<String>,
}

fn default_auto_verify_timeout_s() -> u64 {
    120
}

fn default_auto_verify_rounds() -> u32 {
    3
}

fn default_verify_on_stop() -> bool {
    true
}

fn default_swarm_max_concurrent_agents() -> usize {
    32
}

impl Default for AgentsConfig {
    fn default() -> Self {
        Self {
            wait_for_usage_max_s: None,
            swarm_effort: None,
            swarm_root_effort: None,
            swarm_deep_root_effort: None,
            swarm_spawn_mode: SwarmSpawnMode::default(),
            swarm_gallery_max_pct: None,
            swarm_max_concurrent_agents: default_swarm_max_concurrent_agents(),
            verify_on_stop: default_verify_on_stop(),
            auto_verify: default_verify_on_stop(),
            auto_verify_timeout_s: default_auto_verify_timeout_s(),
            auto_verify_rounds: default_auto_verify_rounds(),
            environment_snapshot: default_verify_on_stop(),
            repl: default_verify_on_stop(),
            repl_sub_model: None,
        }
    }
}

impl AgentsConfig {
    /// Resolve a swarm mode's root effort without allowing orchestration
    /// sentinels to recurse into another mode. Unknown values preserve the
    /// historical maximum-effort behavior without invalidating other settings.
    pub fn root_effort_for_swarm(&self, deep: bool) -> &'static str {
        let configured = if deep {
            self.swarm_deep_root_effort.as_deref()
        } else {
            self.swarm_root_effort.as_deref()
        };
        let value = configured.unwrap_or("max").trim();
        ["none", "minimal", "low", "medium", "high", "xhigh", "max"]
            .into_iter()
            .find(|level| level.eq_ignore_ascii_case(value))
            .unwrap_or("max")
    }
}

/// How swarm-created agents should be spawned. Workers always run in-process
/// (there is no terminal window); the legacy `visible`/`headed`/`auto` values
/// still parse and mean `inline`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum SwarmSpawnMode {
    /// Create the worker in-process.
    Headless,
    /// In-process worker whose streaming output tail is also tapped.
    #[default]
    #[serde(alias = "visible", alias = "headed", alias = "auto")]
    Inline,
}

impl SwarmSpawnMode {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "headless" => Some(Self::Headless),
            "inline" | "visible" | "headed" | "auto" => Some(Self::Inline),
            _ => None,
        }
    }

    /// Canonical lowercase string for this mode (matches the config/env values).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Headless => "headless",
            Self::Inline => "inline",
        }
    }
}

/// Automatic end-of-turn code review configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct AutoReviewConfig {
    /// Enable autoreview by default for new/resumed sessions (default: false)
    pub enabled: bool,
    /// Optional model override for autoreview reviewer sessions.
    pub model: Option<String>,
}

/// Automatic end-of-turn execution judging configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct AutoJudgeConfig {
    /// Enable autojudge by default for new/resumed sessions (default: false)
    pub enabled: bool,
    /// Optional model override for autojudge sessions.
    pub model: Option<String>,
}



/// Runtime feature toggles
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct FeatureConfig {
    /// Check for and install factr updates during startup (default: true).
    /// Set this to false for the persistent equivalent of `--no-update`.
    pub check_updates: bool,
    /// Enable swarm coordination features (default: true)
    pub swarm: bool,
    /// Enable Mermaid rendering and Mermaid-specific model guidance (default: true)
    pub mermaid: bool,
    /// Default state of auto-poke (automatic follow-up when the model stops with
    /// incomplete todos). `/poke on` / `/poke off` still override this per session
    /// (default: true)
    pub auto_poke: bool,
    /// Inject timestamps into user messages and tool results sent to the model (default: true)
    pub message_timestamps: bool,
    /// Persist auto-recalled memory injections into normal session history instead of sending
    /// them as request-only ephemeral suffix messages (default: false)
    pub persist_memory_injections: bool,
    /// Surface an in-chat system message whenever a request misses the KV cache
    /// for a harness-caused (avoidable) reason: the system prompt, tool set, or
    /// message prefix changed without the conversation legitimately growing.
    /// These should essentially never happen, so the notice acts as a loud alarm
    /// that something in the harness silently invalidated the prefix cache
    /// (default: true).
    pub kv_cache_miss_notices: bool,
    /// Update channel: "stable" (releases only) or "main" (latest commits)
    pub update_channel: UpdateChannel,
}

impl Default for FeatureConfig {
    fn default() -> Self {
        Self {
            check_updates: true,
            swarm: true,
            mermaid: true,
            auto_poke: true,
            message_timestamps: true,
            persist_memory_injections: false,
            kv_cache_miss_notices: true,
            update_channel: UpdateChannel::default(),
        }
    }
}

/// Search engine used by the websearch tool.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "lowercase")]
pub enum WebSearchEngine {
    /// DuckDuckGo HTML search, no API key required.
    #[default]
    Duckduckgo,
    /// Bing search. Uses the Bing API when configured, otherwise Bing HTML search.
    Bing,
    /// SearXNG metasearch instance (JSON API). Requires `searxng_url` (or the
    /// `FACTR_SEARXNG_URL` env var) to point at a SearXNG instance. Useful on
    /// hosts where DuckDuckGo/Bing block the request via TLS fingerprinting.
    Searxng,
}

impl WebSearchEngine {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Duckduckgo => "duckduckgo",
            Self::Bing => "bing",
            Self::Searxng => "searxng",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "duckduckgo" | "ddg" => Some(Self::Duckduckgo),
            "bing" => Some(Self::Bing),
            "searxng" | "searx" => Some(Self::Searxng),
            _ => None,
        }
    }
}

/// Configuration for the websearch tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WebSearchConfig {
    /// Preferred engine when the tool input does not specify one.
    pub engine: WebSearchEngine,
    /// Keyless HTML engines to try after the preferred engine fails.
    pub fallback_engines: Vec<WebSearchEngine>,
    /// Optional Bing API key for primary Bing searches. Fallback Bing uses keyless HTML search.
    pub bing_api_key: Option<String>,
    /// Environment variable containing the Bing API key.
    pub bing_api_key_env: String,
    /// Bing market, e.g. "en-US" or "zh-CN".
    pub bing_market: String,
    /// Base URL of a SearXNG instance (e.g. "https://searx.example.org"), used
    /// by the `searxng` engine. When empty, the `searxng_url_env` variable is
    /// consulted instead.
    pub searxng_url: Option<String>,
    /// Environment variable containing the SearXNG base URL.
    pub searxng_url_env: String,
    /// Whether an empty search falls back to the Wikipedia opensearch API
    /// (default true). Set false to keep all traffic on the configured engine.
    pub last_resort_wikipedia: bool,
}

/// Configuration for the webfetch tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WebFetchConfig {
    /// When non-empty, only these host names / IPs may be fetched (redirect
    /// targets included). Empty means no restriction.
    pub allowed_hosts: Vec<String>,
    /// Whether a 403/404/410 fetch may fall back to a Wayback (archive.org)
    /// snapshot (default true).
    pub wayback_fallback: bool,
}

impl Default for WebFetchConfig {
    fn default() -> Self {
        Self { allowed_hosts: Vec::new(), wayback_fallback: true }
    }
}

impl Default for WebSearchConfig {
    fn default() -> Self {
        Self {
            engine: WebSearchEngine::Duckduckgo,
            fallback_engines: vec![WebSearchEngine::Bing],
            bing_api_key: None,
            bing_api_key_env: "FACTR_BING_API_KEY".to_string(),
            bing_market: "en-US".to_string(),
            searxng_url: None,
            searxng_url_env: "FACTR_SEARXNG_URL".to_string(),
            last_resort_wikipedia: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ProviderConfig {
    /// Default model to use (e.g. "claude-opus-4-8", "copilot:claude-opus-4.6")
    pub default_model: Option<String>,
    /// Default provider to use (claude|openai|copilot|openrouter)
    pub default_provider: Option<String>,
    /// Reasoning effort for OpenAI Responses API (none|minimal|low|medium|high|xhigh|max)
    pub openai_reasoning_effort: Option<String>,
    /// Reasoning effort for Anthropic Messages API output_config (none|low|medium|high|xhigh; max aliases to strongest supported)
    pub anthropic_reasoning_effort: Option<String>,
    /// Request one-hour Anthropic prompt caching instead of five minutes.
    pub anthropic_cache_ttl_1h: bool,
    /// OpenAI transport mode (auto|websocket|https)
    pub openai_transport: Option<String>,
    /// OpenAI service tier override (priority|flex)
    pub openai_service_tier: Option<String>,
    /// OpenAI native compaction mode: "auto", "explicit", or "off".
    pub openai_native_compaction_mode: String,
    /// Token threshold at which OpenAI auto native compaction should trigger.
    pub openai_native_compaction_threshold_tokens: usize,
    /// Preserve provider-native reasoning/thinking items for future-turn context when supported.
    pub preserve_reasoning_context: bool,
    /// How to handle cross-provider failover when the same input would be resent elsewhere.
    pub cross_provider_failover: CrossProviderFailoverMode,
    /// Whether factr should automatically try another account on the same provider
    /// before falling back to a different provider.
    pub same_provider_account_failover: bool,
    /// Copilot premium request mode: "normal", "one", or "zero"
    /// "zero" means all requests are free (no premium requests consumed)
    pub copilot_premium: Option<String>,
    /// When set (non-empty), /model only lists routes from these providers.
    /// Entries match provider labels ("openai", "anthropic", "copilot",
    /// "openrouter", ...), api methods ("claude-oauth",
    /// "openai-compatible:myprofile", ...), or openai-compatible profile ids
    /// ("myprofile"). The active model's routes always stay visible.
    pub model_picker_providers: Option<Vec<String>>,
    /// Max seconds to wait for streaming data before timing out a request with
    /// no data received. Base budget only: high reasoning efforts scale it up
    /// automatically (see `factr_base::provider::stream_idle_timeout_for_effort`).
    /// Default: 180. Overridable via `FACTR_STREAM_IDLE_TIMEOUT_SECS`.
    pub stream_idle_timeout_secs: u64,
    /// Maximum request attempts for transient provider errors, including the
    /// initial attempt. Default: 8. Overridable via `FACTR_MAX_RETRIES`.
    pub max_retries: u32,
    /// Maximum exponential-backoff delay between transient-error retries.
    /// Default: 30 seconds. Overridable via `FACTR_RETRY_BACKOFF_CAP_SECS`.
    pub retry_backoff_cap_secs: u64,
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            default_model: None,
            default_provider: None,
            openai_reasoning_effort: Some("low".to_string()),
            anthropic_reasoning_effort: None,
            anthropic_cache_ttl_1h: true,
            openai_transport: None,
            openai_service_tier: Some("priority".to_string()),
            openai_native_compaction_mode: "auto".to_string(),
            openai_native_compaction_threshold_tokens: 200_000,
            preserve_reasoning_context: true,
            cross_provider_failover: CrossProviderFailoverMode::Countdown,
            same_provider_account_failover: true,
            copilot_premium: None,
            model_picker_providers: None,
            stream_idle_timeout_secs: 180,
            max_retries: 8,
            retry_backoff_cap_secs: 30,
        }
    }
}

/// Desktop notification configuration for interactive sessions.
///
/// This section controls lightweight local desktop notifications for the normal
/// interactive TUI, e.g. "agent finished a long turn".
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct NotificationsConfig {
    /// Send a desktop notification when an agent turn completes (default: true).
    /// Notifications fire only for long turns (see thresholds below) and, by
    /// default, only while the terminal window is unfocused.
    pub turn_complete: bool,
    /// Minimum turn duration, in seconds, before a completed turn notifies
    /// (default: 120).
    pub turn_complete_min_secs: u64,
    /// Lower duration threshold, in seconds, used when the session has todos
    /// recorded, since todos indicate longer task-style work (default: 30).
    pub turn_complete_todo_min_secs: u64,
    /// Only notify while the terminal window is unfocused (default: true).
    /// Requires a terminal that reports focus events (most modern terminals).
    pub turn_complete_only_when_unfocused: bool,
    /// macOS Notification Center sound name played on turn completion
    /// (e.g. "Glass", "Ping", "Hero"). Empty string disables the sound.
    /// Ignored on non-macOS platforms. Default: "Glass".
    pub turn_complete_sound: String,
}

impl Default for NotificationsConfig {
    fn default() -> Self {
        Self {
            turn_complete: true,
            turn_complete_min_secs: 120,
            turn_complete_todo_min_secs: 30,
            turn_complete_only_when_unfocused: true,
            turn_complete_sound: "Glass".to_string(),
        }
    }
}

/// Power-management configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PowerConfig {
    /// Prevent automatic system sleep while any factr session is actively
    /// streaming/processing. Linux also asks logind to block lid-switch suspend.
    /// Windows cannot override a user-initiated lid close or power-button action;
    /// those remain controlled by the active Windows power plan. The display is
    /// still allowed to sleep. Default: true.
    ///
    /// Honored by the shared `factr serve` daemon. The `FACTR_DISABLE_POWER_INHIBIT`
    /// environment variable forces this off regardless of the config value.
    pub prevent_sleep_while_streaming: bool,
}

impl Default for PowerConfig {
    fn default() -> Self {
        Self {
            prevent_sleep_while_streaming: true,
        }
    }
}
