//! Native dashboard reads match real Python goldens; uncertain cases must proxy.

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


const ROUTES: [(&str, &str); 8] = [
    ("schema", "/api/config/schema"), ("model_auxiliary", "/api/model/auxiliary"), ("model_moa", "/api/model/moa"),
    ("elevenlabs_voices", "/api/audio/elevenlabs/voices"), ("pairing", "/api/pairing"), ("cron_jobs", "/api/cron/jobs"),
    ("cron_blueprints", "/api/cron/blueprints"),
    ("cron_delivery_targets", "/api/cron/delivery-targets"),
];

fn fixture(name: &str) -> serde_json::Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ui_routes").join(format!("{name}.json"));
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// One test owns process environment. Inputs are shared with the real Python golden generator.
#[test]
fn ui_pages_are_answered_natively_and_match_python() {
    let work = std::env::temp_dir().join(format!("factr-ui-routes-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).unwrap();
    let defaults = work.join("defaults.json");
    std::fs::write(&defaults, fixture("pristine")["_defaults"].to_string()).unwrap();
    // SAFETY: this integration binary has one test; no other test mutates environment.
    unsafe {
        std::env::set_var("FACTR_BACKEND_DEFAULTS", &defaults);
        for (k, _) in std::env::vars().collect::<Vec<_>>() {
            if ["ELEVENLABS", "FACTR_MANAGED_DIR", "TELEGRAM", "DISCORD", "SLACK", "WHATSAPP", "SIGNAL", "MATRIX", "MATTERMOST", "EMAIL", "SMS", "TWILIO", "HOMEASSISTANT", "FEISHU", "DINGTALK", "WECOM", "WEIXIN", "QQ", "LINE_", "TEAMS", "WEBHOOK", "API_SERVER", "BLUEBUBBLES", "YUANBAO", "IRC", "MASTODON"].iter().any(|p| k.contains(p)) {
                std::env::remove_var(k);
            }
        }
    }
    let cases: serde_json::Value = serde_json::from_str(include_str!("fixtures/ui_routes/cases.json")).unwrap();
    for (name, case) in cases.as_object().unwrap() {
        let root = work.join(name);
        let home = root.join(case["home"].as_str().unwrap_or(""));
        std::fs::create_dir_all(&home).unwrap();
        if let Some(files) = case["files"].as_object() {
            for (rel, text) in files {
                let path = root.join(rel);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, text.as_str().unwrap()).unwrap();
            }
        }
        unsafe {
            std::env::set_var("FACTR_CONFIG_HOME", &home);
            std::env::set_var("FACTR_HOME", &home);
            if let Some(managed) = case["managed"].as_str() {
                std::env::set_var("FACTR_MANAGED_DIR", root.join(managed));
            } else {
                std::env::remove_var("FACTR_MANAGED_DIR");
            }
        }
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        runtime.block_on(async {
        let features = std::sync::Arc::new(factr_gateway::features::Features::new(vec!["/usr/bin/false".into()]));
        let config = Config {
            bind: "127.0.0.1:0".parse().unwrap(), token: TOKEN.into(), version: "test".into(), legacy_socket: PathBuf::from("/dev/null"),
            default_cwd: home.to_string_lossy().into(), allow_non_loopback: false, provider: "ollama".into(), model: "local".into(),
            reasoning_efforts: Vec::new(), profile_model_applies: true, home: home.to_string_lossy().into(), complete: None,
            features: Some(features.clone()), learning: None,
        };
        let gateway = Gateway::bind(config).await.unwrap();
        let port = gateway.local_addr().port();
        let server = tokio::spawn(async move { let _ = gateway.serve().await; });
        let gold = fixture(name);
        for (key, path) in ROUTES {
            if gold.get(key).is_none() { continue; }
            let url = if let Some(q) = case["query"].as_str() { format!("{path}?{q}") } else { path.to_string() };
            let (status, body) = http(port, "GET", &url, "").await;
            let fallback = case["fallback"].as_array().is_some_and(|a| a.iter().any(|v| v == key));
            if fallback {
                assert_eq!(status, 503, "{name} {key}: must proxy; Python golden is {}", gold[key]);
            } else {
                assert_eq!(status, gold[key]["status"].as_u64().unwrap() as u16, "{name} {key}");
                assert_eq!(body, gold[key]["body"], "{name} {key} equals Python");
                assert!(!features.is_running().await, "{name} {key}: Python stayed stopped");
            }
        }
        server.abort();
        let _ = server.await;
        });
    }
    std::fs::remove_dir_all(work).unwrap();
}
