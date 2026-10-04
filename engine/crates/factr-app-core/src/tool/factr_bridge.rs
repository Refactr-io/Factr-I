//! The Factr tools only Factr's Python backend implements, reached from the Rust agent.
//!
//! One tool, `factr`, lists, describes and calls them. Schemas come from the running backend's
//! tool registry when the model asks (`describe`), so nothing is copied here and the system prompt
//! carries one small definition however many tools Factr has. Tools the engine already has natively
//! (files, shell, web, memory, todo, delegation, skills, browser) are never offered. The engine that
//! embeds this crate installs a [`FactrHost`] (the feature backend connection and the person);
//! without one, every call answers "Factr feature backend not available".
//!
//! `clarify` is native, not bridged: it must pause this turn and ask the person, which the
//! Python process cannot do for a Rust turn.

use super::{Tool, ToolContext, ToolOutput};
use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Value, json};
use std::sync::{Arc, RwLock};

pub const UNAVAILABLE: &str = "Factr feature backend not available";

/// Factr toolsets the engine already implements natively (hidden from `factr`, no duplicates).
/// Owner of each: `file` read/write/edit/apply_patch; `terminal` bash/bg; `web` the two Factr tools
/// (`web_search`, `web_extract`) are the native websearch/webfetch; `memory` memory; `todo` todo;
/// `session_search` session_search; `code_execution` repl/bash; `delegation`
/// delegate/communicate; `skills` skill; `browser` browser (the vault is excepted, see `native`);
/// `clarify` the native clarify tool below.
///
/// Deliberately NOT here, because no native tool does their job, so they are offered through
/// `factr`: `project` (desktop_project), `connections` (manage_connections) and `setup`
/// (manage_catalog). Each still hides itself on the Factr side (`check_fn`) when its precondition
/// (connectors, tips enabled) does not hold. The `desktop_ui` toolset (previews, panes, terminal
/// reads, tours) is hidden instead, see [`desktop_only`].
const NATIVE_TOOLSETS: &[&str] = &[
    "file", "terminal", "web", "memory", "todo", "session_search", "code_execution", "delegation", "skills", "browser",
    "clarify",
];

/// A Factr tool the engine already has: its toolset is native, except the browser vault (logins,
/// codes, form fill), which native `browser` lacks, so it is reached through `factr`.
fn native(name: &str, toolset: &str) -> bool {
    NATIVE_TOOLSETS.contains(&toolset) && !name.starts_with("browser_vault_")
}

/// Tools that drive the Python desktop session's window (previews, panes, tours). Python refuses them
/// outside that session ("no GUI callback here"), and a Rust turn is never inside it: never offered.
fn desktop_only(name: &str, toolset: &str) -> bool {
    toolset == "desktop_ui" || name.ends_with("_preview")
}

/// The answer to a `clarify` question.
pub enum ClarifyReply {
    Answer(String),
    /// Unattended run, or no window shows this session: nobody can answer.
    NoUser,
    TimedOut,
}

/// What the embedding engine provides: the feature backend, and the person.
#[async_trait]
pub trait FactrHost: Send + Sync {
    /// A JSON request to the feature backend's `/api/agent-tools` routes. An error is shown to the model as is.
    async fn call(&self, method: &str, path: &str, body: Option<Value>) -> Result<Value>;
    /// Ask the person a question and wait for the answer.
    async fn clarify(&self, session_id: &str, question: &str, choices: &[String]) -> ClarifyReply;
}

static HOST: RwLock<Option<Arc<dyn FactrHost>>> = RwLock::new(None);

/// Called once by the embedding engine (the gateway) at startup.
pub fn install_host(host: Arc<dyn FactrHost>) {
    *HOST.write().unwrap_or_else(|e| e.into_inner()) = Some(host);
}

fn installed() -> Option<Arc<dyn FactrHost>> {
    HOST.read().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Whether the Factr backend is up and offers `tool` (it lists only tools whose own
/// credentials and setup are in place). False when there is no backend.
pub(crate) async fn offers_tool(tool: &str) -> bool {
    let Some(host) = installed() else { return false };
    let Ok(reply) = host.call("GET", "/api/agent-tools", None).await else { return false };
    reply["tools"].as_array().into_iter().flatten().any(|t| t["name"].as_str() == Some(tool))
}

pub struct FactrTool {
    host: Option<Arc<dyn FactrHost>>,
}

impl FactrTool {
    pub fn new() -> Self {
        Self { host: None }
    }

    #[cfg(test)]
    fn with_host(host: Arc<dyn FactrHost>) -> Self {
        Self { host: Some(host) }
    }

    fn host(&self) -> Result<Arc<dyn FactrHost>> {
        self.host.clone().or_else(installed).ok_or_else(|| anyhow::anyhow!(UNAVAILABLE))
    }

    async fn list(&self, host: &dyn FactrHost, session_id: &str) -> Result<String> {
        let reply = host.call("GET", "/api/agent-tools", None).await?;
        let mut lines = Vec::new();
        for tool in reply["tools"].as_array().into_iter().flatten() {
            let (name, toolset) = (tool["name"].as_str().unwrap_or(""), tool["toolset"].as_str().unwrap_or(""));
            if name.is_empty() || native(name, toolset) || desktop_only(name, toolset) || !super::session_tool_allows(session_id, name, toolset) {
                continue;
            }
            lines.push(format!("{name}: {}", tool["description"].as_str().unwrap_or("")));
        }
        Ok(if lines.is_empty() {
            "No Factr tools are available (each needs its own credentials or setup in Factr).".to_string()
        } else {
            format!("{}\n\nUse action=describe for a tool's parameters, then action=call.", lines.join("\n"))
        })
    }

    /// The tool's toolset, from the backend, after the run's policy and the native-tool rule allowed it.
    async fn checked(host: &dyn FactrHost, name: &str, session_id: &str) -> Result<Value> {
        anyhow::ensure!(!name.is_empty(), "`tool` is required");
        let reply = match host.call("GET", &format!("/api/agent-tools/{}", urlencoding::encode(name)), None).await {
            Ok(reply) => reply,
            // A wrong name is the model's mistake, not a disabled backend: say what exists so it can
            // retry (a bare "404" was read as "the web UI is disabled").
            Err(err) if err.to_string().contains("404") => {
                let listed = host.call("GET", "/api/agent-tools", None).await.ok();
                let all: Vec<String> = listed.iter().flat_map(|l| l["tools"].as_array().into_iter().flatten())
                    .filter(|t| { let (n, ts) = (t["name"].as_str().unwrap_or(""), t["toolset"].as_str().unwrap_or("")); !native(n, ts) && !desktop_only(n, ts) })
                    .filter_map(|t| t["name"].as_str().map(str::to_string)).collect();
                let lower = name.to_lowercase();
                let close: Vec<&String> = all.iter().filter(|n| n.contains(&lower) || lower.contains(n.as_str()) || n.split('_').next() == lower.split('_').next()).collect();
                let hint = if close.is_empty() { format!("Available: {}.", all.join(", ")) } else { format!("Did you mean: {}?", close.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")) };
                anyhow::bail!("Factr has no tool named '{name}' (the backend is running; the name is wrong or the tool is unavailable). {hint} Use action=list.");
            }
            Err(err) => return Err(err),
        };
        let toolset = reply["toolset"].as_str().unwrap_or("");
        anyhow::ensure!(!native(name, toolset), "'{name}' is a native tool; call it directly");
        anyhow::ensure!(!desktop_only(name, toolset), "'{name}' drives the desktop window and is not available to this agent");
        anyhow::ensure!(super::session_tool_allows(session_id, name, toolset), "Tool '{name}' is disabled in this run");
        Ok(reply)
    }
}

#[async_trait]
impl Tool for FactrTool {
    fn name(&self) -> &str {
        "factr"
    }

    fn description(&self) -> &str {
        "Factr-only tools (cron jobs, image/video generation, vision, text to speech, Home Assistant, kanban, x_search, computer use, browser vault and CDP, desktop projects, connectors). action=list shows what is available, describe gives a tool's parameters, call runs it."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "intent": super::intent_schema_property(),
                "action": { "type": "string", "enum": ["list", "describe", "call"] },
                "tool": { "type": "string", "description": "Tool name, for describe and call" },
                "args": { "type": "object", "description": "Arguments for call, as described by describe" }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, input: Value, ctx: ToolContext) -> Result<ToolOutput> {
        let host = self.host()?;
        let tool = input["tool"].as_str().unwrap_or("").trim();
        match input["action"].as_str().unwrap_or("list") {
            "list" => Ok(ToolOutput::new(self.list(host.as_ref(), &ctx.session_id).await?)),
            "describe" => {
                let reply = Self::checked(host.as_ref(), tool, &ctx.session_id).await?;
                Ok(ToolOutput::new(serde_json::to_string_pretty(&reply["schema"])?))
            }
            "call" => {
                Self::checked(host.as_ref(), tool, &ctx.session_id).await?;
                let args = match &input["args"] {
                    Value::Null => json!({}),
                    args if args.is_object() => args.clone(),
                    _ => anyhow::bail!("factr: args must be an object (as shown by describe)"),
                };
                let body = json!({ "name": tool, "args": args, "session_id": ctx.session_id });
                let reply = host.call("POST", "/api/agent-tools/invoke", Some(body)).await?;
                Ok(ToolOutput::new(reply["result"].as_str().map(str::to_string).unwrap_or_else(|| reply.to_string())))
            }
            other => anyhow::bail!("unknown action '{other}'; use list, describe or call"),
        }
    }
}

/// Ask the person a question mid-run.
pub struct ClarifyTool {
    host: Option<Arc<dyn FactrHost>>,
}

impl ClarifyTool {
    pub fn new() -> Self {
        Self { host: None }
    }

    #[cfg(test)]
    fn with_host(host: Arc<dyn FactrHost>) -> Self {
        Self { host: Some(host) }
    }
}

#[async_trait]
impl Tool for ClarifyTool {
    fn name(&self) -> &str {
        "clarify"
    }

    fn description(&self) -> &str {
        "Ask the user one question when you cannot proceed without their answer. Not available in unattended runs."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "intent": super::intent_schema_property(),
                "question": { "type": "string" },
                "choices": { "type": "array", "items": { "type": "string" }, "maxItems": 4, "description": "Optional short answers to offer" }
            },
            "required": ["question"]
        })
    }

    async fn execute(&self, input: Value, ctx: ToolContext) -> Result<ToolOutput> {
        let question = input["question"].as_str().unwrap_or("").trim();
        anyhow::ensure!(!question.is_empty(), "`question` is required");
        let choices: Vec<String> = input["choices"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|c| c.as_str().map(str::to_string))
            .take(4)
            .collect();
        let reply = match self.host.clone().or_else(installed) {
            Some(host) => host.clarify(&ctx.session_id, question, &choices).await,
            None => ClarifyReply::NoUser,
        };
        Ok(ToolOutput::new(match reply {
            ClarifyReply::Answer(a) if a.trim().is_empty() => "The user skipped the question. Continue with your best judgment.".to_string(),
            ClarifyReply::Answer(a) => format!("The user answered: {a}"),
            ClarifyReply::NoUser => "No user is available to answer (unattended run). Do not wait: proceed with your best judgment and state the assumption you made.".to_string(),
            ClarifyReply::TimedOut => "The user did not answer in time. Proceed with your best judgment and state the assumption you made.".to_string(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use factr_tool_core::ToolExecutionMode;
    use std::sync::Mutex;

    struct Fake {
        up: bool,
        clarify: Mutex<Option<ClarifyReply>>,
        calls: Mutex<Vec<(String, String, Option<Value>)>>,
    }

    impl Fake {
        fn new(up: bool) -> Arc<Self> {
            Arc::new(Self { up, clarify: Mutex::new(None), calls: Mutex::default() })
        }
    }

    #[async_trait]
    impl FactrHost for Fake {
        async fn call(&self, method: &str, path: &str, body: Option<Value>) -> Result<Value> {
            anyhow::ensure!(self.up, UNAVAILABLE);
            self.calls.lock().unwrap().push((method.into(), path.into(), body));
            if path == "/api/agent-tools/cronjob" {
                anyhow::bail!("Factr backend answered 404 Not Found: tool 'cronjob' is unknown or unavailable");
            }
            Ok(match path {
                "/api/agent-tools" => json!({ "tools": [
                    { "name": "image_generate", "toolset": "image_gen", "description": "Make an image" },
                    { "name": "cronjob_manage", "toolset": "cronjob", "description": "Manage cron jobs" },
                    { "name": "send_message", "toolset": "messaging", "description": "Send a message" },
                    { "name": "read_file", "toolset": "file", "description": "Native duplicate" },
                    { "name": "browser_click", "toolset": "browser", "description": "Native browser duplicate" },
                    { "name": "browser_vault_fill", "toolset": "browser", "description": "Fill a saved login" },
                    { "name": "web_search", "toolset": "web", "description": "Native alias" },
                    { "name": "focus_pane", "toolset": "desktop_ui", "description": "Focus a desktop pane" },
                    { "name": "desktop_preview", "toolset": "browser", "description": "Preview a page" },
                    { "name": "desktop_project", "toolset": "project", "description": "Desktop projects" },
                    { "name": "manage_connections", "toolset": "connections", "description": "Connectors" },
                    { "name": "manage_catalog", "toolset": "setup", "description": "Connector catalog" },
                ]}),
                "/api/agent-tools/invoke" => json!({ "result": "{\"ok\":true}" }),
                other => {
                    let name = other.rsplit('/').next().unwrap_or("");
                    let toolset = match name { "cronjob_manage" => "cronjob", "read_file" => "file", "ha_call_service" => "homeassistant", "focus_pane" => "desktop_ui", "web_search" => "web", "manage_catalog" => "setup", _ => "image_gen" };
                    json!({ "name": name, "toolset": toolset, "schema": { "name": name, "parameters": { "type": "object" } } })
                }
            })
        }
        async fn clarify(&self, _: &str, _: &str, _: &[String]) -> ClarifyReply {
            self.clarify.lock().unwrap().take().unwrap_or(ClarifyReply::NoUser)
        }
    }

    fn ctx(session: &str) -> ToolContext {
        ToolContext {
            session_id: session.into(),
            message_id: "m".into(),
            tool_call_id: "t".into(),
            working_dir: None,
            stdin_request_tx: None,
            graceful_shutdown_signal: None,
            execution_mode: ToolExecutionMode::AgentTurn,
        }
    }

    async fn run(tool: &FactrTool, session: &str, input: Value) -> Result<String> {
        tool.execute(input, ctx(session)).await.map(|o| o.output)
    }

    #[tokio::test]
    async fn a_wrong_tool_name_names_the_close_matches_instead_of_a_bare_404() {
        let fake = Fake::new(true);
        let tool = FactrTool::with_host(fake.clone());
        let err = run(&tool, "hb-404", json!({"action": "call", "tool": "cronjob", "args": {}})).await.unwrap_err().to_string();
        assert!(err.contains("no tool named 'cronjob'") && err.contains("cronjob_manage") && !err.contains("disabled"), "{err}");
    }

    #[tokio::test]
    async fn call_args_that_are_not_an_object_are_an_error_not_an_empty_object() {
        let fake = Fake::new(true);
        let tool = FactrTool::with_host(fake.clone());
        let err = run(&tool, "hb-args", json!({"action": "call", "tool": "image_generate", "args": "a cat"})).await.unwrap_err().to_string();
        assert!(err.contains("args must be an object"), "{err}");
        assert!(!fake.calls.lock().unwrap().iter().any(|(_, path, _)| path == "/api/agent-tools/invoke"));
    }

    #[tokio::test]
    async fn list_fetches_from_the_backend_and_hides_native_tools() {
        let fake = Fake::new(true);
        let out = run(&FactrTool::with_host(fake.clone()), "hb-list", json!({"action": "list"})).await.unwrap();
        assert!(out.contains("image_generate: Make an image") && out.contains("cronjob_manage"));
        assert!(!out.contains("read_file") && !out.contains("web_search"), "native tools are not offered twice");
        for name in ["desktop_project", "manage_connections", "manage_catalog"] {
            assert!(out.contains(name), "{name} has no native equivalent");
        }
        assert!(!out.contains("focus_pane") && !out.contains("desktop_preview"), "desktop window tools cannot work here");
        assert!(!out.contains("browser_click") && out.contains("browser_vault_fill"), "only the vault escapes the browser toolset");
        assert_eq!(fake.calls.lock().unwrap()[0].1, "/api/agent-tools");
    }

    #[test]
    fn nothing_is_offered_twice_when_a_native_equivalent_exists() {
        for toolset in NATIVE_TOOLSETS {
            assert!(native("any_tool", toolset), "{toolset}");
        }
        for toolset in ["project", "connections", "setup"] {
            assert!(!native("any_tool", toolset), "{toolset} has no native equivalent");
        }
        assert!(!native("browser_vault_fill", "browser") && !native("browser_cdp", "browser-cdp"));
    }

    #[tokio::test]
    async fn desktop_window_tools_are_hidden_and_refused_but_other_tools_describe_and_call() {
        let fake = Fake::new(true);
        let tool = FactrTool::with_host(fake.clone());
        for name in ["focus_pane", "desktop_preview"] {
            let err = run(&tool, "hb-ui", json!({"action": "describe", "tool": name})).await.unwrap_err().to_string();
            assert!(err.contains("desktop window"), "{err}");
            assert!(run(&tool, "hb-ui", json!({"action": "call", "tool": name})).await.is_err());
        }
        assert!(!fake.calls.lock().unwrap().iter().any(|c| c.1.ends_with("invoke")), "never reached Python");
        let out = run(&tool, "hb-ui", json!({"action": "call", "tool": "manage_catalog", "args": {}})).await.unwrap();
        assert_eq!(out, "{\"ok\":true}");
        assert!(run(&tool, "hb-ui", json!({"action": "describe", "tool": "web_search"})).await.is_err(), "native duplicates stay hidden");
    }

    #[tokio::test]
    async fn describe_returns_the_backends_schema_and_call_runs_the_tool() {
        let fake = Fake::new(true);
        let tool = FactrTool::with_host(fake.clone());
        let schema = run(&tool, "hb-call", json!({"action": "describe", "tool": "image_generate"})).await.unwrap();
        assert!(schema.contains("\"name\": \"image_generate\""));
        let out = run(&tool, "hb-call", json!({"action": "call", "tool": "image_generate", "args": {"prompt": "a fox"}})).await.unwrap();
        assert_eq!(out, "{\"ok\":true}");
        let calls = fake.calls.lock().unwrap();
        let body = calls.last().unwrap().2.clone().unwrap();
        assert_eq!((body["name"].as_str(), body["args"]["prompt"].as_str(), body["session_id"].as_str()), (Some("image_generate"), Some("a fox"), Some("hb-call")));
    }

    #[tokio::test]
    async fn a_missing_backend_is_a_clean_error() {
        let down = FactrTool::with_host(Fake::new(false));
        let err = run(&down, "hb-down", json!({"action": "list"})).await.unwrap_err();
        assert_eq!(err.to_string(), UNAVAILABLE);
        let none = FactrTool { host: None };
        if installed().is_none() {
            assert_eq!(run(&none, "hb-none", json!({"action": "call", "tool": "x"})).await.unwrap_err().to_string(), UNAVAILABLE);
        }
    }

    #[tokio::test]
    async fn the_run_policy_denylist_blocks_the_tool_and_hides_it() {
        let fake = Fake::new(true);
        let tool = FactrTool::with_host(fake.clone());
        super::super::set_session_tool_policy("hb-deny", None, ["cronjob_manage".to_string()].into());
        let err = run(&tool, "hb-deny", json!({"action": "call", "tool": "cronjob_manage", "args": {"action": "create"}})).await.unwrap_err();
        assert!(err.to_string().contains("disabled"));
        assert!(!fake.calls.lock().unwrap().iter().any(|c| c.1.ends_with("invoke")), "the backend never ran it");
        // Naming the toolset in the denylist blocks its tools too, without a copied tool table.
        super::super::set_session_tool_policy("hb-deny", None, ["cronjob".to_string()].into());
        assert!(run(&tool, "hb-deny", json!({"action": "call", "tool": "cronjob_manage"})).await.is_err());
        super::super::set_session_tool_policy("hb-deny", None, ["cronjob_manage".to_string()].into());
        let listed = run(&tool, "hb-deny", json!({"action": "list"})).await.unwrap();
        assert!(!listed.contains("cronjob_manage") && listed.contains("image_generate"));
        super::super::clear_session_tool_policy("hb-deny");
    }

    #[tokio::test]
    async fn an_allowlist_limits_which_factr_tools_run() {
        let tool = FactrTool::with_host(Fake::new(true));
        super::super::set_session_tool_policy("hb-allow", Some(["factr".to_string(), "image_generate".to_string()].into()), Default::default());
        assert!(run(&tool, "hb-allow", json!({"action": "call", "tool": "image_generate"})).await.is_ok());
        assert!(run(&tool, "hb-allow", json!({"action": "call", "tool": "ha_call_service"})).await.is_err());
        super::super::set_session_tool_policy("hb-allow", Some(["factr".to_string(), "homeassistant".to_string()].into()), Default::default());
        assert!(run(&tool, "hb-allow", json!({"action": "call", "tool": "ha_call_service"})).await.is_ok(), "allowed through its toolset");
        super::super::clear_session_tool_policy("hb-allow");
    }

    #[tokio::test]
    async fn clarify_returns_the_answer_and_never_hangs_without_a_user() {
        let fake = Fake::new(true);
        let tool = ClarifyTool::with_host(fake.clone());
        *fake.clarify.lock().unwrap() = Some(ClarifyReply::Answer("blue".into()));
        let out = tool.execute(json!({"question": "Which colour?", "choices": ["blue", "red"]}), ctx("c1")).await.unwrap().output;
        assert_eq!(out, "The user answered: blue");
        // Headless: the host answers NoUser at once and the tool says so.
        let out = tool.execute(json!({"question": "Which colour?"}), ctx("c1")).await.unwrap().output;
        assert!(out.starts_with("No user is available"), "{out}");
        assert!(tool.execute(json!({"question": " "}), ctx("c1")).await.is_err());
        if installed().is_none() {
            let bare = ClarifyTool::new().execute(json!({"question": "q"}), ctx("c2")).await.unwrap().output;
            assert!(bare.starts_with("No user is available"));
        }
    }
}
