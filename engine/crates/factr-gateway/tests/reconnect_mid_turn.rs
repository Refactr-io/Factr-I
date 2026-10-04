//! E3: a turn running when its window's `/api/ws` socket drops is not lost with the socket. The
//! gateway keeps the turn's engine link until the turn ends (the reply is saved), a window that
//! reconnects takes the turn over and sees the rest, and the link is let go once the turn is over.
//!
//! The engine daemon is a scripted stand-in speaking the legacy wire the bridge talks: it streams a
//! slow reply and, like the real engine (crash on disconnect), aborts the turn if its link closes.

// A stand-in engine daemon on a Unix socket: these tests are Unix-only.
#![cfg(unix)]

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use factr_gateway::{Config, Gateway};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpStream, UnixListener};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

/// The scripted reply: this many chunks, one per step, so the turn outlives a dropped socket.
const CHUNKS: usize = 12;
const STEP: Duration = Duration::from_millis(150);
/// A long reply (the fake model's `DRIP:1500`): far more deltas than the replay buffer has
/// entries. It streams without pause, then holds its last chunk until the test lets it go, so the
/// window that dropped stays away for the whole reply.
const LONG_CHUNKS: usize = 1500;
const LONG_PROMPT: &str = "long";
/// Longest wait for anything the test expects (the whole turn is `CHUNKS * STEP`).
const WAIT: Duration = Duration::from_secs(20);

#[derive(Default, Debug, Clone)]
struct Turn {
    completed: bool,
    aborted: bool,
    /// The daemon saw the session's link end.
    link_closed: bool,
    /// Chunks sent so far.
    streamed: usize,
}

type Turns = Arc<Mutex<HashMap<String, Turn>>>;

fn chunk(i: usize) -> String {
    format!("part{i} ")
}

fn full_reply() -> String {
    reply_of(CHUNKS)
}

fn reply_of(chunks: usize) -> String {
    (0..chunks).map(chunk).collect()
}

/// One daemon connection: answers attach and history, and streams a slow reply to a message.
async fn serve_link(stream: tokio::net::UnixStream, turns: Turns, hold: Arc<tokio::sync::Notify>) {
    let (read, write) = stream.into_split();
    let write = Arc::new(tokio::sync::Mutex::new(write));
    let mut lines = BufReader::new(read).lines();
    let mut session: Option<String> = None;
    let closed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let send = |write: Arc<tokio::sync::Mutex<tokio::net::unix::OwnedWriteHalf>>, value: Value| async move {
        write.lock().await.write_all(format!("{value}\n").as_bytes()).await.is_ok()
    };
    while let Ok(Some(line)) = lines.next_line().await {
        let Ok(request) = serde_json::from_str::<Value>(&line) else { continue };
        let id = request["id"].as_u64().unwrap_or(0);
        match request["type"].as_str() {
            Some("subscribe") => session = request["target_session_id"].as_str().map(str::to_string),
            Some("state") => {
                let sid = session.clone().unwrap_or_default();
                send(write.clone(), json!({"type": "state", "id": id, "session_id": sid})).await;
            }
            Some("get_history") => {
                let sid = session.clone().unwrap_or_default();
                send(write.clone(), json!({"type": "history", "id": id, "session_id": sid, "messages": []})).await;
            }
            Some("message") => {
                let sid = session.clone().unwrap_or_default();
                turns.lock().unwrap().insert(sid.clone(), Turn::default());
                send(write.clone(), json!({"type": "ack", "id": id})).await;
                let long = request["content"] == LONG_PROMPT;
                let (chunks, step) = if long { (LONG_CHUNKS, Duration::ZERO) } else { (CHUNKS, STEP) };
                let (write, turns, closed, hold) = (write.clone(), turns.clone(), closed.clone(), hold.clone());
                tokio::spawn(async move {
                    for i in 0..chunks {
                        tokio::time::sleep(step).await;
                        if long && i == chunks - 1 {
                            hold.notified().await;
                        }
                        // The engine aborts a turn whose client link went away.
                        if closed.load(std::sync::atomic::Ordering::SeqCst)
                            || !send(write.clone(), json!({"type": "text_delta", "text": chunk(i)})).await
                        {
                            turns.lock().unwrap().get_mut(&sid).unwrap().aborted = true;
                            return;
                        }
                        turns.lock().unwrap().get_mut(&sid).unwrap().streamed = i + 1;
                    }
                    send(write, json!({"type": "done", "id": id})).await;
                    turns.lock().unwrap().get_mut(&sid).unwrap().completed = true;
                });
            }
            // Anything else (a rename, the model catalog) is simply acknowledged.
            _ => {
                send(write.clone(), json!({"type": "ack", "id": id})).await;
            }
        }
    }
    closed.store(true, std::sync::atomic::Ordering::SeqCst);
    if let Some(sid) = session {
        turns.lock().unwrap().entry(sid).or_default().link_closed = true;
    }
}

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

struct Window {
    ws: Ws,
    next: u64,
}

impl Window {
    async fn open(port: u16, token: &str) -> Self {
        let url = format!("ws://127.0.0.1:{port}/api/ws?token={token}");
        let (ws, _) = tokio_tungstenite::connect_async(url).await.expect("window connects");
        Self { ws, next: 1 }
    }

    async fn send(&mut self, method: &str, params: Value) -> u64 {
        let id = self.next;
        self.next += 1;
        let frame = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        self.ws.send(Message::Text(frame.to_string())).await.unwrap();
        id
    }

    /// The next frame `pick` accepts, within `WAIT`.
    async fn until(&mut self, mut pick: impl FnMut(&Value) -> bool) -> Value {
        tokio::time::timeout(WAIT, async {
            loop {
                let Some(Ok(Message::Text(text))) = self.ws.next().await else { panic!("socket ended") };
                let frame: Value = serde_json::from_str(&text).unwrap();
                if pick(&frame) {
                    return frame;
                }
            }
        })
        .await
        .expect("frame arrives in time")
    }

    async fn call(&mut self, method: &str, params: Value) -> Value {
        let id = self.send(method, params).await;
        let reply = self.until(|f| f["id"] == id).await;
        assert!(reply["error"].is_null(), "{method}: {reply}");
        reply["result"].clone()
    }
}

fn event_of<'a>(frame: &'a Value, ty: &str, sid: &str) -> Option<&'a Value> {
    (frame["method"] == "event" && frame["params"]["type"] == ty && frame["params"]["session_id"] == sid).then(|| &frame["params"])
}

async fn wait_for(turns: &Turns, sid: &str, done: impl Fn(&Turn) -> bool) -> Turn {
    tokio::time::timeout(WAIT, async {
        loop {
            if let Some(turn) = turns.lock().unwrap().get(sid).filter(|t| done(t)) {
                return turn.clone();
            }
            tokio::time::sleep(STEP).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{sid}: {:?}", turns.lock().unwrap().get(sid)))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_turn_survives_its_window_socket_dropping_and_a_reconnecting_window_sees_it_end() {
    let root = std::env::temp_dir().join(format!("factr-e3-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    for dir in ["home", "factr", "factr"] {
        std::fs::create_dir_all(root.join(dir)).unwrap();
    }
    // SAFETY: set before the gateway starts any thread that reads them; this file holds one test.
    unsafe {
        std::env::set_var("FACTR_HOME", root.join("factr"));
        std::env::set_var("FACTR_CONFIG_HOME", root.join("factr"));
    }
    // Unix socket paths are short: keep it directly under /tmp.
    let socket = PathBuf::from(format!("/tmp/factr-e3-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&socket);
    let turns: Turns = Arc::default();
    let listener = UnixListener::bind(&socket).unwrap();
    let daemon_turns = turns.clone();
    let hold = Arc::new(tokio::sync::Notify::new());
    let daemon_hold = hold.clone();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            tokio::spawn(serve_link(stream, daemon_turns.clone(), daemon_hold.clone()));
        }
    });

    let token = "e3-reconnect-mid-turn-token-0123456789".to_string();
    let home = root.join("home");
    let gateway = Gateway::bind(Config {
        bind: "127.0.0.1:0".parse().unwrap(),
        token: token.clone(),
        version: "test".into(),
        legacy_socket: socket.clone(),
        default_cwd: home.to_string_lossy().into(),
        allow_non_loopback: false,
        provider: "p".into(),
        model: "m".into(),
        reasoning_efforts: Vec::new(),
        profile_model_applies: true,
        home: home.to_string_lossy().into(),
        complete: None,
        features: None,
        learning: None,
    })
    .await
    .unwrap();
    let port = gateway.local_addr().port();
    let server = tokio::spawn(gateway.serve());

    // One window runs three chats, then its socket drops two steps into the slow turns and one
    // delta into the long one.
    let (watched, left, long) = ("e3-watched", "e3-left", "e3-long");
    let mut first = Window::open(port, &token).await;
    first.call("session.resume", json!({"session_id": long})).await;
    first.call("prompt.submit", json!({"session_id": long, "text": LONG_PROMPT})).await;
    let long_seen = first.until(|f| event_of(f, "message.delta", long).is_some()).await["params"]["seq"].as_u64().unwrap();
    let mut seen = 0;
    for sid in [watched, left] {
        first.call("session.resume", json!({"session_id": sid})).await;
        first.call("prompt.submit", json!({"session_id": sid, "text": "go"})).await;
    }
    for _ in 0..2 {
        let delta = first.until(|f| event_of(f, "message.delta", watched).is_some()).await;
        seen = delta["params"]["seq"].as_u64().unwrap();
    }
    drop(first);

    // A window reconnects for `watched` mid-turn: what it missed is replayed, the rest arrives live.
    let mut second = Window::open(port, &token).await;
    let replay = second.call("session.events.since", json!({"session_id": watched, "last_seen": seen})).await;
    let events = replay["events"].as_array().unwrap().clone();
    assert!(events.iter().all(|e| e["type"] != "message.complete"), "the turn is still running when the window is back");
    let mut text: String = events
        .iter()
        .filter(|e| e["type"] == "message.delta")
        .map(|e| e["payload"]["text"].as_str().unwrap_or_default().to_string())
        .collect();
    let complete = loop {
        let frame = second.until(|f| event_of(f, "message.delta", watched).is_some() || event_of(f, "message.complete", watched).is_some()).await;
        if frame["params"]["type"] == "message.complete" {
            break frame["params"].clone();
        }
        text.push_str(frame["params"]["payload"]["text"].as_str().unwrap_or_default());
    };
    assert_eq!(complete["payload"]["text"], full_reply(), "the reconnected window gets the whole reply");
    assert_eq!(text, full_reply()[chunk(0).len() + chunk(1).len()..], "every delta after the drop reaches the new window once, in order");
    let turn = wait_for(&turns, watched, |t| t.completed || t.aborted).await;
    assert!(turn.completed && !turn.aborted, "{turn:?}");
    assert!(!turns.lock().unwrap()[watched].link_closed, "the window that took the turn over holds the link");

    // The long reply streamed all but its last chunk with no window: a window back now replays
    // every delta it missed, once and in order, and nothing was dropped to make room.
    wait_for(&turns, long, |t| t.streamed == LONG_CHUNKS - 1).await;
    let replay = second.call("session.events.since", json!({"session_id": long, "last_seen": long_seen})).await;
    assert_eq!(replay["truncated"], false, "the whole long reply is still buffered");
    let events = replay["events"].as_array().unwrap();
    let deltas: Vec<&Value> = events.iter().filter(|e| e["type"] == "message.delta").collect();
    assert_eq!(deltas.len(), LONG_CHUNKS - 2, "every missed delta, each as its own event");
    assert!(deltas.iter().zip(long_seen + 1..).all(|(e, seq)| e["seq"] == seq), "in order with no gap");
    let mut text: String = deltas.iter().map(|e| e["payload"]["text"].as_str().unwrap_or_default()).collect();
    hold.notify_one();
    let complete = loop {
        let frame = second.until(|f| event_of(f, "message.delta", long).is_some() || event_of(f, "message.complete", long).is_some()).await;
        if frame["params"]["type"] == "message.complete" {
            break frame["params"].clone();
        }
        text.push_str(frame["params"]["payload"]["text"].as_str().unwrap_or_default());
    };
    assert_eq!(complete["payload"]["text"], reply_of(LONG_CHUNKS));
    assert_eq!(text, reply_of(LONG_CHUNKS)[chunk(0).len()..], "the long reply after the drop, once, in order");

    // Nobody came back for `left`: its turn still completed, then its link was let go.
    let turn = wait_for(&turns, left, |t| t.link_closed).await;
    assert!(turn.completed && !turn.aborted, "the link was let go only after the turn ran to its end: {turn:?}");
    // A window that comes back after that turn ended still learns it is over, with its reply.
    let replay = second.call("session.events.since", json!({"session_id": left, "last_seen": 1})).await;
    let done = replay["events"].as_array().unwrap().iter().find(|e| e["type"] == "message.complete").cloned().expect("the end of the turn is replayed");
    assert_eq!(done["payload"]["text"], full_reply());

    // Closing the window that holds `watched` (idle now) lets its link go.
    drop(second);
    wait_for(&turns, watched, |t| t.link_closed).await;

    server.abort();
    let _ = std::fs::remove_file(&socket);
    let _ = std::fs::remove_dir_all(&root);
}
