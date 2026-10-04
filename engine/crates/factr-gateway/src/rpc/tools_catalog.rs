//! `tools.list`, `toolsets.list`, `tools.show` and `tools.configure` from the engine's own tools
//! (grouped by the toolset names Factr users know, via `toolsets::tools`), not Factr's Python
//! registry. The on/off state is `platform_toolsets.cli` in config.yaml, the one setting the engine
//! already reads back (`factr_disabled_engine_tools`); absent means every toolset is on.

use super::toolsets;
use serde_json::{Value, json};
use std::path::Path;

/// (toolset, label, description) for the toolsets the engine implements natively.
const TOOLSETS: &[(&str, &str, &str)] = &[
    ("terminal", "Terminal", "Run shell commands and background processes"),
    ("file", "Files", "Read, write, edit and list files"),
    ("web", "Web", "Search the web and fetch pages"),
    ("browser", "Browser", "Drive a web browser"),
    ("skills", "Skills", "Use and manage skills"),
    ("memory", "Memory", "Remember and recall across chats"),
    ("todo", "Task planning", "Keep a task list for the current work"),
    ("delegation", "Delegation", "Hand work to sub-agents"),
    ("session_search", "Session search", "Search past conversations"),
    ("code_execution", "Code execution", "Run Python and batch tool calls"),
    ("clarify", "Clarifying questions", "Ask you a question when blocked"),
    ("cronjob", "Scheduling", "Heartbeats, goals and cron jobs"),
    ("messaging", "Messaging", "Send messages to connected platforms"),
];

fn describe(tool: &str) -> &'static str {
    match tool {
        "bash" => "Run a shell command",
        "bg" => "Manage background tasks",
        "read" => "Read a file",
        "write" => "Write a file",
        "edit" | "replace" => "Edit a file",
        "patch" | "apply_patch" => "Apply a patch",
        "ls" => "List a directory",
        "agentgrep" => "Search code",
        "webfetch" => "Fetch a web page",
        "websearch" => "Search the web",
        "browser" => "Control a browser",
        "skill" => "Use a skill",
        "skill_manage" => "Create and edit skills",
        "memory" => "Save and recall memories",
        "todo" => "Keep a task list",
        "delegate" => "Delegate to a sub-agent",
        "swarm" => "Coordinate several agents",
        "agent_message" => "Message another agent",
        "session_search" => "Search past chats",
        "repl" => "Run Python",
        "batch" => "Run several tool calls at once",
        "clarify" => "Ask the user a question",
        "heartbeat" | "session_goal" | "cronjob_manage" => "Schedule unattended work",
        "send_message" => "Send a message",
        _ => "",
    }
}

fn tools_of(toolset: &str) -> Vec<String> {
    toolsets::tools(&[toolset.to_string()])
}

/// `platform_toolsets.cli`, or `None` when the user never set it (everything on).
fn saved(home: &Path) -> Option<Vec<String>> {
    let list = super::settings::read(home, "platform_toolsets.cli")?;
    Some(list.as_array()?.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
}

fn is_on(saved: &Option<Vec<String>>, name: &str) -> bool {
    saved.as_ref().is_none_or(|l| l.iter().any(|n| n == name))
}

/// One row per engine toolset.
pub(super) fn toolset_rows(home: &Path, with_tools: bool) -> Vec<Value> {
    let saved = saved(home);
    TOOLSETS
        .iter()
        .map(|(name, label, description)| {
            let tools = tools_of(name);
            let mut row = json!({ "name": name, "label": label, "description": description, "tool_count": tools.len(), "enabled": is_on(&saved, name) });
            if with_tools {
                row["tools"] = json!(tools);
            }
            row
        })
        .collect()
}

/// `tools.show`: the tools of the enabled toolsets, by toolset.
pub(super) fn show(home: &Path) -> Value {
    let saved = saved(home);
    let sections: Vec<Value> = TOOLSETS
        .iter()
        .filter(|(name, ..)| is_on(&saved, name))
        .map(|(name, ..)| json!({ "name": name, "tools": tools_of(name).iter().map(|t| json!({ "name": t, "description": describe(t) })).collect::<Vec<_>>() }))
        .collect();
    let total = sections.iter().map(|s| s["tools"].as_array().map_or(0, Vec::len)).sum::<usize>();
    json!({ "sections": sections, "total": total })
}

/// `tools.configure` for plain toolset names; `None` when a name is an MCP `server:tool` target
/// (Factr's MCP config owns those) so the caller can hand the whole call on.
pub(super) fn configure(home: &Path, p: &Value) -> Option<Result<Value, super::RpcError>> {
    let action = p["action"].as_str().unwrap_or("").trim().to_lowercase();
    let names: Vec<String> = p["names"].as_array().map(|l| l.iter().filter_map(|n| n.as_str()).map(|n| n.trim().to_string()).filter(|n| !n.is_empty()).collect()).unwrap_or_default();
    if names.iter().any(|n| n.contains(':')) {
        return None;
    }
    if !matches!(action.as_str(), "enable" | "disable") {
        return Some(Err(super::RpcError { code: 4017, message: format!("unknown tools action: {action}"), data: None }));
    }
    if names.is_empty() {
        return Some(Err(super::RpcError { code: 4018, message: "names required".into(), data: None }));
    }
    let valid = |n: &str| n.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-'));
    let (known, unknown): (Vec<String>, Vec<String>) = names.into_iter().partition(|n| valid(n));
    let mut list = saved(home).unwrap_or_else(|| TOOLSETS.iter().map(|(n, ..)| n.to_string()).collect());
    for n in &known {
        list.retain(|x| x != n);
        if action == "enable" {
            list.push(n.clone());
        }
    }
    list.sort();
    list.dedup();
    if let Err(e) = super::settings::write(home, "platform_toolsets.cli", &json!(list)) {
        return Some(Err(e));
    }
    Some(Ok(json!({ "changed": known, "enabled_toolsets": list, "info": null, "missing_servers": [], "reset": false, "unknown": unknown })))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home() -> std::path::PathBuf {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let d = std::env::temp_dir().join(format!("tools-catalog-{}-{}", std::process::id(), N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn rows_are_engine_tools_all_on_until_configured() {
        let _lock = crate::factr_env::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let h = home();
        let rows = toolset_rows(&h, true);
        let terminal = rows.iter().find(|r| r["name"] == "terminal").unwrap();
        assert_eq!(terminal["tools"], json!(["bash", "bg"]));
        assert!(rows.iter().all(|r| r["enabled"] == true));
        assert!(rows.iter().all(|r| r.get("tools").is_some()) && toolset_rows(&h, false).iter().all(|r| r.get("tools").is_none()));
        assert!(rows.iter().any(|r| r["name"] == "file" && r["tools"].as_array().unwrap().iter().any(|t| t == "edit")));
        assert!(show(&h)["total"].as_u64().unwrap() > 10);
        let _ = std::fs::remove_dir_all(h);
    }

    #[test]
    fn configure_writes_platform_toolsets_and_the_rows_follow() {
        let _lock = crate::factr_env::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let h = home();
        let out = configure(&h, &json!({ "action": "disable", "names": ["terminal", "web"] })).unwrap().unwrap();
        assert_eq!(out["changed"], json!(["terminal", "web"]));
        assert!(!out["enabled_toolsets"].as_array().unwrap().iter().any(|t| t == "terminal"));
        let yaml = std::fs::read_to_string(h.join("config.yaml")).unwrap();
        assert!(yaml.contains("platform_toolsets") && yaml.contains("- file") && !yaml.contains("- terminal"), "{yaml}");
        let rows = toolset_rows(&h, false);
        let on = |n: &str| rows.iter().find(|r| r["name"] == n).unwrap()["enabled"].clone();
        assert_eq!((on("terminal"), on("web"), on("file")), (json!(false), json!(false), json!(true)));
        assert!(show(&h)["sections"].as_array().unwrap().iter().all(|s| s["name"] != "terminal"));
        configure(&h, &json!({ "action": "enable", "names": ["terminal"] })).unwrap().unwrap();
        assert_eq!(toolset_rows(&h, false).iter().find(|r| r["name"] == "terminal").unwrap()["enabled"], true);
        assert!(configure(&h, &json!({ "action": "enable", "names": ["srv:tool"] })).is_none(), "MCP targets go to Factr's MCP config");
        assert_eq!(configure(&h, &json!({ "action": "toggle", "names": ["x"] })).unwrap().unwrap_err().code, 4017);
        assert_eq!(configure(&h, &json!({ "action": "enable", "names": [] })).unwrap().unwrap_err().code, 4018);
        let _ = std::fs::remove_dir_all(h);
    }
}
