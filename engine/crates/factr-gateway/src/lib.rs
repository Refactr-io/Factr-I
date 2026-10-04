//! Factr `tui_gateway`-compatible front end for the factr engine.
//!
//! The Factr desktop app speaks JSON-RPC 2.0 over `ws://host/api/ws?token=…`
//! plus a few HTTP probes (`/api/health`, `/api/status`). This crate serves that
//! surface and translates it onto the stable factr harness API, running the
//! harness bridge in-process over an in-memory duplex per WebSocket client.
//! Contract: factr-backend `apps/shared/src/gateway-contract.openrpc.json`
//! (vendored under `contract/`).

pub mod approvals;
pub mod auth;
pub mod cron_tick;
mod cron_runs;
pub mod deployment;
pub mod features;
pub mod learn;
mod boot_stubs;
mod dangerous;
mod factr_env;
mod factr_host;
mod learning_rest;
mod memory_rest;
mod oneshot;
pub mod map;
pub mod observability;
pub mod profile;
mod rewind;
mod undo;
mod rpc;
mod sessions_rest;
mod surface_sessions;
mod skill_slash;
mod slash_forward;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Semaphore;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::protocol::{Role, WebSocketConfig};

const MAX_HEADER_BYTES: usize = 16 * 1024;
const MAX_CONNECTIONS: usize = 64;
const HEADER_TIMEOUT: Duration = Duration::from_secs(10);
/// Capacity of the in-process pipe to a bridge, per direction. Tokio keeps the grown buffer, so a
/// frame-sized pipe pinned 8 MiB per chat; a full pipe only makes the writer wait, and the bridge
/// bounds frame size itself.
pub(crate) const LINK_PIPE_BYTES: usize = 128 * 1024;
/// WebSocket message cap: room for the biggest attach RPC (a 50 MB file as base64), not just 8 MiB,
/// which closed the socket on a 6.5 MB image.
pub(crate) const MAX_WS_MESSAGE_BYTES: usize = rpc::attach::MAX_WIRE_BYTES;

pub(crate) fn ws_config() -> WebSocketConfig {
    let mut config = WebSocketConfig::default();
    config.max_message_size = Some(MAX_WS_MESSAGE_BYTES);
    config.max_frame_size = Some(MAX_WS_MESSAGE_BYTES);
    config
}

/// Method names in the vendored Factr gateway contract (parsed once).
pub(crate) fn contract_methods() -> &'static std::collections::HashSet<String> {
    static METHODS: std::sync::OnceLock<std::collections::HashSet<String>> =
        std::sync::OnceLock::new();
    METHODS.get_or_init(|| {
        let contract: Value =
            serde_json::from_str(include_str!("../contract/gateway-contract.openrpc.json"))
                .unwrap_or_default();
        contract["methods"]
            .as_array()
            .map(|list| {
                list.iter()
                    .filter_map(|m| m["name"].as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    })
}

/// Log each unsupported method/route once per process (stderr), so parity
/// gaps show up in the engine log without flooding it.
pub(crate) fn note_unsupported(kind: &str, what: &str) {
    static SEEN: std::sync::Mutex<Option<std::collections::HashSet<String>>> =
        std::sync::Mutex::new(None);
    let key = format!("{kind} {what}");
    let mut seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    if seen
        .get_or_insert_with(Default::default)
        .insert(key.clone())
    {
        eprintln!(
            "factr-gateway: unsupported {}",
            key.chars().take(200).collect::<String>()
        );
    }
}

pub struct Config {
    pub bind: SocketAddr,
    pub token: String,
    pub version: String,
    /// factr daemon socket the in-process bridge dials.
    pub legacy_socket: PathBuf,
    /// Working directory for new sessions when the client names none.
    pub default_cwd: String,
    /// Binding a non-loopback address must be asked for explicitly.
    pub allow_non_loopback: bool,
    /// Provider and model the engine started with (reported to setup screens).
    pub provider: String,
    pub model: String,
    /// Reasoning levels the served provider accepts (empty: none), so the model
    /// catalog reports what the engine can actually honour.
    pub reasoning_efforts: Vec<String>,
    /// Whether the Factr profile's default model belongs to the served
    /// provider. False when the engine was started with an explicit provider
    /// and the profile only says "auto": its model (e.g. a cloud model) is
    /// then not something this provider can serve.
    pub profile_model_applies: bool,
    /// Engine state directory, reported as the single profile's path.
    pub home: String,
    /// One-shot model call `(system, user) -> text and usage`, used by learning and `/refine`.
    pub complete: Option<Complete>,
    /// Factr's Python backend for everything the Rust harness does not own.
    pub features: Option<Arc<features::Features>>,
    /// Automatic learning (factr-learn loop); `None` when switched off.
    pub learning: Option<learn::Learning>,
}

pub type Complete = std::sync::Arc<
    dyn Fn(
            String,
            String,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<Output = Result<factr_provider_core::SimpleCompletion>>
                    + Send,
            >,
        > + Send
        + Sync,
>;

struct AlertLoop(tokio::task::JoinHandle<()>);

impl Drop for AlertLoop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// The git commit this engine was built from (`/api/status` `sha`, `factr __version`).
pub fn build_sha() -> &'static str {
    factr_build_meta::git_hash()
}

/// The `factr.db` schema this engine migrates to (`factr __version` `db_schema`; the desktop packager
/// records it in the manifest so the update script can allow a slower first start when it went up).
pub fn db_schema() -> u32 {
    factr_base::migrate::CURRENT
}

pub struct Gateway {
    listener: TcpListener,
    local: SocketAddr,
    config: Arc<Config>,
    hub: Arc<approvals::Hub>,
    observer: Arc<observability::Observer>,
    _alert_loop: AlertLoop,
}

impl Gateway {
    pub async fn bind(config: Config) -> Result<Self> {
        register_credentials();
        if !config.bind.ip().is_loopback() && !config.allow_non_loopback {
            bail!("refusing to bind {} without --allow-remote", config.bind);
        }
        if config.token.len() < 32 {
            bail!("gateway token must be at least 32 characters");
        }
        let listener = TcpListener::bind(config.bind)
            .await
            .context("binding gateway")?;
        let local = listener.local_addr()?;
        // First opener of factr.db: applies (and backs up before) any schema migration.
        let store = factr_learn::entries::EntryStore::open_cached(std::path::Path::new(&config.home))
            .context("opening factr.db")?;
        let (alert_tx, alert_rx) = std::sync::mpsc::sync_channel::<Value>(64);
        let observer = observability::Observer::open(
            std::path::Path::new(&config.home),
            &config.provider,
            &config.model,
            Some(alert_tx),
        )?;
        let span_observer = observer.clone();
        factr_base::obs_sink::install(move |span| span_observer.span(span));
        let hub = Arc::new(approvals::Hub::default());
        hub.set_observer(observer.clone());
        hub.set_store(store).await;
        let alert_hub = hub.clone();
        let alert_loop = AlertLoop(tokio::spawn(async move {
            loop {
                match alert_rx.try_recv() {
                    Ok(payload) => {
                        let frame = json!({
                            "jsonrpc": "2.0",
                            "method": "event",
                            "params": { "type": "observability.alert", "payload": payload },
                        })
                        .to_string();
                        alert_hub.broadcast_text(frame).await;
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => tokio::time::sleep(std::time::Duration::from_millis(250)).await,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
                }
            }
        }));
        Ok(Self {
            listener,
            local,
            config: Arc::new(config),
            hub,
            observer,
            _alert_loop: alert_loop,
        })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.local
    }

    pub async fn serve(self) -> Result<()> {
        rpc::start_driver(self.config.clone(), self.hub.clone(), self.observer.clone());
        factr_host::install(self.config.features.clone(), self.hub.clone());
        // The engine's one approval gate: bash, repl and factr calls are denied unless this is installed.
        factr_base::hooks::set_approval_hook(Some(Arc::new(approvals::HubGate(self.hub.clone()))));
        if let Some(features) = self.config.features.clone() {
            let idle = features.clone();
            tokio::spawn(async move {
                let mut tick = tokio::time::interval(
                    features::idle_stop_after()
                        .clamp(Duration::from_secs(1), Duration::from_secs(60)),
                );
                loop {
                    tick.tick().await;
                    if let Some(home) = factr_base::factr_config::home() {
                        idle.ensure_bots(&home).await;
                    }
                    idle.stop_if_idle().await;
                }
            });
            tokio::spawn(cron_tick::run(features));
        }
        let permits = Arc::new(Semaphore::new(MAX_CONNECTIONS));
        loop {
            let (stream, _) = self.listener.accept().await?;
            let Ok(permit) = permits.clone().try_acquire_owned() else {
                drop(stream); // over the connection cap: refuse without reading
                continue;
            };
            let config = self.config.clone();
            let hub = self.hub.clone();
            let observer = self.observer.clone();
            let local = self.local;
            tokio::spawn(async move {
                let _permit = permit;
                let _ = handle(stream, local, config, hub, observer).await;
            });
        }
    }
}

struct Request {
    method: String,
    path: String,
    query: Option<String>,
    headers: Vec<(String, String)>,
    /// Body bytes read along with the headers.
    body_prefix: Vec<u8>,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

async fn read_request(stream: &mut TcpStream) -> Result<Request> {
    let mut buf = Vec::with_capacity(2048);
    let mut chunk = [0u8; 2048];
    loop {
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            bail!("connection closed before headers");
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
        if buf.len() > MAX_HEADER_BYTES {
            bail!("headers too large");
        }
    }
    let mut headers = [httparse::EMPTY_HEADER; 64];
    let mut req = httparse::Request::new(&mut headers);
    let header_len = match req.parse(&buf)? {
        httparse::Status::Complete(n) => n,
        httparse::Status::Partial => bail!("incomplete request"),
    };
    let target = req.path.unwrap_or("/");
    let (path, query) = match target.split_once('?') {
        Some((p, q)) => (p.to_string(), Some(q.to_string())),
        None => (target.to_string(), None),
    };
    Ok(Request {
        method: req.method.unwrap_or("GET").to_string(),
        path,
        query,
        headers: req
            .headers
            .iter()
            .map(|h| {
                (
                    h.name.to_string(),
                    String::from_utf8_lossy(h.value).into_owned(),
                )
            })
            .collect(),
        body_prefix: buf[header_len..].to_vec(),
    })
}

const MAX_BODY_BYTES: usize = 64 * 1024;
/// `/api/agent/run` carries whole prompts (cron context, bot history).
const MAX_RUN_BODY_BYTES: usize = 4 * 1024 * 1024;

fn declared_len(req: &Request) -> usize {
    req.header("content-length")
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0)
}

async fn read_body(stream: &mut TcpStream, req: &Request) -> Result<Vec<u8>> {
    read_body_max(stream, req, MAX_BODY_BYTES).await
}

async fn read_body_max(stream: &mut TcpStream, req: &Request, max: usize) -> Result<Vec<u8>> {
    let len = declared_len(req);
    if len > max {
        bail!("body too large");
    }
    let mut body = req.body_prefix.clone();
    body.truncate(len);
    while body.len() < len {
        let mut chunk = vec![0u8; len - body.len()];
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            bail!("connection closed mid-body");
        }
        body.extend_from_slice(&chunk[..n]);
    }
    Ok(body)
}

async fn respond(stream: &mut TcpStream, status: &str, body: &Value) -> Result<()> {
    let body = body.to_string();
    let head = format!(
        "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\ncache-control: no-store\r\nx-content-type-options: nosniff\r\nconnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(body.as_bytes()).await?;
    stream.shutdown().await?;
    Ok(())
}

/// Reverse-proxy one request (or WebSocket upgrade) to the Factr feature
/// backend. The client's credential is replaced by the backend's private
/// token; bytes then flow both ways untouched.
async fn proxy_http(
    mut client: TcpStream,
    req: &Request,
    features: &Arc<features::Features>,
    upgrade: bool,
) -> Result<()> {
    let _lease = holds_lease(&req.path, upgrade).then(|| features.lease());
    if std::env::var_os("FACTR_TRACE_FORWARD").is_some() {
        eprintln!("factr: forward HTTP {} {}", req.method, req.path);
    }
    let port = features.port().await?;
    let mut upstream = TcpStream::connect(("127.0.0.1", port)).await?;
    let query: Vec<&str> = req
        .query
        .as_deref()
        .unwrap_or_default()
        .split('&')
        .filter(|kv| !kv.is_empty() && !kv.starts_with("token=") && !kv.starts_with("ticket="))
        .collect();
    let target = if query.is_empty() {
        req.path.clone()
    } else {
        format!("{}?{}", req.path, query.join("&"))
    };
    let mut head = format!("{} {} HTTP/1.1\r\n", req.method, target);
    for (name, value) in &req.headers {
        let lower = name.to_ascii_lowercase();
        let dropped = matches!(
            lower.as_str(),
            "host" | "x-factr-session-token" | "authorization" | "origin"
        ) || (!upgrade && lower == "connection");
        if !dropped {
            head.push_str(&format!("{name}: {value}\r\n"));
        }
    }
    head.push_str(&format!(
        "Host: 127.0.0.1:{port}\r\nX-Factr-Session-Token: {}\r\n",
        features.token
    ));
    if upgrade {
        // WebSocket routes authenticate by query token on the backend.
        let sep = if target.contains('?') { '&' } else { '?' };
        head = head.replacen(
            &target,
            &format!("{target}{sep}token={}", features.token),
            1,
        );
    } else {
        head.push_str("Connection: close\r\n");
    }
    head.push_str("\r\n");
    upstream.write_all(head.as_bytes()).await?;
    upstream.write_all(&req.body_prefix).await?;
    if upgrade {
        let (client_read, client_write) = client.split();
        let (upstream_read, upstream_write) = upstream.split();
        // Either side closing ends the socket.
        tokio::select! {
            _ = pump(client_read, upstream_write, features) => {}
            _ = pump(upstream_read, client_write, features) => {}
        }
    } else {
        let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
    }
    features.touch(&req.path, "forwarded-http");
    Ok(())
}

/// Whether a proxied request keeps the backend from idle-stopping while it is open. A request holds
/// a lease for its whole life (idle-stop must not kill a backend mid-response). So does a WebSocket
/// whose backend end owns a process (a terminal, a console, a display): stopping the backend would
/// kill it under an open window. An event-stream socket holds none (it can stay open for days): its
/// frames mark the backend active instead.
fn holds_lease(path: &str, upgrade: bool) -> bool {
    !upgrade || matches!(path, "/api/pty" | "/api/console" | "/api/display/ws")
}

/// Copy `from` to `to`, marking the backend active for every chunk.
async fn pump(mut from: impl AsyncReadExt + Unpin, mut to: impl AsyncWriteExt + Unpin, features: &features::Features) {
    let mut buf = [0_u8; 8192];
    while let Ok(n @ 1..) = from.read(&mut buf).await {
        features.mark_active();
        if to.write_all(&buf[..n]).await.is_err() {
            break;
        }
    }
    let _ = to.shutdown().await;
}

fn query_u64(req: &Request, key: &str) -> Option<u64> {
    auth::query_param(req.query.as_deref()?, key)?.parse().ok()
}

/// The one answer for every Factr curator route and RPC: the engine does not run it.
pub(crate) const CURATOR_UNAVAILABLE: &str = "curator is not available in this engine";

fn bundled_startup_request(req: &Request) -> bool {
    std::env::var_os("FACTR_BACKEND_PYTHON").is_some()
        && auth::query_param(req.query.as_deref().unwrap_or_default(), "feature").as_deref()
            != Some("1")
}

/// A request only a user action produces, made while the desktop's chat is open and so without
/// `feature=1`: the Capabilities overlay (Tools tab: toolsets, computer use; hub installs, skill
/// toggles, MCP edits). Unlike a boot probe, it is the user asking for the feature, so Python may
/// start for it, on demand. Reads of the boot-probe routes keep their native answers above.
fn capabilities_request(req: &Request) -> bool {
    let read = matches!(req.method.as_str(), "GET" | "HEAD");
    let path = req.path.as_str();
    let under = |prefix: &str| path == prefix || path.strip_prefix(prefix).is_some_and(|r| r.starts_with('/'));
    // Sign-in (start / poll / submit / cancel / disconnect) is a user action. Only the bare list
    // GET `/api/providers/oauth` stays a boot probe (the onboarding falls back to API-key setup).
    let oauth_action = path.strip_prefix("/api/providers/oauth/").is_some_and(|r| !r.is_empty());
    if read {
        return under("/api/tools/toolsets") || under("/api/tools/computer-use") || oauth_action;
    }
    under("/api/tools") || under("/api/skills") || under("/api/mcp") || under("/api/providers/oauth")
}

fn bundled_defaults() -> Value {
    std::env::var("FACTR_BACKEND_DEFAULTS")
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| json!({}))
}

/// One request/reply against the engine over a short-lived in-process bridge,
/// for HTTP routes that have no WebSocket connection to ride on.
async fn harness_request(legacy_socket: &std::path::Path, request: Value) -> Result<Value> {
    harness_requests(legacy_socket, &[request]).await
}

/// Send requests in order on one bridge connection (e.g. attach, then act);
/// returns the reply to the last one.
async fn harness_requests(legacy_socket: &std::path::Path, requests: &[Value]) -> Result<Value> {
    harness_requests_with(legacy_socket, requests, None).await
}

/// [`harness_requests`], but the last request is fire-and-linger: its reply is waited for at most
/// `last_wait` (it may never come) and the connection then closes. For requests whose work carries
/// on inside the engine after it has been sent.
async fn harness_requests_with(legacy_socket: &std::path::Path, requests: &[Value], last_wait: Option<Duration>) -> Result<Value> {
    let (ours, theirs) = tokio::io::duplex(LINK_PIPE_BYTES);
    let (their_read, their_write) = tokio::io::split(theirs);
    let bridge = tokio::spawn(factr_harness_api_server::run_bridge_stream(
        their_read,
        their_write,
        legacy_socket.to_path_buf(),
    ));
    let (our_read, mut our_write) = tokio::io::split(ours);
    let mut lines = BufReader::new(our_read).lines();
    let result = async {
        let hello = json!({"v": 1, "id": 0, "req": "hello", "min_version": 1, "max_version": 1, "client": "factr-gateway-http"});
        our_write.write_all(format!("{hello}\n").as_bytes()).await?;
        lines.next_line().await?.context("engine closed")?;
        let mut last = Value::Null;
        for (i, request) in requests.iter().enumerate() {
            let id = i as u64 + 1;
            let mut frame = request.clone();
            frame["v"] = json!(1);
            frame["id"] = json!(id);
            our_write.write_all(format!("{frame}\n").as_bytes()).await?;
            let reply = async {
                loop {
                    let line = lines.next_line().await?.context("engine closed")?;
                    let reply: Value = serde_json::from_str(&line)?;
                    if reply["reply_to"] == id {
                        if reply["ev"] == "error" {
                            bail!("{}", reply["message"].as_str().unwrap_or("engine error"));
                        }
                        return Ok::<Value, anyhow::Error>(reply);
                    }
                }
            };
            last = match last_wait.filter(|_| i + 1 == requests.len()) {
                Some(wait) => tokio::time::timeout(wait, reply).await.unwrap_or(Ok(Value::Null))?,
                None => reply.await?,
            };
        }
        Ok(last)
    };
    let result = tokio::time::timeout(Duration::from_secs(20), result)
        .await
        .context("engine timed out")?;
    bridge.abort();
    result
}

/// Make the Factr credential store readable (its `auth.json` logins, its `.env` keys) and take the
/// baseline `reload_if_credentials_moved` compares against. The binary calls this BEFORE it builds
/// the model provider: bound later (in `Gateway::bind`), the startup provider init could not see a
/// Factr login, printed "model provider not ready", and the readiness probes answered "not
/// configured" from that stale auth snapshot, so the onboarding overlay covered a valid login.
/// Why the engine started without a usable login for its provider (`None`: it has one). Shown on
/// `/api/status` as `missing_login` so a runner can fail fast instead of discovering it per task.
static MISSING_LOGIN: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// Record (and print once to stderr) that `provider` found no readable login at boot.
pub fn note_missing_login(provider: &str, reason: &str) {
    let message = format!("no readable {provider} login ({reason}); run `factr auth add openai-codex` or add an API key");
    let mut slot = MISSING_LOGIN.lock().unwrap_or_else(|e| e.into_inner());
    if slot.as_deref() != Some(message.as_str()) {
        eprintln!("factr: {message}; the {provider} provider starts without it (browser-backed models only)");
        *slot = Some(message);
    }
}

/// A login was found after all (a later reload): the status no longer reports it missing.
pub fn clear_missing_login() {
    *MISSING_LOGIN.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

pub fn missing_login() -> Option<String> {
    MISSING_LOGIN.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

pub fn register_credentials() {
    factr_env::register();
    if OAUTH_SEEN.lock().unwrap_or_else(|e| e.into_inner()).is_none()
        && let Some(home) = factr_base::factr_config::home()
    {
        oauth_credentials_moved(&OAUTH_SEEN, &home);
    }
}

/// A login written to the Factr store while the engine runs (`factr login`, another window) is
/// picked up on the next readiness probe: the auth snapshot is dropped and the provider told, so the
/// first use finds it ready without a restart.
pub(crate) async fn reload_if_credentials_moved(config: &Arc<Config>) {
    let Some(home) = factr_base::factr_config::home() else { return };
    if oauth_credentials_moved(&OAUTH_SEEN, &home) {
        factr_base::auth::AuthStatus::invalidate_cache();
        let config = config.clone();
        tokio::spawn(async move { notify_auth_changed(&config, None).await });
    }
}

/// Whether a forwarded Factr RPC changes a provider credential (it wrote `$FACTR_CONFIG_HOME/.env`).
pub(crate) fn changes_credentials(method: &str) -> bool {
    matches!(method, "model.save_key" | "model.disconnect" | "reload.env")
}

type CredentialStamp = Vec<Option<(std::time::SystemTime, u64)>>;
static OAUTH_SEEN: std::sync::Mutex<Option<CredentialStamp>> = std::sync::Mutex::new(None);

/// Whether Factr's OAuth credential files (`auth.json` and the Anthropic PKCE file, for the home and
/// each profile) differ from the last time this was asked: a device-code login lands in them on a
/// poll or submit, a logout removes from them. The first ask only records the baseline.
fn oauth_credentials_moved(seen: &std::sync::Mutex<Option<CredentialStamp>>, factr_home: &Path) -> bool {
    let mut roots = vec![factr_home.to_path_buf()];
    if let Ok(entries) = std::fs::read_dir(factr_home.join("profiles")) {
        roots.extend(entries.flatten().map(|e| e.path()));
    }
    let now: CredentialStamp = roots
        .iter()
        .flat_map(|r| [r.join("auth.json"), r.join(".anthropic_oauth.json")])
        .map(|f| std::fs::metadata(f).ok().map(|m| (m.modified().unwrap_or(std::time::UNIX_EPOCH), m.len())))
        .collect();
    let mut last = seen.lock().unwrap_or_else(|e| e.into_inner());
    let moved = last.as_ref().is_some_and(|l| *l != now);
    *last = Some(now);
    moved
}

/// Tell the engine a credential changed, so its providers re-resolve their keys: one that had no
/// key starts, one whose key was removed stops using it. (A rotated key already applies per request.)
/// The engine only takes this from a subscribed connection, so it rides on a throwaway empty session,
/// which is never saved. Failures are logged; the change then applies at the next restart.
pub(crate) async fn notify_auth_changed(config: &Config, provider: Option<&str>) {
    let provider = provider
        .filter(|p| !p.is_empty() && p.len() <= 64 && p.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'))
        .unwrap_or("factr");
    let requests = [
        json!({"req": "create_session", "working_dir": config.default_cwd}),
        json!({"req": "notify_auth_changed", "provider": provider}),
    ];
    if let Err(err) = harness_requests_with(&config.legacy_socket, &requests, Some(Duration::from_secs(3))).await {
        eprintln!("factr: could not tell the engine a key changed (it applies after a restart): {err:#}");
    }
}

async fn session_infos(config: &Config, limit: u64, include_archived: bool) -> Result<Vec<Value>> {
    let reply = harness_request(
        &config.legacy_socket,
        json!({"req": "list_sessions", "limit": limit, "include_archived": include_archived}),
    )
    .await?;
    Ok(reply["sessions"]
        .as_array()
        .map(|list| {
            list.iter()
                .filter(|s| include_archived || s["archived"] != true)
                .map(map::session_info)
                .collect()
        })
        .unwrap_or_default())
}

async fn handle(
    mut stream: TcpStream,
    local: SocketAddr,
    config: Arc<Config>,
    hub: Arc<approvals::Hub>,
    observer: Arc<observability::Observer>,
) -> Result<()> {
    let req = tokio::time::timeout(HEADER_TIMEOUT, read_request(&mut stream)).await??;
    let bound_ip = local.ip().to_string();
    let host_reason = auth::host_origin_reason(
        req.header("host"),
        req.header("origin"),
        local.port(),
        &bound_ip,
    );
    let token_ok = auth::token_matches(
        &config.token,
        auth::presented_token(
            req.query.as_deref(),
            req.header("authorization"),
            req.header("x-factr-session-token"),
        )
        .as_deref(),
    );
    let wants_ws = req
        .header("upgrade")
        .is_some_and(|v| v.eq_ignore_ascii_case("websocket"));

    if wants_ws && req.path != "/api/ws" {
        if host_reason.is_some() || !token_ok {
            return respond(
                &mut stream,
                "401 Unauthorized",
                &json!({"detail": "unauthorized"}),
            )
            .await;
        }
        // Packaged: only wake Python for explicit feature=1 (Cron UI, etc.).
        if config.features.is_some() && bundled_startup_request(&req) {
            note_unsupported("ws", &req.path);
            return respond(
                &mut stream,
                "404 Not Found",
                &json!({"detail": "not supported by engine", "reason": "feature_not_requested"}),
            )
            .await;
        }
        return match config.features.clone() {
            Some(features) => proxy_http(stream, &req, &features, true).await,
            None => {
                respond(
                    &mut stream,
                    "404 Not Found",
                    &json!({"detail": "not supported by engine"}),
                )
                .await
            }
        };
    }
    if wants_ws && req.path == "/api/ws" {
        let Some(key) = req.header("sec-websocket-key") else {
            return respond(
                &mut stream,
                "400 Bad Request",
                &json!({"detail": "missing websocket key"}),
            )
            .await;
        };
        let accept = tokio_tungstenite::tungstenite::handshake::derive_accept_key(key.as_bytes());
        let head = format!(
            "HTTP/1.1 101 Switching Protocols\r\nupgrade: websocket\r\nconnection: Upgrade\r\nsec-websocket-accept: {accept}\r\n\r\n"
        );
        stream.write_all(head.as_bytes()).await?;
        let ws = WebSocketStream::from_raw_socket(stream, Role::Server, Some(ws_config())).await;
        // Close after accepting, with Factr's codes, so the desktop reports
        // an auth failure rather than a network error.
        if let Some(reason) = host_reason {
            return rpc::close(ws, 4403, &reason).await;
        }
        if !token_ok {
            return rpc::close(ws, 4401, "unauthorized").await;
        }
        return rpc::run(ws, config, hub, observer).await;
    }

    if host_reason.is_some() {
        return respond(
            &mut stream,
            "403 Forbidden",
            &json!({"detail": "host not allowed"}),
        )
        .await;
    }
    let public = json!({"ok": true, "version": config.version, "auth_required": false});
    // Chats live only in the engine's store; never proxy these to Python.
    // Session data (transcripts, search, delete, rename) is sensitive, so this
    // early-dispatch path must carry its own token check: it runs before the
    // `!token_ok` catch-all below, which only guards the arms of the match
    // that follows.
    if (req.path == "/api/sessions" || req.path.starts_with("/api/sessions/"))
        && req.path != "/api/sessions/owner-backfill"
    {
        if !token_ok {
            return respond(
                &mut stream,
                "401 Unauthorized",
                &json!({"detail": "unauthorized"}),
            )
            .await;
        }
        if let Some(result) = sessions_rest::route(&mut stream, &req, &config, &observer).await {
            return result;
        }
    }
    // A job's run history is the engine's own record of its runs (the hidden sessions it linked); the
    // Python backend only knows runs booked into state.db, so never proxy this one.
    if req.path.starts_with("/api/cron/jobs/") && req.path.ends_with("/runs") {
        if !token_ok {
            return respond(&mut stream, "401 Unauthorized", &json!({"detail": "unauthorized"})).await;
        }
        if let Some(result) = cron_runs::route(&mut stream, &req, &config).await {
            return result;
        }
    }
    if req.path.starts_with("/api/learning") {
        if !token_ok {
            return respond(
                &mut stream,
                "401 Unauthorized",
                &json!({"detail": "unauthorized"}),
            )
            .await;
        }
        if let Some(result) = learning_rest::route(&mut stream, &req, &config).await {
            return result;
        }
    }
    if req.path == "/api/memory" || req.path.starts_with("/api/memory/") {
        if !token_ok {
            return respond(
                &mut stream,
                "401 Unauthorized",
                &json!({"detail": "unauthorized"}),
            )
            .await;
        }
        if let Some(result) = memory_rest::route(&mut stream, &req).await {
            return result;
        }
    }
    if req.path == "/api/curator" || req.path.starts_with("/api/curator/") {
        if !token_ok {
            return respond(&mut stream, "401 Unauthorized", &json!({"detail": "unauthorized"})).await;
        }
        // factr-learn refine already dedupes and updates skills; the engine has no curator telemetry.
        return respond(&mut stream, "404 Not Found", &json!({"detail": CURATOR_UNAVAILABLE, "reason": "not_available"})).await;
    }
    match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/api/health") => respond(&mut stream, "200 OK", &public).await,
        ("GET", "/api/status") => {
            let body = json!({
                "version": config.version,
                // The build this engine was compiled from; the desktop checks it against its bundle manifest.
                "sha": build_sha(),
                "engine": "factr",
                "gateway_running": true,
                "gateway_state": "running",
                "auth_required": false,
                // "private": no logins; keys come from the environment only (the UI hides login).
                "deployment": if deployment::is_private() { "private" } else { "public" },
                // Why the last daily database backup failed (null: fine or not yet run).
                "backup_error": factr_base::migrate::backup_error(),
                // Why the provider has no login (null: it has one): a runner fails fast on this.
                "missing_login": missing_login(),
            });
            respond(&mut stream, "200 OK", &body).await
        }
        _ if !token_ok => {
            respond(
                &mut stream,
                "401 Unauthorized",
                &json!({"detail": "unauthorized"}),
            )
            .await
        }
        ("POST", "/api/agent/reset" | "/api/agent/interrupt") => {
            let body: Value = serde_json::from_slice(&read_body(&mut stream, &req).await?).unwrap_or(Value::Null);
            let Some(key) = body["session_key"].as_str().filter(|k| !k.is_empty()) else {
                return respond(&mut stream, "400 Bad Request", &json!({"detail": "session_key is required"})).await;
            };
            if req.path.ends_with("/reset") {
                rpc::reset_bot_session(&config.home, key).await;
                respond(&mut stream, "200 OK", &json!({"ok": true})).await
            } else {
                respond(&mut stream, "200 OK", &json!({"interrupted": rpc::interrupt_run(key).await})).await
            }
        }
        ("POST", "/api/agent/run") => {
            if declared_len(&req) > MAX_RUN_BODY_BYTES {
                return respond(
                    &mut stream,
                    "413 Payload Too Large",
                    &json!({"detail": format!("body exceeds {MAX_RUN_BODY_BYTES} bytes")}),
                )
                .await;
            }
            let body = tokio::time::timeout(HEADER_TIMEOUT, read_body_max(&mut stream, &req, MAX_RUN_BODY_BYTES)).await??;
            let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
            let Some(prompt) = body["prompt"].as_str().filter(|p| !p.trim().is_empty()) else {
                return respond(
                    &mut stream,
                    "400 Bad Request",
                    &json!({"detail": "prompt is required"}),
                )
                .await;
            };
            let timeout_s = body["timeout_s"].as_u64().unwrap_or(600).clamp(1, 3600);
            let surface = if body["surface"] == "bot" { "bot" } else { "cron" };
            let result = rpc::agent_run(
                config.clone(),
                hub.clone(),
                observer.clone(),
                surface,
                prompt,
                body["cwd"].as_str(),
                body["title"].as_str(),
                body["session_key"].as_str().filter(|k| !k.is_empty()),
                rpc::RunOpts {
                    surface,
                    instructions: body["instructions"].as_str().filter(|i| !i.trim().is_empty()),
                    model: body["model"].as_str().filter(|m| !m.trim().is_empty()),
                    provider: body["provider"].as_str().filter(|p| !p.trim().is_empty()),
                    enabled_toolsets: body["enabled_toolsets"].as_array().map(|l| l.iter().filter_map(|t| t.as_str().map(str::to_owned)).collect()),
                    disabled_toolsets: body["disabled_toolsets"].as_array().map(|l| l.iter().filter_map(|t| t.as_str().map(str::to_owned)).collect()).unwrap_or_default(),
                    run_id: body["run_id"].as_str().filter(|r| !r.is_empty()),
                },
                Duration::from_secs(timeout_s),
            )
            .await;
            match result {
                Ok(body) => respond(&mut stream, "200 OK", &body).await,
                Err(err) => {
                    respond(
                        &mut stream,
                        "503 Service Unavailable",
                        &json!({"ok": false, "error": err.to_string()}),
                    )
                    .await
                }
            }
        }
        ("GET", "/api/model/info") => {
            // The default new sessions start on: the user's last saved pick, else the startup model.
            let (model, provider) = profile::effective_default(&config);
            let mut body = json!({"model": model, "provider": provider, "capabilities": {}});
            // The user's last effort pick (`agent.reasoning_effort`): what a new chat starts on.
            if let Some(effort) = profile::current().reasoning_effort {
                body["reasoning_effort"] = json!(effort);
            }
            respond(&mut stream, "200 OK", &body).await
        }
        // The same payload as the `model.options` RPC; the onboarding and model picker read it over HTTP.
        ("GET", "/api/model/options") => {
            let include_unconfigured = auth::query_param(req.query.as_deref().unwrap_or_default(), "include_unconfigured").is_some_and(|v| v == "1" || v == "true");
            respond(&mut stream, "200 OK", &rpc::model_options(&config, Vec::new(), include_unconfigured)).await
        }
        // The saved config: defaults beneath it, the profile's config.yaml on top (what a PUT wrote).
        // Always native, like the PUT: with Python up, its view of the file would differ from what was just saved.
        ("GET", "/api/config") => {
            let include_defaults = auth::query_param(req.query.as_deref().unwrap_or_default(), "include_defaults").as_deref() != Some("false");
            let home = factr_base::factr_config::home().unwrap_or_else(|| std::path::PathBuf::from(&config.home));
            respond(&mut stream, "200 OK", &rpc::config_record(&home, &bundled_defaults(), include_defaults)).await
        }
        ("GET", "/api/config/defaults") if bundled_startup_request(&req) => {
            respond(&mut stream, "200 OK", &bundled_defaults()).await
        }
        // Factr's body is `{config: {...}}` and its answer `{ok: true}`; a deep merge, so a sparse patch is fine.
        ("PUT", "/api/config") => {
            let body: Value = serde_json::from_slice(&read_body(&mut stream, &req).await?).unwrap_or(Value::Null);
            let home = factr_base::factr_config::home().unwrap_or_else(|| std::path::PathBuf::from(&config.home));
            match rpc::put_config(&home, &body["config"]) {
                Ok(()) => respond(&mut stream, "200 OK", &json!({"ok": true})).await,
                Err((true, detail)) => respond(&mut stream, "400 Bad Request", &json!({"detail": detail})).await,
                Err((false, detail)) => respond(&mut stream, "500 Internal Server Error", &json!({"detail": detail})).await,
            }
        }
        ("GET", "/api/local-models/status") if bundled_startup_request(&req) => {
            respond(
                &mut stream,
                "200 OK",
                &json!({
                    "enabled": false, "runtime_installed": false, "server_running": false,
                    "update_available": false, "loaded_models": {}, "loading": {}, "models": []
                }),
            )
            .await
        }
        ("GET", "/api/local-models/jobs") if bundled_startup_request(&req) => {
            respond(&mut stream, "200 OK", &json!({"jobs": []})).await
        }
        ("GET", "/api/tools/terminal/backends") if bundled_startup_request(&req) => {
            respond(
                &mut stream,
                "200 OK",
                &json!({"active": "local", "backends": [{
                    "name": "local", "label": "Local", "description": "Run on this computer",
                    "active": true, "status": "ready", "detail": null
                }]}),
            )
            .await
        }
        ("POST", "/api/sessions/owner-backfill") if bundled_startup_request(&req) => {
            respond(
                &mut stream,
                "200 OK",
                &json!({"ok": true, "stamped": 0, "profile": "default"}),
            )
            .await
        }
        ("GET", "/api/profiles") => {
            let skill_count = factr_base::skill::SkillRegistry::shared_snapshot()
                .list()
                .len();
            let selected = profile::current();
            let name = selected.name.as_deref().unwrap_or("default");
            let path = selected
                .home
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|| config.home.clone());
            let body = json!({"profiles": [{
                "name": name, "display_name": name, "path": path, "is_default": name == "default",
                "has_env": false, "model": config.model, "provider": config.provider, "skill_count": skill_count,
            }]});
            respond(&mut stream, "200 OK", &body).await
        }
        ("GET", "/api/profiles/active") => {
            let selected = profile::current();
            let name = selected.name.as_deref().unwrap_or("default");
            respond(
                &mut stream,
                "200 OK",
                &json!({"active": name, "default": name}),
            )
            .await
        }
        ("GET", "/api/profiles/sessions") => {
            let limit = query_u64(&req, "limit").unwrap_or(50).clamp(1, 1000);
            let archived = auth::query_param(req.query.as_deref().unwrap_or_default(), "archived")
                .as_deref()
                == Some("true");
            match session_infos(&config, limit, archived).await {
                Ok(sessions) => {
                    let total = sessions.len();
                    let body =
                        json!({"sessions": sessions, "total": total, "limit": limit, "offset": 0});
                    respond(&mut stream, "200 OK", &body).await
                }
                Err(err) => {
                    respond(
                        &mut stream,
                        "503 Service Unavailable",
                        &json!({"detail": err.to_string()}),
                    )
                    .await
                }
            }
        }
        ("GET", "/api/profiles/sessions/sidebar") => {
            let limit = |name: &str| query_u64(&req, name).unwrap_or(50).clamp(1, 1000) as usize;
            let limits = (limit("recents_limit"), limit("cron_limit"), limit("messaging_limit"));
            let reply = harness_request(
                &config.legacy_socket,
                json!({"req": "list_sessions", "limit": u64::MAX, "include_archived": true}),
            )
            .await;
            match reply {
                Ok(reply) => {
                    let sessions = reply["sessions"].as_array().cloned().unwrap_or_default();
                    let tags = surface_sessions::tags(&config.home);
                    let body = surface_sessions::sidebar(&sessions, &tags, limits, map::session_info);
                    respond(&mut stream, "200 OK", &body).await
                }
                Err(err) => {
                    respond(
                        &mut stream,
                        "503 Service Unavailable",
                        &json!({"detail": err.to_string()}),
                    )
                    .await
                }
            }
        }
        ("GET", "/api/factr/update/check") => {
            let body = json!({
                "install_method": "factr-engine", "current_version": config.version, "behind": 0,
                "update_available": false, "can_apply": false, "update_command": null,
                "message": "Update checks are handled by the desktop app.",
            });
            respond(&mut stream, "200 OK", &body).await
        }
        ("GET", "/api/fs/default-cwd") => {
            respond(
                &mut stream,
                "200 OK",
                &json!({"cwd": config.default_cwd, "branch": null}),
            )
            .await
        }
        ("GET", "/api/cron/jobs") if bundled_startup_request(&req) => {
            respond(&mut stream, "200 OK", &json!([])).await
        }
        ("GET", "/api/cron/delivery-targets" | "/api/cron/blueprints")
            if bundled_startup_request(&req) =>
        {
            respond(&mut stream, "200 OK", &json!([])).await
        }
        // The engine lists and views skills from the factr skill registry
        // (~/.factr/engine/skills + project overlays). On/off is `skills.disabled` in
        // config.yaml, owned by Factr: the toggle PUT is forwarded to it and
        // this list (and the model's skill list) reads the same key. Hub browse/install is forwarded to Factr Python, which installs into
        // the same skills dir (the engine passes it FACTR_HOME); the registry
        // here only reads and toggles what is installed.
        ("GET", "/api/skills") => {
            let registry = factr_base::skill::SkillRegistry::shared_snapshot();
            let disabled = factr_base::skill::disabled_skill_names();
            let origin = SkillOrigins::load();
            let skills: Vec<Value> = registry
                .list()
                .iter()
                .map(|skill| {
                    json!({
                        "name": skill.name,
                        "description": skill.description,
                        "category": "general",
                        "enabled": !disabled.contains(&skill.name),
                        "provenance": origin.of(&skill.name, &skill.path),
                    })
                })
                .collect();
            respond(&mut stream, "200 OK", &json!(skills)).await
        }
        ("GET", "/api/skills/content") => {
            let Some(name) = auth::query_param(req.query.as_deref().unwrap_or_default(), "name")
            else {
                return respond(
                    &mut stream,
                    "400 Bad Request",
                    &json!({"detail": "name is required"}),
                )
                .await;
            };
            let registry = factr_base::skill::SkillRegistry::shared_snapshot();
            match registry.get(&name) {
                Some(skill) => {
                    let content = std::fs::read_to_string(&skill.path)
                        .unwrap_or_else(|_| skill.content.clone());
                    let body = json!({
                        "content": content,
                        "name": skill.name,
                        "path": skill.path.to_string_lossy(),
                    });
                    respond(&mut stream, "200 OK", &body).await
                }
                None => {
                    respond(
                        &mut stream,
                        "404 Not Found",
                        &json!({"detail": "skill not found"}),
                    )
                    .await
                }
            }
        }
        ("GET", "/api/skills/hub/official" | "/api/skills/hub/sources") => {
            respond(&mut stream, "200 OK", &json!([])).await
        }
        // Boot probes never wake Python: list what the engine's MCP client loads
        // (FACTR_CONFIG_HOME/config.yaml); the approved-server catalog needs Python.
        ("GET", "/api/mcp/servers") if bundled_startup_request(&req) => {
            let raw = profile::current()
                .home
                .and_then(|h| std::fs::read_to_string(h.join("config.yaml")).ok())
                .unwrap_or_default();
            respond(&mut stream, "200 OK", &profile::mcp_servers_body(&raw)).await
        }
        ("GET", "/api/mcp/catalog") if bundled_startup_request(&req) => {
            respond(&mut stream, "200 OK", &json!({"entries": [], "diagnostics": []})).await
        }
        ("GET", "/api/plugins") if bundled_startup_request(&req) => {
            respond(&mut stream, "200 OK", &json!({"plugins": []})).await
        }
        ("GET", "/api/host/identity") => {
            let body = json!({"ok": true, "protocolVersion": 1, "pid": std::process::id(), "role": "serve"});
            respond(&mut stream, "200 OK", &body).await
        }
        ("GET", "/api/analytics/usage") => {
            let days = query_u64(&req, "days").unwrap_or(30).clamp(1, 3650);
            match tokio::task::spawn_blocking(move || observer.analytics(days)).await? {
                Ok(body) => respond(&mut stream, "200 OK", &body).await,
                Err(err) => {
                    respond(
                        &mut stream,
                        "503 Service Unavailable",
                        &json!({"detail": err.to_string()}),
                    )
                    .await
                }
            }
        }
        ("GET", "/api/factr/observability/runs") => {
            let limit = query_u64(&req, "limit").unwrap_or(50).clamp(1, 200);
            let query = req.query.clone();
            let result = tokio::task::spawn_blocking(move || {
                let q = query.as_deref().unwrap_or_default();
                let status = auth::query_param(q, "status");
                let kind = auth::query_param(q, "kind");
                let text = auth::query_param(q, "q");
                let outcome = auth::query_param(q, "outcome");
                let session = auth::query_param(q, "session");
                observer.list(limit, status.as_deref(), kind.as_deref(), text.as_deref(), outcome.as_deref(), session.as_deref())
            })
            .await?;
            match result {
                Ok(body) => respond(&mut stream, "200 OK", &body).await,
                Err(err) => {
                    respond(
                        &mut stream,
                        "503 Service Unavailable",
                        &json!({"detail":err.to_string()}),
                    )
                    .await
                }
            }
        }
        ("GET", "/api/factr/observability/monitors") => {
            let window = auth::query_param(req.query.as_deref().unwrap_or_default(), "window")
                .unwrap_or_else(|| "24h".into());
            let result = tokio::task::spawn_blocking(move || observer.monitors(&window)).await?;
            match result {
                Ok(body) => respond(&mut stream, "200 OK", &body).await,
                Err(msg) => respond(&mut stream, "400 Bad Request", &json!({"detail": msg})).await,
            }
        }
        ("GET", path @ ("/api/factr/observability/sessions" | "/api/factr/observability/facts" | "/api/factr/observability/alerts" | "/api/factr/observability/memory-audit")) => {
            let view = path.rsplit('/').next().unwrap_or_default().to_string();
            let limit = query_u64(&req, "limit").unwrap_or(50).clamp(1, 200);
            let days = query_u64(&req, "days").unwrap_or(7).clamp(1, 90);
            let result = tokio::task::spawn_blocking(move || observer.view(&view, limit, days)).await?;
            match result {
                Ok(body) => respond(&mut stream, "200 OK", &body).await,
                Err(err) => respond(&mut stream, "503 Service Unavailable", &json!({"detail":err.to_string()})).await,
            }
        }
        ("GET", "/api/factr/observability/memory") => {
            let limit = query_u64(&req, "limit").unwrap_or(50).clamp(1, 200);
            let session = auth::query_param(req.query.as_deref().unwrap_or_default(), "session").filter(|s| !s.is_empty());
            let result = tokio::task::spawn_blocking(move || observer.memory(session.as_deref(), limit)).await?;
            match result {
                Ok(body) => respond(&mut stream, "200 OK", &body).await,
                Err(err) => respond(&mut stream, "503 Service Unavailable", &json!({"detail":err.to_string()})).await,
            }
        }
        ("GET", "/api/factr/observability/budget") => {
            let result = tokio::task::spawn_blocking(move || observer.budget()).await?;
            match result {
                Ok(body) => respond(&mut stream, "200 OK", &body).await,
                Err(err) => {
                    respond(
                        &mut stream,
                        "503 Service Unavailable",
                        &json!({"detail":err.to_string()}),
                    )
                    .await
                }
            }
        }
        ("GET", "/api/factr/observability/approvals") => {
            let limit = query_u64(&req, "limit").unwrap_or(100).clamp(1, 500);
            let result = tokio::task::spawn_blocking(move || observer.approvals(limit)).await?;
            match result {
                Ok(body) => respond(&mut stream, "200 OK", &body).await,
                Err(err) => {
                    respond(
                        &mut stream,
                        "503 Service Unavailable",
                        &json!({"detail":err.to_string()}),
                    )
                    .await
                }
            }
        }
        ("POST", "/api/factr/observability/promote") => {
            let body = tokio::time::timeout(HEADER_TIMEOUT, read_body(&mut stream, &req)).await??;
            let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
            let Some(run_id) = body["run_id"].as_str().filter(|s| !s.is_empty()) else {
                return respond(
                    &mut stream,
                    "400 Bad Request",
                    &json!({"detail":"run_id is required"}),
                )
                .await;
            };
            let run_id = run_id.to_string();
            let result = tokio::task::spawn_blocking(move || observer.promote(&run_id)).await?;
            match result {
                Ok(body) => respond(&mut stream, "200 OK", &body).await,
                Err(rusqlite::Error::QueryReturnedNoRows) => {
                    respond(
                        &mut stream,
                        "404 Not Found",
                        &json!({"detail":"run not found"}),
                    )
                    .await
                }
                Err(err) => {
                    respond(
                        &mut stream,
                        "503 Service Unavailable",
                        &json!({"detail":err.to_string()}),
                    )
                    .await
                }
            }
        }
        ("POST", "/api/factr/observability/replay") => {
            let body = tokio::time::timeout(HEADER_TIMEOUT, read_body(&mut stream, &req)).await??;
            let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
            let Some(run_id) = body["run_id"].as_str().filter(|s| !s.is_empty()) else {
                return respond(
                    &mut stream,
                    "400 Bad Request",
                    &json!({"detail":"run_id is required"}),
                )
                .await;
            };
            let run_id = run_id.to_string();
            let config = config.clone();
            let hub = hub.clone();
            let observer = observer.clone();
            match rpc::replay_run(config, hub, observer, &run_id).await {
                Ok(body) => respond(&mut stream, "200 OK", &body).await,
                Err(err) => {
                    respond(
                        &mut stream,
                        "503 Service Unavailable",
                        &json!({"detail": err.to_string()}),
                    )
                    .await
                }
            }
        }
        ("GET", "/api/factr/observability/run") => {
            let Some(id) = auth::query_param(req.query.as_deref().unwrap_or_default(), "id") else {
                return respond(
                    &mut stream,
                    "400 Bad Request",
                    &json!({"detail":"id is required"}),
                )
                .await;
            };
            let result = tokio::task::spawn_blocking(move || observer.detail(&id)).await?;
            match result {
                Ok(body) => respond(&mut stream, "200 OK", &body).await,
                Err(rusqlite::Error::QueryReturnedNoRows) => {
                    respond(
                        &mut stream,
                        "404 Not Found",
                        &json!({"detail":"run not found"}),
                    )
                    .await
                }
                Err(err) => {
                    respond(
                        &mut stream,
                        "503 Service Unavailable",
                        &json!({"detail":err.to_string()}),
                    )
                    .await
                }
            }
        }
        (m, path) if token_ok && deployment::is_private() && deployment::blocks_rest(m, path) => {
            respond(
                &mut stream,
                "403 Forbidden",
                &json!({"detail": deployment::PRIVATE_MSG, "reason": "private_deployment"}),
            )
            .await
        }
        (_, path) if path.starts_with("/api/") && config.features.is_some() => {
            // Without feature=1, refuse to start Python for unknown probes so a
            // chat-only first paint on Windows/macOS stays Python-free.
            if bundled_startup_request(&req) && !capabilities_request(&req) {
                // The onboarding's OAuth-provider list is a boot probe: sign-in belongs to the Python
                // feature runtime, which stays off at first paint, and the desktop falls back to API-key
                // setup on this 404. It is by design, not a parity gap, so it is not logged as one.
                if !(req.method == "GET" && path == "/api/providers/oauth") {
                    note_unsupported("http", &format!("{} {path}", req.method));
                }
                return respond(
                    &mut stream,
                    "404 Not Found",
                    &json!({"detail": "not supported by engine", "reason": "feature_not_requested"}),
                )
                .await;
            }
            let features = config.features.clone().expect("checked");
            let sets_key = matches!(req.method.as_str(), "PUT" | "DELETE") && path == "/api/env";
            // A provider OAuth login (submit / poll to approval) or logout writes Factr's auth store, not `.env`.
            let oauth = path.starts_with("/api/providers/oauth");
            let factr_home = factr_base::factr_config::home().filter(|_| oauth);
            if let Some(home) = &factr_home {
                oauth_credentials_moved(&OAUTH_SEEN, home); // first sight: take the baseline
            }
            let proxied = proxy_http(stream, &req, &features, false).await;
            let moved = factr_home.is_some_and(|home| oauth_credentials_moved(&OAUTH_SEEN, &home));
            if sets_key || moved {
                let config = config.clone();
                let provider = path.strip_prefix("/api/providers/oauth/").and_then(|r| r.split('/').next()).filter(|_| moved).map(str::to_string);
                tokio::spawn(async move { notify_auth_changed(&config, provider.as_deref()).await });
            }
            proxied
        }
        (method, path) => {
            note_unsupported("http", &format!("{method} {path}"));
            let body =
                json!({"detail": "not supported by engine", "reason": "not_supported_by_engine"});
            respond(&mut stream, "404 Not Found", &body).await
        }
    }
}

/// Where each skill came from, for the Skills screen: `hub` (in the skills hub lock file),
/// `bundled` (seeded by Factr' `.bundled_manifest` or shipped with the engine), else `agent`
/// (created by the model through `skill_manage` or learned by refine).
struct SkillOrigins {
    hub: std::collections::HashSet<String>,
    bundled: std::collections::HashSet<String>,
}

impl SkillOrigins {
    fn load() -> Self {
        let root = factr_base::storage::factr_dir().map(|h| h.join("skills")).unwrap_or_default();
        Self::read(&root, &factr_learn::shipped_skill_dirs())
    }

    fn read(skills_root: &Path, shipped: &[&str]) -> Self {
        let mut hub = std::collections::HashSet::new();
        if let Some(lock) = std::fs::read_to_string(skills_root.join(".hub/lock.json")).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) {
            for (name, info) in lock["installed"].as_object().into_iter().flatten() {
                hub.insert(name.clone());
                if let Some(path) = info["install_path"].as_str() {
                    hub.extend(path.rsplit('/').next().map(str::to_string));
                }
            }
        }
        let mut bundled: std::collections::HashSet<String> = shipped.iter().map(|s| s.to_string()).collect();
        for line in std::fs::read_to_string(skills_root.join(".bundled_manifest")).unwrap_or_default().lines() {
            let name = line.split(':').next().unwrap_or_default().trim();
            if !name.is_empty() {
                bundled.insert(name.to_string());
            }
        }
        Self { hub, bundled }
    }

    fn of(&self, name: &str, path: &Path) -> &'static str {
        let dir = path.parent().and_then(|p| p.file_name()).map(|d| d.to_string_lossy().to_string()).unwrap_or_default();
        let is = |set: &std::collections::HashSet<String>| set.contains(name) || set.contains(&dir);
        if is(&self.hub) {
            "hub"
        } else if is(&self.bundled) {
            "bundled"
        } else {
            "agent"
        }
    }
}

#[cfg(test)]
mod skill_origin_tests {
    use super::*;

    #[test]
    fn skills_report_bundled_hub_or_agent_provenance() {
        let root = std::env::temp_dir().join(format!("skill-origins-{}", std::process::id()));
        std::fs::create_dir_all(root.join(".hub")).unwrap();
        std::fs::write(root.join(".bundled_manifest"), "plan:abc123\nnotes\n").unwrap();
        std::fs::write(root.join(".hub/lock.json"), r#"{"version":1,"installed":{"web-research":{"install_path":"research/web-research"}}}"#).unwrap();
        let origin = SkillOrigins::read(&root, &["learn-goal"]);
        let at = |dir: &str| root.join(dir).join("SKILL.md");
        assert_eq!(origin.of("plan", &at("plan")), "bundled");
        assert_eq!(origin.of("Notes skill", &at("notes")), "bundled");
        assert_eq!(origin.of("learn-goal", &at("learn-goal")), "bundled");
        assert_eq!(origin.of("web-research", &at("web-research")), "hub");
        assert_eq!(origin.of("release-codeword", &at("release-codeword")), "agent");
        std::fs::remove_dir_all(root).ok();
    }
}

#[cfg(test)]
mod capabilities_gate_tests {
    use super::{Request, capabilities_request};

    fn req(method: &str, path: &str) -> Request {
        Request { method: method.into(), path: path.into(), query: None, headers: Vec::new(), body_prefix: Vec::new() }
    }

    #[test]
    fn capabilities_screens_reach_python_but_boot_probes_do_not() {
        for (method, path) in [
            ("GET", "/api/tools/toolsets"),
            ("GET", "/api/tools/toolsets/web/config"),
            ("GET", "/api/tools/computer-use/status"),
            ("PUT", "/api/skills/toggle"),
            ("POST", "/api/skills/hub/install"),
            ("POST", "/api/mcp/servers"),
            ("DELETE", "/api/mcp/servers/x"),
            ("POST", "/api/tools/terminal/backend"),
            ("POST", "/api/providers/oauth/openai-codex/start"),
            ("GET", "/api/providers/oauth/openai-codex/poll/sess-1"),
            ("POST", "/api/providers/oauth/openai-codex/submit"),
            ("DELETE", "/api/providers/oauth/sessions/sess-1"),
            ("DELETE", "/api/providers/oauth/openai-codex"),
        ] {
            assert!(capabilities_request(&req(method, path)), "{method} {path}");
        }
        for (method, path) in [
            ("GET", "/api/tools/terminal/backends"),
            ("GET", "/api/mcp/servers"),
            ("GET", "/api/skills"),
            ("GET", "/api/plugins"),
            ("GET", "/api/tools/toolsetsx"),
            ("GET", "/api/config"),
            ("GET", "/api/providers/oauth"),
            ("GET", "/api/providers/oauthx/start"),
            ("POST", "/api/toolsx"),
        ] {
            assert!(!capabilities_request(&req(method, path)), "{method} {path}");
        }
    }
}

#[cfg(test)]
mod contract_gate_tests {
    #[test]
    fn only_contract_methods_are_forwardable() {
        let methods = super::contract_methods();
        assert_eq!(methods.len(), 212);
        assert!(methods.contains("cron.manage") && methods.contains("config.show"));
        assert!(!methods.contains("definitely.not.a.method"));
    }
}

#[cfg(test)]
mod auth_notice_tests {
    use super::*;

    #[test]
    fn an_oauth_login_or_logout_moves_the_credential_stamp_and_a_plain_poll_does_not() {
        let dir = std::env::temp_dir().join(format!("oauth-stamp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("profiles/work")).unwrap();
        let seen = std::sync::Mutex::new(None);
        assert!(!oauth_credentials_moved(&seen, &dir), "the first look is only a baseline");
        assert!(!oauth_credentials_moved(&seen, &dir), "a pending poll writes nothing");
        std::fs::write(dir.join("auth.json"), "{}").unwrap(); // the device flow was approved
        assert!(oauth_credentials_moved(&seen, &dir));
        assert!(!oauth_credentials_moved(&seen, &dir), "reported once");
        std::fs::write(dir.join("profiles/work/auth.json"), "{\"a\":1}").unwrap(); // a profile's login
        assert!(oauth_credentials_moved(&seen, &dir));
        std::fs::remove_file(dir.join("auth.json")).unwrap(); // logout
        assert!(oauth_credentials_moved(&seen, &dir));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn only_key_writing_factr_rpcs_signal_an_auth_change() {
        for method in ["model.save_key", "model.disconnect", "reload.env"] {
            assert!(changes_credentials(method), "{method}");
        }
        assert!(!changes_credentials("model.options") && !changes_credentials("config.set"));
    }

    /// A stand-in engine daemon that answers `subscribe`/`state`/`notify_auth_changed` and records what it was sent.
    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn a_credential_change_reaches_the_engine_as_notify_auth_changed_on_a_subscribed_link() {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        let dir = std::env::temp_dir().join(format!("auth-notice-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let socket = dir.join("d.sock");
        let _ = std::fs::remove_file(&socket);
        let listener = tokio::net::UnixListener::bind(&socket).unwrap();
        let seen = Arc::new(std::sync::Mutex::new(Vec::<Value>::new()));
        let log = seen.clone();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let log = log.clone();
                tokio::spawn(async move {
                    let (read, mut write) = stream.into_split();
                    let mut lines = BufReader::new(read).lines();
                    while let Ok(Some(line)) = lines.next_line().await {
                        let Ok(request) = serde_json::from_str::<Value>(&line) else { continue };
                        log.lock().unwrap().push(request.clone());
                        let reply = match request["type"].as_str() {
                            Some("state") => json!({ "type": "state", "id": request["id"], "session_id": "session_throwaway", "is_processing": false }),
                            Some("notify_auth_changed") => json!({ "type": "done", "id": request["id"] }),
                            _ => continue,
                        };
                        let _ = write.write_all(format!("{reply}\n").as_bytes()).await;
                    }
                });
            }
        });
        let config = Config {
            bind: "127.0.0.1:0".parse().unwrap(), token: "t".repeat(32), version: "test".into(), legacy_socket: socket,
            default_cwd: "/".into(), allow_non_loopback: false, provider: "p".into(), model: "m".into(), reasoning_efforts: Vec::new(), profile_model_applies: true,
            home: dir.to_string_lossy().into(), complete: None, features: None, learning: None,
        };
        notify_auth_changed(&config, Some("openrouter")).await;
        let sent: Vec<String> = seen.lock().unwrap().iter().map(|r| r["type"].as_str().unwrap_or("").to_string()).collect();
        let (subscribe, notify) = (sent.iter().position(|t| t == "subscribe"), sent.iter().position(|t| t == "notify_auth_changed"));
        assert!(subscribe.is_some() && notify > subscribe, "subscribed first, then notified: {sent:?}");
        let notice = seen.lock().unwrap().iter().find(|r| r["type"] == "notify_auth_changed").cloned().unwrap();
        assert_eq!(notice["provider"], "openrouter");
        // A provider name that is not a plain identifier falls back to the generic hint.
        notify_auth_changed(&config, Some("bad name\n")).await;
        assert_eq!(seen.lock().unwrap().iter().rev().find(|r| r["type"] == "notify_auth_changed").unwrap()["provider"], "factr");
        std::fs::remove_dir_all(dir).ok();
    }
}

#[cfg(test)]
mod proxy_activity_tests {
    use super::*;

    /// A terminal, console or display socket owns a backend process: it holds a lease while open. An
    /// event stream does not, and every plain request does.
    #[test]
    fn process_sockets_hold_a_lease_and_event_streams_do_not() {
        for path in ["/api/pty", "/api/console", "/api/display/ws"] {
            assert!(holds_lease(path, true), "{path}");
        }
        for path in ["/api/events", "/api/pub"] {
            assert!(!holds_lease(path, true), "{path}");
        }
        assert!(holds_lease("/api/events", false), "a plain request always holds one");
    }

    /// An open event-stream socket holds no lease: only its frames keep the backend from idling out.
    #[tokio::test]
    async fn frames_on_a_proxied_socket_mark_the_backend_active_without_a_lease() {
        let features = Arc::new(features::Features::new(vec![]));
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert!(features.idle_for() >= Duration::from_millis(50));
        let (mut client, from) = tokio::io::duplex(64);
        let (to, mut upstream) = tokio::io::duplex(64);
        let pumping = tokio::spawn({
            let features = features.clone();
            async move { pump(from, to, &features).await }
        });
        client.write_all(b"frame").await.unwrap();
        let mut got = [0_u8; 5];
        upstream.read_exact(&mut got).await.unwrap();
        assert_eq!(&got, b"frame");
        assert!(features.idle_for() < Duration::from_millis(50), "the frame counted as use");
        drop(client);
        pumping.await.unwrap();
        let mut rest = Vec::new();
        upstream.read_to_end(&mut rest).await.unwrap();
        assert!(rest.is_empty(), "closing one side closes the other");
    }
}
