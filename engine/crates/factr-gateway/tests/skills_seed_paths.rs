//! The lazy bundled-skills seed runs only for an interactive chat. With a Factr backend configured
//! and an empty skill store: a headless `POST /api/agent/run` (cron, bots) never spawns the seed,
//! never waits for it, creates no skills directory, and sends the engine the same requests as a
//! gateway with no backend at all; a desktop window's `session.create` over the WebSocket still
//! seeds and waits for it (bounded by 3 s).

// A stand-in engine daemon on a Unix socket and a shell seed script: Unix-only.
#![cfg(unix)]

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use factr_gateway::{Config, Gateway};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_tungstenite::tungstenite::Message;

type Seen = Arc<Mutex<Vec<(Instant, Value)>>>;
const TOKEN: &str = "skills-seed-paths-token-0123456789-abcdefgh";

/// Stand-in engine socket: records every legacy request (with its arrival time) and answers it.
async fn engine(dir: &Path) -> (PathBuf, Seen) {
    let socket = dir.join("d.sock");
    let _ = std::fs::remove_file(&socket);
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let seen: Seen = Default::default();
    let log = seen.clone();
    tokio::spawn(async move {
        use tokio::io::{AsyncBufReadExt, BufReader};
        while let Ok((stream, _)) = listener.accept().await {
            let log = log.clone();
            tokio::spawn(async move {
                let (read, mut write) = stream.into_split();
                let mut lines = BufReader::new(read).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let Ok(req) = serde_json::from_str::<Value>(&line) else { continue };
                    log.lock().unwrap().push((Instant::now(), req.clone()));
                    let reply = match req["type"].as_str() {
                        Some("set_model") => json!({"type":"model_changed","id":req["id"],"model":req["model"],"provider_name":"openai"}),
                        Some("state") => json!({"type":"state","id":req["id"],"session_id":"session_seed_paths","is_processing":false}),
                        _ => json!({"type":"done","id":req["id"]}),
                    };
                    let _ = write.write_all(format!("{reply}\n").as_bytes()).await;
                }
            });
        }
    });
    (socket, seen)
}

/// A `skills seed` stand-in: logs its call, takes `delay` seconds, then writes one skill.
fn seed_script(dir: &Path, delay: &str) -> (PathBuf, PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let (calls, script) = (dir.join("seed.calls"), dir.join("seed.sh"));
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\necho \"$@\" >> '{}'\nsleep {delay}\nmkdir -p \"$FACTR_HOME/skills/demo\"\nprintf -- '---\\nname: demo\\ndescription: d\\n---\\nbody\\n' > \"$FACTR_HOME/skills/demo/SKILL.md\"\necho '{{\"copied\": 1, \"total_bundled\": 1}}'\n",
            calls.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    (calls, script)
}

async fn gateway(dir: &Path, socket: PathBuf, backend: Option<&Path>) -> u16 {
    let gateway = Gateway::bind(Config {
        bind: "127.0.0.1:0".parse().unwrap(),
        token: TOKEN.into(),
        version: "test".into(),
        legacy_socket: socket,
        default_cwd: dir.to_string_lossy().into(),
        allow_non_loopback: false,
        provider: "OpenAI".into(),
        model: "gpt-6-luna".into(),
        reasoning_efforts: Vec::new(),
        profile_model_applies: false,
        home: dir.to_string_lossy().into(),
        complete: None,
        features: backend.map(|b| Arc::new(factr_gateway::features::Features::new(vec![b.to_string_lossy().into()]))),
        learning: None,
    })
    .await
    .expect("gateway binds");
    let port = gateway.local_addr().port();
    tokio::spawn(gateway.serve());
    port
}

/// Start `POST /api/agent/run` (the stand-in never ends the turn, so the answer is not awaited) and
/// give the run 1.5 s to create its session and submit the prompt.
async fn agent_run(port: u16, cwd: &Path) {
    let body = json!({ "prompt": "Reply with OK.", "cwd": cwd, "timeout_s": 2 }).to_string();
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let req = format!(
        "POST /api/agent/run HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-Factr-Session-Token: {TOKEN}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(req.as_bytes()).await.unwrap();
    tokio::spawn(async move {
        let mut buf = Vec::new();
        let _ = stream.read_to_end(&mut buf).await;
    });
    tokio::time::sleep(Duration::from_millis(1500)).await;
}

/// The requests that set up a headless run's session, from its `subscribe` (the new session) to the
/// title, without the bridge's running request ids.
fn run_requests(seen: &Seen) -> Vec<Value> {
    let seen = seen.lock().unwrap();
    let start = seen.iter().position(|(_, r)| r["type"] == "subscribe").expect("the run created a session");
    let mut setup: Vec<Value> = seen[start..].iter().map(|(_, r)| r.clone()).take(5).collect();
    for request in &mut setup {
        request.as_object_mut().unwrap().remove("id");
    }
    setup
}

#[tokio::test(flavor = "multi_thread")]
async fn only_an_interactive_chat_seeds_the_skills_a_headless_run_never_does() {
    let root = std::env::temp_dir().join(format!("skills-seed-paths-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let (home, work) = (root.join("factr"), root.join("work"));
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&work).unwrap();
    // SAFETY: this is the only test in this binary.
    unsafe {
        std::env::set_var("FACTR_HOME", &home);
        std::env::set_var("FACTR_CONFIG_HOME", &home);
    }
    let (calls, script) = seed_script(&root, "1.5");
    let (socket, seen) = engine(&root).await;

    // Reference: the same run on a gateway with no Factr backend (the benchmark shape).
    let plain = gateway(&root, socket.clone(), None).await;
    agent_run(plain, &work).await;
    let reference = run_requests(&seen);
    assert_eq!(reference.last().map(|r| r["type"].clone()), Some(json!("rename_session")), "{reference:?}");
    seen.lock().unwrap().clear();

    // Backend configured, empty skill store: the headless run neither seeds nor waits.
    let port = gateway(&root, socket.clone(), Some(&script)).await;
    let posted = Instant::now();
    agent_run(port, &work).await;
    let first = seen.lock().unwrap().iter().find(|(_, r)| r["type"] == "subscribe").map(|(at, _)| *at).expect("a session was created");
    let _ = (first, posted); // that no seed ran is proven by the two file checks below; a wall-clock bound here fails under CPU load
    assert!(!calls.exists(), "a headless run spawned the skills seed");
    assert!(!home.join("skills").exists(), "a headless run created the skills directory");
    assert_eq!(run_requests(&seen), reference, "the engine sees the same requests as without a backend");

    // A desktop window's new chat on the same gateway: seeds, and waits for it (at most 3 s).
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/api/ws?token={TOKEN}")).await.expect("ws connect");
    let started = Instant::now();
    ws.send(Message::Text(json!({"jsonrpc":"2.0","id":1,"method":"session.create","params":{}}).to_string())).await.unwrap();
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
    let waited = started.elapsed();
    assert!(reply["result"]["session_id"].is_string(), "{reply}");
    assert_eq!(std::fs::read_to_string(&calls).unwrap().lines().count(), 1, "the interactive chat seeded once");
    assert!(home.join("skills/demo/SKILL.md").is_file(), "its first turn sees the seeded skill");
    assert!(waited >= Duration::from_millis(1400) && waited <= Duration::from_millis(8000), "waited for the seed, bounded: {waited:?}");

    let _ = std::fs::remove_dir_all(&root);
}
