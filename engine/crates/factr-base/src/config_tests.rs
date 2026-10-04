use super::{
    CompactionMode, Config, McpToolsMode, ProviderConfig, SwarmSpawnMode, ToolConfig, WebSearchEngine,
    config_env_fingerprint, populate_context_limits_from_config_ref,
};
use std::ffi::OsString;
use std::path::Path;

fn restore_env_var(key: &str, previous: Option<OsString>) {
    if let Some(previous) = previous {
        crate::env::set_var(key, previous);
    } else {
        crate::env::remove_var(key);
    }
}

#[test]
fn test_openai_reasoning_effort_defaults_to_low() {
    assert_eq!(
        ProviderConfig::default().openai_reasoning_effort.as_deref(),
        Some("low")
    );
}

#[test]
fn test_openai_fast_mode_defaults_to_priority() {
    assert_eq!(
        ProviderConfig::default().openai_service_tier.as_deref(),
        Some("priority")
    );
}

#[test]
fn preserve_reasoning_context_defaults_to_enabled() {
    assert!(ProviderConfig::default().preserve_reasoning_context);
}

#[test]
fn swarm_spawn_mode_defaults_to_inline() {
    assert_eq!(
        Config::default().agents.swarm_spawn_mode,
        SwarmSpawnMode::Inline
    );
}

#[test]
fn swarm_max_concurrent_agents_defaults_to_safe_live_worker_budget() {
    // Keep enough parallelism for deep fan-out without allowing recursive ad hoc
    // spawns to grow until the 1000-member hard cap exhausts machine memory.
    assert_eq!(Config::default().agents.swarm_max_concurrent_agents, 32);
}

#[test]
fn mermaid_feature_defaults_on_and_parses_false() {
    assert!(Config::default().features.mermaid);

    let cfg: Config =
        toml::from_str("[features]\nmermaid = false\n").expect("features.mermaid should parse");
    assert!(!cfg.features.mermaid);
}

#[test]
fn mermaid_environment_override_uses_standard_boolean_values() {
    let _guard = crate::storage::lock_test_env();
    let previous = std::env::var_os("FACTR_ENABLE_MERMAID");
    crate::env::set_var("FACTR_ENABLE_MERMAID", "off");

    let mut cfg = Config::default();
    cfg.apply_env_overrides();
    assert!(!cfg.features.mermaid);

    restore_env_var("FACTR_ENABLE_MERMAID", previous);
}

#[test]
fn auto_poke_feature_defaults_on_and_parses_false() {
    assert!(Config::default().features.auto_poke);

    let cfg: Config =
        toml::from_str("[features]\nauto_poke = false\n").expect("features.auto_poke should parse");
    assert!(!cfg.features.auto_poke);
}

#[test]
fn auto_poke_environment_override_uses_standard_boolean_values() {
    let _guard = crate::storage::lock_test_env();
    let previous = std::env::var_os("FACTR_AUTO_POKE");
    crate::env::set_var("FACTR_AUTO_POKE", "off");

    let mut cfg = Config::default();
    cfg.apply_env_overrides();
    assert!(!cfg.features.auto_poke);

    restore_env_var("FACTR_AUTO_POKE", previous);
}

#[test]
fn swarm_max_concurrent_agents_parses_and_allows_zero_for_unbounded() {
    let cfg: Config = toml::from_str("[agents]\nswarm_max_concurrent_agents = 64\n")
        .expect("swarm_max_concurrent_agents should parse");
    assert_eq!(cfg.agents.swarm_max_concurrent_agents, 64);

    let cfg: Config = toml::from_str("[agents]\nswarm_max_concurrent_agents = 0\n")
        .expect("zero should parse (disables the configurable live-agent guard)");
    assert_eq!(cfg.agents.swarm_max_concurrent_agents, 0);
}

#[test]
fn swarm_spawn_mode_parses_supported_values() {
    let cfg: Config = toml::from_str("[agents]\nswarm_spawn_mode = \"headless\"\n")
        .expect("headless swarm_spawn_mode should parse");
    assert_eq!(cfg.agents.swarm_spawn_mode, SwarmSpawnMode::Headless);

    let cfg: Config = toml::from_str("[agents]\nswarm_spawn_mode = \"auto\"\n")
        .expect("auto swarm_spawn_mode should parse");
    assert_eq!(cfg.agents.swarm_spawn_mode, SwarmSpawnMode::Inline);

    let cfg: Config = toml::from_str("[agents]\nswarm_spawn_mode = \"visible\"\n")
        .expect("visible swarm_spawn_mode should parse");
    assert_eq!(cfg.agents.swarm_spawn_mode, SwarmSpawnMode::Inline);

    // Legacy terminal-window values map to inline.
    for legacy in ["visible", "headed", "auto"] {
        assert_eq!(SwarmSpawnMode::parse(legacy), Some(SwarmSpawnMode::Inline));
    }
}

#[test]
fn swarm_spawn_mode_rejects_invalid_values() {
    let result = toml::from_str::<Config>("[agents]\nswarm_spawn_mode = \"background\"\n");
    assert!(result.is_err());
}

#[test]
fn swarm_spawn_mode_as_str_round_trips() {
    for mode in [
        SwarmSpawnMode::Headless,
        SwarmSpawnMode::Inline,
    ] {
        assert_eq!(SwarmSpawnMode::parse(mode.as_str()), Some(mode));
    }
}

#[test]
fn test_env_override_swarm_spawn_mode() {
    let _guard = crate::storage::lock_test_env();
    let prev = std::env::var_os("FACTR_SWARM_SPAWN_MODE");
    crate::env::set_var("FACTR_SWARM_SPAWN_MODE", "headless");

    let mut cfg = Config::default();
    cfg.apply_env_overrides();

    assert_eq!(cfg.agents.swarm_spawn_mode, SwarmSpawnMode::Headless);

    restore_env_var("FACTR_SWARM_SPAWN_MODE", prev);
}

#[test]
fn swarm_effort_parses_from_toml_and_env_override() {
    let _guard = crate::storage::lock_test_env();
    let prev = std::env::var_os("FACTR_SWARM_EFFORT");
    restore_env_var("FACTR_SWARM_EFFORT", None);

    // Public config-file interface (#1165).
    let cfg: Config =
        toml::from_str("[agents]\nswarm_effort = \"medium\"\n")
            .expect("config with swarm_effort parses");
    assert_eq!(cfg.agents.swarm_effort.as_deref(), Some("medium"));
    assert_eq!(Config::default().agents.swarm_effort, None);

    crate::env::set_var("FACTR_SWARM_EFFORT", "low");
    let mut cfg = Config::default();
    cfg.apply_env_overrides();
    assert_eq!(cfg.agents.swarm_effort.as_deref(), Some("low"));

    crate::env::set_var("FACTR_SWARM_EFFORT", " ");
    let mut cfg = Config::default();
    cfg.agents.swarm_effort = Some("preset".to_string());
    cfg.apply_env_overrides();
    assert_eq!(cfg.agents.swarm_effort, None);

    restore_env_var("FACTR_SWARM_EFFORT", prev);
}

#[test]
fn wake_mode_defaults_parses_and_env_overrides() {
    let _guard = crate::storage::lock_test_env();
    let prev = std::env::var_os("FACTR_WAKE_MODE");
    assert_eq!(
        Config::default().server.wake_mode,
        crate::config::WakeMode::Internal
    );
    let parsed: Config = toml::from_str("[server]\nwake_mode = \"external\"\n").unwrap();
    assert_eq!(parsed.server.wake_mode, crate::config::WakeMode::External);

    crate::env::set_var("FACTR_WAKE_MODE", "external");
    let mut cfg = Config::default();
    cfg.apply_env_overrides();
    assert_eq!(cfg.server.wake_mode, crate::config::WakeMode::External);
    restore_env_var("FACTR_WAKE_MODE", prev);
}

#[test]
fn legacy_terminal_section_is_ignored() {
    let cfg: Config = toml::from_str("[terminal]\nspawn_hook = \"tmux new-window\"\npreferred = \"ghostty\"\n")
        .expect("legacy [terminal] section should still parse (ignored)");
    assert_eq!(cfg.agents.swarm_spawn_mode, SwarmSpawnMode::Inline);
}

#[test]
fn an_old_hooks_table_still_parses_but_configures_nothing() {
    // `[hooks]` in config.toml used to run commands; Factr's `hooks:` in config.yaml is the one source now.
    let cfg: Config = toml::from_str(
        "[hooks]\nturn_start = \"notify-start\"\npre_tool = [\"policy-a\", \"policy-b\"]\npre_tool_timeout_ms = 1500\n[agents]\nswarm_spawn_mode = \"inline\"\n",
    )
    .expect("old hook keys are ignored, not an error");
    assert_eq!(cfg.agents.swarm_spawn_mode, SwarmSpawnMode::Inline, "the rest of the file still applies");
}

#[test]
fn tool_config_defaults_to_full_toolset() {
    // `selection()` reads the Factr config's disabled tools: never the developer's real one.
    let _lock = crate::storage::lock_test_env();
    let config = ToolConfig::default();
    let selection = config.selection();
    assert!(selection.allowed_tools.is_none());
    assert!(selection.disabled_tools.is_empty());
    assert_eq!(config.mcp_tools, McpToolsMode::Auto);
    assert_eq!(config.mcp_tools_token_threshold, super::DEFAULT_MCP_TOOLS_TOKEN_THRESHOLD);
}

#[test]
fn tool_config_deserializes_all_mcp_exposure_modes() {
    for (raw, expected) in [
        ("auto", McpToolsMode::Auto),
        ("eager", McpToolsMode::Eager),
        ("deferred", McpToolsMode::Deferred),
    ] {
        let config: Config = toml::from_str(&format!("[tools]\nmcp_tools = \"{raw}\"\n"))
            .expect("valid MCP exposure mode");
        assert_eq!(config.tools.mcp_tools, expected);
    }
}

#[test]
fn tool_config_mcp_exposure_env_overrides() {
    let _guard = crate::storage::lock_test_env();
    let previous_mode = std::env::var_os("FACTR_MCP_TOOLS");
    let previous_threshold = std::env::var_os("FACTR_MCP_TOOLS_TOKEN_THRESHOLD");
    crate::env::set_var("FACTR_MCP_TOOLS", "deferred");
    crate::env::set_var("FACTR_MCP_TOOLS_TOKEN_THRESHOLD", "4321");

    let mut config = Config::default();
    config.apply_env_overrides();

    assert_eq!(config.tools.mcp_tools, McpToolsMode::Deferred);
    assert_eq!(config.tools.mcp_tools_token_threshold, 4_321);
    restore_env_var("FACTR_MCP_TOOLS", previous_mode);
    restore_env_var("FACTR_MCP_TOOLS_TOKEN_THRESHOLD", previous_threshold);
}

#[test]
fn tool_config_explicit_enabled_uses_allow_list() {
    let cfg = ToolConfig {
        enabled: vec!["example_tool".to_string()],
        ..ToolConfig::default()
    };
    let selection = cfg.selection();
    let allowed = selection
        .allowed_tools
        .expect("explicit enabled is an allow-list");

    assert!(allowed.contains("example_tool"));
    assert!(!selection.disabled_tools.contains("example_tool"));
}

#[test]
fn tool_config_all_enabled_sentinel_keeps_unrestricted_toolset() {
    let cfg = ToolConfig {
        enabled: vec!["*".to_string()],
        ..ToolConfig::default()
    };
    let selection = cfg.selection();

    assert!(selection.allowed_tools.is_none());
    assert!(!selection.disabled_tools.contains("example_tool"));
}

#[test]
fn tool_config_explicit_disabled_overrides_all_enabled_sentinel() {
    let cfg = ToolConfig {
        enabled: vec!["*".to_string()],
        disabled: vec!["example_tool".to_string()],
        ..ToolConfig::default()
    };
    let selection = cfg.selection();

    assert!(selection.allowed_tools.is_none());
    assert!(selection.disabled_tools.contains("example_tool"));
}

#[test]
fn tool_config_acp_profile_allows_core_coding_plus_batch() {
    // `selection()` reads the Factr config's disabled tools: never the developer's real one.
    let _lock = crate::storage::lock_test_env();
    let cfg = ToolConfig {
        profile: "acp".to_string(),
        ..ToolConfig::default()
    };
    let allowed = cfg.allowed_tools().expect("acp profile is an allow-list");

    assert!(allowed.contains("bash"));
    assert!(allowed.contains("read"));
    assert!(allowed.contains("write"));
    assert!(allowed.contains("apply_patch"));
    assert!(allowed.contains("agentgrep"));
    assert!(allowed.contains("batch"));
    assert!(allowed.contains("mcp"));
    assert!(!allowed.contains("subagent"));
}

#[test]
fn acp_config_defaults_to_standard_profile_and_acp_tools() {
    let cfg = Config::default();
    assert_eq!(cfg.acp.profile, "standard");
    assert_eq!(cfg.acp.tool_profile, "acp");
}

#[test]
fn tool_config_minimal_profile_allows_core_coding_tools() {
    // `selection()` reads the Factr config's disabled tools: never the developer's real one.
    let _lock = crate::storage::lock_test_env();
    let cfg = ToolConfig {
        profile: "minimal".to_string(),
        ..ToolConfig::default()
    };
    let allowed = cfg
        .allowed_tools()
        .expect("minimal profile is an allow-list");

    assert!(allowed.contains("bash"));
    assert!(allowed.contains("read"));
    assert!(allowed.contains("write"));
    assert!(allowed.contains("apply_patch"));
    assert!(allowed.contains("agentgrep"));
    assert!(!allowed.contains("browser"));
}

#[test]
fn tool_config_explicit_enabled_and_disabled_lists_compose() {
    // `selection()` reads the Factr config's disabled tools: never the developer's real one.
    let _lock = crate::storage::lock_test_env();
    let cfg = ToolConfig {
        enabled: vec![
            "shell".to_string(),
            "read_file".to_string(),
            "browser".to_string(),
        ],
        disabled: vec!["browser".to_string()],
        ..ToolConfig::default()
    };
    let selection = cfg.selection();
    let allowed = selection
        .allowed_tools
        .expect("explicit enabled is an allow-list");

    assert!(allowed.contains("bash"));
    assert!(allowed.contains("read"));
    assert!(!allowed.contains("shell"));
    assert!(!allowed.contains("read_file"));
    assert!(!allowed.contains("browser"));
    assert!(selection.disabled_tools.contains("browser"));
}

#[test]
fn factr_backend_toolset_toggles_filter_engine_tools() {
    let _env_lock = crate::storage::lock_test_env();
    let previous = std::env::var_os("FACTR_CONFIG_HOME");
    let home = tempfile::tempdir().expect("temporary Factr home");
    std::fs::write(
        home.path().join("config.yaml"),
        "platform_toolsets:\n  cli:\n    - terminal\n",
    )
    .expect("write Factr toolset setting");
    crate::env::set_var("FACTR_CONFIG_HOME", home.path());

    let selection = ToolConfig::default().selection();
    assert!(selection.disabled_tools.contains("read"));
    assert!(selection.disabled_tools.contains("write"));
    assert!(!selection.disabled_tools.contains("bash"));

    restore_env_var("FACTR_CONFIG_HOME", previous);
}

#[test]
fn factr_memory_setting_is_the_engine_chat_source_of_truth() {
    let _env_lock = crate::storage::lock_test_env();
    let previous = std::env::var_os("FACTR_CONFIG_HOME");
    let home = tempfile::tempdir().expect("temporary Factr home");
    std::fs::write(
        home.path().join("config.yaml"),
        "memory:\n  memory_enabled: false\n",
    )
    .expect("write Factr memory setting");
    crate::env::set_var("FACTR_CONFIG_HOME", home.path());

    let enabled = super::memory_enabled();
    restore_env_var("FACTR_CONFIG_HOME", previous);
    assert!(!enabled, "Factr desktop memory off setting must disable chat memory");
}

/// Runs `body` with `FACTR_CONFIG_HOME` at a temp dir holding `config.yaml`, and the named env vars cleared.
fn with_factr_yaml(yaml: &str, cleared: &[&str], body: impl FnOnce()) {
    let _env_lock = crate::storage::lock_test_env();
    let saved: Vec<_> = std::iter::once("FACTR_CONFIG_HOME").chain(cleared.iter().copied()).map(|k| (k, std::env::var_os(k))).collect();
    let home = tempfile::tempdir().expect("temporary Factr home");
    std::fs::write(home.path().join("config.yaml"), yaml).expect("write config.yaml");
    crate::env::set_var("FACTR_CONFIG_HOME", home.path());
    for key in cleared {
        crate::env::remove_var(key);
    }
    body();
    for (key, previous) in saved {
        restore_env_var(key, previous);
    }
}

#[test]
fn memory_switch_is_env_then_factr_then_on() {
    with_factr_yaml("memory:\n  memory_enabled: false\n", &["FACTR_MEMORY_ENABLED"], || {
        assert!(!super::memory_enabled(), "Factr off");
        crate::env::set_var("FACTR_MEMORY_ENABLED", "1");
        assert!(super::memory_enabled(), "the env override outranks Factr");
        crate::env::set_var("FACTR_MEMORY_ENABLED", "off");
        assert!(!super::memory_enabled());
        crate::env::remove_var("FACTR_MEMORY_ENABLED");
    });
    with_factr_yaml("model: x\n", &["FACTR_MEMORY_ENABLED"], || assert!(super::memory_enabled(), "unset everywhere: on"));
}

#[test]
fn swarm_worker_model_is_env_then_factr_delegation() {
    with_factr_yaml("delegation: {model: gpt-5.5, provider: openai-codex}\n", &["FACTR_SWARM_MODEL"], || {
        assert_eq!(crate::factr_config::swarm_model().as_deref(), Some("openai-oauth:gpt-5.5"));
        crate::env::set_var("FACTR_SWARM_MODEL", "claude-opus-4-6");
        assert_eq!(crate::factr_config::swarm_model().as_deref(), Some("claude-opus-4-6"));
        // Set but empty: workers inherit, whatever Factr names.
        crate::env::set_var("FACTR_SWARM_MODEL", "  ");
        assert_eq!(crate::factr_config::swarm_model(), None);
        crate::env::remove_var("FACTR_SWARM_MODEL");
    });
    with_factr_yaml("model: x\n", &["FACTR_SWARM_MODEL"], || assert_eq!(crate::factr_config::swarm_model(), None));
}

#[test]
fn memory_extraction_model_is_env_then_the_background_review_slot() {
    with_factr_yaml("auxiliary:\n  background_review: {provider: openrouter, model: cheap}\n", &[], || {
        assert_eq!(
            crate::factr_config::aux_model(crate::factr_config::AuxConsumer::MemoryExtraction).as_deref(),
            Some("openrouter:cheap")
        );
    });
}

/// config.toml files written before these settings moved to Factr still load: the keys are ignored.
#[test]
fn retired_config_toml_keys_parse_and_change_nothing() {
    let cfg: Config = toml::from_str(
        "[features]\nmemory = false\n\n[agents]\nswarm_model = \"claude-opus-5\"\nmemory_model = \"cheap\"\n\
         memory_sidecar_enabled = false\nswarm_effort = \"medium\"\n\n[websearch]\nbackend = \"exa\"\nengine = \"bing\"\n\n\
         [compaction]\nmax_context_tokens = 50000\nmode = \"proactive\"\n\n[display]\ndebug_socket = true\n",
    )
    .expect("an old config.toml must not fail to parse");
    assert_eq!(cfg.agents.swarm_effort.as_deref(), Some("medium"), "live keys beside retired ones still apply");
    assert_eq!(cfg.websearch.engine, WebSearchEngine::Bing);
    assert_eq!(cfg.compaction.mode, CompactionMode::Proactive);
}

#[test]
fn tool_config_none_profile_disables_all_tools() {
    let cfg = ToolConfig {
        profile: "none".to_string(),
        ..ToolConfig::default()
    };
    assert!(
        cfg.allowed_tools()
            .expect("none profile is empty")
            .is_empty()
    );
}

#[test]
fn tool_config_disabled_only_keeps_full_profile_with_deny_list() {
    let cfg = ToolConfig {
        disabled: vec!["browser".to_string(), "example_tool".to_string()],
        ..ToolConfig::default()
    };
    let selection = cfg.selection();

    assert!(selection.allowed_tools.is_none());
    assert!(selection.disabled_tools.contains("browser"));
    assert!(selection.disabled_tools.contains("example_tool"));
    assert!(!selection.disabled_tools.contains("another_tool"));
}

#[test]
fn test_generated_default_config_has_expected_user_defaults() {
    let _guard = crate::storage::lock_test_env();
    let prev_home = std::env::var_os("FACTR_HOME");
    let dir = tempfile::TempDir::new().expect("tempdir");
    crate::env::set_var("FACTR_HOME", dir.path());

    let path = Config::create_default_config_file().expect("create default config file");
    let content = std::fs::read_to_string(path).expect("read default config file");

    assert!(
        content.contains("openai_reasoning_effort = \"low\""),
        "generated default config should use low OpenAI reasoning effort"
    );
    assert!(
        content.contains("openai_service_tier = \"priority\""),
        "generated default config should enable OpenAI fast mode"
    );
    assert!(
        content.contains("[tools]") && content.contains("profile = \"full\""),
        "generated default config should document tool profiles"
    );
    assert!(
        content.contains("[acp]") && content.contains("tool_profile = \"acp\""),
        "generated default config should document ACP profile settings"
    );
    assert!(
        content.contains("[agents]") && content.contains("swarm_spawn_mode = \"inline\""),
        "generated default config should document agent spawn defaults"
    );
    assert!(
        content.contains("memory.memory_enabled") && content.contains("delegation.model"),
        "generated default config should point at the settings Factr owns"
    );
    for retired in ["swarm_model =", "memory_model =", "memory_sidecar_enabled", "\nmemory = ", "max_context_tokens"] {
        assert!(!content.contains(retired), "generated default config must not advertise the retired key {retired:?}");
    }

    // The generated file must always be valid TOML for the current Config schema.
    let parsed: Config =
        toml::from_str(&content).expect("generated default config should parse as Config");
    assert_eq!(parsed.agents.swarm_spawn_mode, SwarmSpawnMode::Inline);
    assert!(
        parsed.display.show_thinking,
        "freshly created user config should request model reasoning"
    );
    assert_eq!(
        parsed.display.reasoning_display(),
        factr_config_types::ReasoningDisplayMode::Full,
        "freshly created user config should show the full reasoning trace"
    );

    if let Some(prev) = prev_home {
        crate::env::set_var("FACTR_HOME", prev);
    } else {
        crate::env::remove_var("FACTR_HOME");
    }
}

#[test]
fn global_config_cache_reloads_after_manual_file_edit() {
    let _guard = crate::storage::lock_test_env();
    let prev_home = std::env::var_os("FACTR_HOME");
    let dir = tempfile::TempDir::new().expect("tempdir");
    crate::env::set_var("FACTR_HOME", dir.path());
    Config::invalidate_cache();

    let path = Config::path().expect("config path");
    std::fs::create_dir_all(path.parent().expect("config parent")).expect("create config parent");
    std::fs::write(&path, "[display]\nshow_thinking = true\n").expect("write initial config");

    assert!(crate::config::config().display.show_thinking);

    // Different length as well as mtime so the metadata fingerprint notices the
    // manual edit even on filesystems with coarse timestamp resolution.
    std::fs::write(&path, "[display]\nshow_thinking = false\n# edited\n").expect("edit config");

    assert!(!crate::config::config().display.show_thinking);

    restore_env_var("FACTR_HOME", prev_home);
    Config::invalidate_cache();
}

#[test]
fn config_save_invalidates_global_config_cache() {
    let _guard = crate::storage::lock_test_env();
    let prev_home = std::env::var_os("FACTR_HOME");
    let dir = tempfile::TempDir::new().expect("tempdir");
    crate::env::set_var("FACTR_HOME", dir.path());
    Config::invalidate_cache();

    let mut cfg = Config::default();
    cfg.display.show_thinking = true;
    cfg.save().expect("save initial config");
    assert!(crate::config::config().display.show_thinking);

    cfg.display.show_thinking = false;
    cfg.save().expect("save updated config");
    assert!(!crate::config::config().display.show_thinking);

    restore_env_var("FACTR_HOME", prev_home);
    Config::invalidate_cache();
}

#[test]
fn config_env_fingerprint_ignores_runtime_only_factr_vars() {
    let _guard = crate::storage::lock_test_env();
    let prev_runtime_provider = std::env::var_os("FACTR_RUNTIME_PROVIDER");
    let prev_active_provider = std::env::var_os("FACTR_ACTIVE_PROVIDER");
    let prev_show_thinking = std::env::var_os("FACTR_SHOW_THINKING");

    crate::env::remove_var("FACTR_RUNTIME_PROVIDER");
    crate::env::remove_var("FACTR_ACTIVE_PROVIDER");
    crate::env::remove_var("FACTR_SHOW_THINKING");
    let baseline = config_env_fingerprint();

    crate::env::set_var("FACTR_RUNTIME_PROVIDER", "openai");
    crate::env::set_var("FACTR_ACTIVE_PROVIDER", "openai");
    assert_eq!(baseline, config_env_fingerprint());

    crate::env::set_var("FACTR_SHOW_THINKING", "1");
    assert_ne!(baseline, config_env_fingerprint());

    restore_env_var("FACTR_RUNTIME_PROVIDER", prev_runtime_provider);
    restore_env_var("FACTR_ACTIVE_PROVIDER", prev_active_provider);
    restore_env_var("FACTR_SHOW_THINKING", prev_show_thinking);
}

#[test]
fn config_env_fingerprint_tracks_every_apply_env_override_var() {
    let override_source = include_str!("config/env_overrides.rs");
    let mut missing = Vec::new();

    for line in override_source.lines() {
        let Some(start) = line.find("std::env::var(\"") else {
            continue;
        };
        let rest = &line[start + "std::env::var(\"".len()..];
        let Some(end) = rest.find('"') else {
            continue;
        };
        let key = &rest[..end];
        if !crate::config::CONFIG_ENV_KEYS.contains(&key) {
            missing.push(key.to_string());
        }
    }

    missing.sort();
    missing.dedup();
    assert!(
        missing.is_empty(),
        "CONFIG_ENV_KEYS must include every env var read by Config::apply_env_overrides; missing: {missing:?}"
    );
}

#[test]
fn cached_external_auth_trust_observes_manual_revocation() {
    let _guard = crate::storage::lock_test_env();
    let prev_home = std::env::var_os("FACTR_HOME");
    let dir = tempfile::TempDir::new().expect("tempdir");
    crate::env::set_var("FACTR_HOME", dir.path());
    Config::invalidate_cache();

    let auth_file = dir.path().join("external-auth.json");
    std::fs::write(&auth_file, "{}\n").expect("write external auth file");
    Config::allow_external_auth_source_for_path("test_source", &auth_file)
        .expect("trust external auth path");
    assert!(Config::external_auth_source_allowed_for_path_cached(
        "test_source",
        &auth_file
    ));

    let path = Config::path().expect("config path");
    std::fs::write(
        &path,
        "[auth]\ntrusted_external_source_paths = []\n# manually revoked\n",
    )
    .expect("manually revoke external auth trust");

    assert!(!Config::external_auth_source_allowed_for_path_cached(
        "test_source",
        &auth_file
    ));

    restore_env_var("FACTR_HOME", prev_home);
    Config::invalidate_cache();
}

#[test]
fn test_provider_failover_defaults_match_new_behavior() {
    let provider = Config::default().provider;
    assert_eq!(
        provider.cross_provider_failover,
        super::CrossProviderFailoverMode::Countdown
    );
    assert!(provider.same_provider_account_failover);
}

#[test]
fn test_provider_failover_disabled_aliases_parse_as_manual() {
    for value in ["off", "false", "disabled", "none"] {
        let cfg: Config = toml::from_str(&format!(
            "[provider]\ncross_provider_failover = \"{value}\"\n"
        ))
        .unwrap_or_else(|error| panic!("{value} should parse: {error}"));
        assert_eq!(
            cfg.provider.cross_provider_failover,
            super::CrossProviderFailoverMode::Manual
        );
        assert_eq!(
            super::CrossProviderFailoverMode::parse(value),
            Some(super::CrossProviderFailoverMode::Manual)
        );
    }
}

#[test]
fn test_env_override_trusted_external_auth_splits_source_and_path_entries() {
    let _guard = crate::storage::lock_test_env();
    let prev = std::env::var_os("FACTR_TRUSTED_EXTERNAL_AUTH_SOURCES");
    crate::env::set_var(
        "FACTR_TRUSTED_EXTERNAL_AUTH_SOURCES",
        "legacy_source,claude_code_credentials|/tmp/auth.json",
    );

    let mut cfg = Config::default();
    cfg.apply_env_overrides();

    assert_eq!(cfg.auth.trusted_external_sources, vec!["legacy_source"]);
    assert_eq!(
        cfg.auth.trusted_external_source_paths,
        vec!["claude_code_credentials|/tmp/auth.json"]
    );

    if let Some(prev) = prev {
        crate::env::set_var("FACTR_TRUSTED_EXTERNAL_AUTH_SOURCES", prev);
    } else {
        crate::env::remove_var("FACTR_TRUSTED_EXTERNAL_AUTH_SOURCES");
    }
}

#[test]
fn test_external_auth_source_allowed_for_path_matches_saved_entry() {
    let _guard = crate::storage::lock_test_env();
    let dir = tempfile::TempDir::new().expect("tempdir");
    let path = dir.path().join("auth.json");
    std::fs::write(&path, "{}\n").expect("write auth file");

    let canonical = std::fs::canonicalize(&path).expect("canonical path");
    let mut cfg = Config::default();
    cfg.auth.trusted_external_source_paths = vec![format!(
        "test_source|{}",
        canonical.to_string_lossy().to_ascii_lowercase()
    )];

    assert!(cfg.external_auth_source_allowed_for_path_config("test_source", &path));
}

#[test]
fn test_external_auth_source_allowed_for_path_ignores_broad_legacy_entry() {
    let _guard = crate::storage::lock_test_env();
    let dir = tempfile::TempDir::new().expect("tempdir");
    let path = dir.path().join("auth.json");
    std::fs::write(&path, "{}\n").expect("write auth file");

    let mut cfg = Config::default();
    cfg.auth.trusted_external_sources = vec!["test_source".to_string()];

    assert!(!cfg.external_auth_source_allowed_for_path_config("test_source", &path));
}

/// Regression test for issue #349: a removed/unknown `update_channel` value
/// (older configs could contain `"manual"`) must not fail the whole config
/// parse. A hard parse failure during the reload handoff left the reload
/// marker stuck in `starting` and clients re-requested the reload forever.
#[test]
fn unknown_update_channel_value_falls_back_to_stable_instead_of_failing_parse() {
    let cfg: Config = toml::from_str("[features]\nupdate_channel = \"manual\"\n")
        .expect("unknown update_channel must not fail config parse");
    assert_eq!(
        cfg.features.update_channel,
        super::UpdateChannel::Stable,
        "unknown channel should fall back to the default"
    );

    // Other settings in the same config must survive the fallback.
    let cfg: Config = toml::from_str(
        "[features]\nupdate_channel = \"manual\"\n\n[display]\nshow_thinking = false\n",
    )
    .expect("config with unknown update_channel should parse");
    assert_eq!(cfg.features.update_channel, super::UpdateChannel::Stable);
    assert!(!cfg.display.show_thinking);
}

#[test]
fn known_update_channel_values_still_parse() {
    let cfg: Config = toml::from_str("[features]\nupdate_channel = \"main\"\n")
        .expect("main update_channel should parse");
    assert_eq!(cfg.features.update_channel, super::UpdateChannel::Main);

    let cfg: Config = toml::from_str("[features]\nupdate_channel = \"stable\"\n")
        .expect("stable update_channel should parse");
    assert_eq!(cfg.features.update_channel, super::UpdateChannel::Stable);
}

#[test]
fn update_channel_parse_accepts_known_aliases_and_rejects_unknown() {
    use super::UpdateChannel;
    assert_eq!(UpdateChannel::parse("stable"), Some(UpdateChannel::Stable));
    assert_eq!(UpdateChannel::parse("release"), Some(UpdateChannel::Stable));
    assert_eq!(UpdateChannel::parse("main"), Some(UpdateChannel::Main));
    assert_eq!(UpdateChannel::parse("nightly"), Some(UpdateChannel::Main));
    assert_eq!(UpdateChannel::parse("edge"), Some(UpdateChannel::Main));
    assert_eq!(UpdateChannel::parse(" Main "), Some(UpdateChannel::Main));
    assert_eq!(UpdateChannel::parse("manual"), None);
    assert_eq!(UpdateChannel::parse(""), None);
}

impl Config {
    fn external_auth_source_allowed_for_path_config(&self, source_id: &str, path: &Path) -> bool {
        let Ok(entry) = Self::trusted_external_auth_path_entry(source_id, path) else {
            return false;
        };
        self.auth
            .trusted_external_source_paths
            .iter()
            .any(|value| value.trim().eq_ignore_ascii_case(&entry))
    }
}

#[test]
fn populate_context_limits_from_config_ref_seeds_global_cache() {
    use super::{NamedProviderConfig, NamedProviderModelConfig};

    // Regression test for issue #366: a named OpenAI-compatible provider with a
    // per-model `context_window` must be honored by the global context-limit
    // resolution path, not just the provider instance's own context_window().
    let model_id = "issue366-custom-gateway-model";
    let mut cfg = Config::default();
    cfg.providers.insert(
        "issue366-gateway".to_string(),
        NamedProviderConfig {
            base_url: "https://gateway.example.test/v1".to_string(),
            models: vec![NamedProviderModelConfig {
                id: model_id.to_string(),
                reasoning: None,
                reasoning_effort: None,
                context_window: Some(1_000_000),
                input: Vec::new(),
            }],
            ..Default::default()
        },
    );

    populate_context_limits_from_config_ref(&cfg);

    assert_eq!(
        crate::provider::context_limit_for_model(model_id),
        Some(1_000_000),
        "global context-limit resolution should respect named provider context_window"
    );
}

#[test]
fn populate_context_limits_from_config_seeds_qualified_runtime_model_shapes() {
    use super::{NamedProviderConfig, NamedProviderModelConfig};

    // Regression test for issue #421: the runtime request model can be
    // provider-qualified (`cachyai-a2000:qwen...`) or a slash path served by
    // llama.cpp (`ornith-box-1:/opt/models/ornith-1.0-35b-Q4_K_M.gguf`). The
    // configured context_window must resolve for every shape, not just the
    // bare id, otherwise budgeting falls back to the 200K default and
    // over-sends context.
    let mut cfg = Config::default();
    cfg.providers.insert(
        "issue421-gateway".to_string(),
        NamedProviderConfig {
            base_url: "http://10.15.15.53:8080/v1".to_string(),
            models: vec![
                NamedProviderModelConfig {
                    id: "issue421-qwen-128k".to_string(),
                    reasoning: None,
                    reasoning_effort: None,
                    context_window: Some(131_072),
                    input: Vec::new(),
                },
                NamedProviderModelConfig {
                    id: "/opt/models/issue421-ornith-35b-q4.gguf".to_string(),
                    reasoning: None,
                    reasoning_effort: None,
                    context_window: Some(131_072),
                    input: Vec::new(),
                },
            ],
            ..Default::default()
        },
    );

    populate_context_limits_from_config_ref(&cfg);

    // Bare id.
    assert_eq!(
        crate::provider::context_limit_for_model("issue421-qwen-128k"),
        Some(131_072)
    );
    // Profile-qualified spec, as persisted by session restore.
    assert_eq!(
        crate::provider::context_limit_for_model("issue421-gateway:issue421-qwen-128k"),
        Some(131_072),
        "profile-qualified model spec must resolve the configured context_window"
    );
    // Slash-path model id: the lookup reduces to the slash base.
    assert_eq!(
        crate::provider::context_limit_for_model("/opt/models/issue421-ornith-35b-q4.gguf"),
        Some(131_072),
        "slash-path model id must resolve the configured context_window"
    );
    // Profile-qualified slash-path spec, exactly as reported in issue #421.
    assert_eq!(
        crate::provider::context_limit_for_model(
            "issue421-gateway:/opt/models/issue421-ornith-35b-q4.gguf"
        ),
        Some(131_072),
        "profile-qualified slash-path spec must resolve the configured context_window"
    );
}

#[test]
fn config_reload_generation_increments_on_cache_invalidation() {
    let before = crate::config::config_reload_generation();
    crate::config::invalidate_config_cache();
    let after = crate::config::config_reload_generation();
    assert!(
        after > before,
        "invalidate_config_cache must bump the reload generation ({before} -> {after})"
    );
}

#[test]
fn swarm_root_effort_config_defaults_and_independent_modes() {
    let defaults = Config::default();
    assert_eq!(defaults.agents.root_effort_for_swarm(false), "max");
    assert_eq!(defaults.agents.root_effort_for_swarm(true), "max");
    let cfg: Config = toml::from_str(
        "[agents]\nswarm_root_effort = 'low'\nswarm_deep_root_effort = ' High '\nswarm_effort = 'medium'\n",
    ).unwrap();
    assert_eq!(cfg.agents.root_effort_for_swarm(false), "low");
    assert_eq!(cfg.agents.root_effort_for_swarm(true), "high");
    assert_eq!(cfg.agents.swarm_effort.as_deref(), Some("medium"));
    let serialized = toml::to_string(&cfg).unwrap();
    let round_trip: Config = toml::from_str(&serialized).unwrap();
    assert_eq!(round_trip.agents.root_effort_for_swarm(true), "high");

    for level in ["none", "minimal", "low", "medium", "high", "xhigh", "max"] {
        let cfg: Config =
            toml::from_str(&format!("[agents]\nswarm_root_effort = '{level}'")).unwrap();
        assert_eq!(cfg.agents.root_effort_for_swarm(false), level);
        assert_eq!(cfg.agents.root_effort_for_swarm(true), "max");
    }
    for invalid in ["", "swarm", "swarm-deep", "turbo"] {
        let cfg: Config = toml::from_str(&format!("[agents]\nswarm_root_effort = '{invalid}'\nswarm_deep_root_effort = '{invalid}'\nswarm_effort = 'low'")).unwrap();
        assert_eq!(cfg.agents.root_effort_for_swarm(false), "max");
        assert_eq!(cfg.agents.root_effort_for_swarm(true), "max");
        assert_eq!(cfg.agents.swarm_effort.as_deref(), Some("low"));
    }
}

#[test]
fn swarm_root_effort_env_overrides_and_shared_resolution() {
    let _guard = crate::storage::lock_test_env();
    let keys = ["FACTR_SWARM_ROOT_EFFORT", "FACTR_SWARM_DEEP_ROOT_EFFORT"];
    let previous = keys.map(std::env::var_os);
    let fingerprint = config_env_fingerprint();
    crate::env::set_var(keys[0], "low");
    crate::env::set_var(keys[1], "high");
    assert_ne!(config_env_fingerprint(), fingerprint);
    let mut cfg = Config::default();
    cfg.apply_env_overrides();
    assert_eq!(cfg.agents.root_effort_for_swarm(false), "low");
    assert_eq!(cfg.agents.root_effort_for_swarm(true), "high");
    assert_eq!(
        crate::prompt::swarm_root_reasoning_effort("swarm"),
        Some("low")
    );
    assert_eq!(
        crate::prompt::swarm_root_reasoning_effort(" Swarm-Deep "),
        Some("high")
    );
    assert_eq!(crate::prompt::swarm_root_reasoning_effort("low"), None);
    crate::env::set_var(keys[0], "none");
    assert_eq!(
        crate::prompt::swarm_root_reasoning_effort("swarm"),
        Some("none")
    );
    // Config changes must not turn orchestration off or misrepresent its effort.
    for mode in ["swarm", "swarm-deep"] {
        let mut split = crate::prompt::SplitSystemPrompt::default();
        crate::prompt::append_swarm_effort_directive(&mut split, Some(mode));
        assert!(split.static_part.contains("swarm"));
        assert!(!split.static_part.contains("maximum reasoning effort"));
    }
    for key in keys {
        crate::env::set_var(key, " ");
    }
    cfg.apply_env_overrides();
    assert_eq!(cfg.agents.swarm_root_effort, None);
    assert_eq!(cfg.agents.swarm_deep_root_effort, None);
    for (key, value) in keys.into_iter().zip(previous) {
        restore_env_var(key, value);
    }
}

#[test]
fn anthropic_cache_preference_persists_and_preserves_other_settings() {
    let _guard = crate::storage::lock_test_env();
    let prev_home = std::env::var_os("FACTR_HOME");
    let dir = tempfile::TempDir::new().unwrap();
    crate::env::set_var("FACTR_HOME", dir.path());
    Config::invalidate_cache();
    let path = Config::path().unwrap();
    std::fs::write(&path, "[provider]\ndefault_model = 'keep-me'\n").unwrap();
    assert!(crate::config::config().provider.anthropic_cache_ttl_1h);
    for enabled in [false, true] {
        Config::set_anthropic_cache_ttl_1h(enabled).unwrap();
        Config::invalidate_cache();
        assert_eq!(Config::load().provider.anthropic_cache_ttl_1h, enabled);
        assert_eq!(crate::provider::anthropic::is_cache_ttl_1h(), enabled);
        assert_eq!(
            crate::config::config().provider.anthropic_cache_ttl_1h,
            enabled
        );
        assert_eq!(
            Config::load().provider.default_model.as_deref(),
            Some("keep-me")
        );
    }
    std::fs::write(&path, "[broken").unwrap();
    assert!(Config::set_anthropic_cache_ttl_1h(false).is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "[broken");
    restore_env_var("FACTR_HOME", prev_home);
    Config::invalidate_cache();
}

#[test]
fn cli_config_save_round_trips_desktop_tables() {
    let _guard = crate::storage::lock_test_env();
    let prev_home = std::env::var_os("FACTR_HOME");
    let dir = tempfile::TempDir::new().expect("tempdir");
    crate::env::set_var("FACTR_HOME", dir.path());
    Config::invalidate_cache();

    let path = dir.path().join("config.toml");
    std::fs::write(
        &path,
        "[display]\nshow_thinking = true\n\n[desktop.voice]\nglobal_hold = true\n\
         global_devices = [\"/dev/input/event3\"]\n\n[desktop.appearance]\ntheme = \"warm-neutral\"\n",
    )
    .unwrap();

    // A CLI read-modify-write must not drop Desktop-owned settings.
    Config::set_default_model(Some("gpt-test"), None).expect("save");
    let saved: toml::Table = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let desktop = saved["desktop"].as_table().expect("desktop table kept");
    assert_eq!(desktop["voice"]["global_hold"].as_bool(), Some(true));
    assert_eq!(
        desktop["voice"]["global_devices"][0].as_str(),
        Some("/dev/input/event3")
    );
    assert_eq!(
        desktop["appearance"]["theme"].as_str(),
        Some("warm-neutral")
    );

    // Configs without Desktop tables stay free of an empty [desktop] header.
    std::fs::write(&path, "[display]\nshow_thinking = true\n").unwrap();
    Config::set_default_model(Some("gpt-test"), None).expect("save");
    assert!(!std::fs::read_to_string(&path).unwrap().contains("[desktop"));

    restore_env_var("FACTR_HOME", prev_home);
    Config::invalidate_cache();
}

#[test]
fn removed_embedding_and_semantic_compaction_keys_still_parse() {
    let cfg: Config = toml::from_str(
        "[compaction]\nmode = \"semantic\"\ntopic_shift_threshold = 0.4\n\
         relevance_keep_threshold = 0.7\ngoal_window_turns = 3\n\
         [agents]\nmemory_embedding_backend = \"openai\"\nmemory_embedding_model = \"x\"\n\
         [keybindings]\nscroll_up = \"ctrl+y\"\nsession_picker_enter = \"new-terminal\"\nopen_resume = \"\"\n\
         [dictation]\ncommand = \"x\"\n",
    )
    .expect("old configs with removed keys must keep parsing");
    assert_eq!(cfg.compaction.mode, crate::config::CompactionMode::Proactive);
}

#[test]
fn removed_terminal_ui_settings_still_parse_and_are_ignored() {
    let cfg: Config = toml::from_str(
        "[display]\ncentered = true\ndiff_mode = \"inline\"\nshow_thinking = false\n\
         [agents]\nswarm_strip_layout = \"horizontal\"\n\
         [launch_hotkeys]\nenabled = true\nimported = true\n\
         [[launch_hotkeys.entries]]\nchord = \"cmd+;\"\ndir = \"$HOME\"\n",
    )
    .expect("old terminal-UI settings must not fail the config parse");
    assert!(!cfg.display.show_thinking);
}
