//! D7 regression: many `PUT /api/config` writes for different keys, back to back and at once, all land in
//! config.yaml and read back, the way a Settings screen flipped switch by switch (and a "flip all" pass) saves.

use factr_gateway::{Config, Gateway};
use std::path::PathBuf;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const TOKEN: &str = "test-token-config-stress-32chars-minimum-ok";

async fn http(port: u16, method: &str, path: &str, body: &str) -> (u16, serde_json::Value) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {TOKEN}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(req.as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    tokio::time::timeout(std::time::Duration::from_secs(20), stream.read_to_end(&mut buf)).await.expect("read timeout").unwrap();
    let text = String::from_utf8_lossy(&buf).into_owned();
    let status = text.lines().next().and_then(|l| l.split_whitespace().nth(1)).and_then(|s| s.parse().ok()).unwrap_or(0);
    let json = text.split("\r\n\r\n").nth(1).and_then(|b| serde_json::from_str(b).ok()).unwrap_or(serde_json::Value::Null);
    (status, json)
}

/// Settings keys of different sections, each with the value a flip writes.
fn flips() -> Vec<(String, serde_json::Value)> {
    let sections = ["display", "memory", "browser", "voice", "workspace", "safety", "gateways", "appearance", "notifications"];
    let mut out = Vec::new();
    for (s, section) in sections.iter().enumerate() {
        for k in 0..5 {
            out.push((format!("{section}.flag_{k}"), serde_json::json!((s + k) % 2 == 0)));
        }
    }
    out.push(("approvals.mode".into(), serde_json::json!("off")));
    out.push(("compression.threshold".into(), serde_json::json!(0.65)));
    out.push(("compression.enabled".into(), serde_json::json!(false)));
    out.push(("memory.memory_enabled".into(), serde_json::json!(false)));
    out.push(("voice.client_direct".into(), serde_json::json!(true)));
    out
}

fn nest(path: &str, value: serde_json::Value) -> serde_json::Value {
    path.rsplit('.').fold(value, |inner, key| serde_json::json!({ key: inner }))
}

/// One gateway on a fresh FACTR_CONFIG_HOME for the whole binary, on its own runtime thread (the tests share it and
/// use disjoint keys, so each test's own runtime can come and go).
async fn gateway() -> (u16, PathBuf) {
    static UP: std::sync::OnceLock<(u16, PathBuf)> = std::sync::OnceLock::new();
    UP.get_or_init(|| {
        let home: PathBuf = std::env::temp_dir().join(format!("factr-config-stress-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        // SAFETY: set once, before the gateway starts; no test changes the environment afterwards.
        unsafe {
            std::env::set_var("FACTR_CONFIG_HOME", &home);
            // The defaults the desktop app bundles (Factr's DEFAULT_CONFIG), as the packaged app points the gateway at.
            std::env::set_var("FACTR_BACKEND_DEFAULTS", concat!(env!("CARGO_MANIFEST_DIR"), "/tests/factr_defaults.json"));
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let home_for_gateway = home.clone();
        std::thread::spawn(move || {
            tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap().block_on(async move {
                let config = Config {
                    bind: "127.0.0.1:0".parse().unwrap(), token: TOKEN.into(), version: "test".into(), legacy_socket: PathBuf::from("/dev/null"),
                    default_cwd: home_for_gateway.to_string_lossy().into(), allow_non_loopback: false, provider: "ollama".into(), model: "local".into(),
                    reasoning_efforts: Vec::new(), profile_model_applies: true, home: home_for_gateway.to_string_lossy().into(), complete: None,
                    features: None, learning: None,
                };
                let gateway = Gateway::bind(config).await.unwrap();
                tx.send(gateway.local_addr().port()).unwrap();
                let _ = gateway.serve().await;
            })
        });
        (rx.recv().unwrap(), home)
    })
    .clone()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn forty_sequential_and_ten_concurrent_config_puts_all_land() {
    let (port, home) = gateway().await;
    let all = flips();
    let (sequential, concurrent) = all.split_at(40);
    for (key, value) in sequential {
        let (status, reply) = http(port, "PUT", "/api/config", &serde_json::json!({ "config": nest(key, value.clone()) }).to_string()).await;
        assert_eq!((status, reply), (200, serde_json::json!({"ok": true})), "{key}");
        let (_, got) = http(port, "GET", "/api/config?include_defaults=false", "").await;
        let mut node = &got;
        for part in key.split('.') {
            node = &node[part];
        }
        assert_eq!(node, value, "{key} reads back right after its save");
    }
    let tasks: Vec<_> = concurrent
        .iter()
        .cloned()
        .map(|(key, value)| {
            tokio::spawn(async move {
                let (status, _) = http(port, "PUT", "/api/config", &serde_json::json!({ "config": nest(&key, value) }).to_string()).await;
                (key, status)
            })
        })
        .collect();
    for task in tasks {
        let (key, status) = task.await.unwrap();
        assert_eq!(status, 200, "{key}");
    }
    let (_, got) = http(port, "GET", "/api/config?include_defaults=false", "").await;
    for (key, value) in &all {
        let mut node = &got;
        for part in key.split('.') {
            node = &node[part];
        }
        assert_eq!(node, value, "{key} lost");
    }
    let yaml: serde_yaml::Value = serde_yaml::from_str(&std::fs::read_to_string(home.join("config.yaml")).unwrap()).unwrap();
    for (key, value) in &all {
        let mut node = &yaml;
        for part in key.split('.') {
            node = &node[part];
        }
        assert_eq!(serde_json::to_value(node).unwrap(), *value, "{key} missing from config.yaml");
    }
}

fn leaves(prefix: &str, node: &serde_json::Value, out: &mut Vec<(String, serde_json::Value)>) {
    match node {
        serde_json::Value::Object(map) if !map.is_empty() => {
            for (k, v) in map {
                leaves(&if prefix.is_empty() { k.clone() } else { format!("{prefix}.{k}") }, v, out);
            }
        }
        _ => out.push((prefix.to_string(), node.clone())),
    }
}

/// A Settings switch can flip any key the desktop's config view shows; none of them may be refused, or one
/// refused key in the autosave patch would keep every later save of that session from landing.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn every_key_of_the_config_view_saves_back() {
    let (port, _home) = gateway().await;
    let (_, view) = http(port, "GET", "/api/config", "").await;
    let mut found = Vec::new();
    leaves("", &view, &mut found);
    assert!(found.len() > 50, "the config view lists the defaults ({})", found.len());
    // The other test in this binary shares this gateway and config.yaml and asserts its own flips: writing a
    // default back over one of those keys (compression.enabled's default is true, its flip is false) races it.
    let flipped: Vec<String> = flips().into_iter().map(|(key, _)| key).collect();
    let mut refused = Vec::new();
    for (key, value) in found {
        if key.split('.').any(|s| s.starts_with('_')) || flipped.contains(&key) {
            continue;
        }
        let (status, reply) = http(port, "PUT", "/api/config", &serde_json::json!({ "config": nest(&key, value) }).to_string()).await;
        if status != 200 {
            refused.push(format!("{key}: {status} {reply}"));
        }
    }
    assert!(refused.is_empty(), "refused:\n{}", refused.join("\n"));
}
