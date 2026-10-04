//! R5-2, engine side: every `config.set reasoning` shape the desktop sends (with a session, without
//! one, scope unset or global) saves the user's last effort pick as `agent.reasoning_effort`; only an
//! explicit `scope: "session"` does not. The saved pick is what `session.create` applies and reports,
//! what `config.get reasoning` and `GET /api/model/info` answer, and what a restart (a new gateway on
//! the same home) starts new chats on. Real WebSocket against a recording stand-in engine.

// A stand-in engine daemon on a Unix socket: these tests are Unix-only.
#![cfg(unix)]

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use factr_gateway::{Config, Gateway};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;

type Seen = Arc<Mutex<Vec<Value>>>;

/// Stand-in engine socket: records every legacy request and refuses `set_model` for models containing `refuse`.
async fn engine(home: &std::path::Path, refuse: Arc<Mutex<Vec<String>>>) -> (std::path::PathBuf, Seen) {
    let socket = home.join("d.sock");
    let _ = std::fs::remove_file(&socket);
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let seen: Seen = Default::default();
    let log = seen.clone();
    tokio::spawn(async move {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        while let Ok((stream, _)) = listener.accept().await {
            let (log, refuse) = (log.clone(), refuse.clone());
            tokio::spawn(async move {
                let (read, mut write) = stream.into_split();
                let mut lines = BufReader::new(read).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let Ok(req) = serde_json::from_str::<Value>(&line) else { continue };
                    log.lock().unwrap().push(req.clone());
                    let reply = match req["type"].as_str() {
                        Some("set_model") => {
                            let model = req["model"].as_str().unwrap_or_default().to_string();
                            let bad = refuse.lock().unwrap().iter().any(|r| model.contains(r.as_str()));
                            if bad {
                                json!({"type":"model_changed","id":req["id"],"model":model,"error":format!("Unsupported model {model}")})
                            } else {
                                json!({"type":"model_changed","id":req["id"],"model":model,"provider_name":"openai"})
                            }
                        }
                        Some("set_reasoning_effort") => json!({"type":"reasoning_effort_changed","id":req["id"],"effort":req["effort"]}),
                        Some("state") => json!({"type":"state","id":req["id"],"session_id":"session_new_chat","is_processing":false}),
                        _ => json!({"type":"done","id":req["id"]}),
                    };
                    let _ = write.write_all(format!("{reply}\n").as_bytes()).await;
                }
            });
        }
    });
    (socket, seen)
}

struct Ws(tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>, u64);

impl Ws {
    /// The reply and the events that arrived before it.
    async fn call(&mut self, method: &str, params: Value) -> (Value, Vec<Value>) {
        self.1 += 1;
        let id = self.1;
        self.0.send(Message::Text(json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}).to_string())).await.unwrap();
        let mut events = Vec::new();
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let Some(Ok(Message::Text(t))) = self.0.next().await else { continue };
                let frame: Value = serde_json::from_str(&t).unwrap();
                if frame["id"] == id {
                    return (frame, events);
                }
                events.push(frame);
            }
        })
        .await
        .expect("a reply")
    }
}

async fn gateway(home: &std::path::Path, socket: std::path::PathBuf, explicit: bool) -> (Ws, u16, String) {
    let token = "effort-persist-token-0123456789-abcdefghijk".to_string();
    let gateway = Gateway::bind(Config {
        bind: "127.0.0.1:0".parse().unwrap(),
        token: token.clone(),
        version: "test".into(),
        legacy_socket: socket,
        default_cwd: home.to_string_lossy().into(),
        allow_non_loopback: false,
        provider: "openai".into(),
        model: "gpt-5.6-luna".into(),
        reasoning_efforts: Vec::new(),
        profile_model_applies: !explicit,
        home: home.to_string_lossy().into(),
        complete: None,
        features: None,
        learning: None,
    })
    .await
    .expect("gateway binds");
    let port = gateway.local_addr().port();
    tokio::spawn(gateway.serve());
    let (ws, _) = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/api/ws?token={token}")).await.expect("ws connect");
    (Ws(ws, 0), port, token)
}


fn saved(home: &std::path::Path) -> Option<String> {
    let raw = std::fs::read_to_string(home.join("config.yaml")).ok()?;
    raw.lines().find_map(|l| l.trim().strip_prefix("reasoning_effort:").map(|v| v.trim().to_string()))
}

async fn model_info(port: u16, token: &str) -> Value {
    let mut http = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    http.write_all(format!("GET /api/model/info HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-Factr-Session-Token: {token}\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
    let mut raw = String::new();
    http.read_to_string(&mut raw).await.unwrap();
    assert!(raw.starts_with("HTTP/1.1 200"), "{raw}");
    serde_json::from_str(raw.split("\r\n\r\n").nth(1).unwrap()).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn every_effort_pick_the_desktop_sends_is_saved_unless_it_is_session_scoped() {
    let home = std::env::temp_dir().join(format!("effort-persist-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    // SAFETY: this is the only test in this binary.
    unsafe { std::env::set_var("FACTR_CONFIG_HOME", &home) };
    let (socket, seen) = engine(&home, Default::default()).await;
    let (mut ws, port, token) = gateway(&home, socket.clone(), true).await;

    assert_eq!(saved(&home), None);
    assert!(model_info(port, &token).await.get("reasoning_effort").is_none(), "nothing picked yet");
    let (created, _) = ws.call("session.create", json!({})).await;
    let session = created["result"]["session_id"].as_str().expect("a session").to_string();

    // The composer slider on a live chat: the session, no scope (what the desktop sends).
    let (reply, _) = ws.call("config.set", json!({ "key": "reasoning", "session_id": session, "value": "high" })).await;
    assert_eq!(reply["result"]["value"], "high", "{reply}");
    assert!(seen.lock().unwrap().iter().any(|r| r["type"] == "set_reasoning_effort"), "the live session was switched");
    assert_eq!(saved(&home).as_deref(), Some("high"), "a session pick with no scope is the new default");

    // An explicit session scope is a one-off: the saved pick stays.
    let (reply, _) = ws.call("config.set", json!({ "key": "reasoning", "session_id": session, "value": "low", "scope": "session" })).await;
    assert_eq!(reply["result"]["value"], "low", "{reply}");
    assert_eq!(saved(&home).as_deref(), Some("high"));

    // A draft chat (no session yet): the pick still has to survive a restart.
    let (reply, _) = ws.call("config.set", json!({ "key": "reasoning", "value": "medium" })).await;
    assert_eq!(reply["result"]["value"], "medium", "{reply}");
    assert_eq!(saved(&home).as_deref(), Some("medium"));

    // A session pick marked global, then a plain session pick again.
    ws.call("config.set", json!({ "key": "reasoning", "session_id": session, "value": "low", "scope": "global" })).await;
    assert_eq!(saved(&home).as_deref(), Some("low"));
    ws.call("config.set", json!({ "key": "reasoning", "session_id": session, "value": "high" })).await;
    assert_eq!(saved(&home).as_deref(), Some("high"));

    // Everything that reads the pick back agrees, now and after a restart (a new gateway on the same home).
    for (round, (mut ws, port, token)) in [(0, (ws, port, token)), (1, gateway(&home, socket, true).await)] {
        let (reply, _) = ws.call("config.get", json!({ "key": "reasoning" })).await;
        assert_eq!(reply["result"]["value"], "high", "round {round}: {reply}");
        assert_eq!(model_info(port, &token).await["reasoning_effort"], "high", "round {round}");
        seen.lock().unwrap().clear();
        let (reply, _) = ws.call("session.create", json!({})).await;
        assert_eq!(reply["result"]["info"]["reasoning_effort"], "high", "round {round}: a new chat starts on the saved pick: {reply}");
        assert!(
            seen.lock().unwrap().iter().any(|r| r["type"] == "set_reasoning_effort" && r["effort"] == "high"),
            "round {round}: the new chat's engine session is set to it"
        );
    }
    let _ = std::fs::remove_dir_all(&home);
}
