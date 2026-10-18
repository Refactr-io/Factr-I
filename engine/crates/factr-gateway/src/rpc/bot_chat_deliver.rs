//! `bot.chat.deliver` / `bot.chat.deliver_status`: a teammate's message becomes a real turn in THIS engine's
//! canonical Bot Chat, through the engine's own turn machinery, with no Python CLI subprocess per message.
//!
//! The Desktop relay (relay.ts) drains the sender's outbox, calls `bot.chat.deliver` on the target bot's engine and
//! writes the reply back for the sender's waiter. A turn can take minutes, so `deliver` returns `pending` after a
//! short wait and the relay polls `deliver_status` with the same `envelope_id` (idempotent: a repeated `deliver`
//! joins the running turn, a finished one returns its stored result).
//!
//! Upstream semantics kept: the message already carries the attribution prefix (`Message from <emoji> <Name>
//! (@from):`, composed by the sender), one retry for a transient failure (the same session and message again),
//! a typed `reason` on failure, `[SILENT]` becomes an empty reply, replies cap at 16,000 characters, and a message
//! chain stops at two relay hops. The turn runs on a headless connection exactly like `/api/agent/run`. A window that
//! holds the chat idle does not get in the way (the engine lets a second connection attach and the window sees the turn
//! live); when the window is mid-turn the delivery waits for that turn to end first, so the two never interleave.
//!
//! `bot.chat.ensure` also passes through here first: the teammate MCP server's tool schema is cached on disk after its
//! first connection, so on a fresh install the bot's first chat advertised `message_agent` that it could not load until
//! the next start. Warming the cache before the chat is created makes the tool available on the first spawn.

use super::*;
use std::sync::LazyLock;
use std::time::Instant;

const TITLE: &str = "Bot Chat";
/// A reply (or message) longer than this is cut, with a note.
const MAX_CHARS: usize = 16_000;
/// A message chain longer than this many relay hops is refused (OpenGrokBot's `MAX_HOPS`).
const MAX_HOPS: u64 = 2;
/// How long `deliver` and `deliver_status` hold the call before answering `pending`.
const POLL_WAIT: Duration = Duration::from_secs(45);
/// One turn's own limit (the relay's per-turn budget).
const TURN_TIMEOUT: Duration = Duration::from_secs(20 * 60);
/// A finished delivery is remembered this long so a repeated call returns it instead of running a second turn.
const KEEP_DONE: Duration = Duration::from_secs(3600);
/// The teammate server whose schema is warmed (the only MCP server this module starts).
const TEAMMATES_SERVER: &str = "factr_teammates";
const WARM_TIMEOUT: Duration = Duration::from_secs(20);

pub(super) fn handles(method: &str) -> bool {
    matches!(method, "bot.chat.deliver" | "bot.chat.deliver_status" | "bot.chat.ensure")
}

// ── failure classification (tools/bot_failure_reasons.py) ────────────────────────────────────────────

/// A closed reason code for raw provider or engine failure text; `unknown` when nothing matches. First match wins.
pub(super) fn classify(text: &str) -> &'static str {
    use fancy_regex::Regex;
    static RULES: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
        let status = r"(?:error code:?\s*|status(?:\s*code)?:?\s*|http\s*)";
        [
            (format!(r"authentication_error|invalid api key|invalid x-api-key|incorrect api key|unauthorized|not signed in|login_missing|{status}(?:401|403)\b"), "provider_auth_or_access"),
            (format!(r"{status}402\b|out of funds|quota|balance"), "provider_quota_limit"),
            (format!(r"{status}429\b|rate.?limit"), "provider_rate_limit"),
            (format!(r"{status}5\d{{2}}\b|server[ _]?error|overloaded"), "provider_server_error"),
            (r"context length|context_overflow|maximum context".to_string(), "context_overflow"),
            (r"no llm provider configured|missing config|no access token".to_string(), "missing_config"),
            (r"model .*(not found|does not exist)|model_not_found".to_string(), "model_unavailable"),
        ]
        .into_iter()
        .filter_map(|(pattern, code)| Regex::new(&format!("(?i){pattern}")).ok().map(|re| (re, code)))
        .collect()
    });
    RULES.iter().find(|(re, _)| re.is_match(text).unwrap_or(false)).map(|(_, code)| *code).unwrap_or("unknown")
}

/// Whether a failure with this reason is worth one more try (transient, or an overflow the retry may compact past).
fn retryable(reason: &str) -> bool {
    matches!(reason, "provider_rate_limit" | "provider_server_error" | "delivery_timeout" | "context_overflow")
}

/// `[SILENT]` and `NO_REPLY` (a bare marker, with stray edge punctuation) end a turn without rendering anything.
pub(super) fn is_silence(reply: &str) -> bool {
    let t = reply.trim();
    if t.is_empty() || t.chars().count() > 24 {
        return false;
    }
    let bare = t.trim_matches(|c: char| c.is_ascii_punctuation() && !matches!(c, '[' | ']')).trim();
    [t, bare].iter().any(|s| matches!(s.to_ascii_uppercase().as_str(), "[SILENT]" | "SILENT" | "NO_REPLY" | "[NO_REPLY]" | "HEARTBEAT_OK"))
}

/// The reply as the sender receives it: at most [`MAX_CHARS`], with a note when it was cut.
pub(super) fn cap_reply(reply: &str) -> String {
    let total = reply.chars().count();
    if total <= MAX_CHARS {
        return reply.to_string();
    }
    let head: String = reply.chars().take(MAX_CHARS).collect();
    format!("{head}\n\n[reply cut at {MAX_CHARS} of {total} characters]")
}

// ── one delivery: the policy around a turn ───────────────────────────────────────────────────────────

/// How a turn ended, as the caller of [`flow`] sees it.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct TurnEnd {
    pub ok: bool,
    pub interrupted: bool,
    /// The turn's final assistant text (on a failure, the provider's own prose when it has any).
    pub text: String,
    pub error: String,
}

/// What the engine does for the flow: run the message once, then run the same message again.
pub(super) trait Turns {
    async fn first(&self, text: &str) -> TurnEnd;
    async fn again(&self) -> TurnEnd;
}

/// The finished delivery `deliver` / `deliver_status` return.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Outcome {
    pub ok: bool,
    pub reply: String,
    pub error: String,
    pub reason: String,
}

impl Outcome {
    fn failed(error: impl Into<String>, reason: &str) -> Self {
        Self { ok: false, reply: String::new(), error: error.into(), reason: reason.into() }
    }

    fn to_json(&self, session_id: &str) -> Value {
        let mut out = json!({ "status": "done", "ok": self.ok, "reply": self.reply, "session_id": session_id });
        if !self.ok {
            out["error"] = json!(self.error);
            out["reason"] = json!(self.reason);
        }
        out
    }
}

/// Run the message and apply the delivery policy: one retry of a transient failure, a typed reason on failure,
/// `[SILENT]` suppressed, the reply capped. `text` is the message (already attributed by its sender).
pub(super) async fn flow(turns: &impl Turns, text: &str) -> Outcome {
    if text.chars().count() > MAX_CHARS + 200 {
        return Outcome::failed(format!("message too long ({} > {MAX_CHARS} characters). Send the essentials.", text.chars().count()), "unknown");
    }
    let mut end = turns.first(text).await;
    for attempt in 0..2 {
        if end.ok && !end.interrupted {
            let reply = end.text.trim();
            return Outcome { ok: true, reply: if is_silence(reply) { String::new() } else { cap_reply(reply) }, error: String::new(), reason: String::new() };
        }
        if end.interrupted {
            return Outcome::failed("the turn was stopped before it finished", "cancelled");
        }
        let detail = [end.error.trim(), end.text.trim()].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join("\n");
        let reason = classify(&detail);
        if attempt == 0 && retryable(reason) {
            end = turns.again().await;
            continue;
        }
        let shown: String = detail.chars().take(500).collect();
        return Outcome::failed(if shown.is_empty() { "the turn failed".to_string() } else { shown }, reason);
    }
    Outcome::failed("the turn failed", "unknown")
}

// ── the engine side ──────────────────────────────────────────────────────────────────────────────────

/// A delivery in flight or finished: callers subscribe to its result.
struct Slot {
    tx: tokio::sync::watch::Sender<Option<Value>>,
    at: Instant,
}

static DELIVERIES: LazyLock<std::sync::Mutex<HashMap<String, Slot>>> = LazyLock::new(Default::default);

fn own_profile() -> String {
    crate::profile::current().name.filter(|n| !n.is_empty()).unwrap_or_else(|| "default".into())
}

/// The install root the relay's files live under (`<root>/bot_relay/...`): the parent of `profiles/`.
fn install_root() -> Option<std::path::PathBuf> {
    factr_base::factr_config::home().map(|home| root_of(&home))
}

fn root_of(home: &std::path::Path) -> std::path::PathBuf {
    let in_profiles = home.parent().and_then(|p| p.file_name()).is_some_and(|n| n == "profiles");
    match home.parent().and_then(|p| p.parent()) {
        Some(root) if in_profiles => root.to_path_buf(),
        _ => home.to_path_buf(),
    }
}

/// Tell the tools running this turn how deep the relay chain is (`tools/bot_relay.py` reads the marker when it
/// queues the next hop). Removed when the guard drops.
struct HopsMarker(Option<std::path::PathBuf>);

impl HopsMarker {
    fn set(profile: &str, hops: u64) -> Self {
        let name: String = profile.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '_' | '-') { c } else { '_' }).collect();
        let path = install_root().map(|r| r.join("bot_relay").join("hops").join(format!("{name}.json")));
        if let Some(path) = &path {
            let _ = std::fs::create_dir_all(path.parent().unwrap_or(path));
            let body = json!({ "hops": hops, "at": chrono::Utc::now().timestamp() });
            let _ = std::fs::write(path, body.to_string());
        }
        Self(path)
    }
}

impl Drop for HopsMarker {
    fn drop(&mut self) {
        if let Some(path) = &self.0 {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// The real turns: a headless connection attached to the Bot Chat session.
struct Engine {
    run: Arc<Conn>,
    rx: tokio::sync::Mutex<mpsc::Receiver<Message>>,
    session: String,
    /// The message being delivered and how many messages the chat held before it: a completion is ours only when the
    /// stored transcript shows this message followed by an answer (another turn's `message.complete` is not).
    text: String,
    before: usize,
}

impl Engine {
    /// Wait for the session's `message.complete` on this run's own connection.
    async fn finish(&self, started: Result<Value, RpcError>) -> TurnEnd {
        if let Err(err) = started {
            let detail = err.data.as_ref().and_then(|d| d["reason"].as_str()).map(|r| format!("{} ({r})", err.message)).unwrap_or(err.message);
            return TurnEnd { ok: false, interrupted: false, text: String::new(), error: detail };
        }
        let waited = tokio::time::timeout(TURN_TIMEOUT, async {
            let mut rx = self.rx.lock().await;
            while let Some(Message::Text(text)) = rx.recv().await {
                let Ok(frame) = serde_json::from_str::<Value>(&text) else { continue };
                if frame["method"] == "event" && frame["params"]["session_id"] == self.session.as_str() && frame["params"]["type"] == "message.complete" {
                    return Some(frame["params"]["payload"].clone());
                }
            }
            None
        })
        .await;
        match waited {
            Ok(Some(payload)) => {
                let reply = turn_reply(&payload, &self.session, "bot");
                let ok = reply["ok"] == true && reply["interrupted"] != true;
                // The answer is read from the stored transcript: our message followed by an assistant message. A
                // completion for some other turn on this chat (one that was already running, or a replayed one) has no
                // answer to our message yet, so it is not ours and we wait on.
                let ours = if ok { self.answer().await } else { None };
                if ok && ours.is_none() {
                    return Box::pin(self.finish(Ok(Value::Null))).await;
                }
                TurnEnd {
                    ok: reply["ok"] == true,
                    interrupted: reply["interrupted"] == true,
                    text: ours.unwrap_or_else(|| reply["final_text"].as_str().or(reply["text"].as_str()).unwrap_or_default().to_string()),
                    error: reply["error"].as_str().unwrap_or_default().to_string(),
                }
            }
            Ok(None) => TurnEnd { ok: false, interrupted: false, text: String::new(), error: "the engine connection closed before the turn finished".into() },
            Err(_) => {
                let _ = self.run.dispatch("session.interrupt", &json!({ "session_id": self.session })).await;
                TurnEnd { ok: false, interrupted: false, text: String::new(), error: "timed out waiting for the turn to finish".into() }
            }
        }
    }
}

impl Engine {
    /// The answer to our message in the stored transcript, if there is one (a short grace for the write).
    async fn answer(&self) -> Option<String> {
        for _ in 0..4 {
            let messages = factr_base::session::Session::load(&self.session).map(|s| s.messages).unwrap_or_default();
            if let Some(Some(reply)) = answer_to(&messages, &self.text, self.before) {
                return Some(reply);
            }
            tokio::time::sleep(Duration::from_millis(400)).await;
        }
        None
    }
}

impl Turns for Engine {
    async fn first(&self, text: &str) -> TurnEnd {
        let started = self.run.dispatch("prompt.submit", &json!({ "session_id": self.session, "text": text })).await;
        self.finish(started).await
    }

    async fn again(&self) -> TurnEnd {
        tokio::time::sleep(Duration::from_secs(2)).await;
        // `session.retry` cuts the failed turn and sends the same user message again: no duplicate in the chat.
        let started = self.run.dispatch("session.retry", &json!({ "session_id": self.session })).await;
        self.finish(started).await
    }
}

/// Run `text` in the bot's own canonical Bot Chat. A window that has the chat open gets the message instead.
async fn deliver_now(conn: &Arc<Conn>, text: &str, hops: u64) -> Value {
    let profile = own_profile();
    warm_teammates().await;
    let chat = match conn.bot_chat("bot.chat.ensure", &json!({ "profile": profile })).await {
        Ok(v) => v,
        Err(err) => return Outcome::failed(err.message, "unknown").to_json(""),
    };
    let Some(session) = chat["canonical_session"]["id"].as_str().map(str::to_string) else {
        return Outcome::failed("this bot has no Bot Chat", "unknown").to_json("");
    };
    // One turn at a time per chat, queued behind a running one.
    let _turn = chat_turn(&format!("bot-chat-deliver:{profile}")).await;
    // A window mid-turn on the chat: a prompt submitted now would not queue (the engine steers a running turn), so wait
    // for that turn to end. Bounded: a chat that never goes idle is `target_busy`.
    let give_up = Instant::now() + TURN_TIMEOUT;
    while session_processing(conn, &session).await {
        if Instant::now() > give_up {
            return Outcome::failed("its Bot Chat is busy with another turn", "target_busy").to_json(&session);
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    let (to_ws, rx) = mpsc::channel::<Message>(1024);
    let client = Arc::new(Client { id: conn.hub.next_client_id(), to_ws: to_ws.clone(), sessions: Mutex::new(Default::default()) });
    let run = Conn::new(conn.config.clone(), to_ws, conn.hub.clone(), client, conn.observer.clone(), false, "bot", Some(TITLE.to_string()), None);
    let outcome = async {
        let control = run.open_link().await.map_err(|e| format!("engine unavailable: {e:#}"))?;
        *run.control.lock().await = Some(control);
        run.dispatch("session.activate", &json!({ "session_id": session, "omit_messages": true })).await.map_err(|e| e.message)?;
        Ok::<_, String>(())
    }
    .await;
    if let Err(message) = outcome {
        end_run(&run, None).await;
        return Outcome::failed(message.clone(), if is_already_live(&message) { "target_busy" } else { "runtime_offline" }).to_json(&session);
    }
    conn.hub.mark_headless(&session, "bot").await;
    let _hops = HopsMarker::set(&profile, hops);
    let before = factr_base::session::Session::load(&session).map(|s| s.messages.len()).unwrap_or(0);
    let engine = Engine { run: run.clone(), rx: tokio::sync::Mutex::new(rx), session: session.clone(), text: text.to_string(), before };
    let done = flow(&engine, text).await;
    conn.hub.unmark_headless(&session).await;
    conn.observer.release_replay(&session);
    end_run(&run, None).await;
    done.to_json(&session)
}

/// Whether the engine says a turn is running on `session` right now.
async fn session_processing(conn: &Arc<Conn>, session: &str) -> bool {
    conn.call(json!({ "req": "list_sessions", "limit": 1000, "include_archived": false }))
        .await
        .ok()
        .and_then(|r| r["sessions"].as_array().and_then(|l| l.iter().find(|s| s["session_id"] == session).map(|s| s["status"] == "processing")))
        .unwrap_or(false)
}

/// Plain text of a stored message (its text blocks).
fn text_of(m: &factr_base::session::StoredMessage) -> String {
    m.content.iter().filter_map(|b| if let factr_base::message::ContentBlock::Text { text, .. } = b { Some(text.as_str()) } else { None }).collect::<Vec<_>>().join("\n")
}

/// What the stored transcript says about a message handed to a window: `None` until the window has submitted `text`,
/// then `Some(None)` while the turn runs and `Some(Some(reply))` when an assistant answer follows it.
pub(super) fn answer_to(messages: &[factr_base::session::StoredMessage], text: &str, before: usize) -> Option<Option<String>> {
    use factr_base::message::Role;
    let want = text.trim();
    let at = messages.iter().enumerate().skip(before.saturating_sub(1)).rev().find(|(_, m)| m.role == Role::User && text_of(m).trim() == want).map(|(i, _)| i)?;
    let reply = messages[at + 1..].iter().rev().filter(|m| m.role == Role::Assistant).map(text_of).find(|t| !t.trim().is_empty());
    Some(reply)
}

/// Connect the teammate MCP server once and store its tool schema, if the cache does not hold it yet.
async fn warm_teammates() {
    use factr_base::mcp::{McpClient, McpConfig, McpSchemaCache};
    let config = McpConfig::load();
    let Some(server) = config.servers.get(TEAMMATES_SERVER).filter(|s| s.is_enabled()) else { return };
    if McpSchemaCache::load().tools_for(TEAMMATES_SERVER, server).is_some() {
        return;
    }
    let server = server.clone();
    let connected = tokio::time::timeout(WARM_TIMEOUT, McpClient::connect(TEAMMATES_SERVER.to_string(), &server)).await;
    if let Ok(Ok(client)) = connected {
        let tools = client.tools();
        if !tools.is_empty() {
            let mut cache = McpSchemaCache::load();
            if cache.update(TEAMMATES_SERVER, &server, tools) {
                cache.save();
            }
        }
    }
}

impl Conn {
    pub(super) async fn bot_deliver(self: &Arc<Self>, method: &str, p: &Value) -> Result<Value, RpcError> {
        match method {
            "bot.chat.ensure" => {
                // The registry call is bot_chat.rs's; the schema cache is warmed first so the chat it may mint spawns
                // with `message_agent` already known.
                if p["profile"].as_str().is_none_or(|n| n.is_empty() || n == own_profile()) {
                    warm_teammates().await;
                }
                self.bot_chat(method, p).await
            }
            "bot.chat.deliver" => self.deliver_rpc(p).await,
            _ => {
                let id = p["envelope_id"].as_str().filter(|s| !s.is_empty()).ok_or_else(|| RpcError::params("envelope_id is required"))?;
                match await_slot(id, POLL_WAIT).await {
                    Some(result) => Ok(result),
                    None => Err(RpcError::params("unknown envelope_id")),
                }
            }
        }
    }

    async fn deliver_rpc(self: &Arc<Self>, p: &Value) -> Result<Value, RpcError> {
        let text = p["text"].as_str().or(p["message"].as_str()).map(str::trim).filter(|t| !t.is_empty()).ok_or_else(|| RpcError::params("text is required"))?.to_string();
        if let Some(target) = p["profile"].as_str().filter(|n| !n.is_empty()) {
            if target != own_profile() {
                return Err(RpcError::params(&format!("this engine serves @{}, not @{target}", own_profile())));
            }
        }
        let hops = p["hops"].as_u64().unwrap_or(0);
        if hops > MAX_HOPS {
            return Ok(Outcome::failed(format!("this message already crossed {MAX_HOPS} bots on its way here; it was not delivered"), "hop_limit").to_json(""));
        }
        let id = p["envelope_id"].as_str().filter(|s| !s.is_empty()).map(str::to_string).unwrap_or_else(|| format!("anon-{:016x}", rand::random::<u64>()));
        let start = {
            let mut slots = DELIVERIES.lock().unwrap_or_else(|e| e.into_inner());
            slots.retain(|_, s| s.tx.borrow().is_none() || s.at.elapsed() < KEEP_DONE);
            if slots.contains_key(&id) {
                false
            } else {
                slots.insert(id.clone(), Slot { tx: tokio::sync::watch::channel(None).0, at: Instant::now() });
                true
            }
        };
        if start {
            let (conn, key) = (self.clone(), id.clone());
            // `dispatch` and this module call each other (a turn runs through `dispatch`), a cycle the compiler cannot prove
            // `Send` across. The turn runs on a blocking-pool thread that drives it on the same runtime instead.
            let runtime = tokio::runtime::Handle::current();
            tokio::task::spawn_blocking(move || {
                runtime.block_on(async move {
                    let result = deliver_now(&conn, &text, hops).await;
                    if let Some(slot) = DELIVERIES.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
                        slot.tx.send_replace(Some(result));
                    }
                })
            });
        }
        Ok(await_slot(&id, POLL_WAIT).await.unwrap_or_else(|| json!({ "status": "pending", "envelope_id": id })))
    }
}

/// The delivery's result if it finishes within [`POLL_WAIT`], else `pending`; None for an unknown id.
async fn await_slot(id: &str, wait: Duration) -> Option<Value> {
    let mut rx = DELIVERIES.lock().unwrap_or_else(|e| e.into_inner()).get(id).map(|s| s.tx.subscribe())?;
    let wait_for = async {
        loop {
            if let Some(done) = rx.borrow_and_update().clone() {
                return done;
            }
            if rx.changed().await.is_err() {
                return json!({ "status": "done", "ok": false, "reply": "", "error": "the delivery was lost", "reason": "unknown" });
            }
        }
    };
    Some(tokio::time::timeout(wait, wait_for).await.unwrap_or_else(|_| json!({ "status": "pending", "envelope_id": id })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    fn end(ok: bool, text: &str, error: &str) -> TurnEnd {
        TurnEnd { ok, interrupted: false, text: text.into(), error: error.into() }
    }

    /// Scripted turns: the first call and each retry pop the next scripted end.
    struct Script {
        ends: StdMutex<Vec<TurnEnd>>,
        asked: StdMutex<Vec<String>>,
    }

    impl Script {
        fn new(mut ends: Vec<TurnEnd>) -> Self {
            ends.reverse();
            Self { ends: StdMutex::new(ends), asked: StdMutex::new(Vec::new()) }
        }
        fn next(&self, what: &str) -> TurnEnd {
            self.asked.lock().unwrap().push(what.to_string());
            self.ends.lock().unwrap().pop().expect("more turns than scripted")
        }
    }

    impl Turns for Script {
        async fn first(&self, text: &str) -> TurnEnd {
            self.next(&format!("first:{text}"))
        }
        async fn again(&self) -> TurnEnd {
            self.next("again")
        }
    }

    #[test]
    fn reasons_are_typed_like_upstream_and_auth_comes_through() {
        for (text, reason) in [
            ("Error code: 401 - {'type': 'authentication_error', 'message': 'invalid x-api-key'}", "provider_auth_or_access"),
            ("Incorrect API key provided: sk-xxx", "provider_auth_or_access"),
            ("HTTP 403 forbidden", "provider_auth_or_access"),
            ("provider returned status 402", "provider_quota_limit"),
            ("429 Too Many Requests: rate limit reached", "provider_rate_limit"),
            ("Error code: 503 overloaded_error", "provider_server_error"),
            ("maximum context length is 8192 tokens", "context_overflow"),
            ("No LLM provider configured", "missing_config"),
            ("model 'x' not found", "model_unavailable"),
            ("the dog ate it", "unknown"),
            ("", "unknown"),
        ] {
            assert_eq!(classify(text), reason, "{text}");
        }
        // An order-sensitive pair: an auth failure is never taken for a server error mentioning a 5xx later.
        assert_eq!(classify("401 unauthorized (upstream said 503)"), "provider_auth_or_access");
    }

    #[test]
    fn only_a_bare_marker_is_silence() {
        for silent in ["[SILENT]", " [silent] ", "NO_REPLY", ".NO_REPLY", "*NO_REPLY*", "[SILENT]."] {
            assert!(is_silence(silent), "{silent:?}");
        }
        for spoken in ["", "   ", "I will stay [SILENT] about it", "The marker [SILENT] is a token", "ok"] {
            assert!(!is_silence(spoken), "{spoken:?}");
        }
    }

    #[test]
    fn a_long_reply_is_cut_at_16000_with_a_note_and_a_short_one_is_untouched() {
        assert_eq!(cap_reply("short"), "short");
        let exact = "x".repeat(MAX_CHARS);
        assert_eq!(cap_reply(&exact), exact);
        let long = "y".repeat(MAX_CHARS + 500);
        let cut = cap_reply(&long);
        assert!(cut.starts_with(&"y".repeat(MAX_CHARS)) && cut.contains("[reply cut at 16000 of 16500 characters]"));
        // Multi-byte text is cut on a character, never inside one.
        let wide = "é".repeat(MAX_CHARS + 1);
        assert!(cap_reply(&wide).starts_with(&"é".repeat(MAX_CHARS)));
    }

    #[tokio::test]
    async fn a_good_turn_returns_its_reply_and_a_silent_one_returns_empty() {
        let ok = flow(&Script::new(vec![end(true, "  here is the answer  ", "")]), "Message from 🤖 A (@a): hi").await;
        assert_eq!(ok, Outcome { ok: true, reply: "here is the answer".into(), error: String::new(), reason: String::new() });
        let silent = flow(&Script::new(vec![end(true, "[SILENT]", "")]), "hi").await;
        assert!(silent.ok && silent.reply.is_empty(), "nothing is rendered or sent back for [SILENT]");
        let prose = flow(&Script::new(vec![end(true, "I will not say [SILENT] aloud", "")]), "hi").await;
        assert_eq!(prose.reply, "I will not say [SILENT] aloud");
    }

    #[tokio::test]
    async fn the_message_is_passed_through_unchanged_with_its_attribution() {
        let script = Script::new(vec![end(true, "ok", "")]);
        let message = "Message from 🤖 Alpha (@alpha): what is the secret?";
        flow(&script, message).await;
        assert_eq!(script.asked.lock().unwrap().as_slice(), [format!("first:{message}")]);
    }

    #[tokio::test]
    async fn an_auth_failure_is_typed_and_never_retried() {
        let script = Script::new(vec![end(false, "", "the turn did not complete cleanly (error): Error code: 401 invalid x-api-key")]);
        let out = flow(&script, "hi").await;
        assert!(!out.ok);
        assert_eq!(out.reason, "provider_auth_or_access");
        assert!(out.error.contains("401"));
        assert_eq!(script.asked.lock().unwrap().len(), 1, "auth/quota/config never retry");
    }

    #[tokio::test]
    async fn a_transient_failure_retries_once_on_the_same_session_and_succeeds() {
        let script = Script::new(vec![end(false, "", "Error code: 503 overloaded"), end(true, "second time lucky", "")]);
        let out = flow(&script, "hi").await;
        assert_eq!((out.ok, out.reply.as_str()), (true, "second time lucky"));
        assert_eq!(script.asked.lock().unwrap().as_slice(), ["first:hi".to_string(), "again".to_string()], "the retry re-runs the same message, it does not send a new one");
    }

    #[tokio::test]
    async fn a_transient_failure_that_keeps_failing_is_retried_exactly_once() {
        let script = Script::new(vec![end(false, "", "429 rate limit"), end(false, "", "429 rate limit")]);
        let out = flow(&script, "hi").await;
        assert_eq!((out.ok, out.reason.as_str()), (false, "provider_rate_limit"));
        assert_eq!(script.asked.lock().unwrap().len(), 2, "one try and one retry, never a loop");
    }

    #[tokio::test]
    async fn a_stopped_turn_and_an_unclassified_failure_do_not_retry() {
        let stopped = Script::new(vec![TurnEnd { ok: false, interrupted: true, text: String::new(), error: String::new() }]);
        let out = flow(&stopped, "hi").await;
        assert_eq!((out.ok, out.reason.as_str()), (false, "cancelled"));
        let odd = Script::new(vec![end(false, "", "something odd happened")]);
        let out = flow(&odd, "hi").await;
        assert_eq!(out.reason, "unknown");
        assert_eq!(odd.asked.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_message_over_the_cap_is_refused_before_any_turn_runs() {
        let script = Script::new(vec![]);
        let out = flow(&script, &"z".repeat(MAX_CHARS + 201)).await;
        assert!(!out.ok && out.error.contains("too long"));
        assert!(script.asked.lock().unwrap().is_empty());
    }

    #[test]
    fn the_result_json_carries_reason_only_on_failure() {
        let ok = Outcome { ok: true, reply: "r".into(), error: String::new(), reason: String::new() }.to_json("s1");
        assert_eq!((ok["status"].as_str(), ok["ok"].as_bool(), ok["reply"].as_str()), (Some("done"), Some(true), Some("r")));
        assert!(ok.get("reason").is_none() && ok.get("error").is_none());
        let bad = Outcome::failed("boom", "provider_server_error").to_json("s1");
        assert_eq!((bad["ok"].as_bool(), bad["reason"].as_str(), bad["error"].as_str()), (Some(false), Some("provider_server_error"), Some("boom")));
    }

    #[test]
    fn the_hops_marker_is_written_for_the_turn_and_removed_when_it_ends() {
        let dir = std::env::temp_dir().join(format!("hops{}", std::process::id()));
        let path = dir.join("bot_relay").join("hops").join("scout.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, r#"{"hops":1}"#).unwrap();
        drop(HopsMarker(Some(path.clone())));
        assert!(!path.exists(), "the marker is removed when the turn ends");
        drop(HopsMarker(None)); // no install root: nothing to remove, nothing to fail
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_install_root_is_the_parent_of_profiles() {
        assert_eq!(root_of(std::path::Path::new("/h/.factr/profiles/scout")), std::path::PathBuf::from("/h/.factr"));
        assert_eq!(root_of(std::path::Path::new("/h/.factr")), std::path::PathBuf::from("/h/.factr"));
    }

    #[test]
    fn a_handed_over_message_is_followed_in_the_transcript_until_an_answer_appears() {
        use factr_base::message::{ContentBlock, Role};
        let text = |t: &str| vec![ContentBlock::Text { text: t.into(), cache_control: None }];
        let mut chat = factr_base::session::Session::create(None, None);
        chat.add_message(Role::User, text("Message from 🤖 A (@a): old"));
        chat.add_message(Role::Assistant, text("old answer"));
        let before = chat.messages.len();
        let ask = "Message from 🤖 A (@a): what now?";
        assert_eq!(answer_to(&chat.messages, ask, before), None, "the window has not submitted it yet");
        chat.add_message(Role::User, text(ask));
        assert_eq!(answer_to(&chat.messages, ask, before), Some(None), "submitted, the turn still runs");
        chat.add_message(Role::Assistant, text("  "));
        assert_eq!(answer_to(&chat.messages, ask, before), Some(None), "an empty assistant message is not an answer");
        chat.add_message(Role::Assistant, text("do the thing"));
        assert_eq!(answer_to(&chat.messages, ask, before), Some(Some("do the thing".to_string())));
        // The same text sent earlier does not count as this delivery.
        let mut again = factr_base::session::Session::create(None, None);
        again.add_message(Role::User, text(ask));
        again.add_message(Role::Assistant, text("stale answer"));
        assert_eq!(answer_to(&again.messages, ask, again.messages.len()), None);
    }

    #[test]
    fn only_the_deliver_and_ensure_methods_are_handled() {
        assert!(handles("bot.chat.deliver") && handles("bot.chat.deliver_status") && handles("bot.chat.ensure"));
        assert!(!handles("bot.chat.get") && !handles("bot.chat.claim") && !handles("bot.chat"));
    }

    #[tokio::test]
    async fn deliver_status_joins_a_running_delivery_and_a_finished_one_stays_available() {
        // The registry itself (no engine): a slot starts empty, a waiter sees `pending` after its wait, and the stored
        // result is returned to every later caller.
        let id = format!("test-{:x}", rand::random::<u64>());
        let tx = tokio::sync::watch::channel(None).0;
        DELIVERIES.lock().unwrap().insert(id.clone(), Slot { tx: tx.clone(), at: Instant::now() });
        assert!(await_slot("no-such-id", Duration::from_millis(10)).await.is_none());
        let pending = await_slot(&id, Duration::from_millis(50)).await.unwrap();
        assert_eq!((pending["status"].as_str(), pending["envelope_id"].as_str()), (Some("pending"), Some(id.as_str())));
        tx.send_replace(Some(json!({ "status": "done", "ok": true, "reply": "late" })));
        assert_eq!(await_slot(&id, Duration::from_millis(50)).await.unwrap()["reply"], "late");
        assert_eq!(await_slot(&id, Duration::from_millis(50)).await.unwrap()["reply"], "late", "asking again returns the same result");
        DELIVERIES.lock().unwrap().remove(&id);
    }
}
