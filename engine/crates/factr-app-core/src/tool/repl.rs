//! `repl`: factr-learn-style recursive REPL (factr engine only).
//!
//! Large context stays in sandboxed REPL variables instead of the transcript;
//! the model inspects it with code and asks focused sub-questions through
//! `llm_query`. Execution happens in a sandboxed CPython worker process (see
//! the `factr-learn` crate), using Factr's bundled Python runtime.

use super::{Tool, ToolContext, ToolOutput};
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_OUTPUT_CHARS: usize = 8_000;
const SUBQUERY_SYSTEM: &str = "You are a focused sub-agent. Answer the request using only the text it contains. Be concise and exact.";
static PENDING_COMPACTION: OnceLock<StdMutex<HashMap<String, Option<String>>>> = OnceLock::new();

pub(crate) fn take_pending_compaction(session_id: &str) -> Option<Option<String>> {
    PENDING_COMPACTION
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .remove(session_id)
}

fn queue_compaction(session_id: &str, instructions: Option<String>) -> bool {
    let mut pending = PENDING_COMPACTION
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if pending.contains_key(session_id) {
        return false;
    }
    pending.insert(session_id.to_owned(), instructions);
    true
}

fn compaction_pending(session_id: &str) -> bool {
    PENDING_COMPACTION
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .contains_key(session_id)
}

pub struct ReplTool {
    host: Arc<factr_learn::ReplHost>,
}

#[cfg(test)]
mod repl_python_tests {
    use super::repl_python;
    use std::ffi::OsString;
    use std::path::PathBuf;

    fn with<'a>(vars: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
        move |key| vars.iter().find(|(k, _)| *k == key).map(|(_, v)| OsString::from(v))
    }

    #[test]
    fn repl_off_always_disables_even_with_an_interpreter_named() {
        let vars = [("FACTR_REPL", "0"), ("FACTR_REPL_PYTHON", "/usr/bin/python3"), ("FACTR_BACKEND_PYTHON", "/h/python")];
        assert_eq!(repl_python(with(&vars), true, true), None);
    }

    #[test]
    fn the_repl_interpreter_is_factr_repl_python_and_never_the_factr_python() {
        let named = [("FACTR_REPL_PYTHON", "/usr/bin/python3")];
        assert_eq!(repl_python(with(&named), false, false), Some(PathBuf::from("/usr/bin/python3")));
        // Factr's backend interpreter (the old shared name) neither enables the REPL nor picks its python.
        let factr_only = [("FACTR_BACKEND_PYTHON", "/h/python")];
        assert_eq!(repl_python(with(&factr_only), false, false), None);
        assert_ne!(repl_python(with(&factr_only), true, true), Some(PathBuf::from("/h/python")));
    }

    #[test]
    fn without_a_sandbox_the_repl_needs_an_explicit_interpreter() {
        let dir = std::env::temp_dir().join(format!("repl-py-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let py = dir.join("python3");
        std::fs::write(&py, "#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        std::fs::set_permissions(&py, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        let path = dir.to_string_lossy().into_owned();
        let on_path = [("PATH", path.as_str())];
        assert_eq!(repl_python(with(&on_path), true, false), None, "no sandbox: off");
        assert_eq!(repl_python(with(&on_path), true, true), Some(py));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod sub_effort_tests {
    use super::*;
    use crate::provider::{EventStream, Provider};
    use std::sync::Mutex;

    /// A model that records the effort it is asked to run at and echoes it in its one-shot answer.
    struct EffortProbe {
        effort: Mutex<Option<String>>,
        refuses: bool,
    }

    #[async_trait]
    impl Provider for EffortProbe {
        async fn complete(&self, _: &[crate::message::Message], _: &[crate::message::ToolDefinition], _: &str, _: Option<&str>) -> Result<EventStream> {
            Ok(Box::pin(futures::stream::empty()))
        }
        fn name(&self) -> &str {
            "probe"
        }
        fn model(&self) -> String {
            "probe-model".into()
        }
        fn set_model(&self, _: &str) -> Result<()> {
            Ok(())
        }
        fn reasoning_effort(&self) -> Option<String> {
            self.effort.lock().unwrap().clone()
        }
        fn set_reasoning_effort(&self, effort: &str) -> Result<()> {
            if self.refuses {
                anyhow::bail!("not supported by this model");
            }
            *self.effort.lock().unwrap() = Some(effort.to_string());
            Ok(())
        }
        fn fork(&self) -> Arc<dyn Provider> {
            Arc::new(EffortProbe { effort: Mutex::new(self.reasoning_effort()), refuses: self.refuses })
        }
        async fn complete_simple_with_usage(&self, _: &str, _: &str) -> Result<factr_provider_core::SimpleCompletion> {
            Ok(factr_provider_core::SimpleCompletion {
                text: format!("ran at {:?}", self.reasoning_effort()),
                usage: Some(factr_provider_core::SimpleUsage { input: 900, output: 70, cache_read: 512, cache_write: 0 }),
            })
        }
    }

    fn main_thread(effort: &str) -> Arc<EffortProbe> {
        Arc::new(EffortProbe { effort: Mutex::new(Some(effort.into())), refuses: false })
    }

    #[tokio::test]
    async fn a_sub_call_runs_at_the_sub_effort_and_the_main_effort_is_untouched() {
        let main = main_thread("medium");
        let fork = main.fork();
        let (used, pinned, configured) = apply_sub_effort(fork.as_ref(), Ok(Some("low".into())));
        assert_eq!((used.as_deref(), pinned.as_deref(), configured.as_deref()), (Some("low"), Some("low"), Some("low")));
        let reply = sub_call_on(fork, "s", "p", used).await.unwrap();
        assert_eq!(reply.text, "ran at Some(\"low\")");
        assert_eq!(main.reasoning_effort().as_deref(), Some("medium"), "the main agent's effort is unchanged");
        // The usage the provider reported comes back for the classify log.
        assert_eq!((reply.input_tokens, reply.output_tokens, reply.cached_tokens, reply.reasoning_tokens), (Some(900), Some(70), Some(512), None));
        assert_eq!(reply.effort.as_deref(), Some("low"));
    }

    #[test]
    fn an_unknown_refused_or_inherited_effort_falls_back_to_the_main_effort() {
        for setting in [Ok(None), Err("repl sub-call effort 'minimal' is not supported".to_string())] {
            let fork = main_thread("medium").fork();
            assert_eq!(apply_sub_effort(fork.as_ref(), setting), (Some("medium".to_string()), None, None));
        }
        let refusing = EffortProbe { effort: Mutex::new(Some("high".into())), refuses: true };
        assert_eq!(apply_sub_effort(&refusing, Ok(Some("low".into()))), (Some("high".to_string()), None, Some("low".to_string())), "a model that refuses `low` keeps the main effort but still reports what was configured");
    }
}

#[cfg(test)]
mod sub_setting_tests {
    use super::*;
    use crate::provider::{EventStream, Provider};
    use std::sync::Mutex;

    /// Accepts any effort when it is set, but its API answers 400 to `low` (a catalog that overstated it).
    struct ApiRefusesLow(Mutex<Option<String>>);

    #[async_trait]
    impl Provider for ApiRefusesLow {
        async fn complete(&self, _: &[crate::message::Message], _: &[crate::message::ToolDefinition], _: &str, _: Option<&str>) -> Result<EventStream> {
            Ok(Box::pin(futures::stream::empty()))
        }
        fn name(&self) -> &str {
            "refuser"
        }
        fn model(&self) -> String {
            "m".into()
        }
        fn set_model(&self, _: &str) -> Result<()> {
            Ok(())
        }
        fn reasoning_effort(&self) -> Option<String> {
            self.0.lock().unwrap().clone()
        }
        fn set_reasoning_effort(&self, effort: &str) -> Result<()> {
            *self.0.lock().unwrap() = Some(effort.to_string());
            Ok(())
        }
        fn fork(&self) -> Arc<dyn Provider> {
            Arc::new(ApiRefusesLow(Mutex::new(self.reasoning_effort())))
        }
        async fn complete_simple_with_usage(&self, _: &str, _: &str) -> Result<factr_provider_core::SimpleCompletion> {
            if self.reasoning_effort().as_deref() == Some("low") {
                anyhow::bail!("400 Unsupported value: reasoning effort 'low' is not supported with this model");
            }
            Ok(factr_provider_core::SimpleCompletion { text: format!("ok at {:?}", self.reasoning_effort()), usage: None })
        }
    }

    /// Its setter refuses `none` (the catalog lists no such effort for the model).
    struct EffortRefusesSet(Mutex<Option<String>>);

    #[async_trait]
    impl Provider for EffortRefusesSet {
        async fn complete(&self, _: &[crate::message::Message], _: &[crate::message::ToolDefinition], _: &str, _: Option<&str>) -> Result<EventStream> {
            Ok(Box::pin(futures::stream::empty()))
        }
        fn name(&self) -> &str {
            "luna"
        }
        fn model(&self) -> String {
            "m".into()
        }
        fn set_model(&self, _: &str) -> Result<()> {
            Ok(())
        }
        fn reasoning_effort(&self) -> Option<String> {
            self.0.lock().unwrap().clone()
        }
        fn set_reasoning_effort(&self, effort: &str) -> Result<()> {
            if effort == "none" {
                anyhow::bail!("unsupported");
            }
            *self.0.lock().unwrap() = Some(effort.to_string());
            Ok(())
        }
        fn fork(&self) -> Arc<dyn Provider> {
            Arc::new(EffortRefusesSet(Mutex::new(self.reasoning_effort())))
        }
        async fn complete_simple_with_usage(&self, _: &str, _: &str) -> Result<factr_provider_core::SimpleCompletion> {
            Ok(factr_provider_core::SimpleCompletion { text: "ok".into(), usage: None })
        }
    }

    #[test]
    fn classify_and_queries_inherit_by_default_and_each_has_its_own_setting() {
        assert_eq!(factr_learn::host::resolve_sub_effort(None, None, false), Ok(None), "classify inherits the main effort unless the user sets one");
        assert_eq!(factr_learn::host::resolve_sub_effort(Some("low"), None, false), Ok(Some("low".into())));
        assert_eq!(factr_learn::host::resolve_query_effort(None, None, false), Ok(None), "reading and summarising keep the main effort");
        assert_eq!(factr_learn::host::resolve_query_effort(Some("medium"), Some("high"), false), Ok(Some("high".into())));
        assert!(factr_learn::host::resolve_query_effort(None, Some("minimal"), false).is_err());
        assert_eq!(factr_learn::host::resolve_query_effort(None, Some("low"), true), Ok(None));
    }

    #[test]
    fn the_setting_is_resolved_once_per_provider_with_effective_efforts() {
        let main = ApiRefusesLow(Mutex::new(Some("medium".into())));
        let setting = resolve_setting(&main, Ok(Some("low".into())), Ok(None));
        assert_eq!(setting.model, "refuser/m");
        assert_eq!(setting.main_effort.as_deref(), Some("medium"), "kept so a change of the main effort re-resolves an inherited one");
        assert_eq!((setting.classify_pin.as_deref(), setting.classify_effective.as_str()), (Some("low"), "low"));
        assert_eq!((setting.query_pin.as_deref(), setting.query_effective.as_str()), (None, "medium"));
        assert_eq!(setting.pin(SubKind::Classify), Some("low"));
        assert_eq!(setting.effective(SubKind::Query), "medium");
        assert_eq!(main.reasoning_effort().as_deref(), Some("medium"), "probing never touches the session's own effort");
    }

    #[tokio::test]
    async fn a_setter_refused_effort_is_reported_as_requested_not_as_the_inherited_one() {
        // The model refuses `none` when it is set (like gpt-6-luna): the call runs at the main effort, and
        // the log must say it asked for `none`.
        let refusing = EffortRefusesSet(Mutex::new(Some("medium".into())));
        let setting = resolve_setting(&refusing, Ok(Some("none".into())), Ok(None));
        assert_eq!((setting.classify_pin.as_deref(), setting.classify_configured.as_deref(), setting.classify_effective.as_str()), (None, Some("none"), "medium"));
        assert_eq!(setting.query_configured, None);
        let make = || -> Result<Arc<dyn factr_provider_core::Provider>> { Ok(refusing.fork()) };
        let reply = call_with_fallback(make, "s2", "p", setting.pin(SubKind::Classify), setting.configured(SubKind::Classify), setting.effective(SubKind::Classify)).await.unwrap();
        assert_eq!((reply.requested_effort.as_deref(), reply.effort.as_deref()), (Some("none"), Some("medium")));
    }

    #[test]
    fn only_effort_complaints_count_as_an_effort_refusal() {
        assert!(is_effort_refusal("400 Unsupported value: reasoning effort 'low' is not supported", "low"));
        assert!(is_effort_refusal("400 Unsupported value: 'low' is not supported with this model. Supported values are: 'medium'.", "low"), "the value alone, without the word effort");
        assert!(!is_effort_refusal("400 Unsupported value: 'temperature' is not supported with this model.", "low"));
        assert!(!is_effort_refusal("429 too many requests", "low"));
        assert!(!is_effort_refusal("400 prompt is too long", "low"));
    }

    #[tokio::test]
    async fn an_api_refusal_of_the_pinned_effort_falls_back_to_the_main_effort_for_that_call() {
        let main = ApiRefusesLow(Mutex::new(Some("medium".into())));
        let make = || -> Result<Arc<dyn factr_provider_core::Provider>> { Ok(main.fork()) };
        let reply = call_with_fallback(make, "fallback-session", "p", Some("low"), Some("low"), "low").await.unwrap();
        assert_eq!(reply.text, "ok at Some(\"medium\")");
        assert_eq!(reply.effort.as_deref(), Some("medium"), "the log shows the effort the call really ran at");
        assert_eq!((reply.requested_effort.as_deref(), reply.refused_ms.is_some()), (Some("low"), true), "the refused attempt is reported");
        // Memoised: the session's resolved setting no longer pins the refused effort, so later calls go once.
        assert!(effort_refused("fallback-session", "refuser/m", "low"));
        let mut setting = resolve_setting(&main, Ok(Some("low".into())), Ok(None));
        forget_refused_pins(&mut setting, "fallback-session");
        assert_eq!((setting.classify_pin.as_deref(), setting.classify_effective.as_str()), (None, "medium"));
        let reply = call_with_fallback(make, "fallback-session", "p", setting.pin(SubKind::Classify), setting.configured(SubKind::Classify), setting.effective(SubKind::Classify)).await.unwrap();
        assert_eq!((reply.refused_ms, reply.effort.as_deref()), (None, Some("medium")), "the second call is sent once");
        // Another session is not affected.
        let mut other = resolve_setting(&main, Ok(Some("low".into())), Ok(None));
        forget_refused_pins(&mut other, "another-session");
        assert_eq!(other.classify_pin.as_deref(), Some("low"));
        // Unpinned calls and other errors are not touched by the fallback.
        let reply = call_with_fallback(make, "s", "p", None, None, "medium").await.unwrap();
        assert_eq!(reply.text, "ok at Some(\"medium\")");
    }
}

#[cfg(test)]
mod spawn_policy_tests {
    use super::*;
    use std::collections::HashSet;

    fn ctx(session: &str) -> ToolContext {
        ToolContext {
            session_id: session.to_string(),
            message_id: "m".into(),
            tool_call_id: "t".into(),
            working_dir: None,
            stdin_request_tx: None,
            graceful_shutdown_signal: None,
            execution_mode: super::super::ToolExecutionMode::Direct,
        }
    }

    fn set(session: &str, allowed: &[&str], disabled: &[&str]) {
        let to_set = |l: &[&str]| l.iter().map(|s| s.to_string()).collect::<HashSet<_>>();
        super::super::set_session_tool_policy(session, Some(to_set(allowed)), to_set(disabled));
    }

    const OP: &str = r#"{"prompt":"x","label":"w"}"#;
    const AWAIT: &str = r#"{"action":"await","target":"nobody","timeout":1}"#;

    #[tokio::test]
    async fn spawn_subagent_is_refused_without_the_delegate_tool() {
        let s = "repl-spawn-policy-denied";
        set(s, &["repl", "bash"], &[]);
        for op in [OP, AWAIT] {
            let err = spawn_subagent_host(op.into(), ctx(s)).await.unwrap_err().to_string();
            assert!(err.contains("not available in this run") && err.contains("delegate"), "{err}");
        }
        set(s, &["repl", "delegate"], &["delegate"]);
        assert!(spawn_subagent_host(OP.into(), ctx(s)).await.unwrap_err().to_string().contains("not available in this run"));
        super::super::clear_session_tool_policy(s);
    }

    #[tokio::test]
    async fn spawn_subagent_passes_the_gate_when_delegate_is_allowed_or_no_policy() {
        let s = "repl-spawn-policy-allowed";
        for policy in [Some(&["repl", "delegate"][..]), None] {
            match policy {
                Some(a) => set(s, a, &[]),
                None => super::super::clear_session_tool_policy(s),
            }
            // Past the gate the call fails for a different reason (no such child), not the policy.
            let err = spawn_subagent_host(AWAIT.into(), ctx(s)).await.unwrap_err().to_string();
            assert!(!err.contains("not available in this run"), "{err}");
        }
        super::super::clear_session_tool_policy(s);
    }
}

#[cfg(test)]
mod compaction_request_tests {
    use super::{compaction_pending, queue_compaction, take_pending_compaction};

    #[test]
    fn compact_intent_is_session_scoped_and_consumed_once() {
        let session = format!("compact-test-{}", std::process::id());
        assert!(!compaction_pending(&session));
        assert!(queue_compaction(&session, Some("keep decisions".into())));
        assert!(!queue_compaction(&session, None));
        assert!(compaction_pending(&session));
        assert_eq!(
            take_pending_compaction(&session),
            Some(Some("keep decisions".into()))
        );
        assert!(!compaction_pending(&session));
        assert_eq!(take_pending_compaction(&session), None);
    }
}

/// REPL `spawn_subagent` / `await_subagent`: the REPL must not widen the run's tool policy, so
/// spawning is refused unless the session's tool policy allows the `delegate` tool.
async fn spawn_subagent_host(op_json: String, context: ToolContext) -> Result<String> {
    anyhow::ensure!(
        super::session_tool_allows(&context.session_id, "delegate", "delegation"),
        "spawn_subagent is not available in this run: the `delegate` tool is not allowed by the run's tool policy"
    );
    let op: Value = serde_json::from_str(&op_json).unwrap_or_default();
    let prompt = op["prompt"].as_str().unwrap_or_default();
    let label = op["label"].as_str().unwrap_or("worker");
    if op["action"].as_str() == Some("await") {
        let target = op["target"].as_str().unwrap_or_default();
        let timeout = op["timeout"].as_u64().unwrap_or(20);
        let result = super::communicate::learn_await_subagent(
            &context.session_id,
            target,
            timeout,
        )
        .await?;
        return Ok(result.to_string());
    }
    let output = super::delegate::DelegateTool::new()
        .execute(
            json!({"action":"spawn","prompt":prompt,"label":label}),
            context,
        )
        .await?;
    let session_id = output
        .output
        .split_once("Spawned new agent:")
        .map(|(_, id)| id.trim())
        .filter(|id| !id.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!("delegate did not return a spawned session id")
        })?;
    Ok(json!({ "session_id": session_id, "status": "spawned" }).to_string())
}

static HOST: OnceLock<Option<Arc<factr_learn::ReplHost>>> = OnceLock::new();

pub async fn stop_session(session_id: &str) {
    if let Some(Some(host)) = HOST.get() {
        host.stop_session(session_id).await;
    }
}

/// The interpreter the REPL runs on, or `None` when it is off. `FACTR_REPL=0`
/// always disables it. `FACTR_REPL_PYTHON` names the interpreter (it also switches the REPL on over
/// the `agents.repl` setting, and is the only way to enable it where there is no sandbox); otherwise
/// the shared environment's `python3` (`factr_base::python_env`) when `agents.repl` allows and the
/// macOS sandbox exists. `FACTR_BACKEND_PYTHON` (Factr's backend) is not an interpreter for the REPL.
fn repl_python(
    env: impl Fn(&str) -> Option<std::ffi::OsString>,
    repl_enabled: bool,
    sandboxed: bool,
) -> Option<PathBuf> {
    if env("FACTR_REPL").is_some_and(|v| v == "0") {
        return None;
    }
    let explicit = env("FACTR_REPL_PYTHON").filter(|p| !p.is_empty());
    if explicit.is_none() && !(repl_enabled && sandboxed) {
        return None;
    }
    factr_base::python_env::interpreter_from(explicit, env("PATH"))
}

/// The REPL's `refine(...)` host call. With learning off (`FACTR_LEARNING_ENABLED=0` or the setting)
/// a `run` schedules nothing, so a benchmark run never gets a review the model asked for.
fn refine_host(store: &factr_learn::entries::EntryStore, session: &str, op: &Value) -> Result<String> {
    match op["op"].as_str().unwrap_or("run") {
        "status" => Ok(json!({ "pending": store.refine_pending(session)? }).to_string()),
        _ if !store.learning_enabled() => Ok(json!({ "scheduled": false, "reason": "disabled" }).to_string()),
        _ => {
            store.schedule_refine(session, op["instructions"].as_str(), op["global"].as_bool().unwrap_or(false))?;
            Ok(json!({ "scheduled": true }).to_string())
        }
    }
}

impl ReplTool {
    /// `None` when the REPL is off or no interpreter is available.
    pub fn from_env() -> Option<Self> {
        let host = HOST.get_or_init(|| {
            let python = repl_python(|key| std::env::var_os(key), crate::config::config().agents.repl, factr_learn::host::sandbox_available())?;
            // Without an explicit interpreter the worker runs on the session environment, which may
            // still be building now: each worker start asks for the interpreter again (after the gate).
            let resolver: Option<fn() -> Option<PathBuf>> = std::env::var_os("FACTR_REPL_PYTHON")
                .filter(|p| !p.is_empty())
                .is_none()
                .then_some(factr_base::python_env::interpreter as fn() -> Option<PathBuf>);
            python
                .is_file()
                .then(|| factr_learn::ReplHost::new_resolving(python, resolver))
        });
        host.clone().map(|host| Self { host })
    }
}

/// The REPL worker's resident-memory cap. The worker persists between cells, so it gets its own
/// cap rather than a build's: the largest single `load` slice held as a Python `str` at its widest
/// code-point width (PEP 393 stores up to 4 bytes per character), so one full load always fits and
/// an ASCII load of that size leaves room for three working copies. A configured
/// `terminal.max_memory_mb` can lower it, never raise it.
fn worker_memory_cap() -> u64 {
    const WIDEST_CODE_POINT_BYTES: u64 = 4;
    let derived = factr_learn::host::MAX_LOAD_BYTES * WIDEST_CODE_POINT_BYTES;
    super::memory_cap::configured_bytes().map_or(derived, |configured| configured.min(derived))
}

fn clip(text: &str) -> String {
    match text.char_indices().nth(MAX_OUTPUT_CHARS) {
        Some((cut, _)) => format!(
            "{}\n… [output truncated at {MAX_OUTPUT_CHARS} chars; page it, e.g. print(s[{MAX_OUTPUT_CHARS}:{}])]",
            &text[..cut],
            MAX_OUTPUT_CHARS * 2
        ),
        None => text.to_string(),
    }
}

/// A call to an async helper without `await` that the worker could not await for the model (one
/// nested in an expression or a function) leaves a coroutine behind. Python reports that in exactly
/// these forms: a `'coroutine' object` error on use, a printed `<coroutine object ...>`, or the
/// `coroutine '...' was never awaited` warning; output that merely says "coroutine" is not one.
fn mentions_coroutine(text: &str) -> bool {
    text.contains("'coroutine' object") || text.contains("<coroutine object") || text.contains("was never awaited")
}

/// An actionable tail for the common REPL failures: un-awaited helpers, an undefined name (list the
/// helpers that exist), a missing module (the installers that exist, then the standard library) and the
/// compute-limit interrupt (run a script through bash).
fn repl_error_hint(text: &str) -> String {
    let mut hint = String::new();
    if mentions_coroutine(text) {
        hint.push_str("\nHint: helpers are async: use `await load(...)`, `await llm_query(...)`.");
    }
    if text.contains("NameError") {
        hint.push_str(&format!("\nHint: the REPL helpers are: {}.", factr_learn::helper_names().join(", ")));
    }
    if text.contains("No module named") || text.contains("ModuleNotFoundError") {
        let python = factr_base::python_env::interpreter();
        hint.push_str(&factr_base::shell::missing_module_hint(
            factr_base::shell::package_routes(),
            python.as_deref().and_then(|p| p.to_str()),
        ));
    }
    if text.contains("of compute and") {
        hint.push_str("\nHint: run a longer job as a script through bash (python3 script.py), or process it in smaller slices.");
    }
    hint
}

/// The end of a traceback, where the exception line is, within half the output budget.
fn tail(error: &str) -> &str {
    let keep = MAX_OUTPUT_CHARS / 2;
    match error.char_indices().rev().nth(keep) {
        Some((cut, _)) => &error[cut..],
        None => error,
    }
}

/// Which REPL sub-call: `classify` (the main agent's effort unless the user pins one) or `llm_query` / `llm_query_batch`
/// (reading and summarising: the main agent's effort unless configured).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SubKind {
    Classify,
    Query,
}

fn sub_effort_setting(kind: SubKind) -> Result<Option<String>, String> {
    let legacy = factr_learn::host::cost_legacy();
    let agents = &crate::config::config().agents;
    match kind {
        SubKind::Classify => factr_learn::host::resolve_sub_effort(
            agents.repl_sub_effort.as_deref(),
            std::env::var("FACTR_REPL_SUB_EFFORT").ok().as_deref(),
            legacy,
        ),
        SubKind::Query => factr_learn::host::resolve_query_effort(
            agents.repl_query_effort.as_deref(),
            std::env::var("FACTR_REPL_QUERY_EFFORT").ok().as_deref(),
            legacy,
        ),
    }
}

/// Pin `provider` (a fork: its own state) to the sub-call effort. Returns (the effort the call runs with,
/// the effort pinned, the effort the user configured, if any). Unpinned (`None`) when the setting is
/// `inherit`, unknown, or refused by the model (a warning, never fatal): the call then runs at whatever the
/// fork inherited, and the configured value is still reported so the log shows the refusal.
fn apply_sub_effort(provider: &dyn factr_provider_core::Provider, setting: Result<Option<String>, String>) -> (Option<String>, Option<String>, Option<String>) {
    match setting {
        Ok(Some(effort)) => match provider.set_reasoning_effort(&effort) {
            Ok(()) => return (Some(effort.clone()), Some(effort.clone()), Some(effort)),
            Err(error) => {
                crate::logging::warn(&format!("repl sub-call effort {effort} refused: unsupported by model ({error}); using the main agent's effort"));
                return (provider.reasoning_effort(), None, Some(effort));
            }
        },
        Ok(None) => {}
        Err(message) => crate::logging::warn(&message),
    }
    (provider.reasoning_effort(), None, None)
}

/// What a session's sub-calls run on, resolved once: the model id and, per kind, the effort pinned (`None`:
/// inherit) and the effective effort (what the request carries; empty when unknown).
#[derive(Clone, Debug, Default, PartialEq)]
struct SubSetting {
    model: String,
    /// The main agent's effort when this was resolved: an inherited effort follows it.
    main_effort: Option<String>,
    classify_pin: Option<String>,
    classify_effective: String,
    /// What the user configured (`None`: inherit), kept even when the model refused it.
    classify_configured: Option<String>,
    query_pin: Option<String>,
    query_effective: String,
    query_configured: Option<String>,
}

impl SubSetting {
    fn pin(&self, kind: SubKind) -> Option<&str> {
        match kind {
            SubKind::Classify => self.classify_pin.as_deref(),
            SubKind::Query => self.query_pin.as_deref(),
        }
    }
    fn configured(&self, kind: SubKind) -> Option<&str> {
        match kind {
            SubKind::Classify => self.classify_configured.as_deref(),
            SubKind::Query => self.query_configured.as_deref(),
        }
    }
    fn effective(&self, kind: SubKind) -> &str {
        match kind {
            SubKind::Classify => &self.classify_effective,
            SubKind::Query => &self.query_effective,
        }
    }
}

/// A fork of the session's provider on the REPL sub-model (never changes the session's own model or effort).
fn sub_fork(session_id: &str) -> Result<Arc<dyn factr_provider_core::Provider>> {
    let provider = crate::provider::session_provider_fork(session_id).context("no active model provider")?;
    let chosen = factr_base::factr_config::aux_model(factr_base::factr_config::AuxConsumer::ReplSub)
        .or_else(|| crate::config::config().agents.repl_sub_model.clone());
    if let Some(sub) = chosen.as_deref() {
        if let Err(error) = provider.set_model(sub) {
            crate::logging::warn(&format!("repl_sub_model {sub}: {error}"));
        }
    }
    Ok(provider)
}

fn resolve_setting(provider: &dyn factr_provider_core::Provider, classify: Result<Option<String>, String>, query: Result<Option<String>, String>) -> SubSetting {
    // One probe fork per kind: a refusal or fallback is decided, and logged, here, once.
    let mut out = SubSetting { model: format!("{}/{}", provider.name(), provider.model()), main_effort: provider.reasoning_effort(), ..Default::default() };
    for (kind, setting) in [(SubKind::Classify, classify), (SubKind::Query, query)] {
        let probe = provider.fork();
        let (effective, pinned, configured) = apply_sub_effort(probe.as_ref(), setting);
        let text = effective.unwrap_or_default();
        match kind {
            SubKind::Classify => (out.classify_pin, out.classify_effective, out.classify_configured) = (pinned, text, configured),
            SubKind::Query => (out.query_pin, out.query_effective, out.query_configured) = (pinned, text, configured),
        }
    }
    out
}

static SUB_SETTINGS: OnceLock<StdMutex<HashMap<String, Arc<SubSetting>>>> = OnceLock::new();

/// The session's sub-call setting, resolved on its first sub-call (or cell) and kept; resolved again only
/// if the sub-model or the main agent's effort changes (an inherited effort, the cache key and the log
/// follow the effort the requests really carry).
fn session_sub_setting(session_id: &str) -> Arc<SubSetting> {
    let Ok(provider) = sub_fork(session_id) else { return Arc::default() };
    let model = format!("{}/{}", provider.name(), provider.model());
    let main_effort = provider.reasoning_effort();
    let mut map = SUB_SETTINGS.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    if let Some(known) = map.get(session_id).filter(|s| s.model == model && s.main_effort == main_effort) {
        return known.clone();
    }
    let mut setting = resolve_setting(
        provider.as_ref(),
        sub_effort_setting(SubKind::Classify),
        sub_effort_setting(SubKind::Query),
    );
    forget_refused_pins(&mut setting, session_id);
    note_pinned_once(session_id, &setting);
    let setting = Arc::new(setting);
    map.insert(session_id.to_string(), setting.clone());
    setting
}

/// Logged once per session: a classify effort the user configured is in force (never a silent default).
fn note_pinned_once(session_id: &str, setting: &SubSetting) {
    static SEEN: OnceLock<StdMutex<std::collections::HashSet<String>>> = OnceLock::new();
    let Some(configured) = setting.classify_configured.as_deref() else { return };
    if SEEN.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner()).insert(session_id.to_string()) {
        crate::logging::info(&format!("repl classify effort pinned to {configured} by FACTR_REPL_SUB_EFFORT / agents.repl_sub_effort (effective: {})", setting.classify_effective));
    }
}

/// Efforts the API refused, per (session, sub-model): never pinned again in that session.
static REFUSED_EFFORTS: OnceLock<StdMutex<std::collections::HashSet<(String, String, String)>>> = OnceLock::new();

fn effort_refused(session_id: &str, model: &str, effort: &str) -> bool {
    REFUSED_EFFORTS.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner()).contains(&(session_id.into(), model.into(), effort.into()))
}

/// Remember that the API refused `effort` for this session's sub-model and drop the resolved setting, so every
/// later sub-call goes straight to the main agent's effort (one refused request per session, not one per call).
fn note_effort_refused(session_id: &str, model: &str, effort: &str) {
    REFUSED_EFFORTS.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner()).insert((session_id.into(), model.into(), effort.into()));
    if let Some(map) = SUB_SETTINGS.get() {
        map.lock().unwrap_or_else(|e| e.into_inner()).remove(session_id);
    }
}

/// A pin the API refused before is dropped: that kind inherits the main agent's effort.
fn forget_refused_pins(setting: &mut SubSetting, session_id: &str) {
    let main = setting.main_effort.clone().unwrap_or_default();
    if setting.classify_pin.as_deref().is_some_and(|e| effort_refused(session_id, &setting.model, e)) {
        (setting.classify_pin, setting.classify_effective) = (None, main.clone());
    }
    if setting.query_pin.as_deref().is_some_and(|e| effort_refused(session_id, &setting.model, e)) {
        (setting.query_pin, setting.query_effective) = (None, main);
    }
}

/// A provider error that says the pinned effort was refused (not a transport problem): it names the effort,
/// reasoning, or the pinned value itself (`'low'`), and says it is not supported.
fn is_effort_refusal(error: &str, pin: &str) -> bool {
    let text = error.to_ascii_lowercase();
    let names_it = text.contains("effort") || text.contains("reasoning") || text.contains(&format!("'{}'", pin.to_ascii_lowercase()));
    names_it && ["unsupported", "not supported", "invalid", "not available", "does not support"].iter().any(|w| text.contains(w))
}

/// One REPL sub-model call on a fork of the session's provider at the session's sub-call effort. If the
/// API refuses the pinned effort, that call (and a logged-once note) falls back to the main agent's effort.
async fn sub_call(session_id: &str, prompt: &str, kind: SubKind) -> Result<factr_learn::host::SubReply> {
    let setting = session_sub_setting(session_id);
    call_with_fallback(|| sub_fork(session_id), session_id, prompt, setting.pin(kind), setting.configured(kind), setting.effective(kind)).await
}

async fn call_with_fallback(
    make_fork: impl Fn() -> Result<Arc<dyn factr_provider_core::Provider>>,
    session_id: &str,
    prompt: &str,
    pin: Option<&str>,
    configured: Option<&str>,
    effective: &str,
) -> Result<factr_learn::host::SubReply> {
    let provider = make_fork()?;
    let model = format!("{}/{}", provider.name(), provider.model());
    if let Some(effort) = pin {
        let _ = provider.set_reasoning_effort(effort);
    }
    let shown = (!effective.is_empty()).then(|| effective.to_string());
    let clock = std::time::Instant::now();
    match sub_call_on(provider, session_id, prompt, shown).await {
        Err(error) if pin.is_some_and(|p| is_effort_refusal(&error.to_string(), p)) => {
            let refused_ms = clock.elapsed().as_millis() as u64;
            let pinned = pin.unwrap_or_default();
            if !effort_refused(session_id, &model, pinned) {
                crate::logging::warn(&format!("the API refused the sub-call effort {pinned}: {error}; this session's sub-calls use the main agent's effort"));
            }
            note_effort_refused(session_id, &model, pinned);
            let main = make_fork()?;
            let inherited = main.reasoning_effort();
            let mut reply = sub_call_on(main, session_id, prompt, inherited).await?;
            // The refused request happened: the classify log gets a row for it (its latency, the refusal).
            reply.requested_effort = Some(pinned.to_string());
            reply.refused_ms = Some(refused_ms);
            Ok(reply)
        }
        Ok(mut reply) => {
            // The configured value (even one the model refused when the setting was resolved), else what ran.
            reply.requested_effort = configured.or(pin).map(str::to_string).or_else(|| reply.effort.clone());
            Ok(reply)
        }
        other => other,
    }
}

async fn sub_call_on(
    provider: Arc<dyn factr_provider_core::Provider>,
    session_id: &str,
    prompt: &str,
    effort: Option<String>,
) -> Result<factr_learn::host::SubReply> {
    let provider_name = provider.name().to_string();
    let model = provider.model();
    let started = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as i64;
    let clock = std::time::Instant::now();
    let result = provider.complete_simple_with_usage(prompt, SUBQUERY_SYSTEM).await;
    let latency_ms = clock.elapsed().as_millis() as u64;
    match &result {
        Ok(reply) => super::report_aux_model_call("REPL subquery", session_id, provider_name, model, started, reply.usage, None),
        Err(error) => super::report_aux_model_call("REPL subquery", session_id, provider_name, model, started, None, Some(&error.to_string())),
    }
    result.map(|reply| factr_learn::host::SubReply {
        text: reply.text,
        input_tokens: reply.usage.map(|u| u.input),
        output_tokens: reply.usage.map(|u| u.output),
        cached_tokens: reply.usage.map(|u| u.cache_read),
        // The providers do not report reasoning tokens separately (the stream's usage event has none).
        reasoning_tokens: None,
        latency_ms,
        effort,
        ..Default::default()
    })
}

#[async_trait]
impl Tool for ReplTool {
    fn name(&self) -> &str {
        "repl"
    }

    fn description(&self) -> &str {
        factr_learn::tool_description()
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "intent": super::intent_schema_property(),
                "code": { "type": "string", "minLength": 1, "description": "Python to run in the persistent REPL" }
            },
            "required": ["code"]
        })
    }

    async fn execute(&self, input: Value, ctx: ToolContext) -> Result<ToolOutput> {
        // Empty or blank code never gets here: the registry rejects it from the schema's `minLength`.
        let code = input["code"].as_str().context("`code` is required")?;
        // Before the cell's compute clock starts: the worker runs on the session environment.
        super::bash::await_session_venv().await;
        let session_id = ctx.session_id.clone();
        let sub_setting = session_sub_setting(&ctx.session_id);
        let llm_query: factr_learn::LlmQuery = {
            let session_id = session_id.clone();
            Arc::new(move |prompt: String| {
                let session_id = session_id.clone();
                Box::pin(async move { sub_call(&session_id, &prompt, SubKind::Query).await.map(|reply| reply.text) })
            })
        };
        let llm_query_meta: factr_learn::host::LlmQueryMeta = {
            let session_id = session_id.clone();
            Arc::new(move |prompt: String| {
                let session_id = session_id.clone();
                Box::pin(async move { sub_call(&session_id, &prompt, SubKind::Classify).await })
            })
        };
        let session_id = ctx.session_id.clone();
        let session_for_host = ctx.session_id.clone();
        let goal: factr_learn::host::HostFn = Arc::new(move |op_json: String| {
            let session_id = session_for_host.clone();
            Box::pin(async move {
                let home = factr_base::storage::factr_dir()?;
                let store = factr_learn::agent_loop::ControlStore::open_cached(&home)?;
                factr_learn::agent_loop_host::goal_host(&store, &session_id, &op_json)
            })
        });
        let session_for_hb = ctx.session_id.clone();
        let heartbeat: factr_learn::host::HostFn = Arc::new(move |op_json: String| {
            let session_id = session_for_hb.clone();
            Box::pin(async move {
                let home = factr_base::storage::factr_dir()?;
                let store = factr_learn::agent_loop::ControlStore::open_cached(&home)?;
                factr_learn::agent_loop_host::heartbeat_host(&store, &session_id, &op_json)
            })
        });
        let refine: factr_learn::host::Refine = Arc::new(move |op_json: String| {
            let session_id = session_id.clone();
            Box::pin(async move {
                let op: Value = serde_json::from_str(&op_json).unwrap_or_default();
                let home = factr_base::storage::factr_dir()?;
                let store = factr_learn::entries::EntryStore::open_cached(&home)?;
                refine_host(&store, &session_id, &op)
            })
        });
        let extra = factr_learn::host::ExtraHostFns {
            llm_query_meta: Some(llm_query_meta),
            sub_effort: sub_setting.effective(SubKind::Classify).to_string(),
            sub_requested: sub_setting.configured(SubKind::Classify).unwrap_or_default().to_string(),
            sub_model: sub_setting.model.clone(),
            goal,
            heartbeat,
            spawn_subagent: {
                let context = ctx.clone();
                Arc::new(move |op_json| {
                    let context = context.clone();
                    Box::pin(spawn_subagent_host(op_json, context))
                })
            },
            agent_message: {
                let context = ctx.clone();
                Arc::new(move |op_json| {
                    let context = context.clone();
                    Box::pin(async move {
                        let op: Value = serde_json::from_str(&op_json).unwrap_or_default();
                        if let Some(operation) = op["host_request"].as_str() {
                            let result = super::communicate::learn_agent_host_request(
                                &context.session_id,
                                operation,
                                &op["payload"],
                            )
                            .await?;
                            return Ok(result.to_string());
                        }
                        let action = op["action"].as_str().unwrap_or("list");
                        let mut request = json!({"action":action});
                        match action {
                            "send" => {
                                request["action"] = json!("message");
                                request["prompt"] = op["message"].clone();
                                request["target_session"] = op["target"].clone();
                            }
                            "read" => {
                                request["target_session"] = op["target"].clone();
                            }
                            "list" => {}
                            other => anyhow::bail!("unknown agent_message action: {other}"),
                        }
                        let output = super::delegate::DelegateTool::new()
                            .execute(request, context)
                            .await?;
                        Ok(output.output)
                    })
                })
            },
            websearch: {
                let context = ctx.clone();
                Arc::new(move |op_json| {
                    let context = context.clone();
                    Box::pin(async move {
                        let op: Value = serde_json::from_str(&op_json).unwrap_or_default();
                        let output = super::websearch::WebSearchTool::new()
                            .execute(
                                json!({
                                    "query": op["query"],
                                    "num_results": op["num_results"],
                                    "engine": "duckduckgo"
                                }),
                                context,
                            )
                            .await?;
                        Ok(output.output)
                    })
                })
            },
            compact: {
                let session_id = ctx.session_id.clone();
                Arc::new(move |op_json| {
                    let session_id = session_id.clone();
                    Box::pin(async move {
                        let op: serde_json::Value = serde_json::from_str(&op_json)?;
                        match op["op"].as_str().unwrap_or_default() {
                            "status" => {
                                Ok(json!({ "scheduled": compaction_pending(&session_id) })
                                    .to_string())
                            }
                            "run" => {
                                let instructions = op["instructions"].as_str().map(str::to_owned);
                                let scheduled = queue_compaction(&session_id, instructions);
                                Ok(json!({ "scheduled": scheduled, "phase": "end_of_turn" })
                                    .to_string())
                            }
                            other => anyhow::bail!("unsupported compact operation: {other}"),
                        }
                    })
                })
            },
            skill: Arc::new(move |input| {
                Box::pin(async move {
                    let input: Value = serde_json::from_str(&input)?;
                    let path = super::skill::create_skill_for_repl(input).await?;
                    let src = path.join("src");
                    Ok(json!({ "path": path, "src": src }).to_string())
                })
            }),
            ..factr_learn::host::ExtraHostFns::default()
        };
        // Approval is the registry's job: the gateway's hook asks before a cell reaches here.
        let out = self
            .host
            .run(
                &ctx.session_id,
                code,
                ctx.working_dir.as_deref(),
                llm_query,
                refine,
                extra,
                worker_memory_cap(),
            )
            .await?;
        let mut text = String::new();
        if out.fresh_state {
            text.push_str("[new REPL state]\n");
        }
        if !out.stdout.is_empty() {
            text.push_str(&out.stdout);
            if !out.stdout.ends_with('\n') {
                text.push('\n');
            }
        }
        if let Some(value) = &out.value {
            text.push_str(&format!("=> {value}\n"));
        }
        // A Python exception is a failure like any tool's: the repeat guard and stop nudge see it.
        if let Some(error) = &out.error {
            let hint = repl_error_hint(&format!("{text}{error}"));
            anyhow::bail!("{}{}{hint}", clip(&text), tail(error));
        }
        if text.is_empty() {
            text.push_str("(no output)");
        }
        Ok(ToolOutput::new(clip(&text)).with_metadata(json!({ "host_calls": out.host_calls })))
    }
}

#[cfg(test)]
mod cell_tests {
    use super::*;

    #[test]
    fn repl_errors_get_actionable_hints() {
        assert!(repl_error_hint("NameError: name 'x' is not defined").contains("llm_query_batch"));
        let m = repl_error_hint("ModuleNotFoundError: No module named 'foo'");
        assert!(m.contains("standard library") && m.contains("Hint"), "{m}");
        assert!(repl_error_hint("the cell exceeded 20s of compute and was interrupted").contains("script through bash"));
        assert!(repl_error_hint("ValueError: bad").is_empty());
    }

    #[test]
    fn refine_run_schedules_nothing_when_learning_is_off() {
        let _env = crate::storage::lock_test_env();
        let home = std::env::temp_dir().join(format!("repl-refine-{}", std::process::id()));
        std::fs::create_dir_all(&home).unwrap();
        let store = factr_learn::entries::EntryStore::open_cached(&home).unwrap();
        let prev = std::env::var_os("FACTR_LEARNING_ENABLED");
        unsafe { std::env::set_var("FACTR_LEARNING_ENABLED", "0") };
        let off = refine_host(&store, "s1", &json!({"op": "run", "instructions": "x"})).unwrap();
        let pending_off = store.refine_pending("s1").unwrap();
        unsafe { std::env::remove_var("FACTR_LEARNING_ENABLED") };
        let on = refine_host(&store, "s1", &json!({"op": "run", "instructions": "x"})).unwrap();
        match prev {
            Some(v) => unsafe { std::env::set_var("FACTR_LEARNING_ENABLED", v) },
            None => {}
        }
        assert_eq!(off, r#"{"reason":"disabled","scheduled":false}"#);
        assert!(!pending_off, "nothing may be queued while learning is off");
        assert!(on.contains("\"scheduled\":true"), "{on}");
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn coroutine_errors_get_the_await_hint_and_tracebacks_keep_their_end() {
        assert!(mentions_coroutine("TypeError: 'coroutine' object is not subscriptable"));
        assert!(mentions_coroutine("<coroutine object llm_query at 0x10>"));
        assert!(mentions_coroutine("RuntimeWarning: coroutine 'load' was never awaited"));
        assert!(!mentions_coroutine("ZeroDivisionError"));
        assert!(!mentions_coroutine("NameError: name 'coroutine_count' is not defined"));
        let long = format!("{}ZeroDivisionError: boom", "x".repeat(10_000));
        assert!(tail(&long).ends_with("ZeroDivisionError: boom") && tail(&long).chars().count() <= MAX_OUTPUT_CHARS / 2 + 1);
        assert_eq!(tail("short"), "short");
    }
}

/// The first bash command and the first REPL cell, issued while the session environment is still
/// being built by a slow fake creator, wait on the gate and then see the venv.
#[cfg(all(test, unix))]
mod session_venv_gate_tests {
    use super::*;
    use crate::tool::{Tool, ToolContext, ToolExecutionMode};
    use factr_base::python_env as pe;
    use std::time::{Duration, Instant};

    fn ctx() -> ToolContext {
        ToolContext {
            session_id: "gate-session".into(),
            message_id: "m".into(),
            tool_call_id: "c".into(),
            working_dir: Some(std::env::temp_dir()),
            stdin_request_tx: None,
            graceful_shutdown_signal: None,
            execution_mode: ToolExecutionMode::Direct,
        }
    }

    /// A slow creator: waits, builds a real venv, preinstalls `dummyprobe_pkg` into it.
    fn slow_gate(base: PathBuf, dir: PathBuf, delay: Duration, works: bool) -> Arc<pe::VenvGate> {
        let made = dir.clone();
        pe::VenvGate::start(dir, Box::new(|| {}), move |_| {
            std::thread::sleep(delay);
            if !works {
                return false;
            }
            let ok = std::process::Command::new(&base).args(["-m", "venv", "--system-site-packages"]).arg(&made).status().is_ok_and(|s| s.success());
            let site = std::process::Command::new(made.join("bin/python3"))
                .args(["-c", "import site; print(site.getsitepackages()[0])"])
                .output()
                .ok()
                .map(|o| PathBuf::from(String::from_utf8_lossy(&o.stdout).trim()));
            if let (true, Some(site)) = (ok, site) {
                std::fs::create_dir_all(site.join("dummyprobe_pkg")).unwrap();
                std::fs::write(site.join("dummyprobe_pkg/__init__.py"), "MARK = 'probe-ok'\n").unwrap();
                return true;
            }
            false
        })
    }

    struct Env(Vec<(&'static str, Option<std::ffi::OsString>)>);
    impl Env {
        fn save() -> Self {
            Self(["PATH", "FACTR_SESSION_VENV", "VIRTUAL_ENV"].into_iter().map(|k| (k, std::env::var_os(k))).collect())
        }
    }
    impl Drop for Env {
        fn drop(&mut self) {
            for (k, v) in &self.0 {
                match v {
                    Some(v) => crate::env::set_var(k, v),
                    None => crate::env::remove_var(k),
                }
            }
            pe::set_gate_for_test(None);
        }
    }

    fn point_at(dir: &std::path::Path) {
        let path = std::env::var_os("PATH").unwrap();
        let joined = std::env::join_paths(std::iter::once(dir.join("bin")).chain(std::env::split_paths(&path))).unwrap();
        crate::env::set_var("PATH", joined);
        crate::env::set_var("FACTR_SESSION_VENV", dir);
        crate::env::set_var("VIRTUAL_ENV", dir);
    }

    fn base_python() -> Option<PathBuf> {
        let base = pe::interpreter_from(None, std::env::var_os("PATH"))?;
        let in_venv = base.parent().and_then(|p| p.parent()).is_some_and(|v| v.join("pyvenv.cfg").is_file());
        (!in_venv).then_some(base)
    }

    #[tokio::test]
    async fn first_bash_command_and_first_repl_cell_wait_for_the_venv_and_see_it() {
        let _lock = crate::storage::lock_test_env();
        let Some(base) = base_python() else { return eprintln!("skipped: no non-venv python3") };
        let _env = Env::save();
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("venv");
        let delay = Duration::from_millis(1500);

        // bash, first command, issued while the venv is still being built.
        pe::set_gate_for_test(Some(slow_gate(base.clone(), dir.clone(), delay, true)));
        point_at(&dir);
        assert!(pe::session_venv_pending());
        let t = Instant::now();
        let out = crate::tool::bash::BashTool::new()
            .execute(serde_json::json!({"command": "command -v python3; python3 -c 'import dummyprobe_pkg; print(dummyprobe_pkg.MARK)'"}), ctx())
            .await
            .unwrap();
        assert!(t.elapsed() >= delay, "the command waited for the gate");
        assert!(out.output.contains(dir.to_str().unwrap()) && out.output.contains("probe-ok"), "{}", out.output);
        assert!(!out.output.contains("No module named"), "{}", out.output);

        // REPL, first cell, again while building (a second, slow gate).
        let dir2 = tmp.path().join("venv2");
        pe::set_gate_for_test(Some(slow_gate(base.clone(), dir2.clone(), delay, true)));
        point_at(&dir2);
        let repl = ReplTool { host: factr_learn::ReplHost::new_resolving(base.clone(), Some(pe::interpreter)) };
        let t = Instant::now();
        let out = repl
            .execute(serde_json::json!({"code": "import dummyprobe_pkg\nprint(dummyprobe_pkg.MARK)"}), ctx())
            .await
            .unwrap();
        assert!(t.elapsed() >= delay, "the cell waited for the gate");
        assert!(out.output.contains("probe-ok") && !out.output.contains("compute"), "{}", out.output);
    }

    #[tokio::test]
    async fn a_failing_build_falls_back_to_the_system_interpreter_without_hanging() {
        let _lock = crate::storage::lock_test_env();
        let Some(base) = base_python() else { return eprintln!("skipped: no non-venv python3") };
        let _env = Env::save();
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("venv");
        pe::set_gate_for_test(Some(slow_gate(base.clone(), dir.clone(), Duration::from_millis(200), false)));
        point_at(&dir);
        let t = Instant::now();
        let out = crate::tool::bash::BashTool::new()
            .execute(serde_json::json!({"command": "python3 -c 'import dummyprobe_pkg'"}), ctx())
            .await
            .unwrap();
        assert!(t.elapsed() < Duration::from_secs(5), "released promptly");
        // The hint names the interpreter that exists, not the missing venv.
        assert!(out.output.contains("No module named") && !out.output.contains(dir.to_str().unwrap()), "{}", out.output);
        let hint = repl_error_hint("ModuleNotFoundError: No module named 'x'");
        assert!(!hint.contains(dir.to_str().unwrap()), "{hint}");
        // The engine did not rewrite its own environment from the build thread; the next command just
        // does not see the dead venv.
        assert!(std::env::var_os("FACTR_SESSION_VENV").is_some(), "no setenv from the build thread");
        let out = crate::tool::bash::BashTool::new()
            .execute(serde_json::json!({"command": "echo \"[${FACTR_SESSION_VENV}][${VIRTUAL_ENV}]\"; echo \"$PATH\""}), ctx())
            .await
            .unwrap();
        assert!(out.output.contains("[][]") && !out.output.contains(dir.to_str().unwrap()), "{}", out.output);
        assert!(!pe::session_venv_active());
    }
}
