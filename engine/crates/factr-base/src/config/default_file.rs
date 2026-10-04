use super::*;
use std::path::PathBuf;

impl Config {
    /// Create a default config file with comments
    pub fn create_default_config_file() -> anyhow::Result<PathBuf> {
        let path = Self::path().ok_or_else(|| anyhow::anyhow!("No config path"))?;

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        std::fs::write(&path, Self::default_config_file_contents())?;
        Ok(path)
    }

    /// The commented config template written by [`Self::create_default_config_file`].
    ///
    /// Exposed separately so tests can check that the template we ship actually
    /// parses and documents the options it claims to.
    pub fn default_config_file_contents() -> String {
        let default_content = r##"# factr configuration file
# Location: ~/.factr/engine/config.toml
#
# Environment variables override these settings.
# Run `/config` in factr to see current settings.

[display]
# Request the model's reasoning from the provider (default: true)
show_thinking = true

# How reasoning is rendered into history: "off", "full", or "current".
# When unset, falls back to show_thinking (true => full, false => off).
reasoning_display = "full"

[features]
# Check for and install updates during startup. Set to false for the persistent
# equivalent of passing --no-update on every invocation.
check_updates = true
# Swarm: multi-session coordination features
swarm = true
# Mermaid: render Mermaid code blocks and tell the model that diagrams are supported
mermaid = true
# Auto-poke: automatically nudge the model to continue when it stops with
# incomplete todos. /poke on and /poke off still override this per session.
auto_poke = true
# Inject timestamps into user messages and tool results sent to the model
message_timestamps = true
# Persist memory injections into session history instead of sending them as request-only ephemeral context
persist_memory_injections = false
# Show an in-chat warning when a request misses the KV cache for a harness-caused
# (avoidable) reason: system prompt, tool set, or message prefix changed. These
# should essentially never happen and indicate a prefix-cache bug.
kv_cache_miss_notices = true
# Update channel: "stable" (releases only) or "main" (latest commits on push)
# Set to "main" for bleeding edge updates every time code is pushed
update_channel = "stable"

[websearch]
# Preferred websearch engine: "duckduckgo", "bing", or "searxng".
engine = "duckduckgo"
# Keyless HTML engines to try if the preferred engine fails. Default falls back to Bing HTML.
fallback_engines = ["bing"]
# Bring your own Bing Search API key for primary Bing searches. Prefer using an env var.
# Fallback Bing searches intentionally use keyless HTML search.
# bing_api_key_env = "FACTR_BING_API_KEY"
# bing_api_key = ""
# Bing market/region, for example "en-US" or "zh-CN".
bing_market = "en-US"
# SearXNG instance for the "searxng" engine. On some hosts (commonly Linux),
# DuckDuckGo and Bing block scraped requests via TLS fingerprinting / IP
# reputation and return an anti-bot page with no results. Pointing at a SearXNG
# instance (self-hosted or trusted public) with the JSON format enabled avoids
# this. Configure here or via the FACTR_SEARXNG_URL environment variable, then
# set engine = "searxng" or add it to fallback_engines.
# searxng_url = "https://searx.example.org"
# When engine = "searxng" AND a SearXNG URL is configured, the per-call `engine`
# argument is ignored, no fallback engine or key-based backend is used.
# Empty searches fall back to the Wikipedia opensearch API; false disables that
# (env FACTR_WEBSEARCH_LAST_RESORT_WIKIPEDIA=0).
# last_resort_wikipedia = true

[webfetch]
# When non-empty, webfetch only contacts these hosts (redirect hops included);
# anything else is refused. Env: FACTR_WEBFETCH_ALLOWED_HOSTS (comma list).
# allowed_hosts = ["127.0.0.1"]
# Try an archive.org (Wayback) snapshot after a 403/404/410. Env: FACTR_WEBFETCH_WAYBACK=0 disables.
# wayback_fallback = true

[tools]
# Controls which built-in tools are sent to the model.
# Profiles: "full" (default), "acp", "minimal"/"lite", or "none".
# acp keeps core coding tools plus batch for generic ACP clients.
# minimal keeps core coding tools only: bash, read, write, edit, multiedit,
# apply_patch, patch, agentgrep, glob, grep, and ls.
profile = "full"
# Explicit allow-list. When non-empty, only these tools are exposed.
# enabled = ["bash", "read", "write", "apply_patch", "agentgrep", "ls"]
# All built-in tools are exposed by the full profile.
# Use enabled = ["*"] to explicitly select the unrestricted full toolset.
# Hide selected tools after applying the profile/allow-list.
# Disable all built-in tools unless enabled is set.
disable_base_tools = false
# MCP tool exposure: "eager" sends every server tool definition, "deferred"
# sends only fixed mcp_search/mcp_call tools, and "auto" switches to deferred
# when the filtered MCP definitions exceed the token threshold below.
# Env overrides: FACTR_MCP_TOOLS, FACTR_MCP_TOOLS_TOKEN_THRESHOLD.
mcp_tools = "auto"
mcp_tools_token_threshold = 2000

[acp]
# Agent Client Protocol adapter compatibility profile: standard, extended, or full.
# standard emits only spec-compatible ACP messages.
# extended/full additionally emit ignorable _factr/* extension notifications.
profile = "standard"
# Tool profile requested when `factr acp` starts the daemon itself.
# Existing daemons keep their current server-wide tool config.
tool_profile = "acp"

[provider]
# Default model (optional, uses provider default if not set)
# Set via /model picker with Ctrl+B to save as default
# default_model = "claude-opus-5-5"
# Default provider (optional: claude|anthropic-api|openai|openai-api|copilot|openrouter|...)
# When set, this provider is preferred on startup if available.
#   claude        = Claude via OAuth/subscription (token in ~/.factr/engine/auth.json)
#   anthropic-api = Claude via direct Anthropic API key (ANTHROPIC_API_KEY env
#                   or ~/.config/factr/anthropic.env). API-key mode does NOT fall
#                   back to OAuth; configure the key first.
# `claude` and `anthropic-api` are distinct providers with distinct credentials.
# See docs/AUTH_CREDENTIAL_SOURCES.md for where each credential lives.
# default_provider = "copilot"
# OpenAI reasoning effort (none|minimal|low|medium|high|xhigh|max)
openai_reasoning_effort = "low"
# Anthropic reasoning effort for Claude reasoning models (none|low|medium|high|xhigh|max)
# xhigh needs Opus 4.7/4.8 or Fable 5; max needs an output_config effort model (Opus/Sonnet 4.6+).
# Defaults to xhigh for Claude Opus 4.7/4.8 (high on older Opus) when unset; other models keep their own default.
# anthropic_reasoning_effort = "medium"
# OpenAI transport mode (auto|websocket|https)
# openai_transport = "auto"
# OpenAI service tier override (priority|flex|off)
# Defaults to `priority` to match Codex /fast behavior for OpenAI OAuth
# (higher speed, higher usage). Set to "off" (or "standard") to disable.
openai_service_tier = "priority"
# Preserve provider-native reasoning/thinking for future-turn context when supported.
# Applies to OpenRouter, Anthropic, and OpenAI native reasoning replay. Display is separate.
preserve_reasoning_context = true
# Cross-provider failover when the same prompt would be resent elsewhere.
# countdown = 3-second countdown before retrying on another provider; press Esc to cancel (default)
# manual = show a notice and let you switch yourself
# cross_provider_failover = "manual"
# Try another account on the same provider before switching providers (default: true)
# same_provider_account_failover = false
cross_provider_failover = "countdown"
# Copilot premium mode: "normal" (default), "one" (first msg only), "zero" (all free)
# Set to "zero" if you have premium Copilot and want free requests
# copilot_premium = "zero"
# Only list these providers in the /model picker (issue #460). Entries match
# provider labels ("openai", "anthropic", "copilot", "openrouter", ...), route
# api methods ("claude-oauth", "openai-compatible:myprofile"), or bare
# openai-compatible profile ids ("myprofile"). The active model's routes always
# stay visible. Unset or empty = show everything.
# model_picker_providers = ["myprofile", "openrouter"]
# Max seconds to wait for streaming data before timing out a request with no
# data received. Raise this for slow reasoning models (e.g. DeepSeek) that think
# silently for minutes before emitting tokens. Default: 180.
# Applies to every streaming provider path (OpenAI native, Anthropic, Copilot,
# OpenRouter/OpenAI-compatible). The TUI's client-side stall guard also extends
# to match this value. Also overridable per-launch via FACTR_STREAM_IDLE_TIMEOUT_SECS.
# This is the base budget: high reasoning efforts scale it up automatically
# (high 2x, xhigh 3x, max/swarm 4x) since they think silently for much longer.
# stream_idle_timeout_secs = 600
# Maximum attempts for transient 429/5xx/network failures, including the first
# request. Retries honor Retry-After and use capped exponential backoff.
# Env overrides: FACTR_MAX_RETRIES, FACTR_RETRY_BACKOFF_CAP_SECS.
# max_retries = 8
# retry_backoff_cap_secs = 30

[server]
# Who executes autonomous wake requests from background completion/stall,
# swarm await completion, and communication delivery.
# "internal" starts or interrupts turns in the daemon (default).
# "external" emits typed wake_requested events for an operator to handle and
# never starts a turn or injects into a running turn.
# Env override: FACTR_WAKE_MODE
wake_mode = "internal"

[agents]
# Swarm root settings and defaults for helper agents (workers, subagents, sidecars).
# All keys are optional; the values below are the built-in defaults.
#
# Default reasoning effort for spawned swarm workers when the spawn call does
# not pass an explicit `effort` ("low", "medium", "high", ...). Leave unset so
# workers inherit the provider-wide reasoning effort.
# Env override: FACTR_SWARM_EFFORT
# swarm_effort = "medium"
#
# Root model reasoning while /effort swarm or /effort swarm-deep is selected.
# These are independent of worker swarm_effort. Supported levels:
# none|minimal|low|medium|high|xhigh|max. Unset/invalid = max (model maximum).
# Providers map unsupported levels to their supported range.
# Env overrides: FACTR_SWARM_ROOT_EFFORT, FACTR_SWARM_DEEP_ROOT_EFFORT
swarm_root_effort = "max"
swarm_deep_root_effort = "max"
#
# How swarm-created agents are spawned:
#   "inline"   - in-process worker (default)
#   "headless" - create the worker in-process
#   (legacy "visible"/"headed"/"auto" still parse and mean "inline")
# The swarm tool's per-call `spawn_mode` overrides this when set.
# Env override: FACTR_SWARM_SPAWN_MODE
swarm_spawn_mode = "inline"
#
# Max live swarm worker agents in one swarm. This RAM-safety budget applies to
# recursive ad hoc spawning and deep-mode run_plan parallelism. Completed/stopped
# workers free their slots. 0 disables this guard and leaves only the absolute
# per-swarm hard cap of 1000. Light mode uses a smaller fixed fan-out.
# Env override: FACTR_SWARM_MAX_CONCURRENT_AGENTS
swarm_max_concurrent_agents = 32
#
# Max percentage (1-90) of the chat height the inline swarm gallery band may use.
# Unset = built-in default (40%). Lower values keep more transcript visible; set
# near the minimum to collapse the gallery to a thin strip.
# swarm_gallery_max_pct = 40
#
# Settings Factr owns are read from $FACTR_CONFIG_HOME/config.yaml, never from here: the memory switch
# (memory.memory_enabled), the swarm worker model (delegation.model), the memory extraction model
# (auxiliary.background_review), the web search backend (web.backend) and compaction (compression.*).
# Env overrides for headless runs: FACTR_MEMORY_ENABLED, FACTR_MEMORY_ENABLED, FACTR_SWARM_MODEL,
# FACTR_MEMORY_MODEL, FACTR_WEB_BACKEND.
#
# Learning from chats beyond facts is the factr-learn
# learning loop (on by default; the `learning.enabled` engine setting).

[notifications]
# Desktop notifications for interactive sessions (macOS Notification Center /
# Linux notify-send).
#
# Notify when an agent turn finishes. Fires only for long turns and, by
# default, only while the terminal window is unfocused. The notification is a
# compact summary: session name, duration, todo progress, and a snippet of the
# final assistant message.
# turn_complete = true
# Minimum turn duration (seconds) before notifying (default: 120)
# turn_complete_min_secs = 120
# Lower threshold (seconds) when the session has todos, since todos indicate
# task-style work worth reporting sooner (default: 30)
# turn_complete_todo_min_secs = 30
# Only notify while the terminal window is unfocused (default: true)
# turn_complete_only_when_unfocused = true
# macOS Notification Center sound played on completion (e.g. "Glass", "Ping",
# "Hero"). Empty string disables the sound. Ignored on non-macOS. (default: "Glass")
# turn_complete_sound = "Glass"

[power]
# Prevent automatic system sleep while any factr session is actively working.
# Linux also blocks lid-switch suspend. Windows still respects explicit lid-close
# and power-button actions from your active power plan. The display may sleep.
# The guard is held only for as long as work is in flight. (default: true)
# Set FACTR_DISABLE_POWER_INHIBIT=1 to force-disable regardless of this setting.
prevent_sleep_while_streaming = true
	"##;

        default_content.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shipped template is a hand-maintained string, so a typo in it ships
    /// a config file that factr itself cannot read. Parse it here.
    #[test]
    fn default_config_template_parses() {
        let template = Config::default_config_file_contents();
        let config =
            toml::from_str::<Config>(&template).expect("the shipped config template must parse");
        assert_eq!(config.tools.mcp_tools, McpToolsMode::Auto);
        assert_eq!(config.tools.mcp_tools_token_threshold, DEFAULT_MCP_TOOLS_TOKEN_THRESHOLD);
        assert!(
            config.display.show_thinking,
            "the shipped user config must request model reasoning"
        );
        assert_eq!(
            config.display.reasoning_display(),
            ReasoningDisplayMode::Full,
            "the shipped user config must keep the full reasoning trace visible"
        );
    }
}
