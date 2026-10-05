//! A process-wide hook so provider crates, which cannot depend on the agent core, can ask whether the
//! session's tool policy allows a tool the provider itself attaches (a hosted tool such as
//! `image_generation`). The agent core installs the hook when it registers a session tool policy.
//! No hook, or no policy for the session, means allowed: default behaviour is unchanged.

use std::sync::OnceLock;

/// `(session_id, tool_name, toolset_group) -> allowed`.
pub type ToolAllowedHook = fn(&str, &str, &str) -> bool;

static HOOK: OnceLock<ToolAllowedHook> = OnceLock::new();

pub fn install_tool_allowed_hook(hook: ToolAllowedHook) {
    let _ = HOOK.set(hook);
}

/// Whether the session's tool policy allows `tool` (also named by its toolset `group`).
/// With no session, no installed hook, or no policy for the session: true.
pub fn tool_allowed(session_id: Option<&str>, tool: &str, group: &str) -> bool {
    match (session_id, HOOK.get()) {
        (Some(session), Some(hook)) => hook(session, tool, group),
        _ => true,
    }
}
