//! `projects.list` reads `<FACTR_CONFIG_HOME>/projects.db` (the Python backend owns that file). Whatever state the file is
//! in (never created, empty, garbage, a directory, a store with a live WAL writer) the gateway starts and serves exactly
//! as before, `projects.list` answers (an empty list when nothing is readable), and the store is never written: not its
//! bytes, not a sibling file. A real WebSocket against a gateway with no engine behind it.

#![cfg(unix)]

use factr_gateway::{Config, Gateway};
use futures_util::{SinkExt, StreamExt};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::path::Path;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_tungstenite::tungstenite::Message;

const TOKEN: &str = "projects-store-token-0123456789-abcdefghijk";
const FOLDER: &str = "/Users/me/Library/Mobile Documents/com~apple~CloudDocs/Refactr";
const SCHEMA: &str = "CREATE TABLE projects (id TEXT PRIMARY KEY, slug TEXT NOT NULL UNIQUE, name TEXT NOT NULL, description TEXT, icon TEXT,
        color TEXT, board_slug TEXT, primary_path TEXT, created_at INTEGER NOT NULL, archived INTEGER NOT NULL DEFAULT 0);
    CREATE TABLE project_folders (project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE, path TEXT NOT NULL, label TEXT,
        is_primary INTEGER NOT NULL DEFAULT 0, added_at INTEGER NOT NULL, PRIMARY KEY (project_id, path));
    CREATE TABLE project_meta (key TEXT PRIMARY KEY, value TEXT);";

fn seed(conn: &Connection) {
    conn.execute_batch(SCHEMA).unwrap();
    for (id, slug, name, path, at) in [("p_297a2943", "refactr", "Refactr", FOLDER, 1791729726), ("p_e04cc579", "second", "Second", "/Users/me/second", 1791729800)] {
        conn.execute("INSERT INTO projects (id, slug, name, primary_path, created_at) VALUES (?1, ?2, ?3, ?4, ?5)", rusqlite::params![id, slug, name, path, at]).unwrap();
        conn.execute("INSERT INTO project_folders VALUES (?1, ?2, NULL, 1, ?3)", rusqlite::params![id, path, at]).unwrap();
    }
    conn.execute("INSERT INTO project_meta VALUES ('active_id', 'p_297a2943')", []).unwrap();
}

/// The store's files (`projects.db*`) with their bytes, so any write to them shows.
fn snapshot(home: &Path) -> Vec<(String, u64, Vec<u8>)> {
    let mut rows: Vec<_> = std::fs::read_dir(home)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| {
            let p = e.path();
            let bytes = if p.is_file() { std::fs::read(&p).unwrap_or_default() } else { Vec::new() };
            (e.file_name().to_string_lossy().into_owned(), bytes.len() as u64, bytes)
        })
        .filter(|(name, ..)| name.starts_with("projects.db"))
        .collect();
    rows.sort();
    rows
}

/// A stand-in engine socket (the WebSocket needs an engine link to open); it answers every legacy request `done`.
async fn engine(home: &Path) -> std::path::PathBuf {
    let socket = home.join("d.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    tokio::spawn(async move {
        use tokio::io::{AsyncBufReadExt, BufReader};
        while let Ok((stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let (read, mut write) = stream.into_split();
                let mut lines = BufReader::new(read).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let Ok(req) = serde_json::from_str::<Value>(&line) else { continue };
                    let reply = json!({"type":"done","id":req["id"]});
                    let _ = write.write_all(format!("{reply}\n").as_bytes()).await;
                }
            });
        }
    });
    socket
}

async fn get_status(port: u16) -> String {
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    stream.write_all(format!("GET /api/status HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-Factr-Session-Token: {TOKEN}\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
    let mut raw = String::new();
    stream.read_to_string(&mut raw).await.unwrap();
    raw
}

async fn list(port: u16) -> Value {
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/api/ws?token={TOKEN}")).await.expect("ws connect");
    ws.send(Message::Text(json!({"jsonrpc":"2.0","id":1,"method":"projects.list","params":{}}).to_string())).await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let Some(Ok(Message::Text(t))) = ws.next().await else { continue };
            let frame: Value = serde_json::from_str(&t).unwrap();
            if frame["id"] == 1 {
                return frame;
            }
        }
    })
    .await
    .expect("a reply")
}

#[tokio::test(flavor = "multi_thread")]
async fn the_gateway_starts_and_lists_projects_whatever_state_projects_db_is_in() {
    let root = std::env::temp_dir().join(format!("factr-projects-store-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let mut writers = Vec::new();
    for kind in ["absent", "empty", "garbage", "directory", "valid", "wal-live"] {
        let home = root.join(kind);
        std::fs::create_dir_all(&home).unwrap();
        let db = home.join("projects.db");
        match kind {
            "absent" => {}
            "empty" => std::fs::write(&db, b"").unwrap(),
            "garbage" => std::fs::write(&db, vec![0xA5u8; 4096]).unwrap(),
            "directory" => std::fs::create_dir(&db).unwrap(),
            "valid" => {
                let conn = Connection::open(&db).unwrap();
                seed(&conn);
            }
            _ => {
                // The backend keeps its connection open: the rows sit in the -wal file.
                let conn = Connection::open(&db).unwrap();
                conn.pragma_update(None, "journal_mode", "WAL").unwrap();
                conn.pragma_update(None, "wal_autocheckpoint", 0).unwrap();
                seed(&conn);
                writers.push(conn);
            }
        }
        // SAFETY: this is the only test in this binary.
        unsafe { std::env::set_var("FACTR_CONFIG_HOME", &home) };
        let gateway = Gateway::bind(Config {
            bind: "127.0.0.1:0".parse().unwrap(),
            token: TOKEN.into(),
            version: "test".into(),
            legacy_socket: engine(&home).await,
            default_cwd: home.to_string_lossy().into(),
            allow_non_loopback: false,
            provider: "openai".into(),
            model: "gpt-6-luna".into(),
            reasoning_efforts: Vec::new(),
            profile_model_applies: false,
            home: home.to_string_lossy().into(),
            complete: None,
            features: None,
            learning: None,
        })
        .await
        .unwrap_or_else(|e| panic!("{kind}: the gateway must bind: {e}"));
        let port = gateway.local_addr().port();
        let task = tokio::spawn(gateway.serve());

        // Startup is untouched: /api/status answers.
        assert!(get_status(port).await.starts_with("HTTP/1.1 200"), "{kind}: status");
        let before = snapshot(&home);

        let reply = list(port).await;
        assert!(reply["error"].is_null(), "{kind}: projects.list must not fail: {reply}");
        let projects = reply["result"]["projects"].as_array().unwrap_or_else(|| panic!("{kind}: {reply}"));
        match kind {
            "valid" | "wal-live" => {
                let names: Vec<&str> = projects.iter().map(|p| p["name"].as_str().unwrap()).collect();
                assert_eq!(names, ["Refactr", "Second"], "{kind}");
                assert_eq!(projects[0]["folders"][0]["path"], FOLDER, "{kind}: a folder path with spaces survives");
                assert_eq!(reply["result"]["active_id"], "p_297a2943", "{kind}");
            }
            _ => {
                assert!(projects.is_empty() && reply["result"]["active_id"].is_null(), "{kind}: {reply}");
            }
        }
        // Read-only: no file in the config home changed or appeared (a read-only open of a WAL store may add -shm; it
        // may not touch the main file or the log).
        let after = snapshot(&home);
        let strip = |v: &[(String, u64, Vec<u8>)]| v.iter().filter(|r| r.0 != "projects.db-shm").cloned().collect::<Vec<_>>();
        assert!(strip(&before) == strip(&after), "{kind}: projects.db and its siblings are untouched");
        // Still serving after the call.
        assert!(get_status(port).await.starts_with("HTTP/1.1 200"), "{kind}: status after");
        task.abort();
    }
    drop(writers);
    let _ = std::fs::remove_dir_all(root);
}
