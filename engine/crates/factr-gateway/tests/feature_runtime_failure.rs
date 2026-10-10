//! Audit B: when the Python feature runtime cannot start (missing interpreter, crash on boot), a
//! forwarded request (`PUT /api/env`, `POST /api/model/set`, the OAuth start the onboarding sends)
//! must still get an HTTP answer the desktop can show. Today `proxy_http` returns the
//! `features.port()` error with `?` and the connection handler drops it (`let _ = handle(..)`), so
//! the client sees the socket close with no status line and the "exited before it was ready"
//! stderr tail is lost.

#![cfg(unix)]

use factr_gateway::{Config, Gateway};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const TOKEN: &str = "test-token-feature-runtime-failure-32chars-ok";

async fn raw(port: u16, method: &str, path: &str, body: &str) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-Factr-Session-Token: {TOKEN}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(req.as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    let _ = tokio::time::timeout(std::time::Duration::from_secs(60), stream.read_to_end(&mut buf)).await;
    String::from_utf8_lossy(&buf).into_owned()
}

#[tokio::test]
async fn a_feature_runtime_that_cannot_start_still_gets_a_readable_http_error() {
    let home = std::env::temp_dir().join(format!("factr-feature-fail-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    let crash = home.join("crash-python");
    std::fs::write(&crash, "#!/bin/sh\necho 'fake python: boom' >&2\nexit 1\n").unwrap();
    std::fs::set_permissions(&crash, std::fs::Permissions::from_mode(0o755)).unwrap();
    // SAFETY: this binary has one test, so nothing else reads the environment concurrently.
    unsafe {
        std::env::set_var("FACTR_CONFIG_HOME", &home);
        std::env::set_var("FACTR_HOME", &home);
        std::env::set_var("FACTR_BACKEND_PYTHON", &crash);
    }
    let features = factr_gateway::features::Features::new(vec![crash.to_string_lossy().into()]);
    let config = Config {
        bind: "127.0.0.1:0".parse().unwrap(), token: TOKEN.into(), version: "test".into(), legacy_socket: PathBuf::from("/dev/null"),
        default_cwd: home.to_string_lossy().into(), allow_non_loopback: false, provider: "openai-codex".into(), model: "m".into(),
        reasoning_efforts: Vec::new(), profile_model_applies: true, home: home.to_string_lossy().into(), complete: None,
        features: Some(std::sync::Arc::new(features)), learning: None,
    };
    let gateway = Gateway::bind(config).await.unwrap();
    let port = gateway.local_addr().port();
    tokio::spawn(async move {
        let _ = gateway.serve().await;
    });

    let mut failures = Vec::new();
    for (method, path, body) in [
        ("PUT", "/api/env", r#"{"key":"OPENROUTER_API_KEY","value":"sk-fake-test-only"}"#),
        ("POST", "/api/model/set", r#"{"scope":"main","provider":"openrouter","model":"m"}"#),
        ("POST", "/api/providers/oauth/openai-codex/start", "{}"),
    ] {
        let reply = raw(port, method, path, body).await;
        let status = reply.lines().next().unwrap_or_default().to_string();
        if !status.starts_with("HTTP/1.1 5") || !reply.contains("boom") {
            failures.push(format!("{method} {path}: got {:?} (want a 5xx JSON naming the runtime failure)", if reply.is_empty() { "connection closed, no response".to_string() } else { reply }));
        }
    }
    let _ = std::fs::remove_dir_all(&home);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
