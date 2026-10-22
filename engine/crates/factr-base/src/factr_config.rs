//! Typed, read-only view of the Factr `config.yaml` keys the engine acts on.
//!
//! `$FACTR_CONFIG_HOME/config.yaml` (the entrypoint exports `~/.factr` when unset) is the one backing store: the Settings screens
//! write it through `config.set` (gateway `rpc/settings.rs`) and Factr itself reads it. This module
//! never writes. The parsed view is cached by file mtime, length and inode, so a `config.set` takes effect on
//! the next read with no restart and a hot path costs one `stat`.
//!
//! Every accessor returns `None`/empty when a key is absent: an unset key leaves the engine's own
//! default untouched.

use serde_yaml::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

/// `compression.*`, as written (unclamped; `CompactionLimits::with_overrides` clamps).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Compression {
    pub enabled: Option<bool>,
    pub threshold: Option<f64>,
    pub target_ratio: Option<f64>,
    pub protect_last_n: Option<i64>,
    /// Absolute trigger: compaction starts at the lower of `threshold` of the window and this count.
    pub threshold_tokens: Option<i64>,
    /// `compression.deterministic`: condense old messages with a model-free digest (off unless set).
    pub deterministic: Option<bool>,
    /// `compression.llm_second_pass`: with `deterministic`, also ask the model for a summary (default on).
    pub llm_second_pass: Option<bool>,
}

/// `memory.*`: the desktop's memory switch.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Memory {
    pub enabled: Option<bool>,
}

/// `web.*`: which key-based search backend (`tavily|exa|firecrawl|brave|serper|linkup`) to use.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Web {
    pub backend: Option<String>,
    pub search_backend: Option<String>,
    /// `web.search_fallback`: when the chosen key-based backend fails, try the other backends that have a
    /// key (a recently failed one is skipped for a few minutes). Off unless set.
    pub search_fallback: bool,
}

impl Web {
    /// The backend web search uses: the per-capability `search_backend`, else the shared `backend`.
    pub fn search(&self) -> Option<&str> {
        self.search_backend.as_deref().or(self.backend.as_deref())
    }
}

/// `terminal.*`: limits on the commands the bash tool runs.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Terminal {
    /// `terminal.max_memory_mb`: resident-memory cap of one command's process group.
    pub max_memory_mb: Option<u64>,
    /// `terminal.reduce_output`: route shell commands through the `rtk` CLI (when installed) so their
    /// output is shorter. Off unless set.
    pub reduce_output: bool,
    /// `terminal.reduce_output_timeout_ms`: how long the rewrite lookup may take before the command
    /// runs unchanged. Clamped to 100..=10000; 1500 when unset.
    pub reduce_output_timeout_ms: Option<u64>,
}

/// `budget.*`: an optional hard stop on what one session may spend. Both unset (the default) means no
/// limit and the engine never looks at usage.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Budget {
    /// `budget.max_cost_usd`: stop starting new requests once the session's estimated spend reaches it.
    /// Only enforced for routes that can be priced per token.
    pub max_cost_usd: Option<f64>,
    /// `budget.max_tokens`: stop once the session's input plus output tokens reach this.
    pub max_tokens: Option<u64>,
}

/// `approvals.*` keys that add questions (never remove any): both are off/empty unless set.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ApprovalExtras {
    /// `approvals.gui_automation_ask`: ask before commands that drive other apps or edit the agent's own
    /// approval and config files.
    pub gui_automation_ask: bool,
    /// `approvals.always_ask`: regular expressions; a shell command matching one is asked about every
    /// time, whatever grants exist. Entries are trimmed; empty ones are dropped.
    pub always_ask: Vec<String>,
}

/// `tools.*`: how tool schemas reach the model.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ToolsView {
    /// `tools.routing`: also load the withheld (deferred) tools a user message clearly asks for, one
    /// `load_tools` round trip earlier. Only ever adds tools; off unless set.
    pub routing: bool,
}

/// `local_models.*`: guardrails for models served on this machine or the local network.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LocalModels {
    /// `local_models.silence_timeout_secs`: how long a local endpoint may stay silent before the stream is
    /// treated as dead. Raises the normal limit for local endpoints only; unset leaves it alone.
    pub silence_timeout_secs: Option<u64>,
}

/// `monitoring.export.otlp.*`: the operator-configured OTLP destination (shared with the backend's gateway-health
/// export). When `enabled` with an endpoint, finished turns are also exported there as traces.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ObservabilityExport {
    /// `monitoring.export.otlp.enabled` (off unless true).
    pub enabled: bool,
    /// `monitoring.export.otlp.endpoint`: an OTLP/HTTP base URL (`http(s)://host:4318`); traces go to `/v1/traces`.
    pub endpoint: Option<String>,
    /// `monitoring.export.otlp.headers_env`: header name to the NAME of the environment variable (or `.env` entry)
    /// holding its value; secrets never sit in config.yaml.
    pub headers_env: Vec<(String, String)>,
}

/// True when `api_base` points at this machine or a private network address.
pub fn is_local_endpoint(api_base: &str) -> bool {
    let Some(host) = url::Url::parse(api_base).ok().and_then(|u| u.host_str().map(str::to_ascii_lowercase)) else {
        return false;
    };
    let host = host.trim_start_matches('[').trim_end_matches(']');
    if host == "localhost" || host.ends_with(".localhost") || host.ends_with(".local") {
        return true;
    }
    match host.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(ip)) => ip.is_loopback() || ip.is_private() || ip.is_link_local(),
        Ok(std::net::IpAddr::V6(ip)) => ip.is_loopback() || (ip.segments()[0] & 0xfe00) == 0xfc00,
        Err(_) => false,
    }
}

/// `persistence.*`: a headless run whose task names a check keeps going, through the goal machinery, while
/// that check fails (on unless disabled).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Persistence {
    pub enabled: Option<bool>,
    /// `persistence.max_retries`: failed checks the run may retry (default 3, 1..=10).
    pub max_retries: Option<u32>,
    /// `persistence.max_tokens`: a fixed ceiling on the fresh tokens (uncached input plus output) one persisting run may
    /// use; unset or 0 leaves the automatic one (5x the first turn, 400k to 2M).
    pub max_tokens: Option<u64>,
    /// `persistence.max_seconds`: stop persisting after this many seconds of the run (never beyond the run's own
    /// time limit); unset or 0 uses the run's limit minus one check. Trades pass rate for time.
    pub max_seconds: Option<u64>,
}

/// `checkpoints.*`, as written (the checkpoint store applies its defaults and floors).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Checkpoints {
    pub enabled: Option<bool>,
    pub max_snapshots: Option<i64>,
    pub max_total_size_mb: Option<i64>,
    pub max_file_size_mb: Option<i64>,
}

/// `auxiliary.<task>.{provider,model}`: where one background task's model calls go.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AuxTarget {
    pub provider: Option<String>,
    pub model: Option<String>,
}

/// `delegation.*`: how subagents are spawned.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Delegation {
    pub model: Option<String>,
    pub provider: Option<String>,
    pub max_iterations: Option<u32>,
    pub reasoning_effort: Option<String>,
}

impl Delegation {
    /// The model spec subagents run on (`None`: inherit the parent's).
    pub fn model_spec(&self) -> Option<String> {
        route_spec(self.provider.as_deref(), self.model.as_deref())
    }
}

/// One entry of the `hooks:` block (Factr shell hooks).
#[derive(Debug, Clone, PartialEq)]
pub struct HookSpec {
    /// The Factr event name it was configured under (`pre_tool_call`, `on_session_end`, ...).
    pub event: String,
    pub command: String,
    /// Tool-name regex, only for `pre_tool_call` / `post_tool_call`.
    pub matcher: Option<String>,
    pub timeout_s: u64,
    /// `pre_tool_call` only: a failed or timed-out hook blocks the call.
    pub fail_closed: bool,
}

pub const HOOK_DEFAULT_TIMEOUT_S: u64 = 60;
pub const HOOK_MAX_TIMEOUT_S: u64 = 300;

fn parse_hooks(root: &Value) -> Vec<HookSpec> {
    let Some(events) = path(root, "hooks").and_then(Value::as_mapping) else { return Vec::new() };
    let mut specs = Vec::new();
    for (event, entries) in events {
        let Some(event) = event.as_str().filter(|e| !matches!(*e, "output_spill" | "outbound")) else { continue };
        let tool_event = matches!(event, "pre_tool_call" | "post_tool_call");
        for entry in entries.as_sequence().into_iter().flatten() {
            let Some(command) = entry.get("command").and_then(as_text) else { continue };
            let timeout_s = match entry.get("timeout").and_then(as_i64) {
                Some(n) if n >= 1 => (n as u64).min(HOOK_MAX_TIMEOUT_S),
                _ => HOOK_DEFAULT_TIMEOUT_S,
            };
            let fail_closed = entry.get("fail_closed").or_else(|| entry.get("failClosed")).and_then(Value::as_bool).unwrap_or(false);
            specs.push(HookSpec {
                event: event.to_string(),
                command,
                matcher: entry.get("matcher").and_then(as_text).filter(|_| tool_event),
                timeout_s,
                fail_closed: fail_closed && event == "pre_tool_call",
            });
        }
    }
    specs
}

/// One `fallback_providers` entry.
#[derive(Debug, Clone, PartialEq)]
pub struct Fallback {
    pub provider: String,
    pub model: String,
}

impl Fallback {
    /// The engine's model spec for this entry (`provider:model`).
    pub fn spec(&self) -> String {
        route_spec(Some(&self.provider), Some(&self.model)).unwrap_or_default()
    }
}

/// The parsed keys. Add a section here when the engine starts acting on another Factr key.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FactrConfig {
    pub compression: Compression,
    pub memory: Memory,
    pub web: Web,
    pub terminal: Terminal,
    pub checkpoints: Checkpoints,
    pub budget: Budget,
    pub tools: ToolsView,
    pub persistence: Persistence,
    pub local_models: LocalModels,
    pub observability_export: ObservabilityExport,
    /// `agent.text_verbosity`: `low`, `medium` or `high`; anything else is unset.
    pub text_verbosity: Option<String>,
    pub approval_extras: ApprovalExtras,
    /// `fallback_providers`, in order. Entries without both a provider and a model are dropped.
    pub fallbacks: Vec<Fallback>,
    pub delegation: Delegation,
    /// `hooks:` entries in file order.
    pub hooks: Vec<HookSpec>,
    /// `hooks_auto_accept`: run configured hooks without a first-use approval.
    pub hooks_auto_accept: bool,
    /// Every `auxiliary.<task>` block that names a provider or a model, by task.
    pub aux: BTreeMap<String, AuxTarget>,
}

/// The engine's background model calls that Factr names as `auxiliary.<task>` slots. This is the one
/// mapping table: a task name appears here once, and nothing else in the engine reads an
/// `auxiliary.*` key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuxConsumer {
    /// The compaction summarizer.
    Compaction,
    /// Automatic memory extraction.
    MemoryExtraction,
    /// The post-turn learning review (gateway `learn.rs`).
    Learning,
    /// The REPL's `llm_query` sub-calls (`agents.repl_sub_model`'s job).
    ReplSub,
    /// `llm.oneshot`: a user-facing one-shot generation. It has no Factr slot and takes no override:
    /// it runs on the chat's own model (or the startup model outside a chat).
    OneShot,
}

/// `(Factr task slot, engine consumer)`. Factr slots with no engine consumer (vision, approval,
/// title_generation, mcp, skills_hub, kanban, tts, ...) belong to the Python backend, which reads the same file.
///
/// - `compression` is Factr's own slot for the context summarizer.
/// - `background_review` is Factr's post-turn "save memory / patch skill" fork: it is what both the
///   engine's memory extraction and its learning review do, so one slot feeds both.
/// - `repl_sub` has no Factr slot (Factr has no REPL sub-queries); it takes the same
///   `{provider, model}` shape so the Settings screen treats it like any other task.
///
/// A slot that names a model takes precedence over the engine-side `agents.repl_sub_model`
/// (config.toml or env), which stays as the standalone default. `FACTR_MEMORY_MODEL` outranks the slot.
pub const AUX_MAP: &[(&str, AuxConsumer)] = &[
    ("compression", AuxConsumer::Compaction),
    ("background_review", AuxConsumer::MemoryExtraction),
    ("background_review", AuxConsumer::Learning),
    ("repl_sub", AuxConsumer::ReplSub),
];

/// The Factr slot that feeds `consumer`.
pub fn aux_task(consumer: AuxConsumer) -> &'static str {
    if consumer == AuxConsumer::OneShot {
        return "one_shot";
    }
    AUX_MAP.iter().find(|(_, c)| *c == consumer).map(|(t, _)| *t).expect("every consumer is in AUX_MAP")
}

/// Provider names Factr uses that the engine spells differently.
fn engine_provider(factr: &str) -> &str {
    match factr {
        "openai-codex" => "openai-oauth",
        "google" | "google-gemini" | "google-ai-studio" => "gemini",
        "claude" => "claude",
        other => other,
    }
}

/// A Factr `provider` + `model` pair as the engine's model spec (`provider:model`, or the bare model
/// for an empty/`auto`/`main` provider). `None` when no model is named.
pub fn route_spec(provider: Option<&str>, model: Option<&str>) -> Option<String> {
    let model = model.map(str::trim).filter(|m| !m.is_empty())?;
    match provider.map(|p| p.trim().to_ascii_lowercase()).filter(|p| !p.is_empty() && p != "auto" && p != "main") {
        Some(p) => Some(format!("{}:{model}", engine_provider(&p))),
        None => Some(model.to_string()),
    }
}

impl FactrConfig {
    /// The model spec the user chose for `consumer` in Settings, if any.
    pub fn aux_model(&self, consumer: AuxConsumer) -> Option<String> {
        if consumer == AuxConsumer::OneShot {
            return None;
        }
        let target = self.aux.get(aux_task(consumer))?;
        route_spec(target.provider.as_deref(), target.model.as_deref())
    }
}

/// `provider`, or a fork of it switched to `spec` (a failed switch keeps `provider`, so a setting that
/// cannot apply never blocks the work). The caller's own provider is never switched.
pub fn switched(provider: Arc<dyn crate::provider::Provider>, spec: Option<&str>, what: &str) -> Arc<dyn crate::provider::Provider> {
    let Some(spec) = spec else { return provider };
    let fork = provider.fork();
    match fork.set_model(spec) {
        Ok(()) => fork,
        Err(error) => {
            crate::logging::warn(&format!("{what} model {spec}: {error}; using the session model"));
            provider
        }
    }
}

/// `base`, or a fork on the model Settings chose for `consumer`.
pub fn provider_for(consumer: AuxConsumer, base: Arc<dyn crate::provider::Provider>) -> Arc<dyn crate::provider::Provider> {
    switched(base, aux_model(consumer).as_deref(), aux_task(consumer))
}

/// A non-empty environment value, trimmed: the top-priority override of a Factr setting (benchmarks
/// and headless runs set these on a fresh home).
pub fn env_text(name: &str) -> Option<String> {
    std::env::var(name).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

/// The Responses `text.verbosity` to request: `FACTR_OPENAI_TEXT_VERBOSITY`, else `agent.text_verbosity`; only
/// `low`, `medium` or `high` count, anything else (the default empty value included) leaves the model default.
pub fn text_verbosity() -> Option<String> {
    let valid = |v: &String| matches!(v.as_str(), "low" | "medium" | "high");
    env_text("FACTR_OPENAI_TEXT_VERBOSITY").filter(valid).or_else(|| current().text_verbosity.clone())
}

/// A boolean environment value (`1/true/yes/on`, `0/false/no/off`); `None` when unset or unrecognised.
pub fn env_bool(name: &str) -> Option<bool> {
    as_bool(&Value::String(std::env::var(name).ok()?))
}

/// The model a swarm worker spawns on when the spawn call names none: `FACTR_SWARM_MODEL` (set but
/// empty: none), else Factr `delegation.model`/`provider`. `None`, or the sentinels
/// `inherit`/`coordinator` (applied by the caller), means the worker takes the coordinator's model.
pub fn swarm_model() -> Option<String> {
    match std::env::var("FACTR_SWARM_MODEL") {
        Ok(v) => Some(v.trim().to_string()).filter(|v| !v.is_empty()),
        Err(_) => current().delegation.model_spec(),
    }
}

/// The live `auxiliary.<task>` model spec for `consumer` (`None`: not set, use the engine's own).
pub fn aux_model(consumer: AuxConsumer) -> Option<String> {
    current().aux_model(consumer)
}

fn path<'a>(root: &'a Value, dotted: &str) -> Option<&'a Value> {
    dotted.split('.').try_fold(root, |node, part| node.get(part)).filter(|v| !v.is_null())
}

fn as_f64(v: &Value) -> Option<f64> {
    v.as_f64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
}

fn as_text(v: &Value) -> Option<String> {
    v.as_str().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

fn as_i64(v: &Value) -> Option<i64> {
    v.as_i64().or_else(|| as_f64(v).map(|f| f as i64))
}

fn as_bool(v: &Value) -> Option<bool> {
    v.as_bool().or_else(|| match v.as_str()?.trim().to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" | "1" => Some(true),
        "false" | "no" | "off" | "0" => Some(false),
        _ => None,
    })
}

impl FactrConfig {
    pub fn parse(yaml: &str) -> Self {
        let root: Value = serde_yaml::from_str(yaml).ok().filter(Value::is_mapping).unwrap_or(Value::Null);
        Self {
            compression: Compression {
                enabled: path(&root, "compression.enabled").and_then(as_bool),
                threshold: path(&root, "compression.threshold").and_then(as_f64),
                target_ratio: path(&root, "compression.target_ratio").and_then(as_f64),
                protect_last_n: path(&root, "compression.protect_last_n").and_then(as_i64),
                threshold_tokens: path(&root, "compression.threshold_tokens").and_then(as_i64).filter(|n| *n > 0),
                deterministic: path(&root, "compression.deterministic").and_then(as_bool),
                llm_second_pass: path(&root, "compression.llm_second_pass").and_then(as_bool),
            },
            memory: Memory { enabled: path(&root, "memory.memory_enabled").and_then(Value::as_bool) },
            web: Web {
                backend: path(&root, "web.backend").and_then(as_text),
                search_backend: path(&root, "web.search_backend").and_then(as_text),
                search_fallback: path(&root, "web.search_fallback").and_then(as_bool).unwrap_or(false),
            },
            terminal: Terminal {
                max_memory_mb: path(&root, "terminal.max_memory_mb").and_then(as_i64).and_then(|n| u64::try_from(n).ok()).filter(|n| *n > 0),
                reduce_output: path(&root, "terminal.reduce_output").and_then(as_bool).unwrap_or(false),
                reduce_output_timeout_ms: path(&root, "terminal.reduce_output_timeout_ms").and_then(as_i64).and_then(|n| u64::try_from(n).ok()).map(|n| n.clamp(100, 10_000)),
            },
            approval_extras: ApprovalExtras {
                gui_automation_ask: path(&root, "approvals.gui_automation_ask").and_then(as_bool).unwrap_or(false),
                always_ask: path(&root, "approvals.always_ask")
                    .and_then(Value::as_sequence)
                    .into_iter()
                    .flatten()
                    .filter_map(as_text)
                    .take(100)
                    .collect(),
            },
            text_verbosity: path(&root, "agent.text_verbosity").and_then(as_text).map(|v| v.to_ascii_lowercase()).filter(|v| matches!(v.as_str(), "low" | "medium" | "high")),
            observability_export: ObservabilityExport {
                enabled: path(&root, "monitoring.export.otlp.enabled").and_then(as_bool).unwrap_or(false),
                endpoint: path(&root, "monitoring.export.otlp.endpoint").and_then(as_text).map(|u| u.trim().trim_end_matches('/').to_string()).filter(|u| u.starts_with("http://") || u.starts_with("https://")),
                headers_env: path(&root, "monitoring.export.otlp.headers_env")
                    .and_then(Value::as_mapping)
                    .into_iter()
                    .flatten()
                    .filter_map(|(k, v)| Some((as_text(k)?, as_text(v)?)))
                    .take(10)
                    .collect(),
            },
            local_models: LocalModels {
                silence_timeout_secs: path(&root, "local_models.silence_timeout_secs").and_then(as_i64).and_then(|n| u64::try_from(n).ok()).filter(|n| *n > 0).map(|n| n.min(7200)),
            },
            persistence: Persistence {
                enabled: path(&root, "persistence.enabled").and_then(as_bool),
                max_retries: path(&root, "persistence.max_retries").and_then(as_i64).filter(|n| *n > 0).map(|n| n.min(10) as u32),
                max_tokens: path(&root, "persistence.max_tokens").and_then(as_i64).and_then(|n| u64::try_from(n).ok()).filter(|n| *n > 0),
                max_seconds: path(&root, "persistence.max_seconds").and_then(as_i64).and_then(|n| u64::try_from(n).ok()).filter(|n| *n > 0),
            },
            tools: ToolsView { routing: path(&root, "tools.routing").and_then(as_bool).unwrap_or(false) },
            budget: Budget {
                max_cost_usd: path(&root, "budget.max_cost_usd").and_then(as_f64).filter(|n| n.is_finite() && *n > 0.0),
                max_tokens: path(&root, "budget.max_tokens").and_then(as_i64).and_then(|n| u64::try_from(n).ok()).filter(|n| *n > 0),
            },
            checkpoints: Checkpoints {
                enabled: path(&root, "checkpoints.enabled").and_then(as_bool),
                max_snapshots: path(&root, "checkpoints.max_snapshots").and_then(as_i64),
                max_total_size_mb: path(&root, "checkpoints.max_total_size_mb").and_then(as_i64),
                max_file_size_mb: path(&root, "checkpoints.max_file_size_mb").and_then(as_i64),
            },
            fallbacks: path(&root, "fallback_providers")
                .and_then(Value::as_sequence)
                .into_iter()
                .flatten()
                .filter_map(|entry| {
                    let provider = entry.get("provider").and_then(as_text)?;
                    let model = entry.get("model").and_then(as_text)?;
                    (!provider.eq_ignore_ascii_case("auto")).then_some(Fallback { provider, model })
                })
                .collect(),
            hooks: parse_hooks(&root),
            hooks_auto_accept: path(&root, "hooks_auto_accept").and_then(as_bool).unwrap_or(false),
            delegation: Delegation {
                model: path(&root, "delegation.model").and_then(as_text),
                provider: path(&root, "delegation.provider").and_then(as_text).filter(|p| !p.eq_ignore_ascii_case("auto")),
                max_iterations: path(&root, "delegation.max_iterations").and_then(as_i64).filter(|n| *n > 0).map(|n| n.min(10_000) as u32),
                reasoning_effort: path(&root, "delegation.reasoning_effort").and_then(as_text),
            },
            aux: path(&root, "auxiliary")
                .and_then(Value::as_mapping)
                .into_iter()
                .flatten()
                .filter_map(|(task, block)| {
                    let target = AuxTarget { provider: block.get("provider").and_then(as_text).filter(|p| !p.eq_ignore_ascii_case("auto")), model: block.get("model").and_then(as_text) };
                    (target != AuxTarget::default()).then_some(task.as_str()?.to_string()).map(|task| (task, target))
                })
                .collect(),
        }
    }
}

/// Per-session cap on model requests in one turn, set for a subagent spawned under
/// `delegation.max_iterations`; the turn loops read it.
static ITERATION_CAPS: Mutex<Option<std::collections::HashMap<String, u32>>> = Mutex::new(None);

pub fn set_iteration_cap(session_id: &str, cap: u32) {
    ITERATION_CAPS.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(Default::default).insert(session_id.to_string(), cap);
}

pub fn iteration_cap(session_id: &str) -> Option<u32> {
    ITERATION_CAPS.lock().unwrap_or_else(|e| e.into_inner()).as_ref()?.get(session_id).copied()
}

/// The Factr config directory: `$FACTR_CONFIG_HOME`. The one place that resolves it. The entrypoint
/// (`src/bin/factr.rs`) exports `~/.factr` when the variable is unset, so no reader carries its own
/// fallback; with it unset (a unit test) the engine reads and writes no Factr files. A test binary treats a
/// `FACTR_CONFIG_HOME` inside the real `~/.factr` as unset.
pub fn home() -> Option<PathBuf> {
    std::env::var_os("FACTR_CONFIG_HOME")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
        // A cargo test binary never reads or writes the developer's real ~/.factr (an exported
        // FACTR_CONFIG_HOME in a dev shell): it behaves as if the variable were unset.
        .filter(|dir| !(crate::real_home_guard::in_test_binary() && crate::real_home_guard::is_real_home_path(dir)))
}

/// What identifies one version of the file: modified time, length and inode. A save is a rename onto a
/// fresh inode, so an edit that keeps the length and lands in the same timestamp tick still re-reads.
type Stamp = (SystemTime, u64, u64);
type Cached = (PathBuf, Option<Stamp>, Arc<FactrConfig>);

fn file_stamp(file: &Path) -> Option<Stamp> {
    let meta = std::fs::metadata(file).ok()?;
    #[cfg(unix)]
    let inode = std::os::unix::fs::MetadataExt::ino(&meta);
    #[cfg(not(unix))]
    let inode = 0;
    Some((meta.modified().unwrap_or(SystemTime::UNIX_EPOCH), meta.len(), inode))
}
static CACHE: Mutex<Option<Cached>> = Mutex::new(None);

/// The config at `dir/config.yaml` (empty when the file is missing or unreadable), re-read when it changed.
pub fn load_from(dir: &Path) -> Arc<FactrConfig> {
    let file = dir.join("config.yaml");
    let stamp = file_stamp(&file);
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((cached_file, cached_stamp, parsed)) = cache.as_ref()
        && *cached_file == file
        && *cached_stamp == stamp
    {
        return parsed.clone();
    }
    let parsed = Arc::new(stamp.and_then(|_| std::fs::read_to_string(&file).ok()).map(|y| FactrConfig::parse(&y)).unwrap_or_default());
    *cache = Some((file, stamp, parsed.clone()));
    parsed
}

/// The live config for this process (`$FACTR_CONFIG_HOME`).
pub fn current() -> Arc<FactrConfig> {
    home().map(|dir| load_from(&dir)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_test_binary_treats_the_real_factr_home_as_unset() {
        let _env = crate::storage::lock_test_env();
        let before = std::env::var_os("FACTR_CONFIG_HOME");
        let real = factr_storage::passwd_home().unwrap().join(".factr");
        crate::env::set_var("FACTR_CONFIG_HOME", &real);
        let seen = home();
        let temp = std::env::temp_dir().join("factr-elsewhere");
        crate::env::set_var("FACTR_CONFIG_HOME", &temp);
        let other = home();
        match before {
            Some(v) => crate::env::set_var("FACTR_CONFIG_HOME", v),
            None => crate::env::remove_var("FACTR_CONFIG_HOME"),
        }
        assert_eq!(seen, None, "the real ~/.factr is never resolved in a test binary");
        assert_eq!(other, Some(temp));
    }

    /// Production code resolves the config dir only through `home()`; `FACTR_CONFIG_HOME` is read raw in this
    /// file and in the entrypoint (`src/bin/factr.rs`, which exports it) and nowhere else.
    #[test]
    fn factr_home_is_read_only_through_the_one_helper() {
        fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
            for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if path.file_name().is_some_and(|n| n != "tests" && n != "target") {
                        rs_files(&path, out);
                    }
                } else if path.extension().is_some_and(|e| e == "rs") {
                    out.push(path);
                }
            }
        }
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut files = Vec::new();
        rs_files(&root.join("src"), &mut files);
        for entry in std::fs::read_dir(root.join("crates")).unwrap().flatten() {
            rs_files(&entry.path().join("src"), &mut files);
        }
        assert!(files.len() > 100, "found the source tree");
        let mut offenders = Vec::new();
        for file in files {
            let rel = file.strip_prefix(&root).unwrap().to_string_lossy().replace("../", "");
            let stem = file.file_stem().unwrap().to_string_lossy().to_string();
            if rel.ends_with("factr-base/src/factr_config.rs") || rel == "src/bin/factr.rs" || stem == "tests" || stem.ends_with("_tests") {
                continue;
            }
            let text = std::fs::read_to_string(&file).unwrap();
            let lines: Vec<&str> = text.lines().collect();
            // Code after the trailing `#[cfg(test)] mod ...` is test code.
            let end = (0..lines.len())
                .find(|&i| lines[i].trim().starts_with("#[cfg(") && lines[i].contains("test") && lines.get(i + 1).is_some_and(|n| n.trim_start().starts_with("mod ") || n.trim_start().starts_with("pub(crate) mod ")))
                .unwrap_or(lines.len());
            for (i, line) in lines[..end].iter().enumerate() {
                let code = line.split("//").next().unwrap_or("");
                if code.contains("env::var_os(\"FACTR_CONFIG_HOME\")") || code.contains("env::var(\"FACTR_CONFIG_HOME\")") {
                    offenders.push(format!("{rel}:{}", i + 1));
                }
            }
        }
        assert!(offenders.is_empty(), "use factr_config::home() instead of reading FACTR_CONFIG_HOME: {offenders:?}");
    }

    #[test]
    fn absent_keys_stay_none_and_garbage_is_ignored() {
        assert_eq!(FactrConfig::parse(""), FactrConfig::default());
        assert_eq!(FactrConfig::parse("- just\n- a list"), FactrConfig::default());
        assert_eq!(FactrConfig::parse("compression: {threshold: lots}").compression.threshold, None);
    }

    #[test]
    fn checkpoint_settings_parse_from_numbers_and_strings() {
        let c = FactrConfig::parse("checkpoints:\n  enabled: 'off'\n  max_snapshots: '7'\n  max_file_size_mb: 2\n").checkpoints;
        assert_eq!(c, Checkpoints { enabled: Some(false), max_snapshots: Some(7), max_total_size_mb: None, max_file_size_mb: Some(2) });
        assert_eq!(FactrConfig::parse("model: x").checkpoints, Checkpoints::default());
    }

    #[test]
    fn compression_values_parse_from_numbers_and_strings() {
        let c = FactrConfig::parse("compression:\n  enabled: false\n  threshold: '0.6'\n  target_ratio: 0.25\n  protect_last_n: 30\n  threshold_tokens: '256000'\n").compression;
        assert_eq!(c, Compression { enabled: Some(false), threshold: Some(0.6), target_ratio: Some(0.25), protect_last_n: Some(30), threshold_tokens: Some(256_000), deterministic: None, llm_second_pass: None });
        let d = FactrConfig::parse("compression: {deterministic: true, llm_second_pass: false}").compression;
        assert_eq!((d.deterministic, d.llm_second_pass), (Some(true), Some(false)));
        assert_eq!(FactrConfig::parse("compression: {threshold_tokens: 0}").compression.threshold_tokens, None, "zero is no cap");
    }

    #[test]
    fn memory_and_web_keys_parse() {
        let c = FactrConfig::parse("memory:\n  memory_enabled: false\nweb:\n  backend: exa\n  search_backend: tavily\nterminal:\n  backend: docker\n");
        assert_eq!(c.memory.enabled, Some(false));
        assert_eq!(c.web.search(), Some("tavily"), "the per-capability key wins");
        assert_eq!(FactrConfig::parse("web: {backend: exa}").web.search(), Some("exa"));
        assert_eq!(FactrConfig::parse("terminal: {backend: docker}").web.search(), None);
        assert_eq!(FactrConfig::parse("terminal: {max_memory_mb: 4096}").terminal.max_memory_mb, Some(4096));
        assert_eq!(FactrConfig::parse("terminal: {max_memory_mb: 0}").terminal.max_memory_mb, None);
        assert_eq!(FactrConfig::parse("terminal: {max_memory_mb: -5}").terminal.max_memory_mb, None);
        assert_eq!(FactrConfig::parse("model: x").memory, Memory::default());
        assert!(!FactrConfig::parse("web: {backend: exa}").web.search_fallback, "fallback is opt-in");
        assert!(FactrConfig::parse("web: {search_fallback: true}").web.search_fallback);
    }

    /// The release's promise: with nothing configured, none of the opt-in controls does anything.
    #[test]
    fn every_opt_in_control_is_off_in_an_empty_or_unrelated_config() {
        for yaml in ["", "model: x\n", "terminal: {backend: local}\napprovals: {mode: smart}\ncompression: {threshold: 0.5}\n"] {
            let c = FactrConfig::parse(yaml);
            assert!(!c.terminal.reduce_output, "{yaml}");
            assert!(!c.web.search_fallback, "{yaml}");
            assert!(!c.tools.routing, "{yaml}");
            assert_eq!(c.budget, Budget::default(), "{yaml}");
            assert_eq!(c.approval_extras, ApprovalExtras::default(), "{yaml}");
            assert_eq!(c.local_models, LocalModels::default(), "{yaml}");
            assert_eq!(c.persistence, Persistence::default(), "{yaml}");
            assert_eq!(c.compression.deterministic.unwrap_or(false), false, "{yaml}");
        }
    }

    #[test]
    fn approval_extras_are_off_unless_set() {
        assert_eq!(FactrConfig::parse("").approval_extras, ApprovalExtras::default());
        assert_eq!(FactrConfig::parse("approvals: {mode: manual}").approval_extras, ApprovalExtras::default());
        let c = FactrConfig::parse("approvals:\n  gui_automation_ask: true\n  always_ask: ['^cargo publish', '  ']\n").approval_extras;
        assert!(c.gui_automation_ask);
        assert_eq!(c.always_ask, vec!["^cargo publish".to_string()]);
    }

    #[test]
    fn text_verbosity_takes_only_the_three_known_levels() {
        assert_eq!(FactrConfig::parse("").text_verbosity, None);
        assert_eq!(FactrConfig::parse("agent: {text_verbosity: ''}").text_verbosity, None);
        assert_eq!(FactrConfig::parse("agent: {text_verbosity: LOW}").text_verbosity.as_deref(), Some("low"));
        assert_eq!(FactrConfig::parse("agent: {text_verbosity: loud}").text_verbosity, None);
    }

    #[test]
    fn the_otlp_export_needs_an_http_endpoint_and_reads_standard_header_pairs() {
        assert_eq!(FactrConfig::parse("").observability_export, ObservabilityExport::default());
        let c = FactrConfig::parse("monitoring:\n  export:\n    otlp:\n      enabled: true\n      endpoint: ' https://cloud.example.com:4318/ '\n      headers_env: {x-api-key: MY_OTLP_KEY}\n").observability_export;
        assert!(c.enabled);
        assert_eq!(c.endpoint.as_deref(), Some("https://cloud.example.com:4318"));
        assert_eq!(c.headers_env, vec![("x-api-key".to_string(), "MY_OTLP_KEY".to_string())]);
        for bad in ["ftp://x", "localhost:4318", ""] {
            assert_eq!(FactrConfig::parse(&format!("monitoring: {{export: {{otlp: {{enabled: true, endpoint: '{bad}'}}}}}}")).observability_export.endpoint, None, "{bad}");
        }
    }

    #[test]
    fn local_endpoints_are_recognised_and_hosted_ones_are_not() {
        for local in ["http://localhost:11434/v1", "http://127.0.0.1:1234/v1", "http://[::1]:8080", "http://192.168.1.20:8000/v1", "http://10.0.0.5/v1", "http://172.16.4.1/v1", "http://box.local:9/v1"] {
            assert!(is_local_endpoint(local), "{local}");
        }
        for hosted in ["https://api.openai.com/v1", "https://openrouter.ai/api/v1", "http://172.32.0.1/v1", "not a url", ""] {
            assert!(!is_local_endpoint(hosted), "{hosted}");
        }
    }

    #[test]
    fn the_local_silence_timeout_is_unset_unless_positive_and_capped() {
        assert_eq!(FactrConfig::parse("").local_models.silence_timeout_secs, None);
        assert_eq!(FactrConfig::parse("local_models: {silence_timeout_secs: 0}").local_models.silence_timeout_secs, None);
        assert_eq!(FactrConfig::parse("local_models: {silence_timeout_secs: 900}").local_models.silence_timeout_secs, Some(900));
        assert_eq!(FactrConfig::parse("local_models: {silence_timeout_secs: 999999}").local_models.silence_timeout_secs, Some(7200));
    }

    #[test]
    fn persistence_is_unset_unless_configured_and_its_numbers_are_clamped() {
        assert_eq!(FactrConfig::parse("").persistence, Persistence::default());
        let c = FactrConfig::parse("persistence: {enabled: false, max_retries: 99, max_tokens: 750000, max_seconds: 300}").persistence;
        assert_eq!((c.enabled, c.max_retries, c.max_tokens, c.max_seconds), (Some(false), Some(10), Some(750_000), Some(300)));
        assert_eq!(FactrConfig::parse("persistence: {max_tokens: 0}").persistence.max_tokens, None);
        assert_eq!(FactrConfig::parse("persistence: {max_retries: 0}").persistence.max_retries, None);
    }

    #[test]
    fn tool_routing_is_off_unless_set() {
        assert!(!FactrConfig::parse("").tools.routing);
        assert!(FactrConfig::parse("tools: {routing: true}").tools.routing);
    }

    #[test]
    fn budget_is_unset_unless_positive_numbers_are_given() {
        assert_eq!(FactrConfig::parse("").budget, Budget::default());
        assert_eq!(FactrConfig::parse("budget: {max_cost_usd: 0, max_tokens: -1}").budget, Budget::default());
        assert_eq!(FactrConfig::parse("budget: {max_cost_usd: '2.5', max_tokens: 500000}").budget, Budget { max_cost_usd: Some(2.5), max_tokens: Some(500_000) });
    }

    #[test]
    fn output_reduction_is_off_unless_set_and_its_timeout_is_clamped() {
        assert!(!FactrConfig::parse("").terminal.reduce_output);
        assert!(!FactrConfig::parse("terminal: {max_memory_mb: 1}").terminal.reduce_output);
        assert!(FactrConfig::parse("terminal: {reduce_output: true}").terminal.reduce_output);
        assert!(FactrConfig::parse("terminal: {reduce_output: 'on'}").terminal.reduce_output);
        assert_eq!(FactrConfig::parse("terminal: {reduce_output_timeout_ms: 1}").terminal.reduce_output_timeout_ms, Some(100));
        assert_eq!(FactrConfig::parse("terminal: {reduce_output_timeout_ms: 999999}").terminal.reduce_output_timeout_ms, Some(10_000));
        assert_eq!(FactrConfig::parse("terminal: {}").terminal.reduce_output_timeout_ms, None);
    }

    #[test]
    fn auxiliary_slots_route_to_engine_model_specs() {
        let c = FactrConfig::parse(
            "auxiliary:\n  compression: {provider: openrouter, model: google/gemini-3-flash}\n  vision: {provider: auto, model: ''}\n  curator: {provider: auto, model: ''}\n",
        );
        assert_eq!(c.aux_model(AuxConsumer::Compaction).as_deref(), Some("openrouter:google/gemini-3-flash"));
        // One Factr slot can feed several engine consumers; each consumer maps to exactly one slot.
        let c = FactrConfig::parse("auxiliary:\n  background_review: {provider: anthropic, model: claude-haiku}\n  repl_sub: {model: small}\n");
        assert_eq!(c.aux_model(AuxConsumer::MemoryExtraction).as_deref(), Some("anthropic:claude-haiku"));
        assert_eq!(c.aux_model(AuxConsumer::Learning).as_deref(), Some("anthropic:claude-haiku"));
        assert_eq!(c.aux_model(AuxConsumer::ReplSub).as_deref(), Some("small"));
        assert_eq!(c.aux_model(AuxConsumer::Compaction), None);
        for consumer in [AuxConsumer::Compaction, AuxConsumer::MemoryExtraction, AuxConsumer::Learning, AuxConsumer::ReplSub] {
            assert_eq!(AUX_MAP.iter().filter(|(_, c)| *c == consumer).count(), 1, "{consumer:?} has one slot");
        }
        assert!(!c.aux.contains_key("vision"), "auto + empty model is no override");
        assert_eq!(route_spec(Some("auto"), Some("m")).as_deref(), Some("m"));
        assert_eq!(route_spec(Some("openai-codex"), Some("gpt-5.5")).as_deref(), Some("openai-oauth:gpt-5.5"));
        assert_eq!(route_spec(Some("anthropic"), Some("")), None);
    }

    #[test]
    fn fallback_providers_keep_their_order_and_drop_incomplete_entries() {
        let c = FactrConfig::parse(
            "fallback_providers:\n  - {provider: openrouter, model: a/b}\n  - {provider: anthropic}\n  - {provider: openai-codex, model: gpt-5.5}\n  - nonsense\n",
        );
        let specs: Vec<String> = c.fallbacks.iter().map(Fallback::spec).collect();
        assert_eq!(specs, ["openrouter:a/b", "openai-oauth:gpt-5.5"]);
    }

    #[test]
    fn delegation_block_parses_and_pins_the_subagent_model() {
        let d = FactrConfig::parse("delegation:\n  model: google/gemini-3-flash\n  provider: openrouter\n  max_iterations: 40\n  reasoning_effort: low\n").delegation;
        assert_eq!(d.model_spec().as_deref(), Some("openrouter:google/gemini-3-flash"));
        assert_eq!((d.max_iterations, d.reasoning_effort.as_deref()), (Some(40), Some("low")));
        let unset = FactrConfig::parse("delegation: {model: '', provider: auto, max_iterations: 0}").delegation;
        assert_eq!(unset, Delegation::default(), "the Factr defaults mean inherit");
        set_iteration_cap("s-cap", 3);
        assert_eq!((iteration_cap("s-cap"), iteration_cap("other")), (Some(3), None));
    }

    #[test]
    fn hooks_block_parses_like_factr_clamps_and_ignores_what_it_ignores() {
        let c = FactrConfig::parse(
            "hooks_auto_accept: true\nhooks:\n  pre_tool_call:\n    - {matcher: 'terminal|patch', command: ./gate.sh, timeout: 900, failClosed: true}\n    - {matcher: x}\n  on_session_end:\n    - {command: ./end.sh, matcher: ignored, fail_closed: true, timeout: 0}\n  outbound: {url: x}\n",
        );
        assert!(c.hooks_auto_accept);
        assert_eq!(c.hooks.len(), 2, "an entry without a command is skipped, reserved sub-sections are not events");
        assert_eq!((c.hooks[0].matcher.as_deref(), c.hooks[0].timeout_s, c.hooks[0].fail_closed), (Some("terminal|patch"), 300, true));
        assert_eq!((c.hooks[1].matcher.clone(), c.hooks[1].timeout_s, c.hooks[1].fail_closed), (None, 60, false));
    }

    #[test]
    fn the_cache_follows_the_file() {
        let dir = std::env::temp_dir().join(format!("factr-config-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(load_from(&dir).compression.threshold, None);
        std::fs::write(dir.join("config.yaml"), "compression: {threshold: 0.7}").unwrap();
        assert_eq!(load_from(&dir).compression.threshold, Some(0.7));
        std::fs::write(dir.join("config.yaml"), "compression: {threshold: 0.65, enabled: false}").unwrap();
        assert_eq!(load_from(&dir).compression.enabled, Some(false));
        // A save is a rename: a same-length edit (0.65 -> 0.75) is a new file, so it is re-read whatever the timestamp says.
        let (live, next) = (dir.join("config.yaml"), dir.join("config.yaml.next"));
        let before = std::fs::metadata(&live).unwrap().modified().unwrap();
        std::fs::write(&next, "compression: {threshold: 0.75, enabled: false}").unwrap();
        std::fs::File::options().write(true).open(&next).unwrap().set_modified(before).unwrap();
        std::fs::rename(&next, &live).unwrap();
        assert_eq!(load_from(&dir).compression.threshold, Some(0.75));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
