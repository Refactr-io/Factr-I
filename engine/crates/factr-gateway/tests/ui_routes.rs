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


const ROUTES: [(&str, &str); 7] = [
    ("schema", "/api/config/schema"), ("model_auxiliary", "/api/model/auxiliary"), ("model_moa", "/api/model/moa"),
    ("elevenlabs_voices", "/api/audio/elevenlabs/voices"), ("pairing", "/api/pairing"), ("cron_jobs", "/api/cron/jobs"),
    ("cron_blueprints", "/api/cron/blueprints"),
];

fn fixture(name: &str) -> serde_json::Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ui_routes").join(format!("{name}.json"));
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// The engine answers these pages from files (Python is `/usr/bin/false`, so a forwarded request is a 503) and
/// the answers equal what the real Python routes returned for the same files (backend/scripts/dump_ui_routes.py).
#[tokio::test]
async fn ui_pages_are_answered_natively_and_match_python() {
    let home: PathBuf = std::env::temp_dir().join(format!("factr-ui-routes-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    let pristine = fixture("pristine");
    let defaults = home.join("defaults.json");
    std::fs::write(&defaults, pristine["_defaults"].to_string()).unwrap();
    // SAFETY: this binary has one test, so nothing else reads the environment concurrently.
    unsafe {
        std::env::set_var("FACTR_CONFIG_HOME", &home);
        std::env::set_var("FACTR_BACKEND_DEFAULTS", &defaults);
        std::env::remove_var("FACTR_MANAGED_DIR");
        std::env::remove_var("ELEVENLABS_API_KEY");
        for (k, _) in std::env::vars().collect::<Vec<_>>() {
            if ["TELEGRAM", "DISCORD", "SLACK", "WHATSAPP", "SIGNAL", "MATRIX", "EMAIL", "TWILIO", "WEBHOOK", "API_SERVER", "SMS", "LINE_", "QQ", "IRC"].iter().any(|p| k.contains(p)) {
                std::env::remove_var(k);
            }
        }
    }
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

    for name in ["pristine", "configured"] {
        let gold = fixture(name);
        let _ = std::fs::remove_file(home.join("config.yaml"));
        if name == "configured" {
            std::fs::write(home.join("config.yaml"), CONFIGURED_YAML).unwrap();
        }
        for (key, path) in ROUTES {
            // MoA and the auxiliary pins read the file; the configured fixture overrides both.
            if name == "configured" && key == "model_moa" {
                let (status, _) = http(port, "GET", path, "").await;
                assert_eq!(status, 503, "a customised moa block is Python's: forwarded");
                continue;
            }
            let (status, body) = http(port, "GET", path, "").await;
            assert_eq!(status, 200, "{name} {key}: answered natively");
            assert_eq!(body, gold[key]["body"], "{name} {key} equals Python");
        }
    }

    // Anything the port does not model goes to Python (503 here, since Python is `false`).
    let _ = std::fs::remove_file(home.join("config.yaml"));
    let forwarded = |what: &'static str, path: &'static str| async move {
        let (status, _) = http(port, "GET", path, "").await;
        assert_eq!(status, 503, "{what}: forwarded to Python");
    };
    std::fs::create_dir_all(home.join("cron")).unwrap();
    std::fs::write(home.join("cron/jobs.json"), "{\"jobs\": []}").unwrap();
    forwarded("a jobs file", "/api/cron/jobs").await;
    std::fs::remove_file(home.join("cron/jobs.json")).unwrap();
    std::fs::write(home.join(".env"), "ELEVENLABS_API_KEY=x\n").unwrap();
    forwarded("an ElevenLabs key", "/api/audio/elevenlabs/voices").await;
    std::fs::write(home.join(".env"), "TELEGRAM_BOT_TOKEN=x\n").unwrap();
    forwarded("a connected platform", "/api/cron/blueprints").await;
    std::fs::remove_file(home.join(".env")).unwrap();
    std::fs::write(home.join("config.yaml"), "tts:\n  providers:\n    mine:\n      type: command\n      command: say\n").unwrap();
    forwarded("a command voice provider", "/api/config/schema").await;
    std::fs::write(home.join("config.yaml"), "model:\n  default: ${MODEL}\n").unwrap();
    forwarded("a ${VAR} reference", "/api/model/auxiliary").await;
    std::fs::write(home.join("config.yaml"), "auxiliary:\n  vision:\n    base_url: http://127.0.0.1:1/v1\n").unwrap();
    forwarded("a pinned aux endpoint", "/api/model/auxiliary").await;
    std::fs::remove_file(home.join("config.yaml")).unwrap();
    std::fs::create_dir_all(home.join("platforms/pairing")).unwrap();
    std::fs::write(home.join("platforms/pairing/telegram-approved.json"), "{}").unwrap();
    forwarded("pairing files", "/api/pairing").await;
    let (status, _) = http(port, "GET", "/api/model/moa?profile=coder", "").await;
    assert_eq!(status, 503, "a named profile is Python's");
}

const CONFIGURED_YAML: &str = "model:\n  default: gpt-5.5\n  provider: openai-codex\nauxiliary:\n  vision:\n    provider: openrouter\n    model: google/gemini-3-flash\nmoa:\n  default_preset: default\n  presets:\n    default:\n      reference_models:\n        - {provider: openrouter, model: deepseek/deepseek-v4-pro}\n      aggregator: {provider: openrouter, model: anthropic/claude-opus-4.8}\ndisplay:\n  language: en\n";
