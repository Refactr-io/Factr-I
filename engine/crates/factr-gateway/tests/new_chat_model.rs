//! A new chat is pinned to the model the user chose, through the real `session.create` over the
//! WebSocket against a recording stand-in engine: the explicit startup model beats a leftover
//! config.yaml pick, `set_model` is always sent, a saved pick the engine refuses falls back to the
//! startup model with a warning, and a refused explicit pick fails and leaves no chat behind.

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

async fn gateway(home: &std::path::Path, socket: std::path::PathBuf, explicit: bool) -> Ws {
    let token = "new-chat-model-token-0123456789-abcdefgh".to_string();
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
    Ws(ws, 0)
}

fn models_set(seen: &Seen) -> Vec<String> {
    seen.lock().unwrap().iter().filter(|r| r["type"] == "set_model").filter_map(|r| r["model"].as_str()).map(|m| m.rsplit(':').next().unwrap_or(m).to_string()).collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn session_create_pins_the_chosen_model_and_never_a_hidden_one() {
    let home = std::env::temp_dir().join(format!("new-chat-model-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    // SAFETY: this is the only test in this binary.
    unsafe { std::env::set_var("FACTR_CONFIG_HOME", &home) };
    let refuse = Arc::new(Mutex::new(Vec::<String>::new()));
    let (socket, seen) = engine(&home, refuse.clone()).await;

    // Started with --provider openai --model gpt-5.6-luna (explicit); config.yaml holds a leftover pick.
    let mut explicit = gateway(&home, socket.clone(), true).await;
    let (reply, _) = explicit.call("session.create", json!({})).await;
    assert!(reply["result"]["session_id"].is_string(), "{reply}");
    assert_eq!(models_set(&seen), ["gpt-5.6-luna"], "nothing saved: set_model is sent, with the startup model");
    assert_eq!(reply["result"]["info"]["model"], "gpt-5.6-luna");

    std::fs::write(home.join("config.yaml"), "model:\n  default: gpt-6-astra\n  provider: openai\n").unwrap();
    seen.lock().unwrap().clear();
    let (reply, _) = explicit.call("session.create", json!({})).await;
    assert_eq!(models_set(&seen), ["gpt-5.6-luna"], "the explicit model beats a leftover config.yaml pick: {reply}");

    // A caller-named model is used as asked.
    seen.lock().unwrap().clear();
    let (reply, _) = explicit.call("session.create", json!({ "model": "gpt-5.6-terra", "provider": "openai" })).await;
    assert_eq!(models_set(&seen), ["gpt-5.6-terra"]);
    assert_eq!(reply["result"]["info"]["model"], "gpt-5.6-terra");

    // The engine refuses an explicit pick: the call fails and no second model is tried (the half-made chat is discarded
    // through `delete_everywhere`, which the bridge serves from the session files, so it is not visible here).
    refuse.lock().unwrap().push("terra".into());
    seen.lock().unwrap().clear();
    let (reply, _) = explicit.call("session.create", json!({ "model": "gpt-5.6-terra", "provider": "openai" })).await;
    assert!(reply["error"]["message"].as_str().is_some_and(|m| m.contains("Unsupported")), "{reply}");
    assert_eq!(models_set(&seen), ["gpt-5.6-terra"]);

    // Started with nothing explicit: the saved pick is the default; one the engine refuses falls back to the startup model.
    refuse.lock().unwrap().clear();
    std::fs::write(home.join("config.yaml"), "model:\n  default: gpt-5.6-sol\n  provider: openai\n").unwrap();
    let mut following = gateway(&home, socket, false).await;
    seen.lock().unwrap().clear();
    let (reply, _) = following.call("session.create", json!({})).await;
    assert_eq!(models_set(&seen), ["gpt-5.6-sol"], "{reply}");
    refuse.lock().unwrap().push("sol".into());
    seen.lock().unwrap().clear();
    let (reply, events) = following.call("session.create", json!({})).await;
    assert_eq!(models_set(&seen), ["gpt-5.6-sol", "gpt-5.6-luna"]);
    assert_eq!(reply["result"]["info"]["model"], "gpt-5.6-luna", "{reply}");
    assert!(
        events.iter().any(|e| e["params"]["payload"]["text"].as_str().is_some_and(|t| t.contains("gpt-5.6-sol") && t.contains("gpt-5.6-luna"))),
        "a warning event names both models: {events:?}"
    );
    let _ = std::fs::remove_dir_all(&home);
}
