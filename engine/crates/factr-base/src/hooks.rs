//! Lifecycle hook dispatch and the engine's approval gate.
//!
//! The one user-hook source is Factr's `hooks:` block in `$FACTR_CONFIG_HOME/config.yaml`
//! ([`crate::factr_hooks`]: events mapped onto the engine's lifecycle points, a first-use approval
//! allowlist, per-tool matchers). This module is the dispatcher the engine's call sites use
//! (`dispatch_observer`, `run_pre_tool_gate`, `hook_configured`) and forwards to it. The old factr
//! `[hooks]` table in `config.toml` and the `FACTR_HOOK_*` variables are gone: they skipped the
//! allowlist, so a second system ran commands Factr never approved.
//!
//! The single exception is the engine's own approval gate: the gateway installs an
//! [`ApprovalHook`] once ([`set_approval_hook`]) and every `bash`, `repl` and `factr` call is judged
//! by it, in process, after the Factr hooks (so the person approves the arguments that will
//! actually run). The gate fails closed: with no hook installed those calls are denied.

use serde_json::Value;
use std::sync::{Arc, RwLock};

/// Decision returned by the pre-tool gates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateDecision {
    Allow,
    Block { reason: String },
    /// A Factr `pre_tool_call` hook rewrote the tool's arguments (a JSON object).
    Modify { input_json: String },
}

/// A lifecycle event to deliver to a hook.
#[derive(Debug, Clone)]
pub struct HookEvent {
    /// Event name: "turn_start", "turn_end", "session_start", "session_end",
    /// "post_tool".
    pub event: &'static str,
    pub session_id: Option<String>,
    pub cwd: Option<String>,
    /// Extra env fields. Keys are suffixes: ("STATUS", "ok") becomes
    /// `FACTR_HOOK_STATUS=ok` and `"status": "ok"` in the JSON payload.
    pub fields: Vec<(&'static str, String)>,
}

impl HookEvent {
    pub fn new(event: &'static str) -> Self {
        Self {
            event,
            session_id: None,
            cwd: None,
            fields: Vec::new(),
        }
    }

    pub fn session_id(mut self, session_id: impl Into<String>) -> Self {
        self.session_id = Some(session_id.into());
        self
    }

    pub fn cwd(mut self, cwd: impl Into<String>) -> Self {
        self.cwd = Some(cwd.into());
        self
    }

    pub fn field(mut self, key: &'static str, value: impl Into<String>) -> Self {
        self.fields.push((key, value.into()));
        self
    }
}

/// A tool call the approval hook judges (the arguments are the ones that will actually run).
pub struct ApprovalCall<'a> {
    pub session_id: &'a str,
    pub working_dir: Option<&'a str>,
    pub tool: &'a str,
    pub input: &'a Value,
}

/// The engine's one approval gate, implemented by the gateway over its approval hub.
#[async_trait::async_trait]
pub trait ApprovalHook: Send + Sync {
    /// `Ok` lets the call run; `Err(reason)` denies it (the reason goes back to the model).
    async fn approve(&self, call: &ApprovalCall<'_>) -> Result<(), String>;
}

/// The tools the approval gate judges. Anything else runs without it.
pub const APPROVAL_TOOLS: &[&str] = &["bash", "repl", "factr"];

static APPROVAL_HOOK: RwLock<Option<Arc<dyn ApprovalHook>>> = RwLock::new(None);

/// Install (or, with `None`, remove) the approval hook. Called once by the gateway at startup.
pub fn set_approval_hook(hook: Option<Arc<dyn ApprovalHook>>) {
    *APPROVAL_HOOK.write().unwrap_or_else(|e| e.into_inner()) = hook;
}

/// Whether a hook could fire for `event`. Cheap; used by hot paths to skip payload construction when
/// none is set.
pub fn hook_configured(event: &str) -> bool {
    crate::factr_hooks::configured(event)
}

/// Whether a call to `tool` goes through [`run_pre_tool_gate`]: the approval gate covers
/// [`APPROVAL_TOOLS`], and any tool may carry a Factr `pre_tool_call` hook.
pub fn pre_tool_applies(tool: &str) -> bool {
    APPROVAL_TOOLS.contains(&tool) || crate::factr_hooks::configured("pre_tool")
}

/// Deliver an observer event to the Factr `hooks:` entries aliased onto it (their own threads, their
/// own payload and allowlist). Fire-and-forget: failures are logged, never propagated.
pub fn dispatch_observer(event: HookEvent) {
    crate::factr_hooks::dispatch_observer(&event);
}

/// Run the pre-tool gates for a tool call: the Factr `pre_tool_call` hooks first (a deny wins, a
/// rewrite replaces the arguments), then the approval gate on the arguments that will actually run,
/// so the person approves the rewritten command and not a command that is then changed.
pub async fn run_pre_tool_gate(
    session_id: &str,
    working_dir: Option<&str>,
    tool_name: &str,
    tool_input_json: &str,
) -> GateDecision {
    let input: Value = serde_json::from_str(tool_input_json).unwrap_or(Value::Null);
    let rewritten = match crate::factr_hooks::pre_tool(session_id, working_dir, tool_name, &input).await {
        crate::factr_hooks::Pre::Allow => None,
        crate::factr_hooks::Pre::Block(reason) => return GateDecision::Block { reason },
        crate::factr_hooks::Pre::Modify(args) => Some(args),
    };
    if APPROVAL_TOOLS.contains(&tool_name) {
        let hook = APPROVAL_HOOK.read().unwrap_or_else(|e| e.into_inner()).clone();
        let reason = match hook {
            None => Some(format!(
                "approval unavailable: no approval hub is attached to this engine, so the {tool_name} call was not run"
            )),
            Some(hook) => {
                let effective = rewritten.as_ref().unwrap_or(&input);
                let call = ApprovalCall { session_id, working_dir, tool: tool_name, input: effective };
                hook.approve(&call).await.err()
            }
        };
        if let Some(reason) = reason {
            crate::logging::info(&format!("Approval gate denied tool '{tool_name}' for session {session_id}: {reason}"));
            return GateDecision::Block { reason };
        }
    }
    match rewritten {
        Some(args) => GateDecision::Modify { input_json: args.to_string() },
        None => GateDecision::Allow,
    }
}

#[cfg(test)]
#[allow(clippy::await_holding_lock)]
mod tests {
    use super::*;

    /// Judges by the tool input: refuses any call whose JSON mentions "secret", records what it saw.
    struct Policy(std::sync::Mutex<Vec<(String, String)>>);

    #[async_trait::async_trait]
    impl ApprovalHook for Policy {
        async fn approve(&self, call: &ApprovalCall<'_>) -> Result<(), String> {
            self.0.lock().unwrap().push((call.tool.to_string(), call.input.to_string()));
            if call.input.to_string().contains("secret") { Err("no secrets".into()) } else { Ok(()) }
        }
    }

    fn policy() -> Arc<Policy> {
        Arc::new(Policy(Default::default()))
    }

    #[tokio::test]
    async fn the_gate_fails_closed_without_a_hook_and_only_for_the_gated_tools() {
        let _guard = crate::storage::lock_test_env();
        set_approval_hook(None);
        for tool in APPROVAL_TOOLS {
            match run_pre_tool_gate("s", None, tool, "{}").await {
                GateDecision::Block { reason } => assert!(reason.contains("approval unavailable"), "{reason}"),
                other => panic!("{tool}: {other:?}"),
            }
        }
        assert_eq!(run_pre_tool_gate("s", None, "read", "{}").await, GateDecision::Allow, "other tools are not gated");
        assert!(pre_tool_applies("bash") && pre_tool_applies("repl") && pre_tool_applies("factr"));
    }

    #[tokio::test]
    async fn the_hook_decides_and_its_reason_goes_back() {
        let _guard = crate::storage::lock_test_env();
        let hook = policy();
        set_approval_hook(Some(hook.clone()));
        let blocked = run_pre_tool_gate("s", None, "bash", r#"{"command":"cat secret"}"#).await;
        let allowed = run_pre_tool_gate("s", None, "bash", r#"{"command":"true"}"#).await;
        let skipped = run_pre_tool_gate("s", None, "read", r#"{"file":"secret"}"#).await;
        set_approval_hook(None);
        assert_eq!(blocked, GateDecision::Block { reason: "no secrets".into() });
        assert_eq!(allowed, GateDecision::Allow);
        assert_eq!(skipped, GateDecision::Allow);
        assert_eq!(hook.0.lock().unwrap().len(), 2, "the hook is never asked about an ungated tool");
    }
}
