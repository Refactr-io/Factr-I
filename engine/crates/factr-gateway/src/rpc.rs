//! One WebSocket client: JSON-RPC 2.0 in, harness API calls out, harness
//! events translated back into Factr `event` notifications.

use crate::approvals::{Client, Hub};
use crate::learn::Trigger;
use crate::map::{self, Out, SessionState};
use crate::observability::Observer;
use crate::rewind;
use crate::{Config, LINK_PIPE_BYTES};
use anyhow::{Context, Result, anyhow, bail};
use futures_util::{SinkExt, StreamExt};
use factr_base::obs_sink::{Span, emit};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::{Mutex, mpsc, oneshot};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;

mod detached;
mod driver;
pub(crate) use driver::start as start_driver;
pub(crate) mod attach;
mod local_state;
mod settings;
pub(crate) use settings::{config_record, yolo_active};

/// `PUT /api/config`; the error is `(rejected as a bad request, message)`.
/// The `model.options` payload (also `GET /api/model/options`). `refresh` ("Refresh Models") has nothing
/// to re-fetch: the catalog is read fresh on every call (the account's cache, kept current by the
/// provider itself), so the current list is the refreshed one. `current_models`: the served provider's
/// models from a session's catalog, when one is named.
pub(crate) fn model_options(config: &Config, current_models: Vec<String>, include_unconfigured: bool) -> Value {
    let (model, provider) = crate::profile::effective_default(config);
    let mut options = provider_state::model_options(&provider, &model, current_models, include_unconfigured, &config.reasoning_efforts, &config.model);
    // The effort a new chat starts on: the saved pick, else the engine's configured default for the
    // provider. Lets a draft chat's effort pill show it before any session exists.
    if let Some(effort) = default_new_chat_effort(&provider) {
        options["reasoning_effort"] = json!(effort);
    }
    options
}

/// What a new chat on `provider` runs at when the request names no effort.
fn default_new_chat_effort(provider: &str) -> Option<String> {
    crate::profile::current().reasoning_effort.or_else(|| configured_default_effort(provider).map(str::to_owned))
}

pub(crate) fn put_config(home: &std::path::Path, config: &Value) -> Result<(), (bool, String)> {
    settings::put_config(home, config).map_err(|e| (e.code == 4002, e.message))
}
mod projects;
mod provider_state;
pub(crate) use provider_state::runtime_provider_id;
mod side_agents;
mod spawn_tree;
mod subagents;
mod tools_catalog;
mod toolsets;
mod undo_turn;

const HARNESS_CALL_TIMEOUT: Duration = Duration::from_secs(60);
/// How long a closing session (or a finished headless run) waits for its last learning review.
const DISPOSE_LEARNING_WAIT: Duration = Duration::from_secs(30);
/// How long `prompt.submit` waits for factr to acknowledge the message. The
/// turn itself streams afterwards and may run for minutes.
const ACCEPT_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_IN_FLIGHT: usize = 256;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;
const PARSE_ERROR: i64 = -32700;
const INTERNAL: i64 = -32603;

type Ws = WebSocketStream<TcpStream>;

fn foreign_handle(source: &str, path: &str) -> String {
    format!(
        "{:x}",
        Sha256::digest(format!("{source}:{path}").as_bytes())
    )
}

struct ForeignCandidate {
    source: &'static str,
    path: std::path::PathBuf,
    external_id: String,
    title: String,
    cwd: Option<String>,
    mtime: f64,
    turn_count: usize,
    excerpt: String,
}

fn foreign_candidates(source: Option<&str>) -> Result<Vec<ForeignCandidate>> {
    let mut out = Vec::new();
    let home = factr_base::platform::user_home_dir().unwrap_or_else(|| Path::new("/nonexistent").to_path_buf());
    let max_log_bytes = 32 * 1024 * 1024;
    if source.is_none_or(|s| s == "claude") {
        let root = home.join(".claude/projects").canonicalize().ok();
        for s in factr_base::import::list_claude_code_sessions()? {
            let Ok(path) = Path::new(&s.full_path).canonicalize() else {
                continue;
            };
            let Ok(meta) = path.metadata() else { continue };
            if meta.len() > max_log_bytes
                || root.as_ref().is_none_or(|root| !path.starts_with(root))
            {
                continue;
            }
            out.push(ForeignCandidate {
                source: "claude",
                path,
                external_id: s.session_id,
                title: s.summary.unwrap_or_else(|| s.first_prompt.clone()),
                cwd: s.project_path,
                mtime: s
                    .modified
                    .or(s.created)
                    .map(|t| t.timestamp() as f64)
                    .unwrap_or_default(),
                turn_count: s.message_count as usize,
                excerpt: s.first_prompt,
            });
        }
    }
    if source.is_none_or(|s| s == "codex") {
        let root = home.join(".codex/sessions").canonicalize().ok();
        let mut pending = root.iter().cloned().collect::<Vec<_>>();
        while let Some(dir) = pending.pop() {
            let Ok(entries) = std::fs::read_dir(dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let Ok(kind) = entry.file_type() else {
                    continue;
                };
                if kind.is_dir() {
                    pending.push(path);
                    continue;
                }
                if !kind.is_file() || path.extension().is_none_or(|e| e != "jsonl") {
                    continue;
                }
                let Ok(path) = path.canonicalize() else {
                    continue;
                };
                let Ok(meta) = path.metadata() else { continue };
                if meta.len() > max_log_bytes
                    || root.as_ref().is_none_or(|root| !path.starts_with(root))
                {
                    continue;
                }
                let Ok(Some(record)) = factr_base::import::load_codex_external_session(&path)
                else {
                    continue;
                };
                let first = record
                    .messages
                    .iter()
                    .find(|m| m.role == "user")
                    .map(|m| m.text.as_str())
                    .unwrap_or_default();
                out.push(ForeignCandidate {
                    source: "codex",
                    path,
                    external_id: record.session_id,
                    title: record.title.unwrap_or_else(|| {
                        first
                            .lines()
                            .next()
                            .unwrap_or_default()
                            .chars()
                            .take(180)
                            .collect()
                    }),
                    cwd: record.working_dir,
                    mtime: record.updated_at.timestamp() as f64,
                    turn_count: record.messages.len(),
                    excerpt: first.chars().take(200).collect(),
                });
            }
        }
    }
    out.sort_by(|a, b| {
        b.mtime
            .total_cmp(&a.mtime)
            .then_with(|| a.path.cmp(&b.path))
    });
    Ok(out)
}

fn foreign_turns(candidate: &ForeignCandidate) -> Result<Vec<Value>> {
    if candidate.source == "claude" {
        let session = factr_base::import::preview_claude_code_session_from_file(
            &candidate.path,
            &candidate.external_id,
        )?;
        return Ok(session.messages.into_iter().map(|m| json!({"role":serde_json::to_value(m.role).unwrap_or(Value::Null),"content":m.content.into_iter().filter_map(|b| match b { factr_base::message::ContentBlock::Text{text,..} => Some(text), _=>None }).collect::<Vec<_>>().join("\n")})).collect());
    }
    let record = factr_base::import::load_codex_external_session(&candidate.path)?
        .ok_or_else(|| anyhow!("unreadable Codex session"))?;
    Ok(record
        .messages
        .into_iter()
        .map(|m| json!({"role":m.role,"content":m.text}))
        .collect())
}

pub async fn close(mut ws: Ws, code: u16, reason: &str) -> Result<()> {
    let frame = CloseFrame {
        code: CloseCode::from(code),
        reason: reason.chars().take(120).collect::<String>().into(),
    };
    let _ = ws.close(Some(frame)).await;
    Ok(())
}

fn send_message_request(session_id: &str, text: &str, reminder: Option<&str>, images: Vec<(String, String)>) -> Value {
    json!({ "req": "send_message", "session_id": session_id, "content": text, "system_reminder": reminder, "images": images })
}

#[derive(Debug)]
struct RpcError {
    code: i64,
    message: String,
    data: Option<Value>,
}

impl RpcError {
    fn unsupported(method: &str) -> Self {
        Self {
            code: METHOD_NOT_FOUND,
            message: format!("{method} is not supported by this engine"),
            data: Some(json!({ "reason": "not_supported_by_engine", "method": method })),
        }
    }
    fn params(message: &str) -> Self {
        Self {
            code: INVALID_PARAMS,
            message: message.into(),
            data: None,
        }
    }
    fn internal(err: anyhow::Error) -> Self {
        Self {
            code: INTERNAL,
            message: err.to_string(),
            data: None,
        }
    }
}

/// A WebSocket to the Factr feature backend for one desktop connection.
struct Upstream {
    tx: mpsc::Sender<String>,
    pending: Mutex<HashMap<String, oneshot::Sender<Value>>>,
    tasks: Vec<tokio::task::AbortHandle>,
}

impl Drop for Upstream {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

/// Server requests from the backend are relayed to the desktop under this
/// prefix so their ids can never collide with ours.
const UPSTREAM_REQUEST_PREFIX: &str = "up-";
const FORWARD_TIMEOUT: Duration = Duration::from_secs(120);

static STORE_WARNED: std::sync::LazyLock<std::sync::Mutex<std::collections::HashSet<&'static str>>> =
    std::sync::LazyLock::new(Default::default);

/// A store failed to open: say so once (log line plus an error `status.update` to every window),
/// not on every retry. Goals and learning stay off until it opens again.
async fn store_unavailable(hub: &Hub, what: &'static str, err: &anyhow::Error) {
    error_once(hub, what, format!("The {what} store could not be opened, so those features are off: {err:#}")).await;
}

/// One log line and one error `status.update` per `key` per process.
async fn error_once(hub: &Hub, key: &'static str, text: String) {
    if !STORE_WARNED.lock().unwrap_or_else(|e| e.into_inner()).insert(key) {
        return;
    }
    eprintln!("factr: {text}");
    let event = json!({ "jsonrpc": "2.0", "method": "event", "params": { "type": "status.update", "payload": { "kind": "error", "text": text } } });
    hub.broadcast_text(event.to_string()).await;
}

/// Log `what` failing to open once per process (no window to tell: callers without a `Conn`).
/// True the first time.
fn log_once(what: &'static str, err: &anyhow::Error) -> bool {
    let first = STORE_WARNED.lock().unwrap_or_else(|e| e.into_inner()).insert(what);
    if first {
        eprintln!("factr: {what} unavailable: {err:#}");
    }
    first
}

/// The entry store, or None after logging once why (for callers that have no `Conn` to report through).
pub(crate) fn entries_or_log(home: &str) -> Option<Arc<factr_learn::entries::EntryStore>> {
    factr_learn::entries::EntryStore::open_cached(Path::new(home)).map_err(|e| log_once("learning store (no window)", &e)).ok()
}

/// The goal-control store, or None after logging once why.
pub(crate) fn control_or_log(home: &str) -> Option<Arc<factr_learn::agent_loop::ControlStore>> {
    factr_learn::agent_loop::ControlStore::open_cached(Path::new(home)).map_err(|e| log_once("goal control store (no window)", &e)).ok()
}

/// The daily database backup failed (`migrate::backup_error`): say so once.
pub(crate) async fn report_backup_error(hub: &Hub, err: Option<String>) {
    if let Some(err) = err {
        error_once(hub, "backup", format!("The daily database backup failed: {err}")).await;
    }
}

/// `session.active_list` status of a session with a turn in flight (Factr `_session_live_status`).
/// `config.set model` value `"<model> [--provider <slug>] [--session] [--global]"` as
/// `(model, provider, session_only)`. Only `--session` keeps the pick from becoming the default.
fn parse_model_pick(value: &str) -> Result<(String, Option<String>, bool), RpcError> {
    let mut words = value.split_whitespace();
    let model = words
        .next()
        .filter(|model| !model.starts_with('-'))
        .ok_or_else(|| RpcError::params("model value must start with a model name"))?;
    let (mut provider, mut session_only) = (None, false);
    while let Some(option) = words.next() {
        match option {
            "--provider" => provider = words.next().map(str::to_owned),
            "--session" => session_only = true,
            "--global" => session_only = false,
            _ => {}
        }
    }
    // MoA presets are a transient orchestration choice, never a default.
    let session_only = session_only || provider.as_deref().is_some_and(|p| p.eq_ignore_ascii_case("moa"));
    Ok((model.to_owned(), provider, session_only))
}

fn live_status(waiting_for_input: bool) -> &'static str {
    if waiting_for_input { "waiting" } else { "working" }
}

/// The engine refused an attach because another connection owns the session.
fn is_already_live(message: &str) -> bool {
    message.contains("is already live")
}

/// The reasoning effort the engine's own config applies to `provider` when the user has picked none
/// (`provider.openai_reasoning_effort`, `provider.anthropic_reasoning_effort`).
fn configured_default_effort(provider: &str) -> Option<&'static str> {
    let config = &factr_base::config::config().provider;
    // Callers may pass a display name ("OpenAI"): normalise to the provider id.
    let id = provider.trim().to_lowercase().replace([' ', '_'], "-");
    let configured = match id.as_str() {
        "openai" | "openai-codex" => config.openai_reasoning_effort.as_deref(),
        "anthropic" | "claude" => config.anthropic_reasoning_effort.as_deref(),
        _ => None,
    };
    configured.and_then(factr_provider_core::canonical_reasoning_effort).filter(|effort| *effort != "none")
}

/// A `/plan` or `/learn` send dispatch shows "Plan: <task>" / "Learn: <request>" as the user's bubble
/// (the model still gets the scaffolding); other answers pass through.
fn label_send(name: &str, arg: &str, mut done: Value) -> Value {
    if matches!(name.trim_start_matches('/'), "plan" | "learn")
        && done["type"] == "send"
        && done["display"].as_str().is_none_or(str::is_empty)
    {
        done["display"] = json!(map::display_text(&format!("/{} {arg}", name.trim_start_matches('/'))));
    }
    done
}

/// A session made by the user's `/branch`: the engine appends a fork notice to its history.
fn is_user_branch(session: &factr_base::session::Session) -> bool {
    session.parent_id.is_some()
        && session.messages.iter().any(|m| {
            m.content.iter().any(|b| matches!(b, factr_base::message::ContentBlock::Text { text, .. } if text.contains("This session was forked (split) from session")))
        })
}

/// A stored chat as the engine's `get_history` reply shapes it (the rows the engine renders).
fn stored_history(id: &str) -> Result<Value> {
    let stored = factr_base::session::Session::load(id)?;
    let messages: Vec<Value> = factr_base::session::render_messages(&stored)
        .into_iter()
        .map(|m| {
            let mut row = json!({ "role": m.role, "content": m.content });
            if let Some(tool) = m.tool_data {
                row["tool_name"] = json!(tool.name);
            }
            row
        })
        .collect();
    Ok(json!({ "session_id": id, "messages": messages }))
}

/// Whether any child of `parent` (other than `except`) is running: the engine's own status
/// (whichever connection started the child) or a turn this connection sees running.
fn children_running(list: &[Value], parent: &str, except: Option<&str>, live: &HashMap<String, SessionState>) -> bool {
    list.iter().any(|s| {
        let id = s["session_id"].as_str().unwrap_or_default();
        s["parent_session_id"].as_str() == Some(parent)
            && Some(id) != except
            && (["status", "swarm_status"].iter().any(|f| matches!(s[*f].as_str(), Some("running" | "processing")))
                || live.get(id).is_some_and(SessionState::turn_active))
    })
}

/// Harness request ids, unique across connections: a link taken over by another window
/// (`detached.rs`) may still carry replies to its old connection's requests.
static NEXT_HARNESS_ID: AtomicU64 = AtomicU64::new(1);

/// The tasks pumping one bridge link, kept with the link they serve.
#[derive(Clone)]
struct LinkTasks {
    /// Which link these pump.
    link: mpsc::WeakSender<String>,
    /// The connection the reader hands frames to; a window that takes over a closed window's running
    /// turn repoints it (see `detached.rs`). Held while a frame is handled.
    owner: Arc<Mutex<std::sync::Weak<Conn>>>,
    bridge: tokio::task::AbortHandle,
    writer: tokio::task::AbortHandle,
    reader: tokio::task::AbortHandle,
}

impl LinkTasks {
    fn abort(&self) {
        for task in [&self.bridge, &self.writer, &self.reader] {
            task.abort();
        }
    }

    fn is_finished(&self) -> bool {
        [&self.bridge, &self.writer, &self.reader].iter().all(|task| task.is_finished())
    }
}

pub(crate) struct Conn {
    config: Arc<Config>,
    to_ws: mpsc::Sender<Message>,
    /// Control link: session-less requests (list, ping).
    control: Mutex<Option<mpsc::Sender<String>>>,
    /// One bridge link per session: factr's API bridge attaches exactly one
    /// session per connection, while the desktop multiplexes many.
    links: Mutex<HashMap<String, mpsc::Sender<String>>>,
    link_tasks: Mutex<Vec<LinkTasks>>,
    /// factr `SessionInfo` for sessions this client created or attached, so a
    /// new, still-empty session (not yet persisted) appears in `session.list`.
    known: Mutex<HashMap<String, Value>>,
    /// Sessions created here that have not had a turn yet, so factr has not
    /// persisted them; only these are merged into `session.list` from `known`.
    fresh: Mutex<std::collections::HashSet<String>>,
    /// The Factr backend's `commands.catalog`, fetched once for the popup's quick, plugin and bundle rows.
    backend_catalog: tokio::sync::OnceCell<Value>,
    /// Per chat, the timer that extracts memories once the chat has been quiet for `IDLE_GRACE`.
    idle_extract: std::sync::Mutex<HashMap<String, tokio::task::JoinHandle<()>>>,
    /// Test seam: answers `forward` in place of the Factr backend.
    #[cfg(test)]
    forward_stub: std::sync::Mutex<Option<ForwardStub>>,
    /// Sessions with a learning pass running (std mutex: the guard releases on drop, even when the pass is cancelled).
    learning_now: std::sync::Mutex<std::collections::HashSet<String>>,
    /// One attach at a time per connection: two resumes of one id must not race the engine's
    /// one-live-owner rule against each other.
    attach_lock: Mutex<()>,
    /// Sessions whose context was compacted and not yet reviewed (factr-learn's `_compactAutoRefinePending`).
    compact_pending: std::sync::Mutex<std::collections::HashSet<String>>,
    /// Learning passes started here; a closing window and a headless run (a tool-requested `refine`
    /// still runs there) wait for them before closing their links.
    learning_tasks: std::sync::Mutex<Vec<tokio::task::JoinHandle<()>>>,
    /// The engine-level driver's link (see `driver.rs`), not a desktop window.
    driver: bool,
    /// Sessions this connection lets go of once their running turn ends: dropping a link mid-turn
    /// makes the engine abort the turn as crashed (the driver yielding to a window, a goal cleared).
    release_after_turn: std::sync::Mutex<std::collections::HashSet<String>>,
    /// A desktop window's connection (`run`): only a window takes over a closed window's turn.
    window: AtomicBool,
    /// The window closed: this connection only carries the turns it was running to their end.
    detached: AtomicBool,
    next_server_request: AtomicU64,
    pending: Mutex<HashMap<u64, oneshot::Sender<Value>>>,
    sessions: Mutex<HashMap<String, SessionState>>,
    /// Delegated children this connection follows (see `subagents.rs`), by child session id.
    children: Mutex<HashMap<String, subagents::Child>>,
    /// Factr server-request id → (session, factr permission request id).
    approvals: Mutex<HashMap<String, (String, String)>>,
    in_flight: Arc<tokio::sync::Semaphore>,
    /// `prompt.submit` callers waiting for factr's `message_accepted`.
    accept_waiters: Mutex<HashMap<String, Vec<oneshot::Sender<()>>>>,
    /// Connection to Factr's Python backend, opened on first forwarded call.
    upstream: Mutex<Option<Arc<Upstream>>>,
    next_forward: AtomicU64,
    hub: Arc<Hub>,
    /// This connection as seen by the approval hub.
    client: Arc<Client>,
    pub(crate) observer: Arc<Observer>,
    run_kind: &'static str,
    run_title: Option<String>,
    replay_of: Option<String>,
}

impl Conn {
    #[allow(clippy::too_many_arguments)]
    fn new(
        config: Arc<Config>,
        to_ws: mpsc::Sender<Message>,
        hub: Arc<Hub>,
        client: Arc<Client>,
        observer: Arc<Observer>,
        driver: bool,
        run_kind: &'static str,
        run_title: Option<String>,
        replay_of: Option<String>,
    ) -> Arc<Self> {
        Arc::new(Self {
            config,
            to_ws,
            control: Mutex::new(None),
            links: Mutex::new(HashMap::new()),
            link_tasks: Mutex::new(Vec::new()),
            known: Mutex::new(HashMap::new()),
            fresh: Mutex::new(Default::default()),
            backend_catalog: Default::default(),
            idle_extract: Default::default(),
            #[cfg(test)]
            forward_stub: Default::default(),
            learning_now: Default::default(),
            attach_lock: Default::default(),
            compact_pending: Default::default(),
            learning_tasks: Default::default(),
            driver,
            release_after_turn: Default::default(),
            window: AtomicBool::new(false),
            detached: AtomicBool::new(false),
            next_server_request: AtomicU64::new(1),
            pending: Mutex::new(HashMap::new()),
            sessions: Mutex::new(HashMap::new()),
            children: Mutex::new(HashMap::new()),
            approvals: Mutex::new(HashMap::new()),
            in_flight: Arc::new(tokio::sync::Semaphore::new(MAX_IN_FLIGHT)),
            accept_waiters: Mutex::new(HashMap::new()),
            upstream: Mutex::new(None),
            next_forward: AtomicU64::new(1),
            hub,
            client,
            observer,
            run_kind,
            run_title,
            replay_of,
        })
    }

    /// Send one harness request and await its direct reply.
    async fn call(&self, request: Value) -> Result<Value> {
        let link = self.route(&request).await?;
        self.call_on(&link, request).await
    }

    async fn call_on(&self, link: &mpsc::Sender<String>, request: Value) -> Result<Value> {
        let (id, rx) = self.send_on(link, request).await?;
        let Ok(reply) = tokio::time::timeout(HARNESS_CALL_TIMEOUT, rx).await else {
            self.pending.lock().await.remove(&id);
            bail!("engine did not reply in time");
        };
        check_reply(reply.map_err(|_| anyhow!("engine connection closed"))?)
    }

    /// The link a request belongs on: its session's link, else the control link.
    async fn route(&self, request: &Value) -> Result<mpsc::Sender<String>> {
        if let Some(sid) = request["session_id"].as_str() {
            if let Some(link) = self.links.lock().await.get(sid).filter(|l| !l.is_closed()) {
                return Ok(link.clone());
            }
        }
        self.control
            .lock()
            .await
            .clone()
            .filter(|l| !l.is_closed())
            .ok_or_else(|| anyhow!("engine connection closed"))
    }

    /// Send one harness request; its reply arrives on the returned receiver (the id is the key in
    /// `pending`, for a caller that gives up waiting).
    async fn send_on(
        &self,
        link: &mpsc::Sender<String>,
        request: Value,
    ) -> Result<(u64, oneshot::Receiver<Value>)> {
        let id = NEXT_HARNESS_ID.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);
        let mut frame = request;
        frame["v"] = json!(1);
        frame["id"] = json!(id);
        if link.send(frame.to_string()).await.is_err() {
            self.pending.lock().await.remove(&id);
            bail!("engine connection closed");
        }
        Ok((id, rx))
    }

    /// Open a new in-process bridge link and start pumping its frames.
    async fn open_link(self: &Arc<Self>) -> Result<mpsc::Sender<String>> {
        let (ours, theirs) = tokio::io::duplex(LINK_PIPE_BYTES);
        let (their_read, their_write) = tokio::io::split(theirs);
        let bridge = tokio::spawn(factr_harness_api_server::run_bridge_stream(
            their_read,
            their_write,
            self.config.legacy_socket.clone(),
        ));
        let (our_read, mut our_write) = tokio::io::split(ours);
        let hello = json!({"v": 1, "id": 0, "req": "hello", "min_version": 1, "max_version": 1, "client": "factr-gateway"});
        our_write.write_all(format!("{hello}\n").as_bytes()).await?;
        let mut lines = BufReader::new(our_read).lines();
        let hello_ok: Value = serde_json::from_str(
            &lines
                .next_line()
                .await?
                .ok_or_else(|| anyhow!("engine closed"))?,
        )?;
        if hello_ok["ev"] != "hello_ok" {
            bridge.abort();
            return Err(anyhow!("engine unavailable"));
        }
        let (tx, mut rx) = mpsc::channel::<String>(256);
        let writer = tokio::spawn(async move {
            while let Some(line) = rx.recv().await {
                if our_write
                    .write_all(format!("{line}\n").as_bytes())
                    .await
                    .is_err()
                {
                    break;
                }
            }
            // Every sender dropped (the chat closed): end-of-input lets the bridge finish, and the
            // engine then runs its disconnect cleanup and drops the session's agent.
            let _ = our_write.shutdown().await;
        });
        // Weak: a reader that kept a strong sender would keep the writer, the bridge and the agent alive.
        let origin = Arc::downgrade(self);
        let owner = Arc::new(Mutex::new(origin.clone()));
        let mine = tx.downgrade();
        let reader_owner = owner.clone();
        let reader = tokio::spawn(async move {
            while let Ok(Some(line)) = lines.next_line().await {
                let Ok(frame) = serde_json::from_str::<Value>(&line) else { continue };
                let current = reader_owner.lock().await;
                let Some(conn) = current.upgrade() else { return };
                // A reply to a request sent before another window took this link over.
                let frame = match origin.upgrade().filter(|first| !Arc::ptr_eq(first, &conn)) {
                    Some(first) => first.take_reply(frame).await,
                    None => Some(frame),
                };
                if let Some(frame) = frame {
                    conn.on_harness_frame(frame).await;
                }
            }
            // The bridge closed: forget this link so the next attach or tick opens a fresh one.
            let conn = reader_owner.lock().await.upgrade();
            if let (Some(conn), Some(link)) = (conn, mine.upgrade()) {
                conn.link_lost(&link).await;
            }
        });
        let mut tasks = self.link_tasks.lock().await;
        tasks.retain(|task| !task.is_finished());
        tasks.push(LinkTasks {
            link: tx.downgrade(),
            owner,
            bridge: bridge.abort_handle(), writer: writer.abort_handle(), reader: reader.abort_handle() });
        Ok(tx)
    }

    /// Forget a session's link and per-session state. The bridge ends once the last sender drops,
    /// and the engine's disconnect cleanup then drops the agent. A chat that has had no turn is not
    /// on disk yet, so `known` keeps it for `session.list`.
    async fn forget_link(&self, sid: &str) {
        self.release_after_turn.lock().unwrap_or_else(|e| e.into_inner()).remove(sid);
        if self.is_detached() {
            detached::forget(sid, self);
        }
        self.links.lock().await.remove(sid);
        self.sessions.lock().await.remove(sid);
        // The chat's followed children go with it: their links only fed its subagent rows.
        let followed: Vec<String> = {
            let mut children = self.children.lock().await;
            let followed = children.iter().filter(|(_, row)| row.parent == sid).map(|(child, _)| child.clone()).collect();
            children.retain(|child, row| child != sid && row.parent != sid);
            followed
        };
        for child in followed {
            self.unfollow(&child).await;
        }
        if !self.fresh.lock().await.contains(sid) {
            self.known.lock().await.remove(sid);
        }
    }

    /// Let go of `sid` now when it is idle, else at the end of its running turn: the engine aborts a
    /// turn whose link drops as crashed, and the lost `message.complete` would leave it busy here.
    pub(super) async fn release_at_turn_end(&self, sid: &str) {
        // Marked before the check: a turn ending in between is then let go by one side or the other.
        self.release_after_turn.lock().unwrap_or_else(|e| e.into_inner()).insert(sid.to_string());
        self.release_if_turn_over(sid).await;
    }

    /// A frame for `sid` arrived: if it ended a turn this connection was waiting out, let go now.
    /// True when it let go.
    async fn release_if_turn_over(&self, sid: &str) -> bool {
        if !self.release_after_turn.lock().unwrap_or_else(|e| e.into_inner()).contains(sid) {
            return false;
        }
        if self.sessions.lock().await.get(sid).is_some_and(SessionState::turn_active) {
            return false;
        }
        self.forget_link(sid).await;
        true
    }

    /// Everything per-session that a closed or deleted chat leaves in this process: the observer's
    /// bookkeeping and replay buffer, and the undo lock. (The engine drops the agent's own state when
    /// the link ends.)
    fn forget_session(&self, sid: &str) {
        self.observer.forget_session(sid);
        crate::undo::forget_lock(sid);
    }

    /// A bridge link closed: forget it, and end the turns that were running on it, whose
    /// `message.complete` is lost. Otherwise `busy` stays set and the driver skips the goal forever.
    async fn link_lost(&self, link: &mpsc::Sender<String>) {
        let lost: Vec<String> = {
            let mut links = self.links.lock().await;
            let lost = links.iter().filter(|(_, l)| l.same_channel(link)).map(|(sid, _)| sid.clone()).collect();
            links.retain(|_, l| !l.same_channel(link));
            lost
        };
        {
            let mut control = self.control.lock().await;
            if control.as_ref().is_some_and(|l| l.same_channel(link)) {
                *control = None;
            }
        }
        for sid in lost {
            self.release_after_turn.lock().unwrap_or_else(|e| e.into_inner()).remove(&sid);
            if self.is_detached() {
                detached::forget(&sid, self);
            }
            let was_running = self.sessions.lock().await.get_mut(&sid).is_some_and(SessionState::end_turn);
            // A driver that yields the session to a window leaves the run to that window.
            let closed = !(self.driver && self.hub.has_window(&sid).await) && self.observer.has_active_run(&sid);
            if closed {
                self.observer.event(&sid, "message.complete", &json!({ "status": "interrupted", "text": "" }));
            }
            if was_running && !self.driver {
                self.emit("message.complete", Some(&sid), json!({ "status": "interrupted", "text": "" })).await;
            }
            if self.driver && (was_running || closed) {
                driver::resume_soon(&sid);
            }
        }
        driver::poke();
    }

    async fn control_store(&self) -> Option<Arc<factr_learn::agent_loop::ControlStore>> {
        match factr_learn::agent_loop::ControlStore::open_cached(Path::new(&self.config.home)) {
            Ok(store) => Some(store),
            Err(err) => {
                store_unavailable(&self.hub, "goal control", &err).await;
                None
            }
        }
    }

    async fn entry_store(&self) -> Option<Arc<factr_learn::entries::EntryStore>> {
        match factr_learn::entries::EntryStore::open_cached(Path::new(&self.config.home)) {
            Ok(store) => Some(store),
            Err(err) => {
                store_unavailable(&self.hub, "learning", &err).await;
                None
            }
        }
    }

    /// Open the control link when there is none or its bridge has closed.
    async fn ensure_control(self: &Arc<Self>) -> Result<()> {
        if self.control.lock().await.as_ref().is_some_and(|l| !l.is_closed()) {
            return Ok(());
        }
        let link = self.open_link().await?;
        *self.control.lock().await = Some(link);
        Ok(())
    }

    /// Attach this connection to `session_id` once.
    async fn ensure_attached(self: &Arc<Self>, session_id: &str) -> Result<Value> {
        let _one_at_a_time = self.attach_lock.lock().await;
        let mut tries = 0;
        loop {
            match self.attach_once(session_id).await {
                // The engine allows one live owner. The goal driver holds the sessions it works on;
                // a window that wants one takes it over (the driver yields to windows), so a
                // relaunch that reopens a goal chat does not meet "already live" forever.
                Err(err) if !self.driver && tries < 6 && is_already_live(&format!("{err:#}")) => {
                    tries += 1;
                    driver::release(session_id).await;
                    tokio::time::sleep(Duration::from_millis(300 * tries)).await;
                }
                other => return other,
            }
        }
    }

    async fn attach_once(self: &Arc<Self>, session_id: &str) -> Result<Value> {
        // A closed window's connection only finishes the turns it holds: a new link would outlive them.
        if self.is_detached() {
            bail!("the window that held this chat has closed");
        }
        let held = self.links.lock().await.get(session_id).is_some_and(|l| !l.is_closed());
        if held || self.adopt(session_id).await {
            let info = self.known.lock().await.get(session_id).cloned().unwrap_or(Value::Null);
            return Ok(json!({ "session": info }));
        }
        let link = self.open_link().await?;
        match self
            .call_on(
                &link,
                json!({ "req": "attach_session", "session_id": session_id }),
            )
            .await
        {
            Ok(reply) => {
                self.links.lock().await.insert(session_id.to_string(), link);
                self.client
                    .sessions
                    .lock()
                    .await
                    .insert(session_id.to_string());
                // A turn the engine still runs (the old link only dropped) must not be doubled.
                if reply["session"]["status"].as_str() == Some("processing") {
                    self.sessions.lock().await.entry(session_id.to_string()).or_default().mark_running();
                }
                if reply["session"].is_object() {
                    self.known
                        .lock()
                        .await
                        .insert(session_id.to_string(), reply["session"].clone());
                }
                self.adopt_old_session(session_id).await;
                Ok(reply)
            }
            Err(err) => Err(err),
        }
    }

    /// A session attached for the first time in this process that pre-dates the engine has no
    /// extraction marker or learning watermark: stamp both at its current length, so only messages
    /// that arrive from now on are extracted or reviewed (closing or deleting it costs no model call).
    async fn adopt_old_session(self: &Arc<Self>, session_id: &str) {
        let id = session_id.to_string();
        let _ = tokio::task::spawn_blocking(move || {
            if let Ok(stored) = factr_base::session::Session::load(&id) {
                factr_base::memory_extract::adopt_session(&id, stored.messages.len());
            }
        })
        .await;
        let Some(store) = self.entry_store().await else { return };
        if store.watermark(session_id) > 0 {
            return;
        }
        if let Ok(history) = self.call(json!({ "req": "get_history", "session_id": session_id })).await {
            let seen = history["messages"].as_array().map_or(0, Vec::len);
            if seen > 0 {
                let _ = store.set_watermark(session_id, seen);
            }
        }
    }

    /// Send a message and wait until factr acknowledges it (or rejects it).
    /// Send a user message. `reminder` rides the turn's uncached system-reminder slot (not the
    /// cached static prefix, not the transcript) and lasts for this turn only.
    async fn submit(self: &Arc<Self>, session_id: &str, text: &str, reminder: Option<&str>, images: Vec<(String, String)>) -> Result<()> {
        self.ensure_attached(session_id).await?;
        let (accepted_tx, accepted_rx) = oneshot::channel();
        self.accept_waiters
            .lock()
            .await
            .entry(session_id.to_string())
            .or_default()
            .push(accepted_tx);
        let link = self.route(&json!({ "session_id": session_id })).await?;
        let (_, reply) = self
            .send_on(
                &link,
                send_message_request(session_id, text, reminder, images),
            )
            .await?;
        tokio::select! {
            _ = accepted_rx => Ok(()),
            reply = reply => match reply {
                Ok(reply) => check_reply(reply).map(|_| ()),
                Err(_) => Err(anyhow!("engine connection closed")),
            },
            _ = tokio::time::sleep(ACCEPT_TIMEOUT) => Err(anyhow!("engine did not accept the message in time")),
        }
    }

    async fn send_json(&self, value: Value) {
        let _ = self.to_ws.send(Message::Text(value.to_string())).await;
    }

    async fn emit(&self, ty: &str, session_id: Option<&str>, payload: Value) {
        let mut params = json!({ "type": ty, "payload": payload });
        if let Some(sid) = session_id {
            params["session_id"] = json!(sid);
            params = self.observer.replay_event(params);
        }
        self.send_json(json!({ "jsonrpc": "2.0", "method": "event", "params": params }))
            .await;
    }

    /// Hand `frame` to the request of ours it answers; `None` once it is delivered.
    async fn take_reply(&self, frame: Value) -> Option<Value> {
        let waiter = match frame["reply_to"].as_u64() {
            Some(id) => self.pending.lock().await.remove(&id),
            None => None,
        };
        match waiter {
            Some(tx) => {
                let _ = tx.send(frame);
                None
            }
            None => Some(frame),
        }
    }

    async fn on_harness_frame(self: &Arc<Self>, frame: Value) {
        if std::env::var_os("FACTR_GATEWAY_TRACE").is_some() {
            eprintln!(
                "factr-gateway: harness {}",
                frame.to_string().chars().take(300).collect::<String>()
            );
        }
        if frame["ev"] == "message_accepted" {
            if let Some(sid) = frame["session_id"].as_str() {
                for waiter in self
                    .accept_waiters
                    .lock()
                    .await
                    .remove(sid)
                    .unwrap_or_default()
                {
                    let _ = waiter.send(());
                }
            }
        }
        let Some(frame) = self.take_reply(frame).await else { return };
        // The driver only keeps its session state while a window has the
        // session open: the window observes, renders and prompts for it.
        let yields = self.driver
            && match frame["session_id"].as_str() {
                Some(sid) => self.hub.has_window(sid).await,
                None => false,
            };
        if !yields {
            self.observer.harness_event(&frame);
        }
        let outs = map::map_event(&frame, &mut *self.sessions.lock().await);
        // A closed window's turn that just ended: its chat is reviewed as the window's close would have.
        let released = match frame["session_id"].as_str() {
            Some(sid) => self.release_if_turn_over(sid).await && self.is_detached(),
            None => false,
        };
        if yields {
            return;
        }
        let outs = match frame["session_id"].as_str() {
            Some(sid) if self.children.lock().await.contains_key(sid) => self.child_events(sid, outs).await,
            _ => outs,
        };
        for out in outs {
            match out {
                Out::Event {
                    ty,
                    session_id,
                    payload,
                } => {
                    self.observer.event(&session_id, ty, &payload);
                    let completed = ty == "message.complete";
                    // Tool-call boundaries are the only natural checkpoint
                    // inside a busy, possibly long, multi-tool-call turn:
                    // `message.complete` only fires once at the very end.
                    // Check for a due RLM "steer" heartbeat there so it can
                    // reach the session at its next turn boundary instead of
                    // waiting for the whole turn to finish.
                    if ty == "tool.complete" {
                        // Rows are for a window; the driver renders nothing, and a link it held to the
                        // child would be dropped as stale by its next scan.
                        if !self.driver && !self.is_detached() && let Some(child) = subagents::spawned_child(payload["name"].as_str().unwrap_or(""), payload["result_text"].as_str().unwrap_or("")) {
                            tokio::spawn(self.clone().follow_child(session_id.clone(), child.to_string(), payload["args"].clone()));
                        }
                        factr_learn::agent_loop::observe_tool(
                            &session_id,
                            payload["name"].as_str().unwrap_or(""),
                            &payload["args"],
                            payload["result_text"].as_str().unwrap_or(""),
                        );
                        self.clone().maybe_steer_heartbeat(session_id.clone());
                    }
                    let payload_for_loop = completed.then(|| payload.clone());
                    let compacted = ty == "status.update" && payload["kind"] == "compress";
                    self.emit(ty, Some(&session_id), payload).await;
                    if compacted {
                        // factr-learn reviews after a compaction whatever the count, once the cooldown allows.
                        self.compact_pending.lock().unwrap_or_else(|e| e.into_inner()).insert(session_id.clone());
                        self.schedule_learning(session_id.clone(), Trigger::Compact);
                    }
                    if let Some(loop_payload) = payload_for_loop {
                        self.arm_idle_extraction(&session_id, factr_base::memory_extract::IDLE_GRACE, Arc::new(factr_base::memory_extract::extract_idle));
                        self.schedule_learning(session_id.clone(), if released { Trigger::Dispose } else { Trigger::TurnInterval });
                        driver::turn_done(self.clone(), session_id, loop_payload);
                    }
                }
                Out::Approval {
                    session_id,
                    request_id,
                    tool_name,
                    description,
                } => {
                    if self.driver || self.hub.is_headless(&session_id).await {
                        // Unattended (`/api/agent/run`, or the driver with no window open): no prompt waits;
                        // Factr's approval config decides, else deny and park it for the desktop.
                        let conn = self.clone();
                        tokio::spawn(async move {
                            let choice = conn.hub.unattended(&session_id, &tool_name, &description, "", None).await;
                            let _ = conn.resolve_approval(&session_id, &request_id, &choice).await;
                        });
                        continue;
                    }
                    let id = format!(
                        "srv-{}",
                        self.next_server_request.fetch_add(1, Ordering::Relaxed)
                    );
                    self.approvals
                        .lock()
                        .await
                        .insert(id.clone(), (session_id.clone(), request_id.clone()));
                    self.send_json(json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "method": "approval",
                        "params": {
                            "session_id": session_id,
                            "request_id": request_id,
                            "command": description,
                            "description": description,
                            "tool_name": tool_name,
                            "choices": ["once", "session", "always", "deny"],
                            "allow_permanent": true,
                            "allow_session": true,
                        }
                    }))
                    .await;
                }
            }
        }
    }

    pub(crate) fn config(&self) -> &Config {
        &self.config
    }

    /// Full conversation of `session`: from the engine when this connection holds the chat, else
    /// from disk. Attaching just to read would reopen a chat the user closed (a dispose review runs
    /// after the close), and nothing would close it again.
    pub(crate) async fn history(self: &Arc<Self>, session: &str) -> Result<Value> {
        if self.links.lock().await.get(session).is_some_and(|l| !l.is_closed()) {
            return self.call(json!({ "req": "get_history", "session_id": session })).await;
        }
        let id = session.to_string();
        tokio::task::spawn_blocking(move || stored_history(&id)).await?
    }

    pub(crate) async fn session_cwd(&self, session: &str) -> Option<String> {
        self.known
            .lock()
            .await
            .get(session)
            .and_then(|info| info["working_dir"].as_str().map(str::to_string))
    }

    async fn set_session_cwd(
        self: &Arc<Self>,
        session: &str,
        raw: &str,
    ) -> Result<Value, RpcError> {
        let cwd = std::fs::canonicalize(raw).map_err(|e| RpcError::internal(anyhow!(e)))?;
        if !cwd.is_dir() {
            return Err(RpcError::params("cwd must be an existing directory"));
        }
        if self
            .sessions
            .lock()
            .await
            .get(session)
            .is_some_and(SessionState::turn_active)
        {
            return Err(RpcError {
                code: 409,
                message: "session busy".into(),
                data: None,
            });
        }
        self.ensure_attached(session)
            .await
            .map_err(RpcError::internal)?;
        self.call(json!({ "req": "set_working_dir", "session_id": session, "working_dir": cwd }))
            .await
            .map_err(RpcError::internal)?;
        let cwd = cwd.to_string_lossy().into_owned();
        let mut known = self.known.lock().await;
        let info = known
            .entry(session.to_string())
            .or_insert_with(|| json!({"session_id": session}));
        info["working_dir"] = json!(cwd);
        let result = json!({
            "model": self.config.model, "provider": self.config.provider, "cwd": cwd,
            "running": false, "stored_session_id": session, "desktop_contract": 8,
        });
        drop(known);
        self.emit("session.info", Some(session), result.clone())
            .await;
        Ok(result)
    }

    /// The chat's model, provider or effort changed (a pick the engine accepted): remember it in the
    /// stored session info and tell the client, so a later `session.info` or `session.resume` reports
    /// the pick instead of the default the chat started on.
    async fn set_pick(&self, session: &str, picks: Value) {
        let info = {
            let mut known = self.known.lock().await;
            let info = known.entry(session.to_string()).or_insert_with(|| json!({}));
            for (key, value) in picks.as_object().into_iter().flatten().filter(|(_, value)| !value.is_null()) {
                info[key] = value.clone();
            }
            info.clone()
        };
        let mut event = json!({ "stored_session_id": session, "desktop_contract": 8 });
        for key in ["model", "provider", "reasoning_effort"] {
            if !info[key].is_null() {
                event[key] = info[key].clone();
            }
        }
        self.emit("session.info", Some(session), event).await;
    }

    /// `info` with the chat's accepted picks (effort, provider) laid over what the engine reports.
    async fn with_picks(&self, session: &str, mut info: Value) -> Value {
        if let Some(known) = self.known.lock().await.get(session) {
            for key in ["provider", "reasoning_effort"] {
                if !known[key].is_null() {
                    info[key] = known[key].clone();
                }
            }
        }
        info
    }

    /// A finished turn starts the chat's idle clock: `fire` runs once, `grace` after the last turn
    /// ended, unless a new prompt, a close or a delete comes first. This is how a chat the user walked
    /// away from, or left by starting another one, still has its tail extracted without a close.
    fn arm_idle_extraction(self: &Arc<Self>, session: &str, grace: Duration, fire: Arc<dyn Fn(&str) + Send + Sync>) {
        let (conn, id) = (self.clone(), session.to_string());
        let timer = tokio::spawn(async move {
            tokio::time::sleep(grace).await;
            let id_for_fire = id.clone();
            let busy = conn.sessions.lock().await.get(&id).is_some_and(SessionState::turn_active);
            conn.idle_extract.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
            if !busy {
                fire(&id_for_fire);
            }
        });
        if let Some(old) = self.idle_extract.lock().unwrap_or_else(|e| e.into_inner()).insert(session.to_string(), timer) {
            old.abort();
        }
    }

    fn cancel_idle_extraction(&self, session: &str) {
        if let Some(timer) = self.idle_extract.lock().unwrap_or_else(|e| e.into_inner()).remove(session) {
            timer.abort();
        }
    }

    /// Check, right after a completed turn or a compaction, whether this chat is due an auto-refine
    /// gate call (a signal in the transcript, see `learn.rs`, and a cooldown; no idle wait). Runs as a
    /// spawned task only so it never blocks the turn's own response; the check itself happens
    /// immediately. The task is remembered so a closing window or headless run can let it finish
    /// before it closes its links ([`close_links`]).
    fn schedule_learning(self: &Arc<Self>, session: String, trigger: Trigger) {
        if self.config.learning.is_none() {
            return;
        }
        let conn = self.clone();
        // Read now: a headless run is unmarked as soon as its turn is delivered, which can be before this task runs.
        let headless = factr_base::headless::is(&session);
        let task = tokio::spawn(async move { conn.learning_check(&session, trigger, headless).await });
        let mut tasks = self.learning_tasks.lock().unwrap_or_else(|e| e.into_inner());
        tasks.retain(|t| !t.is_finished());
        tasks.push(task);
    }

    /// One learning check for `session`: skip (with a `learning.skip` span saying why) or run the pass.
    /// `headless`: a run with no user attached (`/api/agent/run`) never auto-reviews itself.
    async fn learning_check(self: &Arc<Self>, session: &str, trigger: Trigger, headless: bool) {
        let Some(learning) = self.config.learning.clone() else {
            return;
        };
        let skip = |reason: &str, why: Option<&crate::learn::Skip>| {
            let span = Span::new("learning.skip").session(session).attr("trigger", trigger.as_str()).attr("reason", reason);
            emit(match why {
                Some(w) => span.attr("assistants", w.assistants as u64).attr("interval", learning.turn_interval as u64).attr("advanced", w.advanced),
                None => span,
            });
        };
        // The model-callable `refine` tool / REPL `refine` schedule a
        // request that runs at the end of the turn, independent of the
        // checkpoint counter (factr-learn runs those immediately, too).
        let store = self.entry_store().await;
        let scheduled = store.as_ref().and_then(|store| store.refine_pending(session).ok()).unwrap_or(false);
        // The `learning.enabled` switch lives in factr.db.
        let enabled = store.as_ref().is_some_and(|store| store.learning_enabled());
        if !enabled && !scheduled {
            return skip("disabled", None);
        }
        // A tool-requested `refine` still runs; the automatic review does not.
        if headless && !scheduled {
            return skip("headless", None);
        }
        let enabled = enabled && !headless;
        // A closing session is reviewed even mid-turn bookkeeping; otherwise wait for the turn to end.
        if trigger != Trigger::Dispose && self.sessions.lock().await.get(session).is_some_and(SessionState::turn_active) {
            return skip("busy", None);
        }
        struct Running<'a>(&'a Conn, String);
        impl Drop for Running<'_> {
            fn drop(&mut self) {
                self.0.learning_now.lock().unwrap_or_else(|e| e.into_inner()).remove(&self.1);
            }
        }
        if !self.learning_now.lock().unwrap_or_else(|e| e.into_inner()).insert(session.to_string()) {
            return skip("in_progress", None);
        }
        let _running = Running(self, session.to_string());
        let mut gate = None;
        if let (true, Some(store)) = (enabled, &store) {
            // Inside the cooldown nothing can be due, so the history is not fetched to count it.
            if store.cooling(session, learning.cooldown.as_millis() as i64, crate::observability::now()) {
                skip("cooldown", None);
            } else {
                let history = match self.history(session).await {
                    Ok(history) => history,
                    Err(err) => return eprintln!("factr: learning check for {session} failed: {err:#}"),
                };
                let raw = history["messages"].as_array().map(Vec::as_slice).unwrap_or_default();
                let compact = self.compact_pending.lock().unwrap_or_else(|e| e.into_inner()).contains(session);
                let hits = factr_base::learn_signal::pending(session);
                match crate::learn::due(store, session, &learning, raw, trigger, compact, hits, crate::observability::now()) {
                    // factr-learn reviews top-level sessions only (`_rlmDepth === 0`).
                    Ok(_) if self.is_child_session(session).await => skip("depth>0", None),
                    Ok(due) => gate = Some(due),
                    Err(why) => skip(why.reason, Some(&why)),
                }
            }
        }
        if gate.is_none() && !scheduled {
            return;
        }
        let compacted = gate.as_ref().is_some_and(|d| d.trigger == Trigger::Compact);
        match crate::learn::pass(self, session, gate).await {
            Ok(result) => {
                if compacted {
                    self.compact_pending.lock().unwrap_or_else(|e| e.into_inner()).remove(session);
                }
                if let Some(text) = result {
                    self.emit("status.update", Some(session), json!({ "kind": "learning", "text": text })).await;
                }
            }
            Err(err) => eprintln!("factr: learning pass for {session} failed: {err:#}"),
        }
    }

    /// Whether `session` is a sub-agent or fork (it has a parent); factr-learn reviews top-level sessions only.
    /// A sub-agent (swarm worker, delegate): factr-learn's `_rlmDepth > 0`. A user's `/branch` also has a
    /// `parent_id` but is a top-level chat of its own and learns like one.
    async fn is_child_session(&self, session: &str) -> bool {
        let id = session.to_string();
        tokio::task::spawn_blocking(move || {
            let Ok(stub) = factr_base::session::Session::load_startup_stub(&id) else { return false };
            if stub.parent_id.is_none() {
                return false;
            }
            !factr_base::session::Session::load(&id).is_ok_and(|full| is_user_branch(&full))
        })
        .await
        .unwrap_or(false)
    }

    /// factr-learn's review before dispose (`_drainPendingRefinementForDisposal`): wait (bounded) for a
    /// pass already running, then run the review if it is due, before the session's state goes.
    pub(crate) async fn learn_before_dispose(self: &Arc<Self>, session: &str) {
        if self.config.learning.is_none() {
            return;
        }
        let review = async {
            while self.learning_now.lock().unwrap_or_else(|e| e.into_inner()).contains(session) {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            self.learning_check(session, Trigger::Dispose, factr_base::headless::is(session)).await;
        };
        if tokio::time::timeout(DISPOSE_LEARNING_WAIT, review).await.is_err() {
            emit(Span::new("learning.skip").session(session).attr("trigger", "dispose").attr("reason", "timeout"));
        }
    }

    /// Deliver a due RLM "steer" heartbeat to a busy session right now, via
    /// the same soft-interrupt primitive `session.steer` uses — not a hard
    /// cancel, and not a wait for the turn to end. Plain `follow_up`
    /// heartbeats are unaffected: they stay on the idle-only path in
    /// `driver::turn_done`. A cheap local SQLite read per tool-call
    /// boundary; a no-op unless this session has a due steer-mode heartbeat.
    fn maybe_steer_heartbeat(self: Arc<Self>, session_id: String) {
        tokio::spawn(async move {
            let Some(store) = self.control_store().await else { return };
            let due = factr_learn::agent_loop::due_steer_heartbeat(&store, &session_id);
            let Ok(Some(factr_learn::agent_loop::Continuation::Heartbeat { prompt, .. })) = due
            else {
                return;
            };
            if let Err(err) = self
                .call(json!({
                    "req": "soft_interrupt",
                    "session_id": session_id,
                    "content": prompt,
                }))
                .await
            {
                eprintln!("factr: steer heartbeat delivery for {session_id}: {err:#}");
            }
        });
    }

    async fn child_sessions_running(self: &Arc<Self>, parent_id: &str) -> bool {
        let Ok(reply) = self.call(json!({ "req": "list_sessions" })).await else {
            return false;
        };
        let sessions = self.sessions.lock().await;
        children_running(reply["sessions"].as_array().map_or(&[][..], Vec::as_slice), parent_id, None, &sessions)
    }

    async fn list_owned_children(self: &Arc<Self>, parent_id: &str) -> Result<Vec<Value>> {
        let reply = self.call(json!({ "req": "list_sessions" })).await?;
        let sessions = self.sessions.lock().await;
        let mut subagents = Vec::new();
        for info in reply["sessions"].as_array().into_iter().flatten().filter(|s| s["parent_session_id"].as_str() == Some(parent_id)) {
            let followed = self.followed(info["session_id"].as_str().unwrap_or_default()).await;
            subagents.push(Self::subagent_snapshot(info, parent_id, &sessions, followed.as_ref()));
        }
        Ok(subagents)
    }

    /// One row per child, keyed by the child's session id (two spawns with the same label are two rows).
    /// `followed`: the child as this connection follows it (its title and whether it still works).
    fn subagent_snapshot(
        info: &Value,
        parent_id: &str,
        sessions: &HashMap<String, SessionState>,
        followed: Option<&subagents::Followed>,
    ) -> Value {
        let child_id = info["session_id"].as_str().unwrap_or_default();
        let started_ms = info["last_active_at_ms"]
            .as_i64()
            .or(info["updated_at_ms"].as_i64())
            .unwrap_or(0);
        json!({
            "subagent_id": child_id,
            "parent_id": parent_id,
            "goal": followed.map(|f| f.goal.as_str()).or(info["title"].as_str()).or(info["agent_label"].as_str()),
            "child_session_id": child_id,
            "status": Self::map_subagent_status(info, sessions, followed.is_some_and(|f| f.running)),
            "model": info["model"],
            "started_at": started_ms as f64 / 1000.0,
            "task_index": 0,
            "task_count": 1,
            "accepting_steer": true,
        })
    }

    /// Desktop SubagentStatus from the child's real state: working now (a turn running here, or a followed
    /// child that has not finished), else what its last turn left: failed, or done (an idle child is
    /// history, not a running one).
    fn map_subagent_status(info: &Value, sessions: &HashMap<String, SessionState>, followed: bool) -> Value {
        let child_id = info["session_id"].as_str().unwrap_or_default();
        if followed || sessions.get(child_id).is_some_and(SessionState::turn_active) {
            return json!("running");
        }
        let status = info["swarm_status"]
            .as_str()
            .or(info["status"].as_str())
            .unwrap_or("ready");
        let mapped = match status {
            "running" | "processing" => "running",
            "ready" | "idle" | "completed" => "completed",
            "queued" => "queued",
            "failed" | "error" | "timeout" => "failed",
            "stopped" | "interrupted" | "cancelled" | "canceled" => "interrupted",
            other => other,
        };
        json!(mapped)
    }

    async fn resolve_child_session(
        self: &Arc<Self>,
        parent_id: &str,
        subagent_id: &str,
    ) -> Option<String> {
        let reply = self.call(json!({ "req": "list_sessions" })).await.ok()?;
        reply["sessions"].as_array()?.iter().find_map(|s| {
            if s["parent_session_id"].as_str() != Some(parent_id) {
                return None;
            }
            let sid = s["session_id"].as_str()?;
            if sid == subagent_id
                || s["agent_label"].as_str() == Some(subagent_id)
                || s["title"].as_str() == Some(subagent_id)
                || s["friendly_name"].as_str() == Some(subagent_id)
            {
                Some(sid.to_string())
            } else {
                None
            }
        })
    }

    async fn resolve_approval(
        &self,
        session_id: &str,
        request_id: &str,
        choice: &str,
    ) -> Result<()> {
        self.call(json!({
            "req": "permission_response",
            "session_id": session_id,
            "request_id": request_id,
            "decision": map::approval_decision(choice),
        }))
        .await?;
        Ok(())
    }

    async fn dispatch(self: &Arc<Self>, method: &str, p: &Value) -> Result<Value, RpcError> {
        let sid = || {
            p["session_id"]
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| RpcError::params("session_id is required"))
        };
        let call = |req: Value| async move { self.call(req).await.map_err(RpcError::internal) };
        match method {
            "ping" | "gateway.ping" => Ok(json!({})),
            m if local_state::handles(m) => self.local_state(m, p).await,
            m if attach::handles(m) => {
                // Only a session this connection has open or the engine has stored may stage files.
                if let Some(id) = p["session_id"].as_str().filter(|s| !s.is_empty()) {
                    if !self.client.sessions.lock().await.contains(id) && !self.known.lock().await.contains_key(id) && !factr_base::session::session_exists(id) {
                        return Err(RpcError::params("unknown session_id"));
                    }
                }
                let cwd = match p["session_id"].as_str() {
                    Some(id) => self.session_cwd(id).await,
                    None => None,
                }
                .unwrap_or_else(|| self.config.default_cwd.clone());
                attach::handle(m, &self.config.home, &cwd, p)
                    .map_err(|(code, message)| RpcError { code, message, data: None })
            }
            "setup.status" => {
                crate::reload_if_credentials_moved(&self.config).await;
                Ok(provider_state::setup_status(&self.config.provider))
            }
            "setup.runtime_check" => {
                crate::reload_if_credentials_moved(&self.config).await;
                Ok(provider_state::runtime_check(&self.config.provider, &self.config.model, p["provider"].as_str().filter(|r| !r.is_empty())))
            }
            "model.options" => {
                // The served provider's models come from the session's catalog when one is named.
                let listed = match p["session_id"].as_str().filter(|s| !s.is_empty()) {
                    Some(id) => tokio::time::timeout(Duration::from_secs(3), self.call(json!({ "req": "list_models", "session_id": id }))).await.ok().and_then(Result::ok),
                    None => None,
                };
                let models = listed.as_ref().and_then(|r| r["models"].as_array()).map(|m| m.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect()).unwrap_or_default();
                Ok(model_options(&self.config, models, p["include_unconfigured"] == true))
            }
            "session.active_list" => {
                let stored = super::session_infos(&self.config, 1000, false)
                    .await
                    .map_err(RpcError::internal)?;
                let current = p["current_session_id"].as_str().unwrap_or_default();
                let mut active: HashMap<String, (bool, Option<String>)> = stored
                    .iter()
                    .filter(|s| s["is_active"] == true)
                    .filter_map(|s| {
                        s["id"].as_str().map(|id| {
                            (
                                id.to_string(),
                                (false, s["model"].as_str().map(str::to_owned)),
                            )
                        })
                    })
                    .collect();
                for (id, state) in self
                    .sessions
                    .lock()
                    .await
                    .iter()
                    .filter(|(_, state)| state.turn_active())
                {
                    active.insert(id.clone(), (true, state.model.clone()));
                }
                if !current.is_empty() && self.observer.has_active_run(current) {
                    active.insert(current.to_string(), (true, None));
                }
                let rows: HashMap<&str, &Value> = stored
                    .iter()
                    .filter_map(|s| s["id"].as_str().map(|id| (id, s)))
                    .collect();
                // The desktop reads `status` as idle | starting | waiting | working and derives its
                // busy flag (and so the Stop button) from it: anything it does not know as working
                // clears busy mid-turn. Every listed session has a turn in flight.
                let mut waiting = std::collections::HashSet::new();
                for id in active.keys() {
                    if !self.hub.pending_for(id).await.is_empty() {
                        waiting.insert(id.clone());
                    }
                }
                let known = self.known.lock().await;
                let items: Vec<Value> = active.into_iter().map(|(id, (_, model))| {
                    let row = rows.get(id.as_str()).copied();
                    let title = row.and_then(|s| s["title"].as_str()).or_else(|| known.get(&id).and_then(|s| s["title"].as_str())).unwrap_or("Untitled");
                    let started = row.map(|s| s["started_at"].clone()).unwrap_or(Value::Null);
                    let last = row.map(|s| s["last_active"].clone()).unwrap_or(Value::Null);
                    json!({"current":id == current,"id":id,"session_key":id,"last_active":last,"started_at":started,
                        "message_count":row.map(|s|s["message_count"].clone()).unwrap_or(json!(0)),
                        "model":model.or_else(||row.and_then(|s|s["model"].as_str().map(str::to_owned))).unwrap_or_else(||self.config.model.clone()),
                        "preview":row.map(|s|s["preview"].clone()).unwrap_or_else(||json!("")),
                        "status":live_status(waiting.contains(&id)),"title":title})
                }).collect();
                Ok(json!({ "sessions": items }))
            }
            // Served natively from the one table in `slash_forward`: no Python round trip, and only
            // commands something handles. Python-backed rows and skill commands need the backend.
            "commands.catalog" => Ok(self.slash_catalog().await),
            "complete.slash" => Ok(crate::slash_forward::complete(
                p["text"].as_str().unwrap_or_default(),
                self.config.features.is_some(),
                &self.slash_skills(),
            )),
            // Factr desktop's Learning UI, served natively over Continual
            // Harness entries instead of forwarded to the Python backend.
            "learning.frames" => {
                let cols = p["cols"].as_u64().unwrap_or(80).max(1) as usize;
                let rows = p["rows"].as_u64().unwrap_or(24).max(1) as usize;
                let store = factr_learn::entries::EntryStore::open_cached(
                    std::path::Path::new(&self.config.home),
                )
                .map_err(RpcError::internal)?;
                let entries = store.list_all(None, None).map_err(RpcError::internal)?;
                let categories: Vec<Value> =
                    ["prompt", "skill", "subagent"].iter().map(|k| json!({ "name": k, "count": entries.iter().filter(|e| e.kind.as_str() == *k).count() })).collect();
                let summary: Vec<String> = entries
                    .iter()
                    .rev()
                    .take(rows.saturating_sub(2).max(1))
                    .map(|e| format!("[{}/{}] {}", e.kind.as_str(), e.scope.as_str(), e.title))
                    .collect();
                let frame = summary.iter().cloned().collect::<Vec<_>>().join("\n");
                Ok(json!({
                    "frames": [{ "text": frame, "cols": cols, "rows": rows }],
                    "legend": { "prompt": "P", "skill": "S", "subagent": "A" },
                    "categories": categories,
                    "buckets": categories,
                    "summary": summary,
                    "axis": { "start": entries.first().map(|e| e.created_at_ms).unwrap_or(0), "end": entries.last().map(|e| e.updated_at_ms).unwrap_or(0) },
                    "count": entries.len(),
                    "cols": cols,
                    "rows": rows,
                }))
            }
            "learning.detail" => {
                let id = p["id"].as_str().unwrap_or_default();
                let store = factr_learn::entries::EntryStore::open_cached(
                    std::path::Path::new(&self.config.home),
                )
                .map_err(RpcError::internal)?;
                Ok(match store.get(id).map_err(RpcError::internal)? {
                    Some(e) => {
                        json!({ "ok": true, "kind": e.kind.as_str(), "id": e.id, "label": e.title, "content": e.content })
                    }
                    None => json!({ "ok": false, "message": format!("no harness entry {id}") }),
                })
            }
            "learning.delete" => {
                let id = p["id"].as_str().unwrap_or_default();
                let store = factr_learn::entries::EntryStore::open_cached(
                    std::path::Path::new(&self.config.home),
                )
                .map_err(RpcError::internal)?;
                Ok(match store.delete(id) {
                    Ok(_) => json!({ "ok": true }),
                    Err(err) => json!({ "ok": false, "message": err.to_string() }),
                })
            }
            "learning.edit" => {
                let id = p["id"].as_str().unwrap_or_default();
                let content = p["content"].as_str().map(str::to_string);
                let store = factr_learn::entries::EntryStore::open_cached(
                    std::path::Path::new(&self.config.home),
                )
                .map_err(RpcError::internal)?;
                Ok(
                    match factr_learn::refine::edit_entry(
                        &store,
                        id,
                        factr_learn::entries::EntryPatch {
                            content,
                            ..Default::default()
                        },
                    ) {
                        Ok(_) => json!({ "ok": true }),
                        Err(err) => json!({ "ok": false, "message": err.to_string() }),
                    },
                )
            }
            "subagent.list" => {
                let id = sid()?;
                let subagents = self
                    .list_owned_children(id)
                    .await
                    .map_err(RpcError::internal)?;
                Ok(json!({ "subagents": subagents, "delegations": [] }))
            }
            "subagent.interrupt" => {
                let parent = sid()?;
                let subagent_id = p["subagent_id"]
                    .as_str()
                    .ok_or_else(|| RpcError::params("subagent_id is required"))?;
                let Some(child) = self.resolve_child_session(parent, subagent_id).await else {
                    return Ok(json!({ "found": false, "subagent_id": subagent_id }));
                };
                // Best-effort: a finished child may reject cancel.
                let cancelled = call(json!({ "req": "cancel", "session_id": child }))
                    .await
                    .is_ok();
                Ok(
                    json!({ "found": true, "subagent_id": subagent_id, "child_session_id": child, "cancelled": cancelled }),
                )
            }
            "subagent.steer" => {
                let parent = sid()?;
                let subagent_id = p["subagent_id"]
                    .as_str()
                    .ok_or_else(|| RpcError::params("subagent_id is required"))?;
                let content = p["content"]
                    .as_str()
                    .or(p["text"].as_str())
                    .unwrap_or_default();
                if content.trim().is_empty() {
                    return Err(RpcError::params("text is required"));
                }
                let Some(child) = self.resolve_child_session(parent, subagent_id).await else {
                    return Ok(
                        json!({ "status": "rejected", "subagent_id": subagent_id, "text": content }),
                    );
                };
                // A finished row this window followed comes alive again with the turn the steer starts.
                if self.children.lock().await.contains_key(&child) {
                    let _ = self.ensure_attached(&child).await; // unwatched, the steer still lands
                }
                match call(
                    json!({ "req": "soft_interrupt", "session_id": child, "content": content }),
                )
                .await
                {
                    Ok(_) => Ok(
                        json!({ "status": "queued", "subagent_id": subagent_id, "text": content }),
                    ),
                    Err(_) => Ok(
                        json!({ "status": "rejected", "subagent_id": subagent_id, "text": content }),
                    ),
                }
            }
            "subagent.tail" => {
                // Desktop SubagentTranscript expects { available, text, truncated } (≤16 KiB).
                const TAIL_BYTES: usize = 16_384;
                let parent = sid()?;
                let subagent_id = p["subagent_id"]
                    .as_str()
                    .ok_or_else(|| RpcError::params("subagent_id is required"))?;
                let Some(child) = self.resolve_child_session(parent, subagent_id).await else {
                    return Ok(
                        json!({ "subagent_id": subagent_id, "available": false, "text": "", "truncated": false }),
                    );
                };
                // Reading is not following: a child this window no longer follows is read from disk.
                let history = self.history(&child).await.map_err(RpcError::internal)?;
                let mut text = String::new();
                if let Some(list) = history["messages"].as_array() {
                    for m in list {
                        let role = m["role"].as_str().unwrap_or("assistant");
                        let body = m["content"].as_str().unwrap_or("").trim();
                        if body.is_empty() {
                            continue;
                        }
                        if !text.is_empty() {
                            text.push_str("\n\n");
                        }
                        text.push_str(role);
                        text.push_str(": ");
                        text.push_str(body);
                    }
                }
                let truncated = text.len() > TAIL_BYTES;
                if truncated {
                    text = text
                        .chars()
                        .rev()
                        .take(TAIL_BYTES)
                        .collect::<String>()
                        .chars()
                        .rev()
                        .collect();
                }
                Ok(json!({
                    "subagent_id": subagent_id,
                    "available": !text.is_empty(),
                    "text": text,
                    "truncated": truncated,
                }))
            }
            "spawn_tree.list" | "spawn_tree.save" | "spawn_tree.load" => self.spawn_tree(method, p).await,
            "delegation.pause" => Ok(json!({ "paused": p["paused"].as_bool().unwrap_or(true) })),
            "delegation.status" => {
                let id = sid()?;
                let children = self
                    .list_owned_children(id)
                    .await
                    .map_err(RpcError::internal)?;
                let active: Vec<Value> = children
                    .into_iter()
                    .filter(|c| matches!(c["status"].as_str(), Some("running" | "queued")))
                    .collect();
                Ok(json!({
                    "active": active,
                    "paused": false,
                    "max_spawn_depth": 4,
                    "max_concurrent_children": 8,
                }))
            }
            "groups.list" => Ok(json!({ "rooms": [], "next_offset": null })),
            "groups.capabilities" => Ok(json!({
                "protocol_version": 1,
                "driver": false,
                "persistent_process": false,
                "authority_gateway_id": "",
                "room_link": { "linked": false, "room_id": null, "gateway_id": null },
                "features": [],
                "methods": [],
                "max_log_limit": 0,
            })),
            "session.control.read" => {
                let id = sid()?;
                let store = factr_learn::agent_loop::ControlStore::open_cached(Path::new(
                    &self.config.home,
                ))
                .map_err(RpcError::internal)?;
                Ok(json!({ "control": store.control_snapshot(id).map_err(RpcError::internal)? }))
            }
            "session.control" => {
                let id = sid()?;
                let action = p["action"]
                    .as_str()
                    .ok_or_else(|| RpcError::params("action is required"))?;
                let args = p.get("args").cloned().unwrap_or(json!({}));
                let store = factr_learn::agent_loop::ControlStore::open_cached(Path::new(
                    &self.config.home,
                ))
                .map_err(RpcError::internal)?;
                let (control, dispatch) =
                    factr_learn::agent_loop::control_action(&store, id, action, &args)
                        .map_err(RpcError::internal)?;
                if action == "goal.unwait" {
                    driver::resume_soon(id);
                }
                driver::poke();
                self.emit("session.control.update", Some(id), json!({ "control": control })).await;
                Ok(json!({ "control": control, "dispatch": dispatch }))
            }
            "complete.path" => {
                let word = p["word"].as_str().unwrap_or_default().to_string();
                let cwd = p["cwd"]
                    .as_str()
                    .unwrap_or(&self.config.default_cwd)
                    .to_string();
                let items = tokio::task::spawn_blocking(move || map::complete_path(&word, &cwd))
                    .await
                    .unwrap_or_default();
                Ok(json!({ "items": items }))
            }
            // The engine's own commands never go to Factr (its Python store knows none of these
            // sessions or checkpoints); everything else is Factr's to answer.
            "command.dispatch" => {
                let arg = p["arg"].as_str().unwrap_or_default();
                if let Some(message) = crate::slash_forward::unserved(p["name"].as_str().unwrap_or_default()) {
                    return Ok(json!({ "status": "error", "message": message, "output": message }));
                }
                match self.run_slash(p["name"].as_str().unwrap_or_default(), arg, p["session_id"].as_str()).await {
                    Some(done) => Ok(done),
                    // A skill from the one skills dir expands here, with no Python.
                    None => match crate::skill_slash::expand(p["name"].as_str().unwrap_or_default(), arg, p["session_id"].as_str()) {
                        Some(done) => Ok(done),
                        // Python-backed and not running (/queue, /plan): the same one-line answer slash.exec gives.
                        None => match self.forward(method, p).await {
                            Ok(done) => Ok(label_send(p["name"].as_str().unwrap_or_default(), arg, done)),
                            Err(err) if err.code == METHOD_NOT_FOUND => {
                                let message = format!("/{} is not a command this engine serves.", p["name"].as_str().unwrap_or_default().trim_start_matches('/'));
                                Ok(json!({ "status": "error", "message": message, "output": message }))
                            }
                            Err(err) => Err(err),
                        },
                    },
                }
            }
            "slash.exec" => {
                let command = p["command"].as_str().unwrap_or_default().chars().take(4000).collect::<String>();
                let command = command.trim_start_matches('/');
                let (name, arg) = match command.split_once(char::is_whitespace) {
                    Some((name, arg)) => (name, arg.trim()),
                    None => (command, ""),
                };
                if let Some(message) = crate::slash_forward::unserved(name) {
                    return Ok(json!({ "status": "error", "message": message, "output": message }));
                }
                if let Some(done) = self.run_slash(name, arg, p["session_id"].as_str()).await {
                    return Ok(done);
                }
                if let Some(done) = crate::skill_slash::expand(name, arg, p["session_id"].as_str()) {
                    return Ok(done);
                }
                // Factr answers quick/plugin/bundle/skill commands and prompt builders without a
                // session. A built-in that needs a Factr session is refused there.
                // Factr-routed table rows (/plan, /learn, /queue ...) go the same way, by canonical name.
                if !name.is_empty() {
                    let canonical = crate::slash_forward::lookup(name).map_or(name, |c| c.name);
                    let forwarded = self.forward("command.dispatch", &json!({ "name": canonical, "arg": arg })).await.ok();
                    if let Some(done) = forwarded {
                        return Ok(label_send(canonical, arg, done));
                    }
                }
                crate::note_unsupported("slash", command);
                let message = format!("/{name} is not a command this engine serves.");
                Ok(json!({ "status": "error", "message": message, "output": message }))
            }
            "gateway.capabilities" => Ok(json!({ "per_session_exclusive_submit": false })),
            "client.capabilities" => Ok(json!({ "server_requests": ["approval"] })),
            "session.create" => {
                let profile = crate::profile::current();
                let cwd = p["cwd"]
                    .as_str()
                    .unwrap_or(&self.config.default_cwd)
                    .to_string();
                let link = self.open_link().await.map_err(RpcError::internal)?;
                let mut create = json!({ "req": "create_session", "working_dir": cwd });
                if let Some(prompt) = p["system_prompt"].as_str() {
                    create["system_prompt"] = json!(prompt);
                } else if let Some(prompt) = crate::profile::new_session_persona(p["headless"].as_bool().unwrap_or(false) || factr_base::headless::process()) {
                    create["system_prompt"] = json!(prompt);
                }
                let reply = self
                    .call_on(&link, create)
                    .await
                    .map_err(RpcError::internal)?;
                let id = reply["session"]["session_id"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
                self.links.lock().await.insert(id.clone(), link);
                self.client.sessions.lock().await.insert(id.clone());
                // The engine's create reply may not echo the directory; undo's checkpoint lookup
                // (`session_cwd`) needs it, so the requested one is recorded here.
                let mut info = reply["session"].clone();
                if info["working_dir"].as_str().is_none_or(str::is_empty) {
                    info["working_dir"] = json!(cwd);
                }
                self.known.lock().await.insert(id.clone(), info);
                self.fresh.lock().await.insert(id.clone());
                let selected_effort = p["reasoning_effort"]
                    .as_str()
                    .filter(|effort| !effort.trim().is_empty())
                    .or(profile.reasoning_effort.as_deref());
                // The model is pinned here, always (`profile::pin_new_chat_model`): never left to the
                // provider's own default. A failure must not leave the half-made chat behind.
                let pinned = {
                    let link = self.links.lock().await.get(&id).cloned();
                    let session = id.clone();
                    crate::profile::pin_new_chat_model(&self.config, p, |model| {
                        let (link, session) = (link.clone(), session.clone());
                        async move {
                            let link = link.ok_or_else(|| anyhow!("new session link was lost"))?;
                            self.call_on(&link, json!({ "req": "set_model", "session_id": session, "model": model })).await.map(|_| ())
                        }
                    })
                    .await
                };
                let (pinned_model, pinned_provider) = match pinned {
                    Ok(crate::profile::Pinned::On { model, provider }) => (model, provider),
                    Ok(crate::profile::Pinned::FellBack { wanted, error, model, provider }) => {
                        let text = format!("The saved model {wanted} is not available ({error}); this chat uses {model}.");
                        eprintln!("factr: {text}");
                        self.emit("status.update", Some(&id), json!({ "kind": "warn", "text": text })).await;
                        (model, Some(provider))
                    }
                    Err(err) => {
                        self.links.lock().await.remove(&id);
                        self.client.sessions.lock().await.remove(&id);
                        self.known.lock().await.remove(&id);
                        self.fresh.lock().await.remove(&id);
                        let _ = crate::sessions_rest::delete_everywhere(&self.config, &id).await;
                        return Err(RpcError::internal(err));
                    }
                };
                // A remembered effort the served model cannot take (a local Ollama
                // model has no effort levels) is a preference that does not apply
                // here, not a reason to refuse the chat: skip it and say so.
                let mut applied_effort = None;
                if let Some(effort) = selected_effort {
                    let session_link =
                        self.links.lock().await.get(&id).cloned().ok_or_else(|| {
                            RpcError::internal(anyhow!("new session link was lost"))
                        })?;
                    match self.call_on(&session_link, json!({
                        "req": "set_reasoning_effort", "session_id": id.clone(), "effort": effort,
                    })).await {
                        Ok(_) => applied_effort = Some(effort),
                        Err(err) => eprintln!("factr: reasoning effort {effort:?} not applied to {id}: {err:#}"),
                    }
                }
                if let Some(title) = p["title"].as_str().filter(|t| !t.is_empty()) {
                    let _ = self
                        .call(json!({ "req": "rename_session", "session_id": id, "title": title }))
                        .await;
                }
                let sessions = self.sessions.lock().await;
                let mut info = map::live_info(
                    &id,
                    sessions.get(&id),
                    &cwd,
                    &self.config.version,
                    &self.config.model,
                    &self.config.provider,
                );
                info["model"] = json!(pinned_model);
                if let Some(provider) = pinned_provider {
                    info["provider"] = json!(provider);
                }
                info["memory_enabled"] = json!(factr_base::config::memory_enabled());
                if let Some(effort) = applied_effort {
                    info["reasoning_effort"] = json!(effort);
                    self.known.lock().await.entry(id.clone()).and_modify(|known| known["reasoning_effort"] = json!(effort));
                } else if selected_effort.is_none()
                    && let Some(effort) = info["provider"].as_str().and_then(configured_default_effort)
                {
                    // No saved pick: the chat runs on the engine's configured effort. Report it, or
                    // the effort pill shows the desktop's own default while requests carry another.
                    info["reasoning_effort"] = json!(effort);
                    self.known.lock().await.entry(id.clone()).and_modify(|known| known["reasoning_effort"] = json!(effort));
                }
                Ok(json!({
                    "session_id": id,
                    "stored_session_id": id,
                    "message_count": 0,
                    "messages": [],
                    "info": info,
                }))
            }
            "session.resume" | "session.activate" => {
                let id = sid()?.to_string();
                // A cron run (`cron_{job}_{stamp}`) lives in Factr's state.db, not the engine store.
                if let Some(run) = cron_run(&self.config, &id) {
                    return Ok(run.resume(&self.config.version, &self.config.model, &self.config.provider, p["omit_messages"].as_bool() == Some(true)));
                }
                let attached = self
                    .ensure_attached(&id)
                    .await
                    .map_err(RpcError::internal)?;
                let cwd = attached["session"]["working_dir"]
                    .as_str()
                    .unwrap_or(&self.config.default_cwd)
                    .to_string();
                let messages = if p["omit_messages"].as_bool() == Some(true) {
                    Vec::new()
                } else {
                    let history = call(json!({ "req": "get_history", "session_id": id })).await?;
                    map::transcript(&history["messages"])
                };
                let (running, info) = {
                    let sessions = self.sessions.lock().await;
                    (
                        sessions.get(&id).is_some_and(SessionState::turn_active),
                        map::live_info(&id, sessions.get(&id), &cwd, &self.config.version, &self.config.model, &self.config.provider),
                    )
                };
                Ok(json!({
                    "session_id": id,
                    "stored_session_id": id,
                    "message_count": messages.len(),
                    "messages": messages,
                    "running": running,
                    "info": self.with_picks(&id, info).await,
                }))
            }
            "session.history" => {
                let id = sid()?;
                if let Some(run) = cron_run(&self.config, id) {
                    let messages = run.transcript();
                    return Ok(json!({ "count": messages.len(), "messages": messages }));
                }
                self.ensure_attached(id).await.map_err(RpcError::internal)?;
                let history = call(json!({ "req": "get_history", "session_id": id })).await?;
                let mut messages = map::transcript(&history["messages"]);
                // Each user/assistant row carries a permanent id (never reused after an undo); the
                // desktop addresses rewinds (`truncate_before_row_id`) by it.
                let ids = self.row_ids(id, &rewind::rows(&history)).await;
                for (m, row) in messages.iter_mut().filter(|m| m["role"] == "user" || m["role"] == "assistant").zip(ids) {
                    m["row_id"] = json!(row);
                }
                Ok(json!({ "count": messages.len(), "messages": messages }))
            }
            // Projects are the engine's sessions grouped by working directory (Factr's state.db never
            // holds engine chats).
            "projects.tree" | "projects.list" | "projects.project_sessions" | "projects.record_repos" => {
                if method == "projects.record_repos" {
                    return Ok(projects::record_repos_reply(p));
                }
                if method == "projects.list" {
                    return Ok(json!({ "projects": [], "active_id": null }));
                }
                let reply = call(json!({ "req": "list_sessions", "limit": 5000 })).await?;
                let rows: Vec<Value> = reply["sessions"].as_array().map(|l| l.iter().filter(|s| s["parent_session_id"].is_null()).map(map::session_info).collect()).unwrap_or_default();
                let home = factr_base::platform::user_home_dir().map(|h| h.to_string_lossy().into_owned()).unwrap_or_default();
                if method == "projects.tree" {
                    return Ok(projects::tree_reply(&rows, &home, p));
                }
                let id = p["project_id"].as_str().unwrap_or_default();
                if id.is_empty() {
                    return Err(RpcError { code: 5063, message: "project_id required".into(), data: None });
                }
                Ok(projects::project_sessions_reply(&rows, &home, id))
            }
            // The engine's own tools, grouped by toolset; Factr's registry has different ones.
            "tools.list" | "toolsets.list" => {
                Ok(json!({ "toolsets": tools_catalog::toolset_rows(Path::new(&self.config.home), method == "tools.list") }))
            }
            "tools.show" => Ok(tools_catalog::show(Path::new(&self.config.home))),
            "tools.configure" => match tools_catalog::configure(Path::new(&self.config.home), p) {
                Some(done) => done,
                None => self.forward(method, p).await,
            },
            "session.list" => {
                let mut req = json!({ "req": "list_sessions" });
                if let Some(limit) = p["limit"].as_u64() {
                    req["limit"] = json!(limit.min(1000));
                }
                let reply = call(req).await?;
                let mut rows: Vec<Value> = reply["sessions"]
                    .as_array()
                    .map(|list| {
                        list.iter()
                            .filter(|s| s["parent_session_id"].is_null())
                            .map(map::session_row)
                            .collect()
                    })
                    .unwrap_or_default();
                let fresh = self.fresh.lock().await;
                for (id, info) in self.known.lock().await.iter() {
                    if fresh.contains(id)
                        && info["parent_session_id"].is_null()
                        && !rows.iter().any(|r| r["id"] == id.as_str())
                    {
                        rows.insert(0, map::session_row(info));
                    }
                }
                Ok(json!({ "sessions": rows }))
            }
            // Chats live only in the engine's store, so every chat-bound method
            // is answered here; forwarding one to Factr's Python backend would
            // act on a database that has never seen these sessions.
            "session.close" => {
                let id = sid()?;
                // Dropping the link mid-turn makes the engine abort the turn as crashed, and the desktop
                // closes before it deletes: refusing here keeps delete's "stop it first" guard reachable.
                // (Its other closes are of chats it just created, which have no turn yet.)
                if self.sessions.lock().await.get(id).is_some_and(SessionState::turn_active) {
                    return Err(RpcError::params("session is running; stop it before closing"));
                }
                self.cancel_idle_extraction(id);
                // The due review runs in the background: closing never waits on a model call.
                let conn = self.clone();
                let review_id = id.to_string();
                tokio::spawn(async move { conn.learn_before_dispose(&review_id).await });
                factr_base::memory_extract::extract_session_end(id);
                factr_app_core::tool::stop_repl_session(id).await;
                let closed = self.links.lock().await.contains_key(id);
                self.forget_link(id).await;
                self.forget_session(id);
                self.client.sessions.lock().await.remove(id);
                driver::poke(); // a goal on the closed chat is the driver's again
                Ok(json!({ "closed": closed }))
            }
            "session.delete" => {
                let id = sid()?.to_string();
                self.cancel_idle_extraction(&id);
                if self
                    .sessions
                    .lock()
                    .await
                    .get(&id)
                    .is_some_and(SessionState::turn_active)
                    || self.observer.has_active_run(&id)
                {
                    return Err(RpcError::params(
                        "session is running; stop it before deleting",
                    ));
                }
                // Only a session attached in this process can have new activity to review.
                if self.links.lock().await.contains_key(&id) {
                    self.learn_before_dispose(&id).await;
                }
                // No session-end extraction here: its job runs after this returns and would write
                // `extracted_through:<id>` back once `forget_rows` removed it. A chat that was
                // closed first was already extracted by `session.close`.
                factr_base::memory_extract::forget_session(&id);
                self.client.sessions.lock().await.remove(&id);
                // The link outlives the engine's delete, so its disconnect cleanup finds no agent to extract from.
                let deleted = crate::sessions_rest::delete_everywhere(&self.config, &id).await;
                self.fresh.lock().await.remove(&id);
                self.forget_link(&id).await;
                self.forget_session(&id);
                deleted.map_err(RpcError::internal)?;
                factr_app_core::tool::stop_repl_session(&id).await;
                Ok(json!({ "deleted": true }))
            }
            "session.set_hidden" => {
                let id = sid()?;
                let hidden = p["hidden"].as_bool().unwrap_or(true);
                let req = if hidden {
                    "archive_session"
                } else {
                    "restore_session"
                };
                call(json!({ "req": req, "session_id": id })).await?;
                Ok(json!({ "hidden": hidden, "session_key": id }))
            }
            "session.most_recent" => {
                let reply = call(json!({ "req": "list_sessions", "limit": 1 }))
                    .await
                    .ok();
                let top = reply
                    .as_ref()
                    .and_then(|r| r["sessions"].as_array())
                    .and_then(|list| list.first())
                    .cloned();
                Ok(match top {
                    Some(info) => {
                        let row = map::session_row(&info);
                        json!({ "session_id": row["id"], "title": row["title"], "started_at": row["started_at"], "source": "factr" })
                    }
                    None => json!({ "session_id": null }),
                })
            }
            "session.branch" => {
                let id = sid()?.to_string();
                self.ensure_attached(&id)
                    .await
                    .map_err(RpcError::internal)?;
                let forked = call(json!({ "req": "fork_session", "session_id": id })).await?;
                let child = forked["session"]["session_id"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
                if child.is_empty() {
                    return Err(RpcError::internal(anyhow!(
                        "engine did not return the branched session"
                    )));
                }
                // `count`: keep only the first N user turns of the fork.
                if let Some(keep) = p["count"].as_u64() {
                    let rows = self.history_rows(&child).await.map_err(RpcError::internal)?;
                    if let Some(cut) = rewind::cut_before_user(&rows, keep as usize) {
                        self.apply_cut(&child, &cut).await.map_err(RpcError::internal)?;
                    }
                }
                let attached = self
                    .ensure_attached(&child)
                    .await
                    .map_err(RpcError::internal)?;
                let title = p["name"]
                    .as_str()
                    .filter(|n| !n.is_empty())
                    .map(str::to_string);
                if let Some(title) = &title {
                    let _ = self
                        .call(
                            json!({ "req": "rename_session", "session_id": child, "title": title }),
                        )
                        .await;
                }
                let history = call(json!({ "req": "get_history", "session_id": child })).await?;
                let messages = map::transcript(&history["messages"]);
                let cwd = attached["session"]["working_dir"]
                    .as_str()
                    .unwrap_or(&self.config.default_cwd)
                    .to_string();
                let sessions = self.sessions.lock().await;
                Ok(json!({
                    "session_id": child,
                    "stored_session_id": child,
                    "title": title.unwrap_or_else(|| "Branch".into()),
                    "parent": id,
                    "message_count": messages.len(),
                    "messages": messages,
                    "info": map::live_info(&child, sessions.get(&child), &cwd, &self.config.version, &self.config.model, &self.config.provider),
                }))
            }
            "session.undo" => {
                let id = sid()?;
                // Conversation only (Factr parity), and redoable.
                match self.undo_turns(id, 1, undo_turn::Mode::Chat).await {
                    Ok((report, _)) => Ok(json!({ "removed": report.messages_removed })),
                    Err(undo_turn::Failure::Refused(m)) if m == "no user messages to undo" => Ok(json!({ "removed": 0 })),
                    Err(f) => Err(f.rpc()),
                }
            }
            // Not in the contract: `/undo` as an RPC (chat and files; `mode` "chat" | "files").
            "session.undo_turn" => {
                let id = sid()?;
                let mode = match p["mode"].as_str() {
                    Some("chat") => undo_turn::Mode::Chat,
                    Some("files") => undo_turn::Mode::Files,
                    _ => undo_turn::Mode::Both,
                };
                let n = p["count"].as_u64().filter(|n| *n > 0).unwrap_or(1) as usize;
                let (r, _) = self.undo_turns(id, n, mode).await.map_err(undo_turn::Failure::rpc)?;
                Ok(json!({
                    "files_restored": r.files.restored, "files_skipped": r.files.skipped, "messages_removed": r.messages_removed,
                    "redo_available": r.redo_available, "irreversible_effects": r.irreversible, "prefill": r.text, "notice": r.notice(mode),
                }))
            }
            "session.redo_turn" => {
                let id = sid()?;
                let r = self.redo_turns(id).await.map_err(undo_turn::Failure::rpc)?;
                Ok(json!({ "files_restored": r.files.restored, "files_skipped": r.files.skipped, "messages_restored": r.messages_restored, "redo_available": r.redo_available }))
            }
            // Not in the contract: rewind the last user turn and send the same message again.
            "session.retry" => {
                let id = sid()?;
                if self.turn_running(id).await {
                    return Err(RpcError::params("session is running; retry works on an idle session"));
                }
                let rows = self.history_rows(id).await.map_err(RpcError::internal)?;
                let cut = rewind::cut_last_users(&rows, 1).ok_or_else(|| RpcError::params("no previous user message to retry"))?;
                self.apply_cut(id, &cut).await.map_err(RpcError::internal)?;
                let mut sent = Box::pin(self.dispatch("prompt.submit", &json!({ "session_id": id, "text": cut.text }))).await?;
                sent["removed"] = json!(cut.removed);
                Ok(sent)
            }
            "session.status" => {
                let id = sid()?;
                let title = self
                    .known
                    .lock()
                    .await
                    .get(id)
                    .and_then(|info| info["title"].as_str().map(str::to_string))
                    .unwrap_or_else(|| "Untitled".into());
                let sessions = self.sessions.lock().await;
                let state = sessions.get(id);
                let usage = state
                    .map(SessionState::usage_json)
                    .unwrap_or_else(|| json!({}));
                let output = format!(
                    "Session: {title}\nID: {id}\nModel: {} ({})\nState: {}\nTokens: {} in, {} out",
                    self.config.model,
                    self.config.provider,
                    if state.is_some_and(SessionState::turn_active) {
                        "running"
                    } else {
                        "idle"
                    },
                    usage["input"].as_u64().unwrap_or(0),
                    usage["output"].as_u64().unwrap_or(0),
                );
                Ok(json!({ "output": output }))
            }
            "session.save" => {
                let id = sid()?.to_string();
                self.ensure_attached(&id)
                    .await
                    .map_err(RpcError::internal)?;
                let history = call(json!({ "req": "get_history", "session_id": id })).await?;
                let dir = std::path::Path::new(&self.config.home)
                    .join("sessions")
                    .join("saved");
                let file = dir.join(format!(
                    "{id}-{}.json",
                    chrono::Utc::now().format("%Y%m%d-%H%M%S")
                ));
                let body = serde_json::to_vec_pretty(
                    &json!({ "session_id": id, "messages": map::transcript(&history["messages"]) }),
                )
                .map_err(|e| RpcError::internal(anyhow!(e)))?;
                std::fs::create_dir_all(&dir)
                    .and_then(|()| std::fs::write(&file, body))
                    .map_err(|e| RpcError::internal(anyhow!(e)))?;
                Ok(json!({ "file": file.to_string_lossy() }))
            }
            "session.redirect" => {
                let id = sid()?;
                let text = map::prompt_text(&p["text"]);
                self.ensure_attached(id).await.map_err(RpcError::internal)?;
                call(json!({ "req": "soft_interrupt", "session_id": id, "content": text })).await?;
                Ok(json!({ "status": "queued", "text": text }))
            }
            "session.events.since" => {
                let id = sid()?;
                let last_seen = p["last_seen"]
                    .as_u64()
                    .ok_or_else(|| RpcError::params("last_seen must be an integer"))?;
                // Taken over before the buffer is read: what follows the read reaches this window live.
                self.adopt(id).await;
                let (events, latest_seq, truncated, epoch) =
                    self.observer.replay_since(id, last_seen);
                let count = events.len();
                let approvals = self.approvals.lock().await;
                let open_requests: Vec<Value> = approvals.iter().filter(|(_, (session, _))| session == id)
                    .map(|(id, (session_id, request_id))| json!({
                        "id": id, "method": "approval", "params": {"session_id": session_id, "request_id": request_id}
                    })).collect();
                Ok(
                    json!({ "events": events, "latest_seq": latest_seq, "truncated": truncated,
                    "count": count, "epoch": epoch, "open_requests": open_requests }),
                )
            }
            "session.events.stats" => Ok(self.observer.replay_stats()),
            "session.context_breakdown" => {
                let id = sid()?;
                let history = self.history(id).await.map_err(RpcError::internal)?;
                let text: String = history["messages"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|message| {
                        message["content"]
                            .as_str()
                            .map(str::to_owned)
                            .unwrap_or_default()
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                let tokens = text.chars().count().div_ceil(4);
                let model = self
                    .sessions
                    .lock()
                    .await
                    .get(id)
                    .and_then(|s| s.model.clone())
                    .unwrap_or_else(|| self.config.model.clone());
                let context_max = factr_base::provider::context_limit_for_model_with_provider(
                    &model,
                    Some(&self.config.provider),
                )
                .unwrap_or(0);
                Ok(json!({
                    "categories": [{"id":"conversation","label":"Conversation","color":"#8a8a8a","tokens":tokens}],
                    "context_max": context_max, "context_percent": if context_max > 0 { tokens * 100 / context_max } else { 0 },
                    "context_used": tokens, "estimated_total": tokens, "context_estimated": true,
                    "context_source": "engine_transcript_estimate", "model": model, "context_files": [],
                }))
            }
            "session.cwd.set" => {
                let id = sid()?;
                let cwd = p["cwd"]
                    .as_str()
                    .filter(|v| !v.trim().is_empty())
                    .ok_or_else(|| RpcError::params("cwd is required"))?;
                self.set_session_cwd(id, cwd).await
            }
            "session.workspace.move" => {
                let id = p["session_key"]
                    .as_str()
                    .filter(|v| !v.trim().is_empty())
                    .ok_or_else(|| RpcError::params("session_key is required"))?;
                let cwd = p["cwd"]
                    .as_str()
                    .filter(|v| !v.trim().is_empty())
                    .ok_or_else(|| RpcError::params("cwd is required"))?;
                let result = self.set_session_cwd(id, cwd).await?;
                let cwd = result["cwd"].as_str().unwrap_or_default();
                Ok(
                    json!({ "cwd": cwd, "branch": map::git_branch(cwd), "git_repo_root": map::git_repo_root(cwd) }),
                )
            }
            "insights.get" => {
                let days = p["days"].as_u64().unwrap_or(30).clamp(1, 3650);
                let observer = self.observer.clone();
                tokio::task::spawn_blocking(move || observer.insights(days))
                    .await
                    .map_err(|e| RpcError::internal(anyhow!(e)))?
                    .map_err(|e| RpcError::internal(anyhow!(e)))
            }
            "session.foreign.list" => {
                let source = p["source"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned);
                if source
                    .as_deref()
                    .is_some_and(|s| !matches!(s, "claude" | "codex"))
                {
                    return Err(RpcError::params("source must be claude or codex"));
                }
                let offset = p["offset"].as_u64().unwrap_or(0).min(usize::MAX as u64) as usize;
                let limit = p["limit"].as_u64().unwrap_or(25).clamp(1, 50) as usize;
                let rows = tokio::task::spawn_blocking(move || foreign_candidates(source.as_deref()).map(|all| {
                    let total = all.len();
                    let rows = all.into_iter().skip(offset).take(limit).map(|s| {
                        json!({"id":foreign_handle(s.source, &s.path.to_string_lossy()),"source":s.source,
                            "label":if s.source=="claude" {"Claude Code"} else {"Codex"},"title":s.title,
                            "cwd":s.cwd,"mtime":s.mtime,"turn_count":s.turn_count,"excerpt":s.excerpt})
                    }).collect::<Vec<_>>();
                    (rows, (offset + limit < total).then_some(offset + limit))
                })).await.map_err(|e| RpcError::internal(anyhow!(e)))?.map_err(RpcError::internal)?;
                Ok(
                    json!({"sessions":rows.0,"next_offset":rows.1,"host":std::env::var("HOSTNAME").unwrap_or_else(|_|"local".into()),"unreadable":0}),
                )
            }
            "session.foreign.preview" | "session.foreign.import" => {
                let handle = p["id"]
                    .as_str()
                    .filter(|s| s.len() == 64)
                    .ok_or_else(|| RpcError::params("id must be a foreign-session handle"))?
                    .to_string();
                let importer = if method == "session.foreign.import" {
                    "import"
                } else {
                    "preview"
                };
                tokio::task::spawn_blocking(move || {
                    let source = foreign_candidates(None)?.into_iter().find(|s| foreign_handle(s.source, &s.path.to_string_lossy()) == handle)
                        .ok_or_else(|| anyhow!("session no longer available; refresh the list"))?;
                    if importer == "import" {
                        let imported_id = if source.source=="claude" { factr_base::import::imported_claude_code_session_id(&source.external_id) } else { factr_base::import::imported_codex_session_id(&source.external_id) };
                        let already = factr_base::session::Session::load(&imported_id).is_ok();
                        let session = if source.source=="claude" { factr_base::import::import_session_from_file(&source.path, &source.external_id)? }
                            else { factr_base::import::import_codex_session_from_path(&source.path, Some(&source.external_id))? };
                        return Ok(json!({"session_id":session.id,"already_imported":already}));
                    }
                    let turns = foreign_turns(&source)?;
                    let total = turns.len();
                    let messages = turns.into_iter().rev().take(40).collect::<Vec<_>>().into_iter().rev().map(|mut m| {
                        if let Some(text)=m["content"].as_str() { m["content"] = json!(text.chars().take(8000).collect::<String>()); }
                        m
                    }).collect::<Vec<_>>();
                    let imported_id = if source.source=="claude" { factr_base::import::imported_claude_code_session_id(&source.external_id) } else { factr_base::import::imported_codex_session_id(&source.external_id) };
                    Ok(json!({"messages":messages,"total":total,"truncated":total>40,"already_imported":if factr_base::session::Session::load(&imported_id).is_ok(){json!(imported_id)}else{Value::Null},"cwd":source.cwd}))
                }).await.map_err(|e| RpcError::internal(anyhow!(e)))?.map_err(RpcError::internal)
            }
            "prompt.submit" => {
                let id = sid()?.to_string();
                let text = map::prompt_text(&p["text"]);
                if text.trim().is_empty() {
                    return Err(RpcError::params("text is required"));
                }
                self.cancel_idle_extraction(&id);
                let busy = self
                    .sessions
                    .lock()
                    .await
                    .get(&id)
                    .is_some_and(SessionState::turn_active);
                self.ensure_attached(&id)
                    .await
                    .map_err(RpcError::internal)?;
                // Rewind / edit / regenerate: cut the history before the target user turn, then
                // submit the new text as a fresh turn. The cut is checked before anything changes.
                let mut survivors = None;
                if ["truncate_before_user_ordinal", "truncate_before_row_id", "truncate_before_message_id"]
                    .iter()
                    .any(|k| !p[*k].is_null())
                {
                    let rows = self.history_rows(&id).await.map_err(RpcError::internal)?;
                    let ids = self.row_ids(&id, &rows).await;
                    let target = rewind::truncation(p, &rows, &ids).map_err(|rewind::Refusal(code, message)| RpcError { code, message, data: None })?;
                    if let Some(turn) = target {
                        if busy {
                            return Err(RpcError { code: 409, message: "session busy".into(), data: None });
                        }
                        if let Some(cut) = rewind::cut_before_user(&rows, turn) {
                            self.apply_cut(&id, &cut).await.map_err(RpcError::internal)?;
                            survivors = Some(rewind::survivors(&rows, &ids, cut.index));
                        }
                    }
                }
                // factr does not auto-title sessions; Factr titles from the first
                // prompt. Rename before sending: factr refuses renames mid-turn.
                let untitled = {
                    let known = self.known.lock().await;
                    known
                        .get(&id)
                        .is_none_or(|info| info["title"].as_str().is_none_or(str::is_empty))
                };
                if untitled
                    && !busy
                    && let Some(title) = map::derive_title(p["title_preview"].as_str(), &text)
                {
                    if self
                        .call(json!({ "req": "rename_session", "session_id": id, "title": title }))
                        .await
                        .is_ok()
                    {
                        if let Some(info) = self.known.lock().await.get_mut(&id) {
                            info["title"] = json!(title);
                        }
                    }
                }
                self.fresh.lock().await.remove(&id);
                // A queued prompt (busy) is no new turn start: no marker, the redo history stays.
                if !busy {
                    self.record_turn(&id).await;
                }
                let run =
                    self.observer
                        .start_turn(&id, &text, self.run_kind, self.run_title.as_deref());
                if let Some(original) = &self.replay_of {
                    self.observer.link_replay(&run, original);
                }
                // Images staged by `image.attach` ride along on this turn.
                let (staged, images, unreadable) = {
                    let (home, id) = (self.config.home.clone(), id.clone());
                    tokio::task::spawn_blocking(move || attach::staged_images(&home, &id)).await.map_err(|e| RpcError::internal(e.into()))?
                };
                if let Err(err) = self.submit(&id, &text, p["system_reminder"].as_str(), images).await {
                    self.observer.failed_submit(&id, &run, &err.to_string());
                    attach::restore_staged(&self.config.home, &id, &staged, &unreadable);
                    return Err(RpcError::internal(err));
                }
                let mut response = json!({ "status": if busy { "queued" } else { "streaming" } });
                if !unreadable.is_empty() {
                    response["dropped_images"] = json!(unreadable);
                }
                if self.replay_of.is_some() {
                    response["run_id"] = json!(run);
                }
                if let Some((users, map)) = survivors {
                    response["survivor_user_row_ids"] = json!(users);
                    response["survivor_row_id_map"] = map;
                }
                Ok(response)
            }
            "prompt.background" => self.prompt_background(p).await,
            "prompt.btw" => self.prompt_btw(p).await,
            "preview.restart" => self.preview_restart(p).await,
            "session.steer" => {
                let id = sid()?;
                let text = map::prompt_text(&p["text"]);
                self.ensure_attached(id).await.map_err(RpcError::internal)?;
                call(json!({ "req": "soft_interrupt", "session_id": id, "content": text })).await?;
                Ok(json!({ "status": "steered" }))
            }
            "session.interrupt" => {
                let id = sid()?;
                let busy = self
                    .sessions
                    .lock()
                    .await
                    .get(id)
                    .is_some_and(SessionState::turn_active)
                    || self.observer.has_active_run(id);
                self.ensure_attached(id).await.map_err(RpcError::internal)?;
                call(json!({ "req": "cancel", "session_id": id })).await?;
                Ok(json!({
                    "status": if busy { "interrupted" } else { "not_interrupted" },
                    "interrupted": busy,
                }))
            }
            "session.title" => {
                let id = sid()?;
                let title = p["title"].as_str().unwrap_or_default();
                self.ensure_attached(id).await.map_err(RpcError::internal)?;
                call(json!({ "req": "rename_session", "session_id": id, "title": title })).await?;
                Ok(json!({ "session_id": id, "title": title }))
            }
            "session.compress" => {
                let id = sid()?;
                self.ensure_attached(id).await.map_err(RpcError::internal)?;
                // The engine's own sentence is the outcome the desktop shows; a refusal ("nothing to
                // compact yet") is information, not an error.
                match self.call(json!({ "req": "compact", "session_id": id })).await {
                    Ok(reply) => Ok(json!({ "status": "compressed", "message": reply["message"] })),
                    Err(err) => match err.downcast::<Refused>() {
                        Ok(Refused(message)) => Ok(json!({ "status": "ok", "message": message })),
                        Err(err) => Err(RpcError::internal(err)),
                    },
                }
            }
            "session.usage" => {
                let id = sid()?;
                let sessions = self.sessions.lock().await;
                let usage = sessions
                    .get(id)
                    .map(SessionState::usage_json)
                    .unwrap_or_else(|| json!({}));
                Ok(usage)
            }
            "config.get" if local_state::is_checkpoint_key(p["key"].as_str().unwrap_or_default()) => {
                local_state::checkpoint_config(p["key"].as_str().unwrap_or_default(), None)
            }
            "config.set" if local_state::is_checkpoint_key(p["key"].as_str().unwrap_or_default()) => {
                local_state::checkpoint_config(p["key"].as_str().unwrap_or_default(), Some(&p["value"]))
            }
            "config.get" => {
                let key = p["key"].as_str().unwrap_or_default();
                let home = std::path::Path::new(&self.config.home);
                if let Some(done) = settings::get(home, key, p) {
                    return done;
                }
                match key {
                    "project" => {
                        let cwd = p["cwd"]
                            .as_str()
                            .filter(|c| !c.is_empty())
                            .unwrap_or(&self.config.default_cwd)
                            .to_string();
                        let lookup = cwd.clone();
                        let branch = tokio::task::spawn_blocking(move || map::git_branch(&lookup))
                            .await
                            .ok()
                            .flatten();
                        Ok(json!({ "cwd": cwd, "branch": branch }))
                    }
                    "model" | "provider" => {
                        let (model, provider) = crate::profile::effective_default(&self.config);
                        Ok(json!({ "value": if key == "model" { &model } else { &provider }, "model": model, "provider": provider }))
                    }
                    "learning.enabled" => Ok(json!({ "value": crate::learn::learning_enabled(&self.config.home) })),
                    // Unknown key: what config.yaml holds (a `PUT /api/config` or Factr wrote it), else
                    // Python's answer when it is up, else what the engine kept.
                    _ if settings::saved_value(home, key).is_some() => Ok(json!({ "value": settings::saved_value(home, key) })),
                    // A read never starts Python (the desktop reads settings at launch): only a backend
                    // that is already up is asked.
                    _ if !self.backend_running().await => Ok(json!({ "value": settings::stashed(home, key) })),
                    _ => match self.forward(method, p).await {
                        Ok(answer) => Ok(answer),
                        Err(_) => Ok(json!({ "value": settings::stashed(home, key) })),
                    },
                }
            }
            "config.set" if p["key"] == "learning.enabled" => {
                let on = crate::learn::set_learning_enabled(&self.config.home, &p["value"]).map_err(RpcError::internal)?;
                Ok(json!({ "value": on }))
            }
            // A model pick with no live session (a draft chat, Settings): nothing to switch, but the
            // pick is still the user's last choice and becomes the default for new sessions.
            "config.set" if p["key"] == "model" && p["session_id"].as_str().is_none_or(str::is_empty) => {
                let (model, provider, session_only) = parse_model_pick(p["value"].as_str().unwrap_or_default())?;
                if !session_only {
                    self.persist_default_model(&model, provider.as_deref())?;
                }
                Ok(json!({ "key": "model", "value": model, "scope": if session_only { "session" } else { "global" } }))
            }
            "config.set"
                if p["session_id"].as_str().is_some_and(|id| !id.is_empty())
                    && (p["key"] == "model"
                        || (p["key"] == "reasoning"
                            && p["scope"] != "global"
                            && p["value"].as_str().is_some_and(|value| {
                                factr_provider_core::canonical_reasoning_effort(value).is_some()
                                    || factr_base::prompt::is_swarm_effort(value)
                            }))) =>
            {
                let id = sid()?;
                if p["key"] == "reasoning" {
                    self.check_reasoning(p).await?;
                }
                self.ensure_attached(id).await.map_err(RpcError::internal)?;
                let value = p["value"].as_str().unwrap_or_default();
                let mut persist = None;
                let request = match p["key"].as_str().unwrap_or_default() {
                    "reasoning" => json!({
                        "req": "set_reasoning_effort", "session_id": id, "effort": value,
                    }),
                    "model" => {
                        let (model, provider, session_only) = parse_model_pick(value)?;
                        if !session_only {
                            persist = Some((model.clone(), provider.clone()));
                        }
                        let model = factr_base::provider::MultiProvider::model_switch_request_for_session_route(
                            &model,
                            provider.as_deref(),
                            p["route_api_method"].as_str(),
                        );
                        json!({ "req": "set_model", "session_id": id, "model": model })
                    }
                    _ => unreachable!(),
                };
                call(request).await?;
                match p["key"].as_str().unwrap_or_default() {
                    "reasoning" => self.set_pick(id, json!({ "reasoning_effort": value })).await,
                    _ => {
                        if let Ok((model, provider, _)) = parse_model_pick(value) {
                            self.set_pick(id, json!({ "model": model, "provider": provider })).await;
                        }
                    }
                }
                // Only a switch the session accepted becomes the default for new sessions.
                // The live switch already happened, so a failed save is a warning, not a failed call.
                if let Some((model, provider)) = persist
                    && let Err(err) = self.persist_default_model(&model, provider.as_deref())
                {
                    self.warn_not_saved(id, "model", &err.message).await;
                }
                // Factr saves an effort pick as `agent.reasoning_effort` unless it is `--session`, so new
                // chats and a restart keep it. Only a level the session accepted is saved.
                if p["key"] == "reasoning"
                    && p["scope"] != "session"
                    && !factr_base::prompt::is_swarm_effort(value)
                    && let Err(err) = self.persist_default_effort(value)
                {
                    self.warn_not_saved(id, "reasoning effort", &err.message).await;
                }
                Ok(json!({ "value": value }))
            }
            "config.set" => {
                let key = p["key"].as_str().unwrap_or_default();
                let home = std::path::Path::new(&self.config.home);
                if key == "reasoning" {
                    if p["value"].as_str().is_some_and(factr_base::prompt::is_swarm_effort) {
                        return Err(RpcError::params("swarm is a per-chat mode, not a default reasoning effort"));
                    }
                    self.check_reasoning(p).await?;
                }
                if let Some(done) = settings::set(home, key, p) {
                    return done;
                }
                // Unknown key: Factr's backend when it is there to take it, else the engine keeps it.
                match self.forward(method, p).await {
                    Err(e) if e.code == INTERNAL || self.config.features.is_none() => settings::stash(home, key, &p["value"]),
                    other => other,
                }
            }
            "approval.received" => {
                sid()?;
                Ok(json!({ "acknowledged": true }))
            }
            "approval.pending" => {
                let id = sid()?;
                Ok(json!({ "approvals": self.hub.pending_for(id).await }))
            }
            "approval.respond" => {
                let id = sid()?.to_string();
                let choice = p["choice"].as_str().unwrap_or("deny").to_string();
                let wanted = p["request_id"].as_str().map(str::to_string);
                let matching: Vec<(String, String)> = {
                    let mut approvals = self.approvals.lock().await;
                    let keys: Vec<String> = approvals
                        .iter()
                        .filter(|(_, (s, r))| *s == id && wanted.as_ref().is_none_or(|w| w == r))
                        .map(|(k, _)| k.clone())
                        .collect();
                    keys.into_iter()
                        .filter_map(|k| approvals.remove(&k))
                        .collect()
                };
                let mut resolved = matching.len();
                resolved += self
                    .hub
                    .answer_session(&id, wanted.as_deref(), &choice)
                    .await;
                for (session, request) in matching {
                    self.resolve_approval(&session, &request, &choice)
                        .await
                        .map_err(RpcError::internal)?;
                }
                Ok(json!({ "resolved": resolved }))
            }
            // One model path: the engine's own provider and key, traced like any other call.
            "llm.oneshot" => {
                let (system, user) = crate::oneshot::prompts(p).map_err(|(code, message)| RpcError { code, message, data: None })?;
                let complete = self.config.complete.clone().ok_or_else(|| RpcError::internal(anyhow!("no model available")))?;
                let started = crate::observability::now();
                // On the chat's own model (never the startup one) and with no `auxiliary.*` override.
                let call = factr_base::provider::with_aux_consumer(factr_base::factr_config::AuxConsumer::OneShot, async move {
                    // Called inside the scopes: `complete` resolves its provider when called.
                    complete(system, user).await
                });
                let reply = match p["session_id"].as_str().filter(|id| !id.is_empty()) {
                    Some(session) => factr_base::provider::with_aux_session(session, call).await,
                    None => call.await,
                };
                self.observer.record_aux(
                    p["session_id"].as_str().unwrap_or(""), "other", Some("One-shot"), None, None, started,
                    reply.as_ref().ok().and_then(|d| d.usage), reply.as_ref().err().map(|e| e.to_string()).as_deref(),
                );
                let done = reply.map_err(|e| RpcError { code: 5030, message: format!("one-shot generation failed: {e}"), data: None })?;
                Ok(json!({ "text": crate::oneshot::strip_code_fence(&done.text) }))
            }
            // Python would run these outside the approval hook: ask first.
            "shell.exec" | "cli.exec" => {
                if let Some(command) = ungated_exec_command(method, p) {
                    let session = p["session_id"].as_str().unwrap_or("");
                    let choice = self.hub.decide(session, method, &command, "runs a command outside the agent loop").await;
                    if !matches!(choice.as_str(), "once" | "session" | "always") {
                        return Err(RpcError::params("denied: the command was not approved"));
                    }
                }
                self.forward(method, p).await
            }
            // Launch-time reads of Python-owned state: stubbed until Python is up for something else.
            _ => {
                if let Some(stub) = crate::boot_stubs::answer(method, p) {
                    if !self.backend_running().await {
                        return Ok(stub);
                    }
                }
                self.forward(method, p).await
            }
        }
    }

    /// `/refine [instructions] [--global]`, `/refine rollback [id] [--global]`,
    /// `/refine status`, `/harness`. `None` for other commands.
    /// Save a model pick as the default for new sessions (`model.default` / `model.provider`).
    fn persist_default_model(&self, model: &str, provider: Option<&str>) -> Result<(), RpcError> {
        // A pick that names no provider stays on the one being served: a saved provider is what marks the
        // file's model as the user's pick (see `profile::saved_pick_applies`).
        // The model's own provider comes first (`gpt-5.6-luna` is OpenAI whatever is served now).
        let owner = factr_base::provider::provider_for_model(model).map(str::to_owned);
        let provider = match provider.map(str::to_owned).or(owner) {
            Some(provider) => provider,
            None => crate::profile::effective_default(&self.config).1,
        };
        settings::save_default_model(Path::new(&self.config.home), model, &provider)?;
        // From now on this process follows the saved pick, even if it was started with an explicit model.
        crate::profile::note_pick_saved(&self.config);
        Ok(())
    }

    /// Tell the window a pick took effect for this chat but could not be remembered for new ones.
    async fn warn_not_saved(&self, id: &str, what: &str, why: &str) {
        let text = format!("The {what} was switched for this chat but could not be saved as the default: {why}");
        eprintln!("factr: {text}");
        self.emit("status.update", Some(id), json!({ "kind": "warn", "text": text })).await;
    }

    /// Save an effort pick as the default for new sessions (`agent.reasoning_effort`, the key Factr uses).
    fn persist_default_effort(&self, effort: &str) -> Result<(), RpcError> {
        // A swarm sentinel is a per-session routing mode, never the global default effort.
        if factr_base::prompt::is_swarm_effort(effort) {
            return Ok(());
        }
        settings::set(Path::new(&self.config.home), "reasoning", &json!({ "value": effort })).transpose().map(|_| ())
    }

    /// Refuse a reasoning effort the model is known not to take (4002) instead of reporting it set.
    /// The model is the session's own (or the default for new sessions); routes the engine cannot
    /// judge are left to the live provider, which rejects them itself.
    async fn check_reasoning(&self, p: &Value) -> Result<(), RpcError> {
        let value = p["value"].as_str().unwrap_or_default();
        if factr_provider_core::canonical_reasoning_effort(value).is_none() && !factr_base::prompt::is_swarm_effort(value) {
            return Ok(());
        }
        let (default_model, default_provider) = crate::profile::effective_default(&self.config);
        let live = p["session_id"].as_str().filter(|id| !id.is_empty() && p["scope"] != "global");
        let model = match live {
            Some(id) => {
                let from_state = self.sessions.lock().await.get(id).and_then(|s| s.model.clone());
                match from_state {
                    Some(model) => model,
                    None => self.known.lock().await.get(id).and_then(|i| i["model"].as_str().map(str::to_owned)).unwrap_or(default_model),
                }
            }
            None => default_model,
        };
        let provider = factr_base::provider::provider_for_model(&model).map(str::to_owned).unwrap_or(default_provider);
        provider_state::check_effort(&provider, &model, value).map_err(|message| RpcError { code: 4002, message, data: None })
    }

    async fn turn_running(&self, id: &str) -> bool {
        self.sessions.lock().await.get(id).is_some_and(SessionState::turn_active) || self.observer.has_active_run(id)
    }

    async fn history_rows(self: &Arc<Self>, id: &str) -> Result<Vec<Value>> {
        self.ensure_attached(id).await?;
        Ok(rewind::rows(&self.call(json!({ "req": "get_history", "session_id": id })).await?))
    }

    /// A rewind that is not an undo (retry, edit/regenerate truncation, branch, rollback restore):
    /// it ends the redo history, see `drop_redo`. `/undo` uses `apply_undo_cut` instead.
    async fn apply_cut(&self, id: &str, cut: &rewind::Cut) -> Result<()> {
        self.call(json!({ "req": "rewind", "session_id": id, "message_index": cut.index })).await?;
        self.drop_redo(id).await;
        Ok(())
    }

    /// Drop the last user turn and everything after it; returns the rows removed.
    /// Callers: `session.undo` and the `rollback.restore` handler in `rpc/local_state.rs` (a full
    /// restore rewinds the transcript like Factr's `history_removed`). No idle check here.
    pub(crate) async fn rewind_history_to_last_user_turn(self: &Arc<Self>, session_id: &str) -> Result<usize> {
        let rows = self.history_rows(session_id).await?;
        let Some(cut) = rewind::cut_last_users(&rows, 1) else { return Ok(0) };
        self.apply_cut(session_id, &cut).await?;
        Ok(cut.removed)
    }

    /// `/undo [N]` and `/retry`, answered in the shape `command.dispatch` uses: undo prefills the
    /// composer with the backed-up message, retry asks the client to send it again.
    async fn rewind_command(self: &Arc<Self>, name: &str, arg: &str, session_id: Option<&str>) -> Value {
        let say = |text: &str| json!({ "status": "ok", "type": "exec", "output": text, "message": text });
        let Some(id) = session_id.filter(|s| !s.is_empty()) else {
            return say(&format!("/{name} needs an open session."));
        };
        let _ = arg;
        if self.turn_running(id).await {
            return say(&format!("session is running; /{name} works on an idle session"));
        }
        let rows = match self.history_rows(id).await {
            Ok(rows) => rows,
            Err(e) => return say(&format!("/{name}: {e}")),
        };
        let Some(cut) = rewind::cut_last_users(&rows, 1) else {
            return say("no previous user message to retry");
        };
        if let Err(e) = self.apply_cut(id, &cut).await {
            return say(&format!("/{name}: {e}"));
        }
        json!({ "status": "ok", "type": "send", "message": cut.text, "removed": cut.removed })
    }

    /// Whether the Factr backend is up right now (a stub counts in tests); never starts it.
    async fn backend_running(&self) -> bool {
        #[cfg(test)]
        if self.forward_stub.lock().unwrap().is_some() {
            return true;
        }
        match &self.config.features {
            Some(features) => features.is_running().await,
            None => false,
        }
    }

    fn has_backend(&self) -> bool {
        #[cfg(test)]
        if self.forward_stub.lock().unwrap().is_some() {
            return true;
        }
        self.config.features.is_some()
    }

    /// The native catalog plus the backend's quick, plugin and bundle commands. The backend is asked
    /// once (and only when one is configured); a failed ask is retried on the next popup.
    async fn slash_catalog(self: &Arc<Self>) -> Value {
        let python = self.has_backend();
        let mut cat = crate::slash_forward::catalog(python, &self.slash_skills());
        // The desktop asks for the catalog at launch. Merging the backend's quick/plugin/bundle
        // commands needs Python, which must not start for a chat-only session: take them only once
        // it is running for something else (or they are already cached); a later catalog call has them.
        if python && (self.backend_catalog.initialized() || self.backend_running().await) {
            let fetched = self.backend_catalog.get_or_try_init(|| async { self.forward("commands.catalog", &json!({})).await }).await;
            if let Ok(backend) = fetched {
                crate::slash_forward::merge_backend(&mut cat, backend);
            }
        }
        cat
    }

    /// Skill commands the popup offers (`/name`), from the engine's registry: the engine expands them.
    fn slash_skills(&self) -> Vec<(String, String)> {
        factr_base::skill::SkillRegistry::shared_snapshot()
            .list()
            .iter()
            .map(|s| (crate::slash_forward::skill_command(&s.name), s.description.clone()))
            .collect()
    }

    /// One slash command by its table route, in `command.dispatch`'s answer shape; `None` when
    /// Factr should answer it (a Factr prompt builder, or a name the table does not know).
    async fn run_slash(self: &Arc<Self>, name: &str, arg: &str, session_id: Option<&str>) -> Option<Value> {
        use crate::slash_forward::Route;
        let cmd = crate::slash_forward::lookup(name)?;
        match cmd.route {
            Route::Engine => Some(self.engine_slash(cmd.name, arg, session_id).await),
            Route::Harness => {
                let words: Vec<&str> = std::iter::once(cmd.name).chain(arg.split_whitespace()).collect();
                let message = self.harness_command(&words, session_id).await.unwrap_or_default();
                // `/goal <text>` and `/goal resume` must start the turn, like Factr' `type: "send"`:
                // the desktop submits `message` and shows `notice`; state alone runs nothing.
                if cmd.name == "goal" {
                    let kickoff = session_id.filter(|s| !s.is_empty()).and_then(|sid| {
                        let store = factr_learn::agent_loop::ControlStore::open_cached(std::path::Path::new(&self.config.home)).ok()?;
                        factr_learn::agent_loop::goal_kickoff(&store, sid, arg)
                    });
                    if let Some(prompt) = kickoff.filter(|_| message.contains("Goal set (") || message.contains("Goal resumed:")) {
                        return Some(json!({ "status": "ok", "type": "send", "output": message, "notice": message, "message": prompt, "display": if arg.trim().eq_ignore_ascii_case("resume") { "Resume goal".to_string() } else { format!("Goal: {}", map::goal_text(arg)) } }));
                    }
                }
                Some(json!({ "status": "ok", "type": "exec", "output": message, "message": message }))
            }
            Route::Desktop => {
                let message = format!("/{} runs from the desktop app, not as typed text.", cmd.name);
                Some(json!({ "status": "error", "message": message, "output": message }))
            }
            Route::Factr => None,
        }
    }

    /// A completion contract (`outcome`, `verification`, `constraints`, `boundaries`, `stop_when`) for
    /// `objective` from the one-shot model, as Factr's `/goal draft` does; `None` when there is no
    /// model, the call fails, or the reply is not a JSON object with a field in it.
    async fn draft_contract(&self, session_id: &str, objective: &str) -> Option<Value> {
        const SYSTEM: &str = "You turn a user's plain-language objective into a structured completion contract for an autonomous coding agent. The contract has five fields:\n\
- outcome: the single end state that must be true when done\n\
- verification: the specific test / command / artifact that PROVES the outcome (must be concrete and checkable)\n\
- constraints: what must NOT change or regress\n\
- boundaries: which files, dirs, tools, or systems are in scope\n\
- stop_when: the condition under which the agent should stop and ask for human input instead of pushing on\n\n\
Infer sensible, specific values from the objective and any project context implied by it. Prefer concrete verification (a named test command, a build, a benchmark) over vague phrases. Keep each field to one or two sentences. If a field genuinely cannot be inferred, use an empty string for it.\n\n\
Reply ONLY with a single JSON object on one line:\n\
{\"outcome\": \"...\", \"verification\": \"...\", \"constraints\": \"...\", \"boundaries\": \"...\", \"stop_when\": \"...\"}";
        let complete = self.config.complete.clone()?;
        let user = format!("Objective:\n{}", objective.chars().take(4000).collect::<String>());
        let started = crate::observability::now();
        let reply = crate::learn::aux_complete_for(&complete, session_id, SYSTEM.to_string(), user).await;
        self.observer.record_aux(session_id, "other", Some("Draft goal contract"), None, None, started, reply.as_ref().ok().and_then(|done| done.usage), reply.as_ref().err().map(|err| err.to_string()).as_deref());
        let text = reply.ok()?.text;
        let json: Value = serde_json::from_str(text.get(text.find('{')?..=text.rfind('}')?)?).ok()?;
        ["outcome", "verification", "constraints", "boundaries", "stop_when"].iter().any(|k| json[*k].as_str().is_some_and(|v| !v.trim().is_empty())).then_some(json)
    }

    async fn harness_command(
        self: &Arc<Self>,
        words: &[&str],
        session_id: Option<&str>,
    ) -> Option<String> {
        use factr_learn::entries::EntryStore;
        let result: anyhow::Result<String> = match words {
            ["harness", ..] => EntryStore::open_cached(std::path::Path::new(&self.config.home))
                .map(|store| store.harness_report(session_id.unwrap_or_default())),
            ["refine", "status", ..] => (|| -> anyhow::Result<String> {
                let sid = session_id
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| anyhow!("/refine needs an open session"))?;
                let store = EntryStore::open_cached(std::path::Path::new(&self.config.home))?;
                Ok(factr_learn::refine::status(&store, sid))
            })(),
            ["refine", "rollback", rest @ ..] => (|| -> anyhow::Result<String> {
                let sid = session_id
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| anyhow!("/refine needs an open session"))?;
                let id = rest.iter().find(|w| **w != "--global").copied();
                let store = EntryStore::open_cached(std::path::Path::new(&self.config.home))?;
                factr_learn::refine::rollback(&store, sid, id)
            })(),
            ["goal", rest @ ..] => {
                async {
                    let sid = session_id
                        .filter(|s| !s.is_empty())
                        .ok_or_else(|| anyhow!("/goal needs an open session"))?;
                    let store = factr_learn::agent_loop::ControlStore::open_cached(
                        std::path::Path::new(&self.config.home),
                    )?;
                    if rest.first().is_some_and(|w| w.eq_ignore_ascii_case("draft")) && rest.len() > 1 {
                        // `/goal draft <text>`: a side model call turns the objective into a completion contract.
                        let objective = rest[1..].join(" ");
                        let drafted = self.draft_contract(sid, &objective).await;
                        let none = drafted.is_none();
                        let mut said = factr_learn::agent_loop::draft_goal(&store, sid, &objective, drafted)?;
                        if none {
                            said.push_str("\nCouldn't draft a contract (the model was unavailable): running as a free-form goal. The goal is still set.");
                        }
                        return Ok(said);
                    }
                    let said = factr_learn::agent_loop::handle_goal_command(&store, sid, &rest.join(" "))?;
                    if said.contains("Wait barrier cleared") {
                        driver::resume_soon(sid);
                    }
                    Ok(said)
                }
                .await
            }
            ["subgoal", rest @ ..] => (|| -> anyhow::Result<String> {
                let sid = session_id
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| anyhow!("/subgoal needs an open session"))?;
                let store = factr_learn::agent_loop::ControlStore::open_cached(
                    std::path::Path::new(&self.config.home),
                )?;
                let (action, args) = match rest {
                    [] => return factr_learn::agent_loop::handle_goal_command(&store, sid, "status"),
                    ["clear"] => ("subgoal.clear", json!({})),
                    ["remove" | "rm", n] => ("subgoal.remove", json!({ "index": n.parse::<u64>().unwrap_or(0) })),
                    _ => ("subgoal.add", json!({ "text": rest.join(" ") })),
                };
                let (_, done) = factr_learn::agent_loop::control_action(&store, sid, action, &args)?;
                Ok(done["output"].as_str().unwrap_or_default().to_string())
            })(),
            ["loop", rest @ ..] => (|| -> anyhow::Result<String> {
                let sid = session_id
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| anyhow!("/loop needs an open session"))?;
                let store = factr_learn::agent_loop::ControlStore::open_cached(
                    std::path::Path::new(&self.config.home),
                )?;
                factr_learn::agent_loop::handle_heartbeat_command(&store, sid, &rest.join(" "))
            })(),
            ["refine", rest @ ..] => {
                async {
                    let sid = session_id
                        .filter(|s| !s.is_empty())
                        .ok_or_else(|| anyhow!("/refine needs an open session"))?;
                    let complete = self
                        .config
                        .complete
                        .clone()
                        .ok_or_else(|| anyhow!("no model is available for /refine"))?;
                    let global = rest.contains(&"--global");
                    let instructions: Vec<&str> =
                        rest.iter().filter(|w| **w != "--global").copied().collect();
                    let instructions = (!instructions.is_empty()).then(|| instructions.join(" "));
                    self.ensure_attached(sid).await?;
                    let history = self
                        .call(json!({ "req": "get_history", "session_id": sid }))
                        .await?;
                    let turns: Vec<factr_learn::refine::Turn> = history["messages"]
                        .as_array()
                        .map(|list| {
                            list.iter()
                                .map(|m| factr_learn::refine::Turn {
                                    role: m["role"].as_str().unwrap_or_default().to_string(),
                                    text: m["content"].as_str().unwrap_or_default().to_string(),
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    if turns.is_empty() {
                        return Ok(
                            "Nothing to learn from yet: this session has no messages.".to_string()
                        );
                    }
                    let store = EntryStore::open_cached(std::path::Path::new(&self.config.home))?;
                    let (system, user) = crate::learn::blocking(|| {
                        factr_learn::refine::build_request(&store, sid, &turns, instructions.as_deref(), global)
                    });
                    let started = crate::observability::now();
                    let reply = crate::learn::aux_complete_for(&complete, sid, system, user).await;
                    self.observer.record_aux(
                        sid,
                        "other",
                        Some("Refine harness entries"),
                        None,
                        None,
                        started,
                        reply.as_ref().ok().and_then(|done| done.usage),
                        reply.as_ref().err().map(|err| err.to_string()).as_deref(),
                    );
                    let reply = reply?.text;
                    match crate::learn::blocking(|| factr_learn::refine::apply(&store, sid, &reply, global, "refine")) {
                        Ok(outcome) => {
                            let mut text = format!(
                                "Refined ({}): {}\n",
                                if global { "global" } else { "this session" },
                                outcome.summary
                            );
                            if !outcome.created.is_empty() {
                                text.push_str(&format!("+ created {}\n", outcome.created.len()));
                            }
                            if !outcome.updated.is_empty() {
                                text.push_str(&format!("~ updated {}\n", outcome.updated.len()));
                            }
                            if !outcome.deleted.is_empty() {
                                text.push_str(&format!("- deleted {}\n", outcome.deleted.len()));
                            }
                            text.push_str(&format!(
                                "Undo with /refine rollback {}",
                                outcome.changeset_id
                            ));
                            Ok(text)
                        }
                        Err(err) => Ok(format!("No change: {err:#}")),
                    }
                }
                .await
            }
            _ => return None,
        };
        driver::poke(); // /goal and /loop may have started work
        if matches!(words.first(), Some(&("goal" | "subgoal" | "loop"))) {
            if let Some(sid) = session_id.filter(|s| !s.is_empty()) {
                self.emit_control(sid).await;
                // A chat opened just to run a loop is named by its task, not left "New chat".
                if words.first() == Some(&"loop") && result.as_ref().is_ok_and(|m| m.starts_with("Loop set")) {
                    let task = self.control_store().await.and_then(|st| st.user_heartbeat(sid).ok().flatten()).map(|h| h.prompt);
                    if let Some(title) = task.and_then(|t| map::derive_title(None, &format!("/loop {t}"))) {
                        self.title_if_untitled(sid, &title).await;
                    }
                }
            }
        }
        Some(match result {
            Ok(message) => message,
            Err(err) => format!("{err:#}"),
        })
    }

    /// Give an untitled chat `title` (a chat that already has one keeps it).
    async fn title_if_untitled(self: &Arc<Self>, id: &str, title: &str) {
        let untitled = self.known.lock().await.get(id).is_none_or(|info| info["title"].as_str().is_none_or(str::is_empty));
        if untitled && self.call(json!({ "req": "rename_session", "session_id": id, "title": title })).await.is_ok() {
            if let Some(info) = self.known.lock().await.get_mut(id) {
                info["title"] = json!(title);
            }
        }
    }

    /// Tell the window the session's goal/loop/heartbeat state changed (the composer footer chip), as
    /// Factr's `session.control.update`. The cleared state is sent too: a chip must not outlive its goal.
    async fn emit_control(self: &Arc<Self>, session_id: &str) {
        let Some(store) = self.control_store().await else { return };
        if let Ok(control) = store.control_snapshot(session_id) {
            self.emit("session.control.update", Some(session_id), json!({ "control": control })).await;
        }
    }

    /// Forward a method the Rust harness does not own to Factr's backend.
    async fn forward(self: &Arc<Self>, method: &str, params: &Value) -> Result<Value, RpcError> {
        if method.starts_with("curator.") {
            return Err(RpcError { code: METHOD_NOT_FOUND, message: crate::CURATOR_UNAVAILABLE.into(), data: None });
        }
        #[cfg(test)]
        if let Some(stub) = self.forward_stub.lock().unwrap().clone() {
            return stub(method, params);
        }
        if std::env::var_os("FACTR_TRACE_FORWARD").is_some() {
            eprintln!("factr: forward RPC {method}");
        }
        if crate::deployment::is_private() && crate::deployment::blocks_rpc(method) {
            return Err(RpcError { code: -32000, message: crate::deployment::PRIVATE_MSG.into(), data: None });
        }
        // Last line of defence: a command the engine serves is never handed to Factr.
        if method == "command.dispatch" && params["name"].as_str().is_some_and(crate::slash_forward::engine_serves) {
            return Err(RpcError { code: METHOD_NOT_FOUND, message: format!("/{} is served by the engine", params["name"].as_str().unwrap_or_default().trim_start_matches('/')), data: None });
        }
        // Only methods Factr actually defines may wake the Python backend;
        // anything else is answered here without starting it.
        if !crate::contract_methods().contains(method) {
            return Err(RpcError {
                code: METHOD_NOT_FOUND,
                message: format!("unknown method {method}"),
                data: None,
            });
        }
        let Some(features) = self.config.features.clone() else {
            crate::note_unsupported("rpc", method);
            return Err(RpcError::unsupported(method));
        };
        let upstream = self.upstream(&features).await.map_err(RpcError::internal)?;
        let id = format!("fwd-{}", self.next_forward.fetch_add(1, Ordering::Relaxed));
        let (tx, rx) = oneshot::channel();
        upstream.pending.lock().await.insert(id.clone(), tx);
        let frame = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        upstream
            .tx
            .send(frame.to_string())
            .await
            .map_err(|_| RpcError::internal(anyhow!("feature backend connection closed")))?;
        let reply = tokio::time::timeout(FORWARD_TIMEOUT, rx)
            .await
            .map_err(|_| RpcError::internal(anyhow!("{method} timed out in the feature backend")))?
            .map_err(|_| RpcError::internal(anyhow!("the feature backend restarted; try again")))?;
        features.touch(method, "forwarded-rpc");
        if reply.get("error").is_none() && crate::changes_credentials(method) {
            let (config, provider) = (self.config.clone(), params["provider"].as_str().map(str::to_string));
            tokio::spawn(async move { crate::notify_auth_changed(&config, provider.as_deref()).await });
        }
        match reply.get("error") {
            Some(err) => Err(RpcError {
                code: err["code"].as_i64().unwrap_or(INTERNAL),
                message: err["message"]
                    .as_str()
                    .unwrap_or("feature backend error")
                    .to_string(),
                data: err.get("data").cloned(),
            }),
            None => Ok(reply["result"].clone()),
        }
    }

    /// The live upstream connection, (re)opened as needed.
    async fn upstream(
        self: &Arc<Self>,
        features: &crate::features::Features,
    ) -> Result<Arc<Upstream>> {
        let mut slot = self.upstream.lock().await;
        if let Some(up) = slot.as_ref() {
            if !up.tx.is_closed() {
                return Ok(up.clone());
            }
        }
        let port = features.port().await?;
        let url = format!("ws://127.0.0.1:{port}/api/ws?token={}", features.token);
        let (ws, _) = tokio_tungstenite::connect_async(url)
            .await
            .context("connecting to the feature backend")?;
        let (mut ws_tx, mut ws_rx) = ws.split();
        let (tx, mut rx) = mpsc::channel::<String>(256);
        let writer = tokio::spawn(async move {
            while let Some(text) = rx.recv().await {
                if ws_tx.send(Message::Text(text)).await.is_err() {
                    break;
                }
            }
        });
        let up = Arc::new_cyclic(|weak: &std::sync::Weak<Upstream>| {
            let weak_up = weak.clone();
            let conn = Arc::downgrade(self);
            let reader = tokio::spawn(async move {
                while let Some(Ok(msg)) = ws_rx.next().await {
                    let Message::Text(text) = msg else { continue };
                    let Ok(mut frame) = serde_json::from_str::<Value>(&text) else {
                        continue;
                    };
                    let (Some(conn), Some(up)) = (conn.upgrade(), weak_up.upgrade()) else {
                        break;
                    };
                    if frame.get("method").is_none() {
                        if let Some(id) = frame["id"].as_str() {
                            if let Some(tx) = up.pending.lock().await.remove(id) {
                                let _ = tx.send(frame);
                            }
                        }
                    } else if frame["method"] == "event" {
                        // The desktop already has our own gateway.ready.
                        if frame["params"]["type"] != "gateway.ready" {
                            conn.send_json(frame).await;
                        }
                    } else if frame.get("id").is_some() {
                        let raw = match &frame["id"] {
                            Value::String(s) => s.clone(),
                            other => other.to_string(),
                        };
                        frame["id"] = json!(format!("{UPSTREAM_REQUEST_PREFIX}{raw}"));
                        conn.send_json(frame).await;
                    }
                }
            });
            Upstream {
                tx,
                pending: Mutex::new(HashMap::new()),
                tasks: vec![writer.abort_handle(), reader.abort_handle()],
            }
        });
        *slot = Some(up.clone());
        Ok(up)
    }

    /// A reply to one of our server requests (approvals, or relayed ones).
    async fn on_client_reply(&self, frame: &Value) {
        let Some(id) = frame["id"].as_str() else {
            return;
        };
        if let Some(raw) = id.strip_prefix(UPSTREAM_REQUEST_PREFIX) {
            if let Some(up) = self.upstream.lock().await.clone() {
                let mut reply = frame.clone();
                reply["id"] = raw
                    .parse::<u64>()
                    .map(|n| json!(n))
                    .unwrap_or_else(|_| json!(raw));
                let _ = up.tx.send(reply.to_string()).await;
            }
            return;
        }
        if id.starts_with("clarify-") {
            self.hub.answer_clarify(&self.client, id, frame).await;
            return;
        }
        let choice = frame["result"]["choice"].as_str().unwrap_or("deny");
        if self.hub.answer(id, choice).await {
            return;
        }
        let Some((session, request)) = self.approvals.lock().await.remove(id) else {
            return;
        };
        let _ = self.resolve_approval(&session, &request, choice).await;
    }
}

/// The engine declined a request it understood (nothing to compact, not enough context): an answer to
/// show as information, not a failure of the connection or the engine.
#[derive(Debug)]
struct Refused(String);

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Refused {}

fn check_reply(reply: Value) -> Result<Value> {
    if reply["ev"] == "error" {
        let message = reply["message"].as_str().unwrap_or("engine error").to_string();
        return Err(if reply["code"] == "invalid_request" { anyhow::Error::new(Refused(message)) } else { anyhow!("{message}") });
    }
    Ok(reply)
}

fn rpc_error(id: Value, err: RpcError) -> Value {
    let mut error = json!({ "code": err.code, "message": err.message });
    if let Some(data) = err.data {
        error["data"] = data;
    }
    json!({ "jsonrpc": "2.0", "id": id, "error": error })
}

/// `/api/agent/run` (`kind` "cron") and the side agents ("background", "preview"): one headless prompt on its own hidden session, run to
/// completion (or `timeout`), with every tool approval denied outright and no
/// desktop involved at all. Mirrors [`run`]'s `Conn` setup, minus the
/// WebSocket: `to_ws` just feeds a channel this function drains itself.
/// Replay a completed run: branch the session, rewind to that turn, resubmit the prompt.
pub async fn replay_run(
    config: Arc<Config>,
    hub: Arc<Hub>,
    observer: Arc<Observer>,
    run_id: &str,
) -> Result<Value> {
    let run_id = run_id.to_string();
    let (session, turn_index, prompt) = tokio::task::spawn_blocking({
        let observer = observer.clone();
        let run_id = run_id.clone();
        move || observer.replay_turn_index(&run_id)
    })
    .await??;
    let prompt =
        prompt.ok_or_else(|| anyhow!("run has no captured prompt; enable content capture"))?;

    let (to_ws, mut ws_out) = mpsc::channel::<Message>(1024);
    let client = Arc::new(Client {
        id: hub.next_client_id(),
        to_ws: to_ws.clone(),
        sessions: Mutex::new(Default::default()),
    });
    let conn = Conn::new(config.clone(), to_ws, hub.clone(), client, observer.clone(), false, "invoke_agent", Some(format!("Replay {run_id}")), Some(run_id.clone()));
    let control = conn.open_link().await.context("engine unavailable")?;
    *conn.control.lock().await = Some(control);

    let branched = conn
        .dispatch(
            "session.branch",
            &json!({ "session_id": session, "name": format!("Replay {}", run_id.chars().take(24).collect::<String>()) }),
        )
        .await
        .map_err(|e| anyhow!(e.message))?;
    let child = branched["session_id"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    if child.is_empty() {
        bail!("branch did not return a session id");
    }

    let history = conn
        .call(json!({ "req": "get_history", "session_id": child }))
        .await
        .context("history")?;
    let messages = history["messages"].as_array().cloned().unwrap_or_default();
    let mut user_seen = 0_i64;
    let mut cut: Option<usize> = None;
    for (i, message) in messages.iter().enumerate() {
        if message["role"] == "user" {
            if user_seen == turn_index {
                cut = Some(i);
                break;
            }
            user_seen += 1;
        }
    }
    match cut {
        None if turn_index == 0 => {
            conn.call(json!({ "req": "clear", "session_id": child }))
                .await?;
        }
        Some(0) => {
            conn.call(json!({ "req": "clear", "session_id": child }))
                .await?;
        }
        Some(index) => {
            conn.call(json!({ "req": "rewind", "session_id": child, "message_index": index }))
                .await?;
        }
        None => bail!("turn index {turn_index} not found in branched session"),
    }

    let submitted = match conn
        .dispatch(
            "prompt.submit",
            &json!({ "session_id": child, "text": prompt }),
        )
        .await
    {
        Ok(submitted) => submitted,
        Err(err) => return Err(anyhow!("{}", err.message)),
    };
    let new_run = submitted["run_id"]
        .as_str()
        .ok_or_else(|| anyhow!("replay run did not start"))?
        .to_string();
    let replay_session = child.clone();
    tokio::spawn(async move {
        while let Some(msg) = ws_out.recv().await {
            let Message::Text(text) = msg else { continue };
            let Ok(frame) = serde_json::from_str::<Value>(&text) else {
                continue;
            };
            if frame["method"] == "event"
                && frame["params"]["session_id"] == replay_session
                && frame["params"]["type"] == "message.complete"
            {
                break;
            }
        }
        for task in conn.link_tasks.lock().await.drain(..) {
            task.abort();
        }
    });
    Ok(json!({
        "ok": true,
        "session_id": child,
        "run_id": new_run,
        "replay_of": run_id,
        "status": "streaming",
    }))
}

/// What differs between headless callers of [`agent_run`].
pub(crate) struct RunOpts<'a> {
    /// "cron" | "bot" | "goal": which Factr unattended-approval setting applies.
    pub surface: &'static str,
    /// Per-turn system context (a bot's platform, user and formatting rules), sent as the turn's
    /// system reminder so it never enters the cached static prefix or the transcript.
    pub instructions: Option<&'a str>,
    /// A cron job's own model / provider, applied when the run creates its session.
    pub model: Option<&'a str>,
    pub provider: Option<&'a str>,
    /// Factr toolset policy for the run: a per-job allowlist and the always-on denylist.
    pub enabled_toolsets: Option<Vec<String>>,
    pub disabled_toolsets: Vec<String>,
    /// A cron run's id (`cron_{job}_{stamp}`): the run history opens this run's engine session by it.
    pub run_id: Option<&'a str>,
}

impl Default for RunOpts<'_> {
    fn default() -> Self {
        Self { surface: "goal", instructions: None, model: None, provider: None, enabled_toolsets: None, disabled_toolsets: Vec::new(), run_id: None }
    }
}

/// The command text to put in front of the approval gate, or None for the fixed
/// profile edits the desktop's own dialogs issue (`profile delete|describe`,
/// `config unset model`). Anything else Factr's CLI can do, `chat -q` included,
/// drives an agent or a shell, so it needs a yes.
fn ungated_exec_command(method: &str, p: &Value) -> Option<String> {
    if method == "shell.exec" {
        return Some(p["command"].as_str().unwrap_or("").to_string());
    }
    let argv: Vec<&str> = p["argv"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    let rest = match argv.as_slice() {
        ["--profile", _, rest @ ..] => rest,
        rest => rest,
    };
    match rest {
        ["profile", "delete" | "describe", ..] | ["config", "unset", "model"] => None,
        _ => Some(format!("factr {}", argv.join(" "))),
    }
}

/// Bot chat key -> engine session, persisted in `factr.db` (`engine_settings`) so a bot
/// conversation survives an engine restart.
fn bot_session_setting(key: &str) -> String {
    format!("bot_session:{key}")
}

fn load_bot_session(home: &str, key: &str) -> Option<String> {
    entries_or_log(home)?.setting(&bot_session_setting(key))
}

fn save_bot_session(home: &str, key: &str, session_id: &str) {
    if let Some(store) = entries_or_log(home) {
        let _ = store.set_setting(&bot_session_setting(key), session_id);
    }
}

/// Bot chat key -> the connection and engine session of the turn it is running now, so
/// `/stop` from the chat can interrupt it.
static ACTIVE_RUNS: std::sync::LazyLock<std::sync::Mutex<HashMap<String, (Arc<Conn>, String)>>> =
    std::sync::LazyLock::new(Default::default);

/// One turn at a time per bot chat: two messages sent together would otherwise run two turns on
/// one engine session, and each would take the other's `message.complete` for its own reply.
static CHAT_TURNS: std::sync::LazyLock<std::sync::Mutex<HashMap<String, Arc<Mutex<()>>>>> =
    std::sync::LazyLock::new(Default::default);

async fn chat_turn(key: &str) -> tokio::sync::OwnedMutexGuard<()> {
    let lock = CHAT_TURNS.lock().unwrap_or_else(|e| e.into_inner()).entry(key.to_string()).or_default().clone();
    lock.lock_owned().await
}

/// Interrupt the turn a chat is running (Factr `/stop`); false when it has none.
pub(crate) async fn interrupt_run(session_key: &str) -> bool {
    let entry = ACTIVE_RUNS.lock().unwrap_or_else(|e| e.into_inner()).get(session_key).cloned();
    let Some((conn, session)) = entry else { return false };
    conn.dispatch("session.interrupt", &json!({ "session_id": session })).await.is_ok()
}

/// Factr `/new`: stop what the chat is running and forget its engine session, so the next
/// message starts a fresh conversation. The old transcript stays in the sidebar.
pub(crate) async fn reset_bot_session(home: &str, key: &str) {
    interrupt_run(key).await;
    if let Ok(store) = factr_learn::entries::EntryStore::open_cached(Path::new(home)) {
        let _ = store.delete_setting(&bot_session_setting(key));
    }
}

/// The user approved a command an unattended run was denied: a goal session picks its work back up.
pub(crate) fn resume_after_approval(session_id: &str, command: &str) {
    driver::resume(session_id, command);
}

/// `session.create` params for a headless run, carrying a cron job's model / provider override.
fn run_session_params(cwd: Option<&str>, title: Option<&str>, opts: &RunOpts<'_>) -> Value {
    let mut params = json!({ "cwd": cwd, "headless": true });
    for (field, value) in [("title", title), ("model", opts.model), ("provider", opts.provider)] {
        if let Some(value) = value {
            params[field] = json!(value);
        }
    }
    params
}

/// The cron run `id` names, read from Factr's stores (the Factr home, else the engine's).
pub(crate) fn cron_run(config: &Config, id: &str) -> Option<crate::cron_runs::CronRun> {
    let home = factr_base::factr_config::home().unwrap_or_else(|| std::path::PathBuf::from(&config.home));
    let linked = crate::cron_runs::linked_session(&config.home, id);
    crate::cron_runs::load_with(&home, id, linked.as_deref()).map(|mut run| {
        if run.cwd.is_empty() {
            run.cwd = config.default_cwd.clone();
        }
        run
    })
}

pub(crate) async fn agent_run(
    config: Arc<Config>,
    hub: Arc<Hub>,
    observer: Arc<Observer>,
    kind: &'static str,
    prompt: &str,
    cwd: Option<&str>,
    title: Option<&str>,
    session_key: Option<&str>,
    opts: RunOpts<'_>,
    timeout: Duration,
) -> Result<Value> {
    // Queued behind the chat's running turn (its `timeout` starts once this turn does).
    let turn = match session_key {
        Some(key) => Some(chat_turn(key).await),
        None => None,
    };
    let home = config.home.clone();
    let (to_ws, mut ws_out) = mpsc::channel::<Message>(1024);
    let client = Arc::new(Client {
        id: hub.next_client_id(),
        to_ws: to_ws.clone(),
        sessions: Mutex::new(Default::default()),
    });
    let conn = Conn::new(config, to_ws, hub.clone(), client, observer, false, kind, title.map(str::to_string), None);

    // Every exit path, including a failed create or tool policy, ends the run's registration
    // and its link tasks below.
    let result: Result<Value> = async {
        let _ = conn.entry_store().await; // report an unopenable store once, up front
        let control = conn.open_link().await.context("engine unavailable")?;
        *conn.control.lock().await = Some(control);

        // A `session_key` (one bot chat) resumes its engine session; otherwise (or on
        // first use) a fresh one is created and remembered under the key.
        let resumed = session_key.and_then(|key| load_bot_session(&home, key));
        let resumed = match resumed {
            Some(id) => conn
                .dispatch("session.activate", &json!({ "session_id": id, "omit_messages": true }))
                .await
                .ok()
                .map(|_| id),
            None => None,
        };
        let session_id = match resumed {
            Some(id) => id,
            None => {
                let create_params = run_session_params(cwd, title, &opts);
                let created = conn
                    .dispatch("session.create", &create_params)
                    .await
                    .map_err(|e| anyhow!(e.message))?;
                let id = created["session_id"].as_str().unwrap_or_default().to_string();
                if id.is_empty() {
                    bail!("engine did not return a session id");
                }
                if let Some(key) = session_key {
                    save_bot_session(&home, key, &id);
                }
                id
            }
        };

        if let Some(key) = session_key {
            ACTIVE_RUNS.lock().unwrap_or_else(|e| e.into_inner()).insert(key.to_string(), (conn.clone(), session_id.clone()));
        }
        // A cron job's toolset policy holds for the whole run (fails closed: no policy, no run).
        if let Some(request) = toolsets::request(&session_id, opts.enabled_toolsets.as_deref(), &opts.disabled_toolsets) {
            conn.call(request).await.context("could not apply the job's tool policy")?;
        }
        // Nobody waits on an approval here: both gates (`Out::Approval`, `decide`) apply
        // the unattended policy (Factr config, else deny and park for the desktop).
        hub.mark_headless(&session_id, opts.surface).await;

        let outcome = tokio::time::timeout(timeout, async {
            conn.dispatch(
                "prompt.submit",
                &json!({ "session_id": session_id, "text": prompt, "system_reminder": opts.instructions }),
            )
            .await
            .map_err(|e| anyhow!(e.message))?;
            while let Some(msg) = ws_out.recv().await {
                let Message::Text(text) = msg else { continue };
                let Ok(frame) = serde_json::from_str::<Value>(&text) else {
                    continue;
                };
                if frame["method"] != "event" || frame["params"]["session_id"] != session_id.as_str() {
                    continue;
                }
                if frame["params"]["type"] == "message.complete" {
                    let payload = frame["params"]["payload"].clone();
                    return Ok(payload);
                }
            }
            bail!("engine connection closed before the turn finished")
        })
        .await;

        // Stop a timed-out turn first: while it can still ask, it must stay unattended.
        if outcome.is_err() {
            let _ = conn.dispatch("session.interrupt", &json!({ "session_id": session_id })).await;
        }
        hub.unmark_headless(&session_id).await;
        // A cron run is over: nothing will poll a command it left running (a foreground command
        // that hit its timeout is promoted to a background task, not killed).
        if opts.surface == "cron" {
            factr_base::background::global().cancel_session(&session_id).await;
        }
        // A one-shot run has no chat to resume, so its Python REPL worker would outlive it.
        if session_key.is_none() {
            factr_app_core::tool::stop_repl_session(&session_id).await;
        }
        // The turn is over and its transcript is the record: free its buffered live events.
        conn.observer.release_replay(&session_id);
        // Only now is the session guaranteed persisted (factr does not write a
        // session record until its first turn), so hide it from session.list
        // here rather than before the turn — the same mechanism a user's own
        // archived chats use. Best-effort: a session that never got this far
        // (e.g. factr was unreachable) has nothing to hide.
        let _ = conn
            .dispatch(
                "session.set_hidden",
                &json!({ "session_id": session_id, "hidden": true }),
            )
            .await;
        // Tag it so the sidebar lists the transcript, and let old one-shot cron ones expire.
        if matches!(opts.surface, "cron" | "bot") {
            crate::surface_sessions::tag(&home, &session_id, opts.surface, crate::observability::now(), title);
        }
        if let (Some(run), "cron") = (opts.run_id, opts.surface) {
            crate::cron_runs::link(&home, run, &session_id);
        }
        if opts.surface == "cron" {
            crate::surface_sessions::prune_cron(&conn.config, crate::observability::now()).await;
        }
        Ok(match outcome {
            Ok(Ok(payload)) => turn_reply(&payload, &session_id, opts.surface),
            Ok(Err(err)) => {
                json!({ "ok": false, "text": "", "error": err.to_string(), "session_id": session_id })
            }
            Err(_) => {
                json!({ "ok": false, "text": "", "error": "timed out waiting for the turn to finish", "session_id": session_id })
            }
        })
    }
    .await;
    end_run(&conn, session_key).await;
    drop(turn);
    if let Some(key) = session_key {
        prune_chat_turn(key);
    }
    result
}

/// The `/api/agent/run` reply for a finished turn. A bot chat's `/stop` interrupts the turn: Factr
/// takes `interrupted: true` (with `ok`) as "stopped" and posts nothing; every other surface
/// keeps it a failed run, since a cron job that was cut short did not do its work.
fn turn_reply(payload: &Value, session_id: &str, surface: &str) -> Value {
    let status = payload["status"].as_str().unwrap_or("unknown");
    let interrupted = status == "interrupted";
    let ok = status == "complete" || (interrupted && surface == "bot");
    let usage = &payload["usage"];
    json!({
        "ok": ok,
        "interrupted": interrupted,
        "text": payload["text"].as_str().unwrap_or_default(),
        // Only the last assistant message of the turn (after any nudge); `text` joins them all.
        "final_text": crate::map::take_final_text(session_id).unwrap_or_else(|| payload["text"].as_str().unwrap_or_default().to_string()),
        "error": if ok { Value::Null } else { json!(match payload["error"].as_str().filter(|m| !m.is_empty()) {
            Some(detail) => format!("the turn did not complete cleanly ({status}): {detail}"),
            None => format!("the turn did not complete cleanly ({status})"),
        }) },
        "session_id": session_id,
        "usage": if usage.is_null() { Value::Null } else { json!({ "input_tokens": usage["input"], "output_tokens": usage["output"], "cached_tokens": usage["cache_read"] }) },
    })
}

/// Forget an idle chat's turn lock (nothing holds or awaits it), so the map does not grow with every chat.
fn prune_chat_turn(key: &str) {
    let mut turns = CHAT_TURNS.lock().unwrap_or_else(|e| e.into_inner());
    if turns.get(key).is_some_and(|lock| Arc::strong_count(lock) == 1) {
        turns.remove(key);
    }
}

/// End a headless run's registration and link tasks, whatever way it ended.
async fn end_run(conn: &Arc<Conn>, session_key: Option<&str>) {
    if let Some(key) = session_key {
        let mut runs = ACTIVE_RUNS.lock().unwrap_or_else(|e| e.into_inner());
        if runs.get(key).is_some_and(|(c, _)| Arc::ptr_eq(c, conn)) {
            runs.remove(key);
        }
    }
    close_links(conn).await;
}

/// Abort a connection's link tasks. A learning pass it started still needs its history link, so
/// while one is running the abort waits (bounded) in the background; the caller is not delayed.
async fn close_links(conn: &Arc<Conn>) {
    let running: Vec<_> = {
        let mut tasks = conn.learning_tasks.lock().unwrap_or_else(|e| e.into_inner());
        tasks.drain(..).filter(|t| !t.is_finished()).collect()
    };
    if running.is_empty() {
        abort_links(conn).await;
        return;
    }
    let conn = conn.clone();
    tokio::spawn(async move {
        let deadline = tokio::time::Instant::now() + DISPOSE_LEARNING_WAIT;
        for task in running {
            let _ = tokio::time::timeout_at(deadline, task).await;
        }
        abort_links(&conn).await;
    });
}

/// Abort every link's tasks but those of a turn a closed window keeps until it ends (`detached.rs`).
async fn abort_links(conn: &Conn) {
    let kept: Vec<mpsc::Sender<String>> = if conn.is_detached() {
        // The connection outlives its window while a kept turn runs: hold nothing else meanwhile.
        *conn.control.lock().await = None;
        let keep = conn.release_after_turn.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let mut links = conn.links.lock().await;
        links.retain(|sid, _| keep.contains(sid));
        links.values().cloned().collect()
    } else {
        Vec::new()
    };
    conn.link_tasks.lock().await.retain(|task| {
        let keep = task.link.upgrade().is_some_and(|link| kept.iter().any(|k| k.same_channel(&link)));
        if !keep {
            task.abort();
        }
        keep
    });
}

pub async fn run(
    ws: Ws,
    config: Arc<Config>,
    hub: Arc<Hub>,
    observer: Arc<Observer>,
) -> Result<()> {
    let (mut ws_tx, mut ws_rx) = ws.split();
    let (to_ws, mut ws_out) = mpsc::channel::<Message>(1024);
    let client = Arc::new(Client {
        id: hub.next_client_id(),
        to_ws: to_ws.clone(),
        sessions: Mutex::new(Default::default()),
    });
    hub.add(client.clone()).await;
    let conn = Conn::new(config, to_ws, hub.clone(), client.clone(), observer, false, "invoke_agent", None, None);
    conn.window.store(true, Ordering::Release);
    driver::register_window(&conn);

    // Control link first: if the engine is unreachable, refuse the client.
    match conn.open_link().await {
        Ok(control) => *conn.control.lock().await = Some(control),
        Err(_) => {
            let _ = ws_tx
                .send(Message::Close(Some(CloseFrame {
                    code: CloseCode::from(1011),
                    reason: "engine unavailable".into(),
                })))
                .await;
            return Ok(());
        }
    }

    let writer = tokio::spawn(async move {
        while let Some(msg) = ws_out.recv().await {
            if ws_tx.send(msg).await.is_err() {
                break;
            }
        }
    });

    conn.emit(
        "gateway.ready",
        None,
        json!({ "skin": {}, "change_events": false, "replay_epoch": conn.observer.replay_epoch() }),
    )
    .await;

    while let Some(msg) = ws_rx.next().await {
        let text = match msg {
            Ok(Message::Text(t)) => t,
            Ok(Message::Close(_)) => break,
            Err(tokio_tungstenite::tungstenite::Error::Capacity(_)) => {
                // Tell the client why (close 1009) instead of dropping the socket unexplained.
                conn.emit("status.update", None, json!({ "kind": "error", "text": "That message is larger than the gateway accepts and was refused." })).await;
                let _ = conn.to_ws.send(Message::Close(Some(CloseFrame { code: CloseCode::Size, reason: "message too big".into() }))).await;
                tokio::time::sleep(Duration::from_millis(200)).await; // let the writer flush it
                break;
            }
            Err(_) => break,
            Ok(_) => continue,
        };
        if std::env::var_os("FACTR_GATEWAY_TRACE").is_some() {
            eprintln!(
                "factr-gateway: client {}",
                text.chars().take(300).collect::<String>()
            );
        }
        let frame: Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(_) => {
                let err = RpcError {
                    code: PARSE_ERROR,
                    message: "parse error".into(),
                    data: None,
                };
                conn.send_json(rpc_error(Value::Null, err)).await;
                continue;
            }
        };
        if frame.get("method").is_none()
            && (frame.get("result").is_some() || frame.get("error").is_some())
        {
            conn.on_client_reply(&frame).await;
            continue;
        }
        let Some(method) = frame["method"].as_str().map(str::to_string) else {
            let err = RpcError {
                code: -32600,
                message: "invalid request".into(),
                data: None,
            };
            conn.send_json(rpc_error(frame["id"].clone(), err)).await;
            continue;
        };
        let id = frame["id"].clone();
        let Ok(permit) = conn.in_flight.clone().try_acquire_owned() else {
            let err = RpcError {
                code: INTERNAL,
                message: "too many requests in flight".into(),
                data: None,
            };
            conn.send_json(rpc_error(id, err)).await;
            continue;
        };
        let conn = conn.clone();
        tokio::spawn(async move {
            let _permit = permit;
            let params = frame.get("params").cloned().unwrap_or_else(|| json!({}));
            let result = conn.dispatch(&method, &params).await;
            if id.is_null() {
                return; // notification: no reply
            }
            let reply = match result {
                Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
                Err(err) => rpc_error(id, err),
            };
            conn.send_json(reply).await;
        });
    }

    writer.abort();
    hub.remove(client.id).await;
    window_closed(&conn).await;
    Ok(())
}

/// The window is gone: review the chats it had open that are due, then let the links go. A turn it
/// was running keeps its link until it ends (see `detached.rs`); a child it only followed keeps
/// running under its owner.
async fn window_closed(conn: &Arc<Conn>) {
    let kept = detached::keep_running_turns(conn).await;
    let open: Vec<String> = conn.links.lock().await.keys().cloned().collect();
    for session in open.into_iter().filter(|s| !kept.contains(s)) {
        if !conn.sessions.lock().await.get(&session).is_some_and(SessionState::turn_active) {
            conn.schedule_learning(session, Trigger::Dispose);
        }
    }
    close_links(conn).await;
}

#[cfg(test)]
type ForwardStub = Arc<dyn Fn(&str, &Value) -> Result<Value, RpcError> + Send + Sync>;

#[cfg(test)]
mod tests {
    use super::*;

    /// The popup is served by the engine itself: no Python, only handled commands, aliases canonical.
    #[test]
    fn the_engines_duplicate_live_refusal_is_recognised() {
        assert!(is_already_live("Session 'abc' is already live but could not be shared safely with this connection."));
        assert!(!is_already_live("engine did not reply in time"));
    }

    #[tokio::test]
    async fn a_window_takes_a_goal_session_from_the_driver() {
        let conn = test_conn_as("release-link", true);
        conn.ensure_control().await.unwrap();
        let link = conn.control.lock().await.clone().unwrap();
        conn.links.lock().await.insert("goal-chat".into(), link.clone());
        conn.release_at_turn_end("goal-chat").await;
        assert!(!conn.links.lock().await.contains_key("goal-chat"), "the driver let go, so the window's attach is not 'already live'");
        // Mid-turn, dropping the link would make the engine abort the turn as crashed: the driver
        // holds on until the turn's end reaches it, then lets go.
        conn.links.lock().await.insert("goal-chat".into(), link);
        conn.sessions.lock().await.entry("goal-chat".into()).or_default().mark_running();
        conn.release_at_turn_end("goal-chat").await;
        assert!(conn.links.lock().await.contains_key("goal-chat"), "kept while the turn runs");
        conn.on_harness_frame(json!({ "ev": "text_delta", "session_id": "goal-chat", "text": "still going" })).await;
        assert!(conn.links.lock().await.contains_key("goal-chat"), "a frame inside the turn changes nothing");
        conn.on_harness_frame(json!({ "ev": "turn_done", "session_id": "goal-chat" })).await;
        assert!(!conn.links.lock().await.contains_key("goal-chat"), "let go at the turn boundary");
        assert!(conn.release_after_turn.lock().unwrap().is_empty());
    }

    #[test]
    fn a_user_branch_is_not_a_sub_agent_for_the_learning_gate() {
        let mut branch = factr_base::session::Session::create(Some("parent".into()), None);
        branch.append_fork_notice("parent", "Parent Chat");
        let worker = factr_base::session::Session::create(Some("parent".into()), Some("scan (@general swarm)".into()));
        let top = factr_base::session::Session::create(None, None);
        assert!(is_user_branch(&branch), "a /branch learns like a top-level chat");
        assert!(!is_user_branch(&worker), "a sub-agent keeps depth>0");
        assert!(!is_user_branch(&top));
    }

    #[tokio::test]
    async fn goal_slash_returns_a_send_so_the_desktop_starts_the_first_turn() {
        let conn = test_conn("goal-send");
        let run = |c: &str| json!({ "command": c, "session_id": "goal-s1" });
        let set = conn.dispatch("slash.exec", &run("/goal create goal_demo.py")).await.unwrap();
        assert_eq!((set["type"].as_str(), set["status"].as_str()), (Some("send"), Some("ok")), "{set}");
        assert!(set["notice"].as_str().unwrap().contains("Goal set (20-turn budget)"));
        assert!(set["message"].as_str().unwrap().contains("create goal_demo.py"));
        assert_eq!(set["display"], "Goal: create goal_demo.py");
        let via = conn.dispatch("command.dispatch", &json!({ "name": "goal", "arg": "status", "session_id": "goal-s1" })).await.unwrap();
        assert_eq!(via["type"], "exec");
        let paused = conn.dispatch("slash.exec", &run("/goal pause")).await.unwrap();
        assert_eq!(paused["type"], "exec");
        let resumed = conn.dispatch("slash.exec", &run("/goal resume")).await.unwrap();
        assert_eq!(resumed["type"], "send");
        let cleared = conn.dispatch("slash.exec", &run("/goal clear")).await.unwrap();
        assert_eq!(cleared["type"], "exec");
        let bad = conn.dispatch("slash.exec", &run("/goal --turns 3")).await.unwrap();
        assert_eq!(bad["type"], "exec", "a usage error starts nothing");
        // /goal carries the old /autonomous flags and Factr's gate / show / wait subcommands.
        let gated = conn.dispatch("slash.exec", &run("/goal fix it --gate true --max-turns 2")).await.unwrap();
        assert_eq!(gated["type"], "send", "{gated}");
        let shown = conn.dispatch("slash.exec", &run("/goal gate add cargo test")).await.unwrap();
        assert!(shown["output"].as_str().unwrap().contains("Gate added: $ cargo test"), "{shown}");
        let waiting = conn.dispatch("slash.exec", &run("/goal wait 1 the build")).await.unwrap();
        assert!(waiting["output"].as_str().unwrap().contains("parked on pid 1"), "{waiting}");
        let unwaited = conn.dispatch("slash.exec", &run("/goal unwait")).await.unwrap();
        assert!(unwaited["output"].as_str().unwrap().contains("Wait barrier cleared"), "{unwaited}");
        // /loop (aliases /heartbeat, /hb, /proactive) is the one scheduler.
        let looped = conn.dispatch("slash.exec", &run("/proactive 5m ping --times 2")).await.unwrap();
        assert!(looped["output"].as_str().unwrap().contains("Loop set: every 5m"), "{looped}");
        let status = conn.dispatch("slash.exec", &run("/hb status")).await.unwrap();
        assert!(status["output"].as_str().unwrap().contains("0/2 ticks"), "{status}");
        conn.dispatch("slash.exec", &run("/loop stop")).await.unwrap();
        conn.dispatch("slash.exec", &run("/goal clear")).await.unwrap();
    }

    #[tokio::test]
    async fn slash_popup_is_served_natively_and_dispatch_follows_the_table() {
        let conn = test_conn("slash-popup");
        let cat = conn.dispatch("commands.catalog", &json!({})).await.unwrap();
        let keys: Vec<&str> = cat["pairs"].as_array().unwrap().iter().map(|p| p[0].as_str().unwrap()).collect();
        assert!(keys.contains(&"/compress") && keys.contains(&"/branch") && keys.contains(&"/steer") && !keys.contains(&"/moa"), "{keys:?}");
        let done = conn.dispatch("complete.slash", &json!({ "text": "/fo" })).await.unwrap();
        assert_eq!(done["items"][0]["text"], "/branch");
        let said = |v: Value| v["output"].as_str().unwrap().to_string();
        let exec = |c: &str| json!({ "command": c, "session_id": "" });
        assert!(said(conn.dispatch("slash.exec", &exec("/compact")).await.unwrap()).starts_with("/compress needs an open session"));
        assert!(said(conn.dispatch("slash.exec", &exec("/fork")).await.unwrap()).starts_with("/branch needs an open session"));
        assert!(said(conn.dispatch("slash.exec", &exec("/steer")).await.unwrap()).starts_with("/steer needs an open session"));
        assert!(said(conn.dispatch("slash.exec", &exec("/moa hi")).await.unwrap()).contains("is not available yet"));
        let memory = said(conn.dispatch("command.dispatch", &json!({ "name": "memory", "arg": "pending" })).await.unwrap());
        assert!(memory.contains("quality gate") && memory.contains("Settings > Memory"), "{memory}");
        assert!(said(conn.dispatch("slash.exec", &exec("/autonomous on")).await.unwrap()).contains("not a command this engine serves"), "/autonomous is /goal now");
        assert!(said(conn.dispatch("slash.exec", &exec("/nonesuch")).await.unwrap()).contains("not a command this engine serves"));
        // /focus needs no session and round-trips through config display.focus_view.
        assert_eq!(said(conn.dispatch("slash.exec", &exec("/focus on")).await.unwrap()), "focus view on");
        assert_eq!(conn.dispatch("config.get", &json!({ "key": "display.focus_view" })).await.unwrap()["value"], true);
        assert_eq!(said(conn.dispatch("slash.exec", &exec("/focus")).await.unwrap()), "focus view off");
        assert_eq!(said(conn.dispatch("slash.exec", &exec("/focus status")).await.unwrap()), "focus view off");
        assert!(said(conn.dispatch("slash.exec", &exec("/focus bogus")).await.unwrap()).starts_with("usage:"));
        assert!(said(conn.dispatch("slash.exec", &exec("/new")).await.unwrap()).contains("desktop app"));
        // Quick and plugin commands come from the backend once and are cached; built-ins are not taken.
        let asked = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = asked.clone();
        *conn.forward_stub.lock().unwrap() = Some(Arc::new(move |m, _| {
            assert_eq!(m, "commands.catalog");
            counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok(json!({ "pairs": [["/deploy", "Ship it"], ["/moa", "x"]], "categories": [{ "name": "User commands", "pairs": [["/deploy", "Ship it"]] }], "skills": {} }))
        }));
        for _ in 0..2 {
            let cat = conn.dispatch("commands.catalog", &json!({})).await.unwrap();
            let keys: Vec<&str> = cat["pairs"].as_array().unwrap().iter().map(|p| p[0].as_str().unwrap()).collect();
            assert!(keys.contains(&"/deploy") && !keys.contains(&"/moa"), "{keys:?}");
        }
        assert_eq!(asked.load(std::sync::atomic::Ordering::Relaxed), 1);
        // Factr-routed rows are forwarded by canonical name, and the reply is final.
        let seen = Arc::new(std::sync::Mutex::new(Vec::<(String, Value)>::new()));
        let log = seen.clone();
        *conn.forward_stub.lock().unwrap() = Some(Arc::new(move |m, p| {
            log.lock().unwrap().push((m.to_string(), p.clone()));
            Ok(json!({ "type": "send", "message": "planned" }))
        }));
        let plan = conn.dispatch("slash.exec", &exec("/plan x")).await.unwrap();
        assert_eq!(plan["message"], "planned");
        let queued = conn.dispatch("slash.exec", &exec("/q later")).await.unwrap();
        assert_eq!(queued["message"], "planned");
        let seen = seen.lock().unwrap();
        assert_eq!(seen[0], ("command.dispatch".to_string(), json!({ "name": "plan", "arg": "x" })));
        assert_eq!(seen[1].1, json!({ "name": "queue", "arg": "later" }));
    }

    /// `/<skill>` is expanded from the engine registry and never reaches Factr, with or without a backend.
    #[tokio::test]
    async fn curator_rpcs_answer_not_available_without_forwarding() {
        let conn = test_conn("curator-rpc");
        let seen = Arc::new(std::sync::Mutex::new(0));
        let log = seen.clone();
        *conn.forward_stub.lock().unwrap() = Some(Arc::new(move |_, _| {
            *log.lock().unwrap() += 1;
            Ok(json!({}))
        }));
        let err = conn.dispatch("curator.status", &json!({})).await.unwrap_err();
        assert_eq!(err.message, crate::CURATOR_UNAVAILABLE);
        assert_eq!(*seen.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn a_skill_slash_command_is_answered_natively_and_never_forwarded() {
        let conn = test_conn("skill-slash");
        let home = std::env::temp_dir().join(format!("rpc-skill-slash-{}", std::process::id()));
        std::fs::create_dir_all(home.join("skills/nativeskill")).unwrap();
        std::fs::write(home.join("skills/nativeskill/SKILL.md"), "---\nname: nativeskill\ndescription: Does a thing\n---\nBody.\n").unwrap();
        let _env = crate::factr_env::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let saved = ["FACTR_HOME", "FACTR_CONFIG_HOME"].map(|k| (k, std::env::var_os(k)));
        // SAFETY: env is only touched under ENV_LOCK.
        unsafe {
            std::env::set_var("FACTR_HOME", &home);
            std::env::remove_var("FACTR_CONFIG_HOME");
        }
        let seen = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let log = seen.clone();
        *conn.forward_stub.lock().unwrap() = Some(Arc::new(move |m, _| {
            log.lock().unwrap().push(m.to_string());
            Err(RpcError { code: -32000, message: "forwarded".into(), data: None })
        }));
        let dispatched = conn.dispatch("command.dispatch", &json!({ "name": "nativeskill", "arg": "go" })).await.unwrap();
        let exec = conn.dispatch("slash.exec", &json!({ "command": "/nativeskill go" })).await.unwrap();
        let listed = conn.dispatch("complete.slash", &json!({ "text": "/native" })).await.unwrap();
        for (k, v) in saved {
            unsafe {
                match v {
                    Some(v) => std::env::set_var(k, v),
                    None => std::env::remove_var(k),
                }
            }
        }
        for done in [&dispatched, &exec] {
            assert_eq!(done["type"], "skill");
            assert_eq!(done["display"], "/nativeskill go");
            assert!(done["message"].as_str().unwrap().contains("The full skill content is loaded below."));
        }
        assert!(listed["items"].as_array().unwrap().iter().any(|i| i["text"] == "/nativeskill"), "{listed}");
        assert!(seen.lock().unwrap().is_empty(), "no Factr round trip: {:?}", seen.lock().unwrap());
    }

    /// D13: the Cron screen opens a run from Run history through `session.resume` (and `session.history`).
    /// A cron run is not an engine session; it answers as a finished, read-only transcript.
    #[tokio::test]
    async fn a_cron_run_opens_as_a_read_only_transcript_through_session_resume() {
        let factr = crate::cron_runs::tests::home();
        let conn = test_conn("cron-run");
        let _env = crate::factr_env::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let before = std::env::var_os("FACTR_CONFIG_HOME");
        // SAFETY: env is only touched under ENV_LOCK.
        unsafe { std::env::set_var("FACTR_CONFIG_HOME", &factr) };
        let done = conn.dispatch("session.resume", &json!({ "session_id": "cron_job1_20261002_235300" })).await;
        let failed = conn.dispatch("session.resume", &json!({ "session_id": "cron_job1_20261002_235601" })).await;
        let history = conn.dispatch("session.history", &json!({ "session_id": "cron_job1_20261002_235300" })).await;
        match before { Some(v) => unsafe { std::env::set_var("FACTR_CONFIG_HOME", v) }, None => unsafe { std::env::remove_var("FACTR_CONFIG_HOME") } }
        let done = done.unwrap();
        assert_eq!(done["running"], false);
        assert_eq!(done["messages"][0]["text"], "Reply with exactly the word CRONOK.");
        assert_eq!(done["messages"][1]["role"], "assistant");
        assert_eq!(done["messages"][1]["text"], "CRONOK");
        let failed = failed.unwrap();
        assert_eq!(failed["messages"][1]["text"], "Run failed: RemoteDisconnected: Remote end closed connection without response");
        assert_eq!(history.unwrap()["count"], 2);
        let _ = std::fs::remove_dir_all(factr);
    }

    /// D15: readiness must follow the credentials, not a startup snapshot. With the provider not
    /// ready at first look (no login yet), a Factr openai-codex login that appears afterwards (fake
    /// tokens) is reported by the very next `setup.status` / `setup.runtime_check`, not 60 s later.
    #[tokio::test]
    async fn a_login_that_appears_after_startup_is_ready_on_the_next_readiness_probe() {
        let _env = crate::factr_env::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("ready-probe-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let factr = dir.join("factr");
        std::fs::create_dir_all(&factr).unwrap();
        let before = (std::env::var_os("FACTR_HOME"), std::env::var_os("FACTR_CONFIG_HOME"));
        // SAFETY: env is only touched under ENV_LOCK.
        unsafe {
            std::env::set_var("FACTR_HOME", dir.join("factr"));
            std::env::set_var("FACTR_CONFIG_HOME", &factr);
        }
        crate::register_credentials();
        let (mut config, home) = test_config("ready-probe");
        Arc::get_mut(&mut config).unwrap().provider = "openai".into();
        let observer = Observer::open(&home, "p", "m", None).unwrap();
        let hub = Arc::new(Hub::default());
        let (to_ws, _rx) = mpsc::channel::<Message>(8);
        let client = Arc::new(Client { id: hub.next_client_id(), to_ws: to_ws.clone(), sessions: Mutex::new(Default::default()) });
        let conn = Conn::new(config, to_ws, hub, client, observer, false, "invoke_agent", None, None);
        factr_base::auth::AuthStatus::invalidate_cache();
        let first = conn.dispatch("setup.status", &json!({})).await.unwrap();
        let first_check = conn.dispatch("setup.runtime_check", &json!({})).await.unwrap();
        // `factr login openai-codex` writes the Factr store (fake, far-future grant).
        std::fs::write(
            factr.join("auth.json"),
            r#"{"credential_pool":{"openai-codex":[{"auth_type":"oauth","access_token":"fake-access","refresh_token":"fake-refresh","expires_at":"2999-01-01T00:00:00Z"}]}}"#,
        )
        .unwrap();
        let next = conn.dispatch("setup.status", &json!({})).await.unwrap();
        let next_check = conn.dispatch("setup.runtime_check", &json!({})).await.unwrap();
        for (k, v) in [("FACTR_HOME", before.0), ("FACTR_CONFIG_HOME", before.1)] {
            match v { Some(v) => unsafe { std::env::set_var(k, v) }, None => unsafe { std::env::remove_var(k) } }
        }
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!((first["provider_configured"].clone(), first_check["ok"].clone()), (json!(false), json!(false)), "no login yet");
        assert_eq!(next["provider_configured"], true, "{next}");
        assert_eq!(next_check["ok"], true, "{next_check}");
    }

    pub(super) fn test_conn(name: &str) -> Arc<Conn> {
        test_conn_as(name, false)
    }

    /// The tasks pumping `link` on `conn`.
    pub(super) async fn tasks_of(conn: &Conn, link: &mpsc::Sender<String>) -> LinkTasks {
        let tasks = conn.link_tasks.lock().await;
        tasks.iter().find(|t| t.link.upgrade().is_some_and(|l| l.same_channel(link))).cloned().expect("a link this connection opened")
    }

    pub(super) fn test_conn_as(name: &str, driver: bool) -> Arc<Conn> {
        test_conn_events(name, driver).0
    }

    /// A test connection whose config names a Factr backend command (never started unless asked).
    #[cfg(unix)]
    pub(super) fn test_conn_with_backend(name: &str, command: &str) -> Arc<Conn> {
        let (config, home) = test_config_with(name, Some(Arc::new(crate::features::Features::new(vec![command.into()]))));
        let observer = Observer::open(&home, "p", "m", None).unwrap();
        let hub = Arc::new(Hub::default());
        let (to_ws, _rx) = mpsc::channel::<Message>(8);
        let client = Arc::new(Client { id: hub.next_client_id(), to_ws: to_ws.clone(), sessions: Mutex::new(Default::default()) });
        Conn::new(config, to_ws, hub, client, observer, false, "invoke_agent", None, None)
    }

    /// A test connection plus the receiver of the frames it sends to its window.
    pub(super) fn test_conn_events(name: &str, driver: bool) -> (Arc<Conn>, mpsc::Receiver<Message>) {
        let (config, home) = test_config(name);
        let observer = Observer::open(&home, "p", "m", None).unwrap();
        let hub = Arc::new(Hub::default());
        let (to_ws, rx) = mpsc::channel::<Message>(64);
        let client = Arc::new(Client { id: hub.next_client_id(), to_ws: to_ws.clone(), sessions: Mutex::new(Default::default()) });
        (Conn::new(config, to_ws, hub, client, observer, driver, "invoke_agent", None, None), rx)
    }

    fn test_config(name: &str) -> (Arc<Config>, std::path::PathBuf) {
        test_config_with(name, None)
    }

    fn test_config_with(name: &str, features: Option<Arc<crate::features::Features>>) -> (Arc<Config>, std::path::PathBuf) {
        crate::factr_env::sandbox_homes();
        let home = std::env::temp_dir().join(format!("c{name}{}", std::process::id()));
        std::fs::create_dir_all(&home).unwrap();
        // A stand-in daemon: accepts the bridge's dials and holds them open.
        let socket = home.join("d.sock");
        let _ = std::fs::remove_file(&socket);
        // (Unix sockets only; elsewhere the path is just never dialled.)
        #[cfg(unix)]
        {
            let listener = tokio::net::UnixListener::bind(&socket).unwrap();
            tokio::spawn(async move {
                let mut held = Vec::new();
                while let Ok((stream, _)) = listener.accept().await {
                    held.push(stream);
                }
            });
        }
        let config = Arc::new(Config {
            bind: "127.0.0.1:0".parse().unwrap(),
            token: "t".repeat(32),
            version: "test".into(),
            legacy_socket: socket,
            default_cwd: "/".into(),
            allow_non_loopback: false,
            provider: "p".into(),
            model: "m".into(),
            reasoning_efforts: Vec::new(),
            profile_model_applies: true,
            home: home.to_string_lossy().into(),
            complete: None,
            features,
            learning: None,
        });
        (config, home)
    }

    /// A real socket pair: the gateway's `run` on one end, a raw client on the other.
    async fn gateway_socket(name: &str) -> WebSocketStream<TcpStream> {
        let (config, home) = test_config(name);
        let observer = Observer::open(&home, "p", "m", None).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let ws = WebSocketStream::from_raw_socket(stream, tokio_tungstenite::tungstenite::protocol::Role::Server, Some(crate::ws_config())).await;
            let _ = run(ws, config, Arc::new(Hub::default()), observer).await;
        });
        let stream = TcpStream::connect(addr).await.unwrap();
        WebSocketStream::from_raw_socket(stream, tokio_tungstenite::tungstenite::protocol::Role::Client, None).await
    }

    async fn reply_to(ws: &mut WebSocketStream<TcpStream>, id: u64) -> Value {
        loop {
            let Message::Text(text) = tokio::time::timeout(Duration::from_secs(20), ws.next()).await.expect("a reply").expect("socket open").expect("no error") else { continue };
            let frame: Value = serde_json::from_str(&text).unwrap();
            if frame["id"] == id {
                return frame;
            }
        }
    }

    #[tokio::test]
    async fn a_model_pick_becomes_the_default_for_new_sessions_unless_it_is_session_only() {
        let _env = crate::factr_env::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let conn = test_conn("model-persist");
        let default = |conn: &Arc<Conn>| crate::profile::effective_default(&conn.config);
        assert_eq!(default(&conn), ("m".to_string(), "p".to_string()));

        // `--session` is a one-off: nothing is saved.
        let reply = conn.dispatch("config.set", &json!({ "key": "model", "value": "gpt-5.6-luna --provider openai --session" })).await.unwrap();
        assert_eq!(reply["scope"], "session");
        assert_eq!(default(&conn).0, "m");

        // A pick with no live session (draft chat, Settings) is the user's last choice.
        let reply = conn.dispatch("config.set", &json!({ "key": "model", "value": "gpt-5.6-luna --provider openai" })).await.unwrap();
        assert_eq!((reply["value"].as_str(), reply["scope"].as_str()), (Some("gpt-5.6-luna"), Some("global")));
        assert_eq!(default(&conn), ("gpt-5.6-luna".to_string(), "openai".to_string()));
        // config.get, model.options and a restart (a new connection on the same home) all agree.
        assert_eq!(conn.dispatch("config.get", &json!({ "key": "model" })).await.unwrap()["model"], "gpt-5.6-luna");
        let options = conn.dispatch("model.options", &json!({})).await.unwrap();
        assert_eq!((options["model"].as_str(), options["provider"].as_str()), (Some("gpt-5.6-luna"), Some("openai")));
        assert_eq!(options["reasoning_effort"], "low", "a draft chat's pill needs the engine default before any session exists");
        let again = test_conn("model-persist");
        assert_eq!(default(&again).0, "gpt-5.6-luna");
        let err = conn.dispatch("config.set", &json!({ "key": "model", "value": "--provider openai" })).await.unwrap_err();
        assert_eq!(err.code, INVALID_PARAMS);
    }

    #[test]
    fn the_engine_default_effort_is_reported_only_for_providers_that_take_one() {
        let _env = crate::factr_env::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(configured_default_effort("ollama"), None);
        assert_eq!(configured_default_effort("openai"), Some("low"), "the shipped default is low, which the request carries");
        assert_eq!(configured_default_effort("OpenAI"), Some("low"), "a display name resolves to the provider id");
    }

    #[tokio::test]
    async fn a_saved_pick_names_the_models_own_provider_and_a_swarm_effort_is_never_the_default() {
        let _env = crate::factr_env::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let conn = test_conn("persist-owner");
        // The served provider is "p"; a pick that names no provider is saved under the model's own.
        conn.persist_default_model("gpt-5.6-luna", None).unwrap();
        let dir = std::path::PathBuf::from(&conn.config.home);
        let (provider, model, _) = crate::profile::parse_config(&std::fs::read_to_string(dir.join("config.yaml")).unwrap());
        assert_eq!((provider.as_deref(), model.as_deref()), (Some("openai"), Some("gpt-5.6-luna")));
        // A model no provider claims stays on the one being served.
        let other = test_conn("persist-owner-local");
        let other_dir = std::path::PathBuf::from(&other.config.home);
        other.persist_default_model("local-model", None).unwrap();
        let (provider, _, _) = crate::profile::parse_config(&std::fs::read_to_string(other_dir.join("config.yaml")).unwrap());
        assert_eq!(provider.as_deref(), Some("p"));
        // The swarm sentinel is a per-chat mode: saving it is a no-op, and the global setter refuses it.
        conn.persist_default_effort("low").unwrap();
        conn.persist_default_effort("swarm").unwrap();
        conn.persist_default_effort("swarm-deep").unwrap();
        assert_eq!(parse_effort(&dir), Some("low".to_string()));
        let err = conn.dispatch("config.set", &json!({ "key": "reasoning", "value": "swarm" })).await.unwrap_err();
        assert_eq!(err.code, INVALID_PARAMS);
        assert_eq!(parse_effort(&dir), Some("low".to_string()));
    }

    #[tokio::test]
    async fn a_put_config_toggle_reads_back_through_config_get_and_survives_a_new_connection() {
        let _env = crate::factr_env::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let conn = test_conn("put-config-rpc");
        let home = std::path::PathBuf::from(&conn.config.home);
        super::put_config(&home, &json!({ "memory": { "memory_enabled": false }, "browser": { "use_real_profile": true }, "approvals": { "mode": "off" }, "agent": { "reasoning_effort": "low" } })).unwrap();
        let again = test_conn("put-config-rpc");
        for c in [&conn, &again] {
            assert_eq!(c.dispatch("config.get", &json!({ "key": "memory.memory_enabled" })).await.unwrap()["value"], false);
            assert_eq!(c.dispatch("config.get", &json!({ "key": "browser.use_real_profile" })).await.unwrap()["value"], true);
            assert_eq!(c.dispatch("config.get", &json!({ "key": "approvals.mode" })).await.unwrap()["value"], "off");
            assert_eq!(c.dispatch("config.get", &json!({ "key": "reasoning" })).await.unwrap()["value"], "low");
        }
        assert_eq!(super::put_config(&home, &json!({ "approvals": { "mode": "smart" } })).unwrap_err().0, true);
    }

    #[tokio::test]
    async fn a_saved_effort_pick_is_the_default_for_new_sessions_after_a_restart() {
        let _env = crate::factr_env::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let conn = test_conn("effort-persist");
        conn.persist_default_effort("low").unwrap();
        let again = test_conn("effort-persist");
        assert_eq!(again.dispatch("config.get", &json!({ "key": "reasoning" })).await.unwrap()["value"], "low");
        // What session.create applies to a new chat comes from the same key.
        let dir = std::path::PathBuf::from(&again.config.home);
        assert_eq!(parse_effort(&dir), Some("low".to_string()));
        assert!(conn.persist_default_effort("bogus-level").is_err(), "an unknown level is refused, not saved");
        assert_eq!(parse_effort(&dir), Some("low".to_string()));
    }

    fn parse_effort(dir: &std::path::Path) -> Option<String> {
        crate::profile::parse_config(&std::fs::read_to_string(dir.join("config.yaml")).ok()?).2
    }

    #[tokio::test]
    async fn an_effort_the_sessions_model_cannot_take_is_refused_not_stashed() {
        let _env = crate::factr_env::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let conn = test_conn("effort-reject");
        conn.known.lock().await.insert("s1".into(), json!({ "model": "gpt-5-pro" }));
        let err = conn.dispatch("config.set", &json!({ "key": "reasoning", "session_id": "s1", "value": "low" })).await.unwrap_err();
        assert_eq!(err.code, 4002, "{}", err.message);
        assert!(err.message.contains("gpt-5-pro") && err.message.contains("high"), "{}", err.message);
        // The default for new sessions is checked the same way once it names a known route.
        conn.dispatch("config.set", &json!({ "key": "model", "value": "gpt-5-pro --provider openai" })).await.unwrap();
        let err = conn.dispatch("config.set", &json!({ "key": "reasoning", "value": "minimal" })).await.unwrap_err();
        assert_eq!(err.code, 4002);
        assert_eq!(conn.dispatch("config.set", &json!({ "key": "reasoning", "value": "high" })).await.unwrap()["value"], "high");
    }

    #[tokio::test]
    async fn private_mode_refuses_credential_rpcs_and_public_mode_does_not() {
        let _g = crate::factr_env::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut ws = gateway_socket("private-rpc").await;
        factr_base::factr_env::set_deployment_override_for_tests(Some(true));
        for (id, method) in [(1, "model.save_key"), (2, "model.disconnect")] {
            ws.send(Message::Text(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": {} }).to_string())).await.unwrap();
            assert_eq!(reply_to(&mut ws, id).await["error"]["message"], crate::deployment::PRIVATE_MSG, "{method}");
        }
        factr_base::factr_env::set_deployment_override_for_tests(Some(false));
        ws.send(Message::Text(json!({ "jsonrpc": "2.0", "id": 3, "method": "model.save_key", "params": {} }).to_string())).await.unwrap();
        assert_ne!(reply_to(&mut ws, 3).await["error"]["message"], crate::deployment::PRIVATE_MSG);
        factr_base::factr_env::set_deployment_override_for_tests(None);
    }

    #[tokio::test]
    async fn a_6_5_mb_image_attach_gets_a_reply_and_the_socket_stays_open() {
        let mut ws = gateway_socket("big-attach").await;
        // 6.5 MB of image is ~8.7 MB of base64: over the old 8 MiB message cap.
        let payload = "A".repeat(6_500_000 / 3 * 4);
        let request = json!({ "jsonrpc": "2.0", "id": 1, "method": "image.attach_bytes", "params": { "session_id": "nobody", "content_base64": payload } });
        ws.send(Message::Text(request.to_string())).await.unwrap();
        let reply = reply_to(&mut ws, 1).await;
        assert_eq!(reply["error"]["message"], "unknown session_id", "answered, not disconnected: {reply}");
        ws.send(Message::Text(json!({ "jsonrpc": "2.0", "id": 2, "method": "ping" }).to_string())).await.unwrap();
        assert!(reply_to(&mut ws, 2).await["result"].is_object(), "the same connection still works");
    }

    #[tokio::test]
    async fn a_message_past_the_cap_ends_the_connection_without_hanging() {
        let mut ws = gateway_socket("too-big").await;
        // The gateway refuses it from the frame header and closes (1009 when the close frame beats
        // the TCP reset); the client's own write may fail once that happens.
        let _ = ws.send(Message::Text("x".repeat(crate::MAX_WS_MESSAGE_BYTES + 1))).await;
        let ended = tokio::time::timeout(Duration::from_secs(20), async {
            while let Some(Ok(msg)) = ws.next().await {
                if let Message::Close(frame) = msg {
                    return frame.map(|f| u16::from(f.code));
                }
            }
            None
        })
        .await
        .expect("the gateway ended the connection");
        assert!(matches!(ended, None | Some(1009)), "{ended:?}");
    }

    #[tokio::test]
    async fn a_link_whose_bridge_closed_is_dropped_and_reopened() {
        let conn = test_conn("dead-link");
        conn.ensure_control().await.unwrap();
        let first = conn.control.lock().await.clone().unwrap();
        tasks_of(&conn, &first).await.bridge.abort(); // the bridge dies
        tokio::time::timeout(Duration::from_secs(5), async {
            while conn.control.lock().await.is_some() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the reader forgets the closed control link");
        assert!(conn.route(&json!({ "req": "list_sessions" })).await.is_err(), "not trusted while closed");
        conn.ensure_control().await.unwrap();
        let second = conn.control.lock().await.clone().unwrap();
        assert!(!second.same_channel(&first) && !second.is_closed());
        conn.route(&json!({ "req": "list_sessions" })).await.unwrap();
        // A session link whose writer is gone is likewise not trusted.
        tasks_of(&conn, &first).await.writer.abort();
        tokio::time::timeout(Duration::from_secs(5), first.closed()).await.expect("writer gone");
        conn.links.lock().await.insert("s".into(), first);
        assert!(conn.route(&json!({ "session_id": "s" })).await.unwrap().same_channel(&second), "falls back to the control link");
    }

    #[tokio::test]
    async fn a_headless_run_is_never_auto_reviewed_and_no_model_is_called() {
        let (mut config, home) = test_config("headless-learn");
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        {
            let (counter, config) = (calls.clone(), Arc::get_mut(&mut config).unwrap());
            config.learning = Some(crate::learn::Learning { turn_interval: 1, cooldown: Duration::ZERO });
            config.complete = Some(Arc::new(move |_system, _user| {
                counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Box::pin(async { Err(anyhow!("no model in this test")) })
            }));
        }
        let observer = Observer::open(&home, "p", "m", None).unwrap();
        let hub = Arc::new(Hub::default());
        let (to_ws, _rx) = mpsc::channel::<Message>(8);
        let client = Arc::new(Client { id: hub.next_client_id(), to_ws: to_ws.clone(), sessions: Mutex::new(Default::default()) });
        let conn = Conn::new(config, to_ws, hub, client, observer, false, "invoke_agent", None, None);
        let spans = Arc::new(std::sync::Mutex::new(Vec::<Span>::new()));
        for attempt in 1..=5 {
            spans.lock().unwrap().clear();
            let sink = spans.clone();
            // Another test may install its own recorder at any moment: retry the whole flow.
            factr_base::obs_sink::install(move |span| sink.lock().unwrap().push(span));
            conn.learning_check("s-headless", Trigger::TurnInterval, true).await;
            let skipped = spans.lock().unwrap().iter().any(|s| s.kind == "learning.skip" && s.session_id.as_deref() == Some("s-headless") && s.attributes["reason"] == "headless");
            if skipped {
                break;
            }
            assert!(attempt < 5, "no learning.skip reason=headless span");
        }
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn a_failed_run_still_ends_its_registration_and_link_tasks() {
        let conn = test_conn("end-run");
        conn.ensure_control().await.unwrap();
        ACTIVE_RUNS.lock().unwrap().insert("telegram:end-run".into(), (conn.clone(), "s".into()));
        end_run(&conn, Some("telegram:end-run")).await;
        assert!(!ACTIVE_RUNS.lock().unwrap().contains_key("telegram:end-run"));
        assert!(conn.link_tasks.lock().await.is_empty());
    }

    #[tokio::test]
    async fn a_headless_run_lets_its_learning_pass_finish_before_its_links_close() {
        let conn = test_conn("end-run-learning");
        conn.ensure_control().await.unwrap();
        let (release, held) = oneshot::channel::<()>();
        conn.learning_tasks.lock().unwrap().push(tokio::spawn(async move {
            let _ = held.await;
        }));
        end_run(&conn, None).await; // returns at once, links stay up for the pass
        assert!(!conn.link_tasks.lock().await.is_empty(), "the pass still needs its history link");
        release.send(()).unwrap();
        for _ in 0..50 {
            if conn.link_tasks.lock().await.is_empty() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("the links were never closed after the pass finished");
    }

    /// The desktop treats only `working` / `waiting` as a live turn; `running` or `streaming` (what
    /// this once answered) read as idle, so its poll cleared busy and Stop vanished mid-turn.
    #[test]
    fn active_list_reports_the_statuses_the_desktop_reads_as_busy() {
        assert_eq!(live_status(false), "working");
        assert_eq!(live_status(true), "waiting");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn the_launch_time_catalog_does_not_start_the_python_backend() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("catalog-lazy-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (script, marker) = (dir.join("factr.sh"), dir.join("started"));
        std::fs::write(&script, format!("#!/bin/sh\ntouch '{}'\n", marker.display())).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let conn = test_conn_with_backend("catalog-lazy", script.to_str().unwrap());
        let cat = conn.dispatch("commands.catalog", &json!({})).await.unwrap();
        assert!(cat["pairs"].as_array().is_some_and(|p| !p.is_empty()), "the native catalog is served");
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(!marker.exists(), "no backend process was launched for the catalog");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The requests the packaged desktop sends within the first seconds of launch (traced with
    /// FACTR_TRACE_FORWARD against the bundled app, tests/gui/pytime.mjs): none may start Python.
    #[cfg(unix)]
    #[tokio::test]
    async fn the_desktop_boot_sequence_does_not_start_the_python_backend() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("boot-lazy-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (script, marker) = (dir.join("factr.sh"), dir.join("started"));
        std::fs::write(&script, format!("#!/bin/sh\ntouch '{}'\n", marker.display())).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        // R5-3: a home with Settings keys persisted and an MCP server configured (the state that
        // looked like it started Python by itself) boots without Python as well. The MCP server is
        // the engine's to load for chats; the Python backend is not needed for it.
        let _env = crate::factr_env::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let factr = dir.join("factr-home");
        std::fs::create_dir_all(&factr).unwrap();
        std::fs::write(
            factr.join("config.yaml"),
            "display:\n  show_reasoning: true\n  resume_last_session: true\n  in_app_tips: true\ndesktop:\n  repo_scan_enabled: true\n\
             browser:\n  use_real_profile: false\nmemory:\n  memory_enabled: true\n  user_profile_enabled: true\nvoice:\n  client_direct: true\n\
             sessions:\n  auto_archive: false\nagent:\n  reasoning_effort: high\napprovals:\n  mode: manual\n  mcp_reload_confirm: true\n\
             mcp_servers:\n  guitest:\n    command: python3\n    args: [\"/nonexistent/mcp_server.py\"]\n",
        )
        .unwrap();
        let before = std::env::var_os("FACTR_CONFIG_HOME");
        // SAFETY: env is only touched under ENV_LOCK.
        unsafe { std::env::set_var("FACTR_CONFIG_HOME", &factr) };
        let conn = test_conn_with_backend("boot-lazy", script.to_str().unwrap());
        let mut boot: Vec<&str> = crate::boot_stubs::METHODS.to_vec();
        boot.push("commands.catalog");
        for method in &boot {
            let reply = conn.dispatch(method, &json!({})).await;
            assert!(reply.is_ok(), "{method} is answered at boot: {reply:?}");
        }
        // The settings the desktop reads and mirrors when it connects, including keys Python owns.
        for key in ["approvals.mode", "reasoning", "fast", "yolo", "model", "voice.voice_chat_mode", "display.resume_last_session", "memory.user_profile_enabled", "browser.use_real_profile", "not.a.key.the.engine.owns"] {
            let reply = conn.dispatch("config.get", &json!({ "key": key })).await;
            assert!(reply.is_ok(), "config.get {key} is answered at boot: {reply:?}");
        }
        for (key, value) in [("display.message_reactions", "false"), ("display.in_app_tips", "true"), ("display.in_app_tours", "false")] {
            let reply = conn.dispatch("config.set", &json!({ "key": key, "value": value })).await;
            assert!(reply.is_ok(), "config.set {key} is answered at boot: {reply:?}");
        }
        // What the gateway's own timers do at boot: neither the bot check nor the cron scan has a
        // reason to start Python for this config (no platform enabled, no job due).
        let features = conn.config.features.clone().unwrap();
        features.ensure_bots(&factr).await;
        assert!(crate::cron_tick::fireable(&factr).is_empty(), "no cron job is due");
        tokio::time::sleep(Duration::from_millis(300)).await;
        match before {
            Some(v) => unsafe { std::env::set_var("FACTR_CONFIG_HOME", v) },
            None => unsafe { std::env::remove_var("FACTR_CONFIG_HOME") },
        }
        assert!(!marker.exists(), "a boot request launched the Python backend");
        // A request that truly needs Python still starts it.
        let _ = conn.dispatch("cron.manage", &json!({"action": "list"})).await;
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert!(marker.exists(), "a Python-owned request starts the backend on demand");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_quiet_chat_is_extracted_once_and_a_new_prompt_or_close_cancels_the_timer() {
        let conn = test_conn("idle-extract");
        let fired = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let fire: Arc<dyn Fn(&str) + Send + Sync> = { let f = fired.clone(); Arc::new(move |id| f.lock().unwrap().push(id.to_string())) };
        let grace = Duration::from_millis(150);
        // Two turns in a row: only the last one's timer fires, once.
        conn.arm_idle_extraction("a", grace, fire.clone());
        conn.arm_idle_extraction("a", grace, fire.clone());
        // A chat that gets a prompt (or is closed) before the timer is not extracted yet.
        conn.arm_idle_extraction("b", grace, fire.clone());
        conn.cancel_idle_extraction("b");
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert_eq!(*fired.lock().unwrap(), vec!["a".to_string()]);
        assert!(conn.idle_extract.lock().unwrap().is_empty(), "no timer is left behind");
        // The clock restarts after the next turn.
        conn.arm_idle_extraction("a", grace, fire.clone());
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert_eq!(fired.lock().unwrap().len(), 2);
    }

    /// The dispose review runs after `session.close` dropped the link: reading the closed chat must
    /// not attach it again (nothing would ever close that link).
    #[tokio::test]
    async fn reading_a_closed_chats_history_does_not_reattach_it() {
        use factr_base::message::{ContentBlock, Role};
        let conn = test_conn("closed-history");
        let _env = crate::factr_env::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("closed-history-{}", std::process::id()));
        let before = std::env::var_os("FACTR_HOME");
        unsafe { std::env::set_var("FACTR_HOME", &dir) };
        let text = |t: &str| vec![ContentBlock::Text { text: t.into(), cache_control: None }];
        let mut chat = factr_base::session::Session::create(None, None);
        chat.add_message(Role::User, text("hello"));
        chat.add_message(Role::Assistant, text("hi"));
        chat.save().unwrap();
        let history = conn.history(&chat.id).await;
        match before {
            Some(v) => unsafe { std::env::set_var("FACTR_HOME", v) },
            None => unsafe { std::env::remove_var("FACTR_HOME") },
        }
        let _ = std::fs::remove_dir_all(dir);
        let history = history.unwrap();
        let shown: Vec<&str> = history["messages"].as_array().unwrap().iter().filter_map(|m| m["content"].as_str()).collect();
        assert_eq!(shown, ["hello", "hi"]);
        assert!(conn.links.lock().await.is_empty(), "nothing was attached");
    }

    #[tokio::test]
    async fn closing_a_session_with_nothing_due_returns_at_once_and_frees_the_running_mark() {
        let conn = test_conn("dispose-nothing");
        // No learning configured in the test config: nothing to review, nothing left marked.
        conn.learn_before_dispose("s1").await;
        assert!(conn.learning_now.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn an_unopenable_store_is_reported_once() {
        let conn = test_conn("store-warn");
        let (to_ws, mut rx) = mpsc::channel::<Message>(8);
        conn.hub.add(Arc::new(Client { id: 99, to_ws, sessions: Mutex::new(Default::default()) })).await;
        let err = anyhow!("database is locked");
        store_unavailable(&conn.hub, "test store", &err).await;
        store_unavailable(&conn.hub, "test store", &err).await;
        let Message::Text(text) = rx.try_recv().unwrap() else { panic!("text frame") };
        let event: Value = serde_json::from_str(&text).unwrap();
        assert_eq!((event["params"]["type"].as_str(), event["params"]["payload"]["kind"].as_str()), (Some("status.update"), Some("error")));
        assert!(rx.try_recv().is_err(), "the second failure is silent");
    }

    #[test]
    fn an_unopenable_store_without_a_window_is_logged_once() {
        let file = std::env::temp_dir().join(format!("not-a-dir-{}", std::process::id()));
        std::fs::write(&file, b"x").unwrap();
        let home = file.to_string_lossy().to_string();
        assert!(entries_or_log(&home).is_none() && control_or_log(&home).is_none());
        assert!(!log_once("learning store (no window)", &anyhow!("again")), "already logged");
        let _ = std::fs::remove_file(file);
    }

    #[tokio::test]
    async fn a_failed_backup_is_reported_once() {
        let conn = test_conn("backup-warn");
        let (to_ws, mut rx) = mpsc::channel::<Message>(8);
        conn.hub.add(Arc::new(Client { id: 98, to_ws, sessions: Mutex::new(Default::default()) })).await;
        report_backup_error(&conn.hub, None).await;
        assert!(rx.try_recv().is_err(), "nothing to report");
        report_backup_error(&conn.hub, Some("disk full".into())).await;
        report_backup_error(&conn.hub, Some("disk full".into())).await;
        let Message::Text(text) = rx.try_recv().unwrap() else { panic!("text frame") };
        let event: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(event["params"]["payload"]["kind"], "error");
        assert!(event["params"]["payload"]["text"].as_str().unwrap().contains("disk full"));
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn a_cron_jobs_model_and_provider_reach_the_run_session() {
        let opts = RunOpts { model: Some("llama-3.3-70b"), provider: Some("groq"), ..RunOpts::default() };
        let params = run_session_params(Some("/w"), Some("nightly"), &opts);
        assert_eq!((params["model"].as_str(), params["provider"].as_str(), params["title"].as_str()), (Some("llama-3.3-70b"), Some("groq"), Some("nightly")));
        assert!(run_session_params(None, None, &RunOpts::default()).get("model").is_none());
    }

    #[test]
    fn a_headless_run_session_is_marked_so_it_gets_no_default_persona() {
        assert_eq!(run_session_params(None, None, &RunOpts::default())["headless"], true);
    }

    #[tokio::test]
    async fn a_chats_turns_run_one_at_a_time_while_other_chats_are_not_held_up() {
        let first = chat_turn("telegram:serial").await;
        let second = tokio::spawn(async { drop(chat_turn("telegram:serial").await) });
        drop(chat_turn("telegram:elsewhere").await); // another chat is free while the first is busy
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!second.is_finished(), "the second message waits for the first turn");
        drop(first);
        tokio::time::timeout(Duration::from_secs(2), second).await.expect("released").unwrap();
    }

    #[test]
    fn a_failed_turn_reply_carries_the_stop_message() {
        let refused = turn_reply(&json!({ "status": "error", "text": "", "error": "Provider refused" }), "s", "cron");
        assert!(refused["error"].as_str().unwrap().ends_with("(error): Provider refused"), "{refused}");
        let bare = turn_reply(&json!({ "status": "error", "text": "" }), "s", "cron");
        assert_eq!(bare["error"], "the turn did not complete cleanly (error)");
    }

    #[tokio::test]
    async fn a_stopped_bot_turn_replies_stopped_not_failed_and_the_queued_message_still_runs() {
        let stopped = json!({ "status": "interrupted", "text": "partial", "usage": null });
        let bot = turn_reply(&stopped, "s", "bot");
        assert_eq!((bot["ok"].clone(), bot["interrupted"].clone(), bot["error"].clone()), (json!(true), json!(true), Value::Null));
        let cron = turn_reply(&stopped, "s", "cron");
        assert_eq!((cron["ok"].clone(), cron["interrupted"].clone()), (json!(false), json!(true)));
        assert!(cron["error"].as_str().unwrap().contains("interrupted"));
        let done = turn_reply(&json!({ "status": "complete", "text": "hi" }), "s", "bot");
        assert_eq!((done["ok"].clone(), done["interrupted"].clone(), done["text"].clone()), (json!(true), json!(false), json!("hi")));
        // The next message for the chat waits for the stopped turn, then gets the lock; idle chats are pruned.
        let key = "telegram:stop-queue";
        let first = chat_turn(key).await;
        let second = tokio::spawn(async move { drop(chat_turn(key).await) });
        tokio::time::sleep(Duration::from_millis(30)).await;
        drop(first);
        prune_chat_turn(key);
        tokio::time::timeout(Duration::from_secs(2), second).await.expect("the queued turn runs").unwrap();
        prune_chat_turn(key);
        assert!(!CHAT_TURNS.lock().unwrap().contains_key(key), "idle chat pruned");
        let held = chat_turn(key).await;
        prune_chat_turn(key);
        assert!(CHAT_TURNS.lock().unwrap().contains_key(key), "a held lock is kept");
        drop(held);
        prune_chat_turn(key);
    }

    #[tokio::test]
    async fn new_in_a_bot_chat_starts_a_fresh_engine_session() {
        let home = std::env::temp_dir().join(format!("bot-reset-{}", std::process::id()));
        let home_str = home.to_string_lossy().to_string();
        save_bot_session(&home_str, "telegram:9", "session_old");
        save_bot_session(&home_str, "telegram:8", "session_other");
        assert!(!interrupt_run("telegram:9").await, "nothing running");
        reset_bot_session(&home_str, "telegram:9").await;
        assert_eq!(load_bot_session(&home_str, "telegram:9"), None, "the next message creates a new session");
        assert_eq!(load_bot_session(&home_str, "telegram:8").as_deref(), Some("session_other"));
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn exec_rpcs_are_gated_unless_they_are_the_desktops_profile_edits() {
        let cli = |argv: Value| ungated_exec_command("cli.exec", &json!({ "argv": argv }));
        assert_eq!(cli(json!(["profile", "delete", "w", "--yes"])), None);
        assert_eq!(cli(json!(["--profile", "w", "config", "unset", "model"])), None);
        assert_eq!(cli(json!(["chat", "-q", "rm -rf /"])).as_deref(), Some("factr chat -q rm -rf /"));
        assert_eq!(cli(json!([])).as_deref(), Some("factr "));
        assert_eq!(ungated_exec_command("shell.exec", &json!({ "command": "ls" })).as_deref(), Some("ls"));
    }

    #[test]
    fn a_bot_chat_keeps_its_engine_session_across_an_engine_restart() {
        let home = std::env::temp_dir().join(format!("bot-sessions-{}", std::process::id()));
        let home_str = home.to_string_lossy().to_string();
        assert_eq!(load_bot_session(&home_str, "telegram:1"), None);
        save_bot_session(&home_str, "telegram:1", "session_a");
        save_bot_session(&home_str, "telegram:2", "session_b");
        save_bot_session(&home_str, "telegram:1", "session_c"); // a chat re-mapped
        // "Restart": a fresh store on the same factr.db, not the process-cached one.
        let reopened = factr_learn::entries::EntryStore::open(&home).unwrap();
        assert_eq!(reopened.setting(&bot_session_setting("telegram:1")).as_deref(), Some("session_c"));
        assert_eq!(reopened.setting(&bot_session_setting("telegram:2")).as_deref(), Some("session_b"));
        assert_eq!(load_bot_session(&home_str, "telegram:3"), None);
        let _ = std::fs::remove_dir_all(home);
    }

    /// Closing or deleting a chat drops its link: the bridge, writer and reader end (so the engine runs its
    /// disconnect cleanup), and the per-session maps return to what they were before the chat opened.
    #[tokio::test]
    async fn a_closed_chat_leaves_no_link_or_state_behind() {
        let conn = test_conn("close-leak");
        for round in 0..5 {
            let sid = format!("chat-{round}");
            let link = conn.open_link().await.unwrap();
            conn.links.lock().await.insert(sid.clone(), link);
            conn.sessions.lock().await.entry(sid.clone()).or_default();
            conn.known.lock().await.insert(sid.clone(), json!({ "working_dir": "/" }));
            conn.forget_link(&sid).await;
        }
        tokio::time::timeout(Duration::from_secs(5), async {
            while conn.link_tasks.lock().await.iter().any(|task| !task.is_finished()) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("every closed chat's bridge, writer and reader ended");
        assert!(conn.links.lock().await.is_empty() && conn.sessions.lock().await.is_empty() && conn.known.lock().await.is_empty());
        // The next open prunes the finished handles instead of piling them up.
        let _live = conn.open_link().await.unwrap();
        assert_eq!(conn.link_tasks.lock().await.len(), 1);
    }

    /// Closing a chat mid-turn would crash the turn and let a following delete skip its "stop it
    /// first" guard: close is refused until the turn ends.
    #[tokio::test]
    async fn a_running_chat_is_not_closed_under_its_turn() {
        let conn = test_conn("close-running");
        let link = conn.open_link().await.unwrap();
        conn.links.lock().await.insert("busy".into(), link);
        conn.sessions.lock().await.entry("busy".into()).or_default().mark_running();
        let refused = conn.dispatch("session.close", &json!({ "session_id": "busy" })).await.unwrap_err();
        assert!(refused.message.contains("running"), "{}", refused.message);
        assert!(conn.links.lock().await.contains_key("busy"), "the turn keeps its link");
        let delete = conn.dispatch("session.delete", &json!({ "session_id": "busy" })).await.unwrap_err();
        assert!(delete.message.contains("stop it before deleting"));
        conn.sessions.lock().await.get_mut("busy").unwrap().end_turn();
        let closed = conn.dispatch("session.close", &json!({ "session_id": "busy" })).await.unwrap();
        assert_eq!(closed["closed"], true);
        assert!(!conn.links.lock().await.contains_key("busy"));
    }

    /// A chat with no turn yet is not on disk, so `known` keeps it for `session.list` after a close.
    #[tokio::test]
    async fn closing_an_unpersisted_chat_keeps_it_listed() {
        let conn = test_conn("close-fresh");
        conn.known.lock().await.insert("new".into(), json!({}));
        conn.fresh.lock().await.insert("new".into());
        conn.forget_link("new").await;
        assert!(conn.known.lock().await.contains_key("new"));
    }
}
