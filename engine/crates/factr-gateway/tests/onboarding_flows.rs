//! ONBOARDING FLOWS: replays the exact HTTP request sequence each desktop sign-in / provider-setup
//! path sends (desktop/app/src/store/onboarding.ts, api/config.ts, api/models.ts) against the real
//! gateway with a stand-in Python feature backend (a local stub server, fake values only), in both
//! modes the desktop uses:
//!
//! - chat-only: the packaged app (`FACTR_BACKEND_PYTHON` set) with the chat route open, so requests
//!   carry no `feature=1`. This is where the first-run onboarding overlay runs.
//! - feature: the same requests with `feature=1` (Settings > Providers is open).
//!
//! Each request is classified: `forwarded` (reached the stub backend), `native` (answered by the
//! engine), `gated` (404 feature_not_requested) or `missing` (404 not_supported_by_engine).
//! A call the onboarding makes AFTER the user started a sign-in must never be `gated`/`missing`.
//! The only `gated` answer allowed is the documented boot probe `GET /api/providers/oauth`.
//! `KNOWN_GATED` lists routes that are gated today but should not be; the test fails if one is
//! fixed without being removed from the list, so the list stays honest.

#![cfg(unix)]

use factr_gateway::{Config, Gateway};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const TOKEN: &str = "test-token-onboarding-flows-32chars-minimum-ok";

/// Gated in chat-only mode on this tree although a user action sends them. Empty once fixed.
const KNOWN_GATED: &[(&str, &str)] = &[];

/// Answered 404 by design in chat-only mode (the desktop falls back to a ChatGPT row).
const BOOT_PROBES: &[(&str, &str)] = &[("GET", "/api/providers/oauth")];

/// Other routes the desktop sends from the chat route (no `feature=1`). Reported, not asserted:
/// they do not block sign-in, but a `gated` answer here is a user-visible failure elsewhere.
const OTHER_CHAT_ROUTES: &[(&str, &str, &str)] = &[
    ("POST", "/api/audio/transcribe", "voice input (lib/voice-client-direct.ts)"),
    ("POST", "/api/audio/speak", "read aloud (lib/voice-playback.ts)"),
    ("POST", "/api/audio/tts-lease", "read aloud lease (lib/tts-lease.ts)"),
    ("POST", "/api/factr/update", "install update (store/updates.ts); stays gated: runs `factr update`"),
    ("GET", "/api/factr/update/check", "update check (store/updates.ts)"),
    ("GET", "/api/profiles/default/soul", "profile switcher (app/chat/sidebar/profile-switcher.tsx)"),
    ("PUT", "/api/profiles/default/soul", "profile switcher save"),
    ("GET", "/api/git/gh-auth", "GitHub suggestions (store/suggestion-providers/github.ts)"),
    ("GET", "/api/logs", "error notification logs (store/notifications.ts)"),
    ("GET", "/api/local-models/catalog", "local-setup tip (components/tips/local-setup-offer.ts)"),
    ("GET", "/api/local-models/hardware", "status bar (app/shell/system-resources-statusbar.tsx)"),
    ("GET", "/api/local-models/status", "model picker local row"),
    ("GET", "/api/model/auxiliary", "auxiliary models read"),
    ("GET", "/api/env", "configured-key read"),
];

/// Stand-in feature backend: announces its port like `factr serve` does, answers every request
/// with a JSON body marked `"stub": true` (and `ok: true` so `/api/model/set` reads as saved).
const STUB: &str = r#"
import http.server, json, socketserver, sys
class H(http.server.BaseHTTPRequestHandler):
    def _reply(self):
        n = int(self.headers.get('Content-Length') or 0)
        if n: self.rfile.read(n)
        body = json.dumps({"ok": True, "stub": True, "status": "pending", "path": self.path,
                           "providers": [], "session_id": "s1", "flow": "device_code",
                           "reachable": True, "models": ["m"], "provider": "p", "model": "m"}).encode()
        self.send_response(200)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)
    do_GET = do_POST = do_PUT = do_DELETE = do_PATCH = _reply
    def log_message(self, *a): pass
s = socketserver.TCPServer(('127.0.0.1', 0), H)
print('FACTR_BACKEND_READY port=%d' % s.server_address[1], flush=True)
s.serve_forever()
"#;

/// One sign-in path: the requests it sends, in order, from first click to "ready to chat".
struct Flow {
    name: &'static str,
    calls: Vec<(&'static str, &'static str, Value)>,
}

fn flows() -> Vec<Flow> {
    let model_confirm = |provider: &'static str| {
        // completeWithModelConfirm: model.options (native), recommended default, persist the model.
        vec![
            ("GET", "/api/model/options?include_unconfigured=1", Value::Null),
            (
                "GET",
                match provider {
                    "openai-codex" => "/api/model/recommended-default?provider=openai-codex",
                    "anthropic" => "/api/model/recommended-default?provider=anthropic",
                    _ => "/api/model/recommended-default?provider=openrouter",
                },
                Value::Null,
            ),
            ("POST", "/api/model/set", json!({"scope": "main", "provider": provider, "model": "m"})),
        ]
    };
    let mut codex = vec![
        ("POST", "/api/providers/oauth/openai-codex/start", json!({})),
        ("GET", "/api/providers/oauth/openai-codex/poll/s1", Value::Null),
    ];
    codex.extend(model_confirm("openai-codex"));
    let mut pkce = vec![
        ("POST", "/api/providers/oauth/anthropic/start", json!({})),
        ("POST", "/api/providers/oauth/anthropic/submit", json!({"session_id": "s1", "code": "fake-code"})),
    ];
    pkce.extend(model_confirm("anthropic"));
    let mut api_key = vec![("PUT", "/api/env", json!({"key": "OPENROUTER_API_KEY", "value": "sk-fake-test-only"}))];
    api_key.extend(model_confirm("openrouter"));
    vec![
        Flow { name: "boot probe (OAuth provider list)", calls: vec![("GET", "/api/providers/oauth", Value::Null)] },
        Flow { name: "ChatGPT/Codex device code", calls: codex },
        Flow { name: "PKCE code paste", calls: pkce },
        Flow { name: "cancel sign-in", calls: vec![("DELETE", "/api/providers/oauth/sessions/s1", Value::Null)] },
        Flow { name: "API key", calls: api_key },
        Flow {
            name: "local / custom endpoint (Ollama, vLLM, llama.cpp)",
            calls: vec![
                ("POST", "/api/providers/validate", json!({"key": "OPENAI_BASE_URL", "value": "http://127.0.0.1:11434/v1", "api_key": ""})),
                ("POST", "/api/model/set", json!({"scope": "main", "provider": "custom", "model": "m", "base_url": "http://127.0.0.1:11434/v1", "api_key": ""})),
            ],
        },
        Flow {
            name: "disconnect",
            calls: vec![
                ("DELETE", "/api/providers/oauth/openai-codex", Value::Null),
                ("DELETE", "/api/env", json!({"key": "OPENROUTER_API_KEY"})),
            ],
        },
    ]
}

async fn http(port: u16, method: &str, path: &str, body: &Value) -> (u16, Value) {
    let body = if body.is_null() { String::new() } else { body.to_string() };
    let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {TOKEN}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(req.as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    tokio::time::timeout(std::time::Duration::from_secs(30), stream.read_to_end(&mut buf)).await.expect("read timeout").unwrap();
    let text = String::from_utf8_lossy(&buf).into_owned();
    let status = text.lines().next().and_then(|l| l.split_whitespace().nth(1)).and_then(|s| s.parse().ok()).unwrap_or(0);
    let json = text.split("\r\n\r\n").nth(1).and_then(|b| serde_json::from_str(b).ok()).unwrap_or(Value::Null);
    (status, json)
}

fn classify(status: u16, body: &Value) -> &'static str {
    match (status, body["reason"].as_str()) {
        (404, Some("feature_not_requested")) => "gated",
        (404, Some("not_supported_by_engine")) => "missing",
        _ if body["stub"] == true => "forwarded",
        _ => "native",
    }
}

fn with_feature(path: &str) -> String {
    format!("{path}{}feature=1", if path.contains('?') { '&' } else { '?' })
}

fn bare(path: &str) -> &str {
    path.split('?').next().unwrap_or(path)
}

async fn start_gateway(home: &Path, stub: &Path) -> u16 {
    let features = factr_gateway::features::Features::new(vec!["python3".into(), stub.to_string_lossy().into()]);
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
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    port
}

#[tokio::test]
async fn every_sign_in_path_reaches_a_handler_after_the_user_starts_it() {
    let home = std::env::temp_dir().join(format!("factr-onboarding-flows-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    let stub = home.join("stub_backend.py");
    std::fs::write(&stub, STUB).unwrap();
    // SAFETY: this binary has one test, so nothing else reads the environment concurrently.
    unsafe {
        std::env::set_var("FACTR_CONFIG_HOME", &home);
        std::env::set_var("FACTR_HOME", &home);
        // The packaged desktop sets this; it is what makes un-marked requests "chat-only".
        std::env::set_var("FACTR_BACKEND_PYTHON", "python3");
    }
    let port = start_gateway(&home, &stub).await;

    let mut failures = Vec::new();
    let mut still_gated = std::collections::BTreeSet::new();
    let mut report = String::new();
    for flow in flows() {
        for (method, path, body) in &flow.calls {
            let key = (*method, bare(path));
            let (s, b) = http(port, method, path, body).await;
            let chat = classify(s, &b);
            let (s, b) = http(port, method, &with_feature(path), body).await;
            let feature = classify(s, &b);
            report.push_str(&format!("{:<48} {method:<6} {:<52} chat-only={chat:<9} feature={feature}\n", flow.name, bare(path)));
            if feature == "gated" || feature == "missing" {
                failures.push(format!("[feature mode] {} {method} {path}: {feature}", flow.name));
            }
            let boot_probe = BOOT_PROBES.contains(&key);
            match chat {
                "gated" if boot_probe => {}
                "gated" if KNOWN_GATED.contains(&key) => {
                    still_gated.insert(key);
                }
                "gated" | "missing" => failures.push(format!("[chat-only] {} {method} {path}: {chat}", flow.name)),
                _ if boot_probe => failures.push(format!("[chat-only] boot probe {method} {path} now wakes Python ({chat})")),
                _ => {}
            }
        }
    }
    for (method, path, who) in OTHER_CHAT_ROUTES {
        let (s, b) = http(port, method, path, &json!({})).await;
        let class = classify(s, &b);
        report.push_str(&format!("{:<48} {method:<6} {path:<52} chat-only={class}\n", format!("[other] {who}")));
        let user_action = path.starts_with("/api/audio/") || path.contains("/soul");
        if user_action && class != "forwarded" {
            failures.push(format!("[chat-only] {method} {path} ({who}): {class}"));
        }
        if *path == "/api/factr/update" && class != "gated" {
            failures.push(format!("[chat-only] {method} {path} must stay gated (installs code): {class}"));
        }
    }
    eprintln!("{report}");
    for known in KNOWN_GATED {
        if !still_gated.contains(known) {
            failures.push(format!("{} {} is no longer gated: remove it from KNOWN_GATED", known.0, known.1));
        }
    }
    let _ = std::fs::remove_dir_all(&home);
    assert!(failures.is_empty(), "onboarding request failures:\n{}\n\nfull table:\n{report}", failures.join("\n"));
}
