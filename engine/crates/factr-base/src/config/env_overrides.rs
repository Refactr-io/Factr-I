use super::*;

impl Config {
    /// Apply environment variable overrides
    #[expect(
        clippy::collapsible_if,
        reason = "Environment override parsing is intentionally explicit and grouped by config area"
    )]
    pub(crate) fn apply_env_overrides(&mut self) {
        // Server/operator behavior
        if let Ok(v) = std::env::var("FACTR_WAKE_MODE")
            && let Some(parsed) = WakeMode::parse(&v)
        {
            self.server.wake_mode = parsed;
        }

        // Tools
        if let Ok(v) = std::env::var("FACTR_TOOL_PROFILE") {
            self.tools.profile = v;
        }
        if let Ok(v) = std::env::var("FACTR_TOOLS") {
            self.tools.enabled = parse_env_list(&v);
        }
        if let Ok(v) = std::env::var("FACTR_DISABLED_TOOLS") {
            self.tools.disabled = parse_env_list(&v);
        }
        if let Ok(v) = std::env::var("FACTR_DISABLE_BASE_TOOLS")
            && let Some(parsed) = parse_env_bool(&v)
        {
            self.tools.disable_base_tools = parsed;
        }
        if let Ok(v) = std::env::var("FACTR_MCP_TOOLS")
            && let Some(mode) = crate::config::McpToolsMode::parse(&v)
        {
            self.tools.mcp_tools = mode;
        }
        if let Ok(v) = std::env::var("FACTR_MCP_TOOLS_TOKEN_THRESHOLD")
            && let Ok(parsed) = v.trim().parse::<usize>()
        {
            self.tools.mcp_tools_token_threshold = parsed;
        }

        // ACP adapter
        if let Ok(v) = std::env::var("FACTR_ACP_PROFILE") {
            let trimmed = v.trim().to_ascii_lowercase();
            if matches!(trimmed.as_str(), "standard" | "extended" | "full") {
                self.acp.profile = trimmed;
            }
        }
        if let Ok(v) = std::env::var("FACTR_ACP_TOOL_PROFILE") {
            let trimmed = v.trim();
            if !trimmed.is_empty() {
                self.acp.tool_profile = trimmed.to_string();
            }
        }

        // Display
        if let Ok(v) = std::env::var("FACTR_SHOW_THINKING") {
            if let Some(parsed) = parse_env_bool(&v) {
                self.display.show_thinking = parsed;
            }
        }
        if let Ok(v) = std::env::var("FACTR_REASONING_DISPLAY") {
            if let Some(mode) = crate::config::ReasoningDisplayMode::parse(&v) {
                self.display.set_reasoning_display(mode);
            }
        }
        // A front-end default, applied only when the user has not made an
        // explicit choice.
        if !self.display.has_explicit_reasoning_display()
            && let Ok(v) = std::env::var("FACTR_DEFAULT_REASONING_DISPLAY")
            && let Some(mode) = crate::config::ReasoningDisplayMode::parse(&v)
        {
            self.display.set_reasoning_display(mode);
        }

        // Features
        if let Ok(v) = std::env::var("FACTR_SWARM_ENABLED") {
            if let Some(parsed) = parse_env_bool(&v) {
                self.features.swarm = parsed;
            }
        }
        if let Ok(v) = std::env::var("FACTR_ENABLE_MERMAID") {
            if let Some(parsed) = parse_env_bool(&v) {
                self.features.mermaid = parsed;
            }
        }
        if let Ok(v) = std::env::var("FACTR_CHECK_UPDATES") {
            if let Some(parsed) = parse_env_bool(&v) {
                self.features.check_updates = parsed;
            }
        }
        if let Ok(v) = std::env::var("FACTR_AUTO_POKE") {
            if let Some(parsed) = parse_env_bool(&v) {
                self.features.auto_poke = parsed;
            }
        }
        if let Ok(v) = std::env::var("FACTR_MESSAGE_TIMESTAMPS") {
            if let Some(parsed) = parse_env_bool(&v) {
                self.features.message_timestamps = parsed;
            }
        }
        if let Ok(v) = std::env::var("FACTR_PERSIST_MEMORY_INJECTIONS") {
            if let Some(parsed) = parse_env_bool(&v) {
                self.features.persist_memory_injections = parsed;
            }
        }
        if let Ok(v) = std::env::var("FACTR_KV_CACHE_MISS_NOTICES") {
            if let Some(parsed) = parse_env_bool(&v) {
                self.features.kv_cache_miss_notices = parsed;
            }
        }
        if let Ok(v) = std::env::var("FACTR_UPDATE_CHANNEL")
            && let Some(channel) = UpdateChannel::parse(&v)
        {
            self.features.update_channel = channel;
        }

        // Agents (spawned helper sessions)
        if let Ok(v) = std::env::var("FACTR_SWARM_EFFORT") {
            let trimmed = v.trim();
            self.agents.swarm_effort = if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            };
        }
        for (key, target) in [
            (
                "FACTR_SWARM_ROOT_EFFORT",
                &mut self.agents.swarm_root_effort,
            ),
            (
                "FACTR_SWARM_DEEP_ROOT_EFFORT",
                &mut self.agents.swarm_deep_root_effort,
            ),
        ] {
            if let Ok(value) = std::env::var(key) {
                let value = value.trim();
                *target = (!value.is_empty()).then(|| value.to_string());
            }
        }
        if let Ok(v) = std::env::var("FACTR_SWARM_SPAWN_MODE") {
            if let Some(parsed) = SwarmSpawnMode::parse(&v) {
                self.agents.swarm_spawn_mode = parsed;
            }
        }
        if let Ok(v) = std::env::var("FACTR_SWARM_MAX_CONCURRENT_AGENTS") {
            if let Ok(parsed) = v.trim().parse::<usize>() {
                self.agents.swarm_max_concurrent_agents = parsed;
            }
        }

        // Web search
        if let Ok(v) = std::env::var("FACTR_WEBSEARCH_ENGINE")
            && let Some(engine) = WebSearchEngine::parse(&v)
        {
            self.websearch.engine = engine;
        }
        if let Ok(v) = std::env::var("FACTR_WEBSEARCH_FALLBACK_ENGINES") {
            let engines = parse_env_list(&v)
                .into_iter()
                .filter_map(|item| WebSearchEngine::parse(&item))
                .collect::<Vec<_>>();
            if !engines.is_empty() {
                self.websearch.fallback_engines = engines;
            }
        }
        if let Ok(v) = std::env::var("FACTR_BING_API_KEY")
            && !v.trim().is_empty()
        {
            self.websearch.bing_api_key = Some(v);
        }
        if let Ok(v) = std::env::var("FACTR_BING_API_KEY_ENV")
            && !v.trim().is_empty()
        {
            self.websearch.bing_api_key_env = v;
        }
        if let Ok(v) = std::env::var("FACTR_BING_MARKET")
            && !v.trim().is_empty()
        {
            self.websearch.bing_market = v;
        }
        if let Ok(v) = std::env::var("FACTR_SEARXNG_URL")
            && !v.trim().is_empty()
        {
            self.websearch.searxng_url = Some(v);
        }

        if let Ok(v) = std::env::var("FACTR_TRUSTED_EXTERNAL_AUTH_SOURCES") {
            let mut source_ids = Vec::new();
            let mut source_paths = Vec::new();
            for value in parse_env_list(&v) {
                let trimmed = value.trim();
                if trimmed.is_empty() {
                    continue;
                }
                if trimmed.contains('|') {
                    source_paths.push(trimmed.to_ascii_lowercase());
                } else {
                    source_ids.push(trimmed.to_ascii_lowercase());
                }
            }
            self.auth.trusted_external_sources = source_ids;
            self.auth.trusted_external_source_paths = source_paths;
        }

        // Autoreview
        if let Ok(v) = std::env::var("FACTR_AUTOREVIEW_ENABLED") {
            if let Some(parsed) = parse_env_bool(&v) {
                self.autoreview.enabled = parsed;
            }
        }
        if let Ok(v) = std::env::var("FACTR_AUTOREVIEW_MODEL") {
            let trimmed = v.trim();
            self.autoreview.model = if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            };
        }

        // Autojudge
        if let Ok(v) = std::env::var("FACTR_AUTOJUDGE_ENABLED") {
            if let Some(parsed) = parse_env_bool(&v) {
                self.autojudge.enabled = parsed;
            }
        }
        if let Ok(v) = std::env::var("FACTR_AUTOJUDGE_MODEL") {
            let trimmed = v.trim();
            self.autojudge.model = if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            };
        }

        // Power management
        if let Ok(v) = std::env::var("FACTR_PREVENT_SLEEP_WHILE_STREAMING") {
            if let Some(parsed) = parse_env_bool(&v) {
                self.power.prevent_sleep_while_streaming = parsed;
            }
        }

        // Provider
        if let Ok(v) = std::env::var("FACTR_MODEL") {
            self.provider.default_model = Some(v);
        }
        if let Ok(v) = std::env::var("FACTR_PROVIDER") {
            let trimmed = v.trim().to_lowercase();
            if !trimmed.is_empty() {
                self.provider.default_provider = Some(trimmed);
            }
        }
        if let Ok(v) = std::env::var("FACTR_OPENAI_REASONING_EFFORT") {
            let trimmed = v.trim().to_string();
            if !trimmed.is_empty() {
                self.provider.openai_reasoning_effort = Some(trimmed);
            }
        }
        if let Ok(v) = std::env::var("FACTR_ANTHROPIC_REASONING_EFFORT") {
            let trimmed = v.trim().to_string();
            if !trimmed.is_empty() {
                self.provider.anthropic_reasoning_effort = Some(trimmed);
            }
        }
        if let Ok(v) = std::env::var("FACTR_OPENAI_TRANSPORT") {
            let trimmed = v.trim().to_string();
            if !trimmed.is_empty() {
                self.provider.openai_transport = Some(trimmed);
            }
        }
        if let Ok(v) = std::env::var("FACTR_OPENAI_SERVICE_TIER") {
            let trimmed = v.trim().to_string();
            if !trimmed.is_empty() {
                self.provider.openai_service_tier = Some(trimmed);
            }
        }
        if let Ok(v) = std::env::var("FACTR_OPENAI_NATIVE_COMPACTION_MODE") {
            let trimmed = v.trim().to_ascii_lowercase();
            if !trimmed.is_empty() {
                self.provider.openai_native_compaction_mode = trimmed;
            }
        }
        if let Ok(v) = std::env::var("FACTR_OPENAI_NATIVE_COMPACTION_THRESHOLD_TOKENS") {
            if let Ok(parsed) = v.trim().parse::<usize>() {
                if parsed > 0 {
                    self.provider.openai_native_compaction_threshold_tokens = parsed;
                }
            }
        }
        if let Ok(v) = std::env::var("FACTR_PRESERVE_REASONING_CONTEXT") {
            if let Some(parsed) = parse_env_bool(&v) {
                self.provider.preserve_reasoning_context = parsed;
            }
        }
        if let Ok(v) = std::env::var("FACTR_CROSS_PROVIDER_FAILOVER") {
            if let Some(mode) = CrossProviderFailoverMode::parse(&v) {
                self.provider.cross_provider_failover = mode;
            }
        }
        if let Ok(v) = std::env::var("FACTR_SAME_PROVIDER_ACCOUNT_FAILOVER") {
            if let Some(enabled) = parse_env_bool(&v) {
                self.provider.same_provider_account_failover = enabled;
            }
        }
        if let Ok(v) = std::env::var("FACTR_STREAM_IDLE_TIMEOUT_SECS") {
            if let Ok(parsed) = v.trim().parse::<u64>() {
                if parsed > 0 {
                    self.provider.stream_idle_timeout_secs = parsed;
                }
            }
        }
        if let Ok(v) = std::env::var("FACTR_MAX_RETRIES")
            && let Ok(parsed) = v.trim().parse::<u32>()
            && parsed > 0
        {
            self.provider.max_retries = parsed;
        }
        if let Ok(v) = std::env::var("FACTR_RETRY_BACKOFF_CAP_SECS")
            && let Ok(parsed) = v.trim().parse::<u64>()
            && parsed > 0
        {
            self.provider.retry_backoff_cap_secs = parsed;
        }

        // Copilot premium mode: env var overrides config
        // If set in config but not in env, propagate config -> env
        if let Ok(v) = std::env::var("FACTR_COPILOT_PREMIUM") {
            self.provider.copilot_premium = Some(v);
        } else if let Some(ref mode) = self.provider.copilot_premium {
            let env_val = match mode.as_str() {
                "zero" | "0" => "0",
                "one" | "1" => "1",
                _ => "",
            };
            if !env_val.is_empty() {
                crate::env::set_var("FACTR_COPILOT_PREMIUM", env_val);
            }
        }
    }
}

fn parse_env_bool(raw: &str) -> Option<bool> {
    match raw.trim().to_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn parse_env_list(raw: &str) -> Vec<String> {
    raw.split([',', '\n'])
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(ToString::to_string)
        .collect()
}
