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
            python
                .is_file()
                .then(|| factr_learn::ReplHost::new(python))
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

/// The end of a traceback, where the exception line is, within half the output budget.
fn tail(error: &str) -> &str {
    let keep = MAX_OUTPUT_CHARS / 2;
    match error.char_indices().rev().nth(keep) {
        Some((cut, _)) => &error[cut..],
        None => error,
    }
}

#[async_trait]
impl Tool for ReplTool {
    fn name(&self) -> &str {
        "repl"
    }

    fn description(&self) -> &str {
        factr_learn::TOOL_DESCRIPTION
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
        let session_id = ctx.session_id.clone();
        let llm_query: factr_learn::LlmQuery = Arc::new(move |prompt: String| {
            let session_id = session_id.clone();
            Box::pin(async move {
                let provider =
                    crate::provider::session_provider_fork(&session_id).context("no active model provider")?;
                // The fork has its own state, so this never changes the session's model.
                let chosen = factr_base::factr_config::aux_model(factr_base::factr_config::AuxConsumer::ReplSub)
                    .or_else(|| crate::config::config().agents.repl_sub_model.clone());
                if let Some(sub) = chosen.as_deref() {
                    if let Err(error) = provider.set_model(sub) {
                        crate::logging::warn(&format!("repl_sub_model {sub}: {error}"));
                    }
                }
                let provider_name = provider.name().to_string();
                let model = provider.model();
                let started = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as i64;
                let result = provider
                    .complete_simple_with_usage(&prompt, SUBQUERY_SYSTEM)
                    .await;
                match &result {
                    Ok(reply) => super::report_aux_model_call(
                        "REPL subquery",
                        &session_id,
                        provider_name,
                        model,
                        started,
                        reply.usage,
                        None,
                    ),
                    Err(error) => super::report_aux_model_call(
                        "REPL subquery",
                        &session_id,
                        provider_name,
                        model,
                        started,
                        None,
                        Some(&error.to_string()),
                    ),
                }
                result.map(|reply| reply.text)
            })
        });
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
            let hint = if mentions_coroutine(&format!("{text}{error}")) {
                "\nHint: helpers are async: use `await load(...)`, `await llm_query(...)`."
            } else {
                ""
            };
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
