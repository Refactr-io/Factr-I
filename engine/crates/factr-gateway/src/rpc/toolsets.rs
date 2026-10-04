//! Factr toolset names (a cron job's `enabled_toolsets` / the cron denylist) as the engine's tool names.

/// The engine's tools behind one Factr toolset. An unknown name passes through as-is, so a
/// job can also name an engine tool or an MCP server directly.
fn tools_of(toolset: &str) -> Vec<&str> {
    match toolset {
        // Loop prevention: anything that re-arms unattended work for later.
        "cronjob" => vec!["heartbeat", "session_goal", "cronjob_manage"],
        "messaging" => vec!["send_message"],
        "terminal" => vec!["bash", "bg"],
        "file" => vec!["read", "write", "edit", "apply_patch", "replace", "ls", "agentgrep"],
        "web" => vec!["webfetch", "websearch"],
        "browser" => vec!["browser"],
        "skills" => vec!["skill_manage"],
        "memory" => vec!["memory"],
        "todo" => vec!["todo"],
        "delegation" => vec!["delegate"],
        "session_search" => vec!["session_search"],
        "code_execution" => vec!["repl", "batch"],
        // Native: pauses the turn and asks the person; an unattended run is told nobody is there.
        "clarify" => vec!["clarify"],
        other => vec![other],
    }
}

/// Toolsets whose tools live only in Factr's Python backend (backend/toolsets.py), reached through the
/// `factr` bridge tool. The bridge matches a tool's own toolset name against the run's policy, so these
/// names pass through `tools_of` as-is; only the tool that carries them, `factr`, has to be allowed too.
/// `cronjob` is also here: it maps to engine tools above and its `cronjob_manage` is a backend tool.
/// Keep sorted; backend/tests/test_engine_bridged_toolsets.py fails when this drifts from the backend.
const BRIDGED_TOOLSETS: &[&str] = &[
    "computer_use", "connections", "cronjob", "desktop_ui", "discord", "discord_admin", "feishu_doc", "feishu_drive",
    "homeassistant", "image_gen", "kanban", "project", "spotify", "tts", "video", "video_gen", "vision", "x_search",
    "yuanbao",
];

/// Whether a toolset needs the `factr` bridge. A name the engine maps itself (terminal, file, ...), an MCP
/// server or an unknown name is not one: an unmapped name never makes `factr` visible.
fn bridged(toolset: &str) -> bool {
    BRIDGED_TOOLSETS.contains(&toolset)
}

/// factr refuses a whole tool policy over one odd name (`no_mcp`-style markers, dots).
fn valid(name: &str) -> bool {
    !name.is_empty() && name.len() <= 128 && name.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-'))
}

pub(super) fn tools(toolsets: &[String]) -> Vec<String> {
    let mut out: Vec<String> = toolsets.iter().flat_map(|t| tools_of(t)).filter(|t| valid(t)).map(str::to_string).collect();
    out.sort();
    out.dedup();
    out
}

/// The `configure_tools` request for a run's policy, or `None` when it names nothing.
pub(super) fn request(session_id: &str, enabled: Option<&[String]>, disabled: &[String]) -> Option<serde_json::Value> {
    if enabled.is_none() && disabled.is_empty() {
        return None;
    }
    let mut tools = serde_json::json!({ "disabled": self::tools(disabled) });
    if let Some(enabled) = enabled {
        // Factr's denylist wins over its allowlist, as in the AIAgent path.
        let blocked = self::tools(disabled);
        let mut allowed: Vec<String> = self::tools(enabled).into_iter().filter(|t| !blocked.contains(t)).collect();
        if enabled.iter().any(|t| bridged(t) && !disabled.contains(t)) {
            allowed.push("factr".into());
        }
        allowed.sort();
        allowed.dedup();
        tools["enabled"] = serde_json::json!(allowed);
    }
    Some(serde_json::json!({ "req": "configure_tools", "session_id": session_id, "tools": tools }))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Listing;

    #[async_trait::async_trait]
    impl factr_base::provider::Provider for Listing {
        async fn complete(
            &self,
            _: &[factr_base::message::Message],
            _: &[factr_base::message::ToolDefinition],
            _: &str,
            _: Option<&str>,
        ) -> anyhow::Result<factr_base::provider::EventStream> {
            Ok(Box::pin(futures_util::stream::empty()))
        }
        fn name(&self) -> &str {
            "listing"
        }
        fn model(&self) -> String {
            "listing".into()
        }
        fn fork(&self) -> std::sync::Arc<dyn factr_base::provider::Provider> {
            std::sync::Arc::new(Listing)
        }
    }

    /// Every engine tool a Factr toolset names is a registered tool; the only others are Factr's own
    /// tools (reached through the `factr` bridge), which the cron denylist also has to block.
    #[tokio::test]
    async fn every_toolset_entry_is_a_registered_tool_or_a_factr_bridge_tool() {
        const FACTR_BACKEND_TOOLS: &[&str] = &["cronjob_manage", "send_message"];
        let _env = crate::factr_env::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let home = std::env::temp_dir().join(format!("toolsets-{}", std::process::id()));
        std::fs::create_dir_all(&home).unwrap();
        // SAFETY: serialized by lock_test_env; restored below.
        let saved = ["FACTR_HOME", "FACTR_REPL_WORKER"].map(|k| (k, std::env::var_os(k)));
        unsafe {
            std::env::set_var("FACTR_HOME", &home);
            std::env::set_var("FACTR_REPL_WORKER", "/bin/factr");
        }
        let registry = factr_app_core::tool::Registry::new(std::sync::Arc::new(Listing)).await;
        for (k, v) in saved {
            unsafe {
                match v {
                    Some(v) => std::env::set_var(k, v),
                    None => std::env::remove_var(k),
                }
            }
        }
        let registered = registry.tool_names().await;
        for toolset in ["cronjob", "messaging", "terminal", "file", "web", "browser", "skills", "memory", "todo", "delegation", "session_search", "code_execution", "clarify"] {
            for tool in tools_of(toolset) {
                assert!(
                    registered.iter().any(|n| n == tool) || FACTR_BACKEND_TOOLS.contains(&tool),
                    "toolset {toolset} names {tool}, which is neither a registered tool nor a Factr tool"
                );
            }
        }
    }

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_cron_denylist_blocks_the_engines_self_scheduling_tools() {
        let req = request("s1", None, &names(&["cronjob", "messaging", "clarify"])).unwrap();
        assert_eq!(req["tools"]["disabled"], serde_json::json!(["clarify", "cronjob_manage", "heartbeat", "send_message", "session_goal"]));
        assert!(req["tools"].get("enabled").is_none(), "no allowlist unless the job set one");
        assert!(request("s1", None, &[]).is_none());
    }

    #[test]
    fn a_per_job_allowlist_maps_toolsets_and_the_denylist_still_wins() {
        let req = request("s1", Some(&names(&["web", "cronjob", "mcp__docs"])), &names(&["cronjob"])).unwrap();
        assert_eq!(req["tools"]["enabled"], serde_json::json!(["mcp__docs", "webfetch", "websearch"]));
    }

    #[test]
    fn factr_only_toolsets_enable_the_bridge_and_the_denylist_reaches_its_tools() {
        let req = request("s1", Some(&names(&["web", "image_gen", "clarify"])), &[]).unwrap();
        assert_eq!(req["tools"]["enabled"], serde_json::json!(["clarify", "factr", "image_gen", "webfetch", "websearch"]));
        let req = request("s1", Some(&names(&["cronjob"])), &names(&["cronjob"])).unwrap();
        assert_eq!(req["tools"]["enabled"], serde_json::json!([]), "a denied bridge toolset does not enable the bridge");
        let req = request("s1", None, &names(&["cronjob", "image_gen"])).unwrap();
        assert_eq!(req["tools"]["disabled"], serde_json::json!(["cronjob_manage", "heartbeat", "image_gen", "session_goal"]));
    }

    #[test]
    fn engine_mapped_toolsets_never_enable_the_bridge() {
        let policy = names(&["terminal", "file", "code_execution", "todo"]);
        let req = request("s1", Some(&policy), &[]).unwrap();
        let allowed: Vec<String> = serde_json::from_value(req["tools"]["enabled"].clone()).unwrap();
        assert!(!allowed.iter().any(|t| t == "factr"), "{allowed:?}");
        let mut expect: Vec<String> = policy.iter().flat_map(|t| tools_of(t)).map(str::to_string).collect();
        expect.sort();
        assert_eq!(allowed, expect, "the allow-list is exactly the engine tools of those toolsets");
        for t in BRIDGED_TOOLSETS {
            assert!(t.eq(&"cronjob") || tools_of(t) == [*t], "{t} is mapped by the engine, so it is not backend-only");
        }
        assert!(BRIDGED_TOOLSETS.windows(2).all(|w| w[0] < w[1]), "keep the list sorted");
        assert!(!bridged("mcp__docs") && !bridged("factr") && !bridged("made_up"));
    }

    #[test]
    fn backend_toolsets_enable_the_bridge() {
        for set in [&["terminal", "image_gen"][..], &["cronjob"][..], &["kanban", "file"][..]] {
            let req = request("s1", Some(&names(set)), &[]).unwrap();
            let allowed: Vec<String> = serde_json::from_value(req["tools"]["enabled"].clone()).unwrap();
            assert!(allowed.iter().any(|t| t == "factr"), "{set:?} -> {allowed:?}");
        }
    }
}
