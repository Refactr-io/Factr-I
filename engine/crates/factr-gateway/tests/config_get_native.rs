//! With the Python backend configured (and never started), `GET /api/config` is still answered natively
//! and reads back what the native `PUT /api/config` wrote.

use factr_gateway::{Config, Gateway};
use std::path::PathBuf;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const TOKEN: &str = "test-token-config-native-get-32chars-minimum-ok";

async fn http(port: u16, method: &str, path: &str, body: &str) -> (u16, serde_json::Value) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {TOKEN}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(req.as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    tokio::time::timeout(std::time::Duration::from_secs(3), stream.read_to_end(&mut buf)).await.expect("read timeout").unwrap();
    let text = String::from_utf8_lossy(&buf).into_owned();
    let status = text.lines().next().and_then(|l| l.split_whitespace().nth(1)).and_then(|s| s.parse().ok()).unwrap_or(0);
    let json = text.split("\r\n\r\n").nth(1).and_then(|b| serde_json::from_str(b).ok()).unwrap_or(serde_json::Value::Null);
    (status, json)
}

#[tokio::test]
async fn get_config_is_native_even_when_python_is_configured() {
    let home: PathBuf = std::env::temp_dir().join(format!("factr-config-native-get-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    // SAFETY: this binary has one test, so nothing else reads the environment concurrently.
    unsafe { std::env::set_var("FACTR_CONFIG_HOME", &home) };
    std::fs::write(home.join("config.yaml"), "custom_providers:\n  - name: mine\n").unwrap();
    let config = Config {
        bind: "127.0.0.1:0".parse().unwrap(), token: TOKEN.into(), version: "test".into(), legacy_socket: PathBuf::from("/dev/null"),
        default_cwd: home.to_string_lossy().into(), allow_non_loopback: false, provider: "ollama".into(), model: "local".into(),
        reasoning_efforts: Vec::new(), profile_model_applies: true, home: home.to_string_lossy().into(), complete: None,
        features: Some(std::sync::Arc::new(factr_gateway::features::Features::new(vec!["/usr/bin/false".into()]))), learning: None,
    };
    let gateway = Gateway::bind(config).await.unwrap();
    let port = gateway.local_addr().port();
    tokio::spawn(async move {
        let _ = gateway.serve().await;
    });
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let body = r#"{"config":{"memory":{"memory_enabled":false,"user_profile_enabled":false},"voice":{"client_direct":true},"approvals":{"mode":"off"}}}"#;
    let (status, reply) = http(port, "PUT", "/api/config", body).await;
    assert_eq!((status, reply), (200, serde_json::json!({"ok": true})));
    let (status, got) = http(port, "GET", "/api/config", "").await;
    assert_eq!(status, 200);
    assert_eq!(got["memory"], serde_json::json!({"memory_enabled": false, "user_profile_enabled": false}));
    assert_eq!(got["voice"]["client_direct"], true);
    assert_eq!(got["custom_providers"][0]["name"], "mine", "unknown keys survive the merge");
    let (_, raw) = http(port, "GET", "/api/config?include_defaults=false", "").await;
    assert_eq!(raw["approvals"]["mode"], "off");

    let (status, err) = http(port, "PUT", "/api/config", r#"{"config":{"approvals":{"mode":"smart"}}}"#).await;
    assert_eq!(status, 400);
    assert!(err["detail"].as_str().unwrap().contains("smart"), "{err}");
    let yaml = std::fs::read_to_string(home.join("config.yaml")).unwrap();
    assert!(yaml.contains("mode: off") && !yaml.contains("smart"), "{yaml}");
}
