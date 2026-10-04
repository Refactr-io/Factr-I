//! Two routes the desktop reads at the start of a chat and on the Cron screen, over a real gateway:
//! `session.create` reports the engine's configured default effort when no pick is saved (the effort
//! pill must match what requests carry), and `GET /api/cron/jobs/{id}/runs` is answered by the engine
//! from its ledger (not proxied to a backend that knows no engine runs).

#![cfg(unix)]

use factr_gateway::{Config, Gateway};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_tungstenite::tungstenite::Message;

const TOKEN: &str = "default-effort-token-0123456789-abcdefghijk";

async fn stand_in_engine(home: &std::path::Path) -> std::path::PathBuf {
    let socket = home.join("d.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                use tokio::io::{AsyncBufReadExt, BufReader};
                let (read, mut write) = stream.into_split();
                let mut lines = BufReader::new(read).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let Ok(req) = serde_json::from_str::<Value>(&line) else { continue };
                    let reply = match req["type"].as_str() {
                        Some("set_model") => json!({"type":"model_changed","id":req["id"],"model":req["model"],"provider_name":"OpenAI"}),
                        Some("set_reasoning_effort") => json!({"type":"reasoning_effort_changed","id":req["id"],"effort":req["effort"]}),
                        Some("state") => json!({"type":"state","id":req["id"],"session_id":"session_new_chat","is_processing":false}),
                        _ => json!({"type":"done","id":req["id"]}),
                    };
                    let _ = write.write_all(format!("{reply}\n").as_bytes()).await;
                }
            });
        }
    });
    socket
}

async fn http_get(port: u16, path: &str) -> (u16, Value) {
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let req = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-Factr-Session-Token: {TOKEN}\r\nConnection: close\r\n\r\n");
    stream.write_all(req.as_bytes()).await.unwrap();
    let mut raw = String::new();
    tokio::time::timeout(Duration::from_secs(5), stream.read_to_string(&mut raw)).await.expect("read timeout").unwrap();
    let status = raw.lines().next().and_then(|l| l.split_whitespace().nth(1)).and_then(|s| s.parse().ok()).unwrap_or(0);
    (status, raw.split("\r\n\r\n").nth(1).and_then(|b| serde_json::from_str(b).ok()).unwrap_or(Value::Null))
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_chat_reports_the_configured_effort_and_the_cron_run_list_is_served_by_the_engine() {
    let home = std::env::temp_dir().join(format!("default-effort-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(home.join("cron")).unwrap();
    // SAFETY: this is the only test in this binary.
    unsafe { std::env::set_var("FACTR_CONFIG_HOME", &home) };
    // A job whose run died before it had output: only the ledger knows it.
    std::fs::write(home.join("cron").join("jobs.json"), r#"{"jobs":[{"id":"abc123def456","name":"morning","prompt":"say hi"}]}"#).unwrap();
    let db = rusqlite::Connection::open(home.join("cron").join("executions.db")).unwrap();
    db.execute_batch(
        "CREATE TABLE executions (id INTEGER PRIMARY KEY, job_id TEXT NOT NULL, status TEXT NOT NULL, error TEXT, claimed_at TEXT NOT NULL);
         INSERT INTO executions (job_id, status, claimed_at) VALUES ('abc123def456', 'completed', '2026-10-04T08:12:33+00:00');",
    )
    .unwrap();
    drop(db);

    let socket = stand_in_engine(&home).await;
    let gateway = Gateway::bind(Config {
        bind: "127.0.0.1:0".parse().unwrap(),
        token: TOKEN.into(),
        version: "test".into(),
        legacy_socket: socket,
        default_cwd: home.to_string_lossy().into(),
        allow_non_loopback: false,
        provider: "openai".into(),
        model: "gpt-5.6-luna".into(),
        reasoning_efforts: Vec::new(),
        profile_model_applies: true,
        home: home.to_string_lossy().into(),
        complete: None,
        features: None,
        learning: None,
    })
    .await
    .expect("gateway binds");
    let port = gateway.local_addr().port();
    tokio::spawn(gateway.serve());

    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/api/ws?token={TOKEN}")).await.expect("ws connect");
    ws.send(Message::Text(json!({"jsonrpc":"2.0","id":1,"method":"session.create","params":{"model":"gpt-5.6-terra"}}).to_string())).await.unwrap();
    let reply = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let Some(Ok(Message::Text(t))) = ws.next().await else { continue };
            let frame: Value = serde_json::from_str(&t).unwrap();
            if frame["id"] == 1 {
                return frame;
            }
        }
    })
    .await
    .expect("a reply");
    assert_eq!(reply["result"]["info"]["provider"].as_str().map(str::to_lowercase).as_deref(), Some("openai"), "{reply}");
    assert_eq!(reply["result"]["info"]["reasoning_effort"], "low", "the pill shows the effort requests carry: {reply}");

    let (status, runs) = http_get(port, "/api/cron/jobs/abc123def456/runs").await;
    assert_eq!(status, 200, "{runs}");
    let rows = runs["runs"].as_array().expect("a runs list");
    assert_eq!(rows.len(), 1, "{runs}");
    assert_eq!((rows[0]["status"].as_str(), rows[0]["source"].as_str()), (Some("completed"), Some("cron")));
    let _ = std::fs::remove_dir_all(&home);
}
