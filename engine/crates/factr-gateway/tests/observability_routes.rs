//! HTTP auth smoke for M10c observability routes (token required).

use factr_gateway::{Config, Gateway};
use std::path::PathBuf;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

fn long_token() -> String {
    "test-token-observability-routes-m10c-32chars-min".into()
}

async fn http_get(port: u16, path: &str, token: Option<&str>) -> (u16, String) {
    let mut stream = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        TcpStream::connect(("127.0.0.1", port)),
    )
    .await
    .expect("connect timeout")
    .unwrap();
    let auth = token.map(|t| format!("Authorization: Bearer {t}\r\n")).unwrap_or_default();
    let req = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n{auth}Connection: close\r\n\r\n");
    stream.write_all(req.as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    tokio::time::timeout(std::time::Duration::from_secs(3), stream.read_to_end(&mut buf))
        .await
        .expect("read timeout")
        .unwrap();
    let text = String::from_utf8_lossy(&buf).into_owned();
    let status = text
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    (status, text)
}

async fn http_post(port: u16, path: &str, body: &str, token: Option<&str>) -> (u16, String) {
    let mut stream = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        TcpStream::connect(("127.0.0.1", port)),
    )
    .await
    .expect("connect timeout")
    .unwrap();
    let auth = token.map(|t| format!("Authorization: Bearer {t}\r\n")).unwrap_or_default();
    let req = format!(
        "POST {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n{auth}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(req.as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    tokio::time::timeout(std::time::Duration::from_secs(3), stream.read_to_end(&mut buf))
        .await
        .expect("read timeout")
        .unwrap();
    let text = String::from_utf8_lossy(&buf).into_owned();
    let status = text
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    (status, text)
}

#[tokio::test]
async fn observability_routes_require_token() {
    let home = std::env::temp_dir().join(format!("factr-obs-route-test-{}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    let token = long_token();
    let config = Config {
        bind: "127.0.0.1:0".parse().unwrap(),
        token: token.clone(),
        version: "test".into(),
        legacy_socket: PathBuf::from("/dev/null"),
        default_cwd: home.to_string_lossy().into(),
        allow_non_loopback: false,
        provider: "ollama".into(),
        model: "local".into(),
        reasoning_efforts: Vec::new(),
        profile_model_applies: true,
        home: home.to_string_lossy().into(),
        complete: None,
        features: None,
        learning: None,
    };
    let gateway = Gateway::bind(config).await.unwrap();
    let port = gateway.local_addr().port();
    let server = tokio::spawn(async move {
        let _ = gateway.serve().await;
    });
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let routes = [
        "/api/factr/observability/runs",
        "/api/factr/observability/monitors?window=24h",
        "/api/factr/observability/budget",
        "/api/factr/observability/approvals",
        "/api/factr/observability/sessions",
        "/api/factr/observability/facts?days=7",
        "/api/factr/observability/alerts",
        "/api/factr/observability/memory-audit",
        "/api/factr/observability/memory?limit=5",
    ];
    for path in routes {
        let (status, _) = http_get(port, path, None).await;
        assert_eq!(status, 401, "{path} without token");
        let (status, _) = http_get(port, path, Some(&token)).await;
        assert_eq!(status, 200, "{path} with token");
    }
    let posts = [
        ("/api/factr/observability/promote", r#"{"run_id":"missing"}"#),
        ("/api/factr/observability/replay", r#"{"run_id":"missing"}"#),
    ];
    for (path, body) in posts {
        let (status, _) = http_post(port, path, body, None).await;
        assert_eq!(status, 401, "{path} without token");
    }
    server.abort();
}

#[tokio::test]
async fn agent_run_takes_bodies_over_64kb_and_answers_413_above_4mb() {
    let home = std::env::temp_dir().join(format!("factr-run-body-test-{}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    let token = long_token();
    let config = Config {
        bind: "127.0.0.1:0".parse().unwrap(),
        token: token.clone(),
        version: "test".into(),
        legacy_socket: PathBuf::from("/dev/null"),
        default_cwd: home.to_string_lossy().into(),
        allow_non_loopback: false,
        provider: "ollama".into(),
        model: "local".into(),
        reasoning_efforts: Vec::new(),
        profile_model_applies: true,
        home: home.to_string_lossy().into(),
        complete: None,
        features: None,
        learning: None,
    };
    let gateway = Gateway::bind(config).await.unwrap();
    let port = gateway.local_addr().port();
    let server = tokio::spawn(async move {
        let _ = gateway.serve().await;
    });
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // 100 KB with no prompt: read in full and refused by the handler (400), not dropped.
    let big = format!(r#"{{"prompt":"","pad":"{}"}}"#, "x".repeat(100_000));
    let (status, text) = http_post(port, "/api/agent/run", &big, Some(&token)).await;
    assert_eq!(status, 400, "{text}");
    assert!(text.contains("prompt is required"));

    // A declared 5 MB body is answered with JSON before it is read.
    let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let head = format!(
        "POST /api/agent/run HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {token}\r\nContent-Length: 5000000\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(head.as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();
    let text = String::from_utf8_lossy(&buf);
    assert!(text.starts_with("HTTP/1.1 413"), "{text}");
    assert!(text.contains("\"detail\""));
    server.abort();
}

#[tokio::test]
async fn curator_routes_answer_not_available() {
    let home = std::env::temp_dir().join(format!("factr-curator-test-{}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    let token = long_token();
    let config = Config {
        bind: "127.0.0.1:0".parse().unwrap(),
        token: token.clone(),
        version: "test".into(),
        legacy_socket: PathBuf::from("/dev/null"),
        default_cwd: home.to_string_lossy().into(),
        allow_non_loopback: false,
        provider: "ollama".into(),
        model: "local".into(),
        reasoning_efforts: Vec::new(),
        profile_model_applies: true,
        home: home.to_string_lossy().into(),
        complete: None,
        features: None,
        learning: None,
    };
    let gateway = Gateway::bind(config).await.unwrap();
    let port = gateway.local_addr().port();
    let server = tokio::spawn(async move {
        let _ = gateway.serve().await;
    });
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let (status, _) = http_get(port, "/api/curator", None).await;
    assert_eq!(status, 401);
    let (status, body) = http_get(port, "/api/curator", Some(&token)).await;
    assert_eq!(status, 404);
    assert!(body.contains("curator is not available"), "{body}");
    server.abort();
}

#[tokio::test]
async fn status_reports_a_missing_login_so_a_runner_can_fail_fast() {
    let home = std::env::temp_dir().join(format!("factr-status-login-{}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    let config = Config {
        bind: "127.0.0.1:0".parse().unwrap(),
        token: long_token(),
        version: "test".into(),
        legacy_socket: PathBuf::from("/dev/null"),
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
    };
    let gateway = Gateway::bind(config).await.unwrap();
    let port = gateway.local_addr().port();
    let server = tokio::spawn(async move {
        let _ = gateway.serve().await;
    });
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    factr_gateway::clear_missing_login();
    let (_, text) = http_get(port, "/api/status", None).await;
    assert!(text.contains("\"missing_login\":null"), "{text}");
    factr_gateway::note_missing_login("openai", "No OpenAI tokens or API key found");
    let (status, text) = http_get(port, "/api/status", None).await;
    assert_eq!(status, 200);
    assert!(text.contains("no readable openai login (No OpenAI tokens or API key found)"), "{text}");
    factr_gateway::clear_missing_login();
    server.abort();
}
