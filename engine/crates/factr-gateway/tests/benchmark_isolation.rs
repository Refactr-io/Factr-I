//! Benchmark isolation. The scored runs start the engine with
//! `factr --provider openai --model gpt-6-luna serve --host 127.0.0.1 --port 0 --token-stdin`
//! and a valid ChatGPT login, no API key. Whatever config.yaml holds, the gateway must resolve that
//! start exactly as main did: OpenAI on gpt-6-luna, the saved pick never applied, no ChatGPT-login
//! warning, no `missing_login`, and no custom endpoint pushed into the OpenAI-compatible route.
//! (The end-to-end comparison against main's binary, sandboxed, is in the PR notes.)
//!
//! Also: a fresh install (nothing saved, no credential anywhere) reports `missing_login`.

#![cfg(unix)]

use factr_gateway::profile::{boot_pick, chatgpt_login_missing, chatgpt_saved_pick, current, effective_default};
use factr_gateway::{Config, Gateway};
use serde_json::Value;
use std::path::{Path, PathBuf};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const TOKEN: &str = "test-token-benchmark-isolation-32chars-minimum";
/// Fixture shape from factr-base `auth/external_tests.rs` (fake values only).
const FAKE_VALID_LOGIN: &str = r#"{"credential_pool":{"openai-codex":[{"auth_type":"oauth","access_token":"fake-access","refresh_token":"fake-refresh","expires_at":"2999-01-01T00:00:00Z"}]}}"#;

async fn get(port: u16, path: &str) -> Value {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let req = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-Factr-Session-Token: {TOKEN}\r\nConnection: close\r\n\r\n");
    stream.write_all(req.as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();
    let text = String::from_utf8_lossy(&buf);
    serde_json::from_str(text.split("\r\n\r\n").nth(1).unwrap_or_default()).unwrap_or(Value::Null)
}

fn config(home: &Path, provider: &str, model: &str, profile_model_applies: bool) -> Config {
    Config {
        bind: "127.0.0.1:0".parse().unwrap(),
        token: TOKEN.into(),
        version: "test".into(),
        legacy_socket: PathBuf::from("/dev/null"),
        default_cwd: home.to_string_lossy().into(),
        allow_non_loopback: false,
        provider: provider.into(),
        model: model.into(),
        reasoning_efforts: Vec::new(),
        profile_model_applies,
        home: home.to_string_lossy().into(),
        complete: None,
        features: None,
        learning: None,
    }
}

async fn serve(config: Config) -> (u16, tokio::task::JoinHandle<()>) {
    let gateway = Gateway::bind(config).await.unwrap();
    let port = gateway.local_addr().port();
    let task = tokio::spawn(async move {
        let _ = gateway.serve().await;
    });
    (port, task)
}

fn fresh_home(root: &Path, name: &str) -> PathBuf {
    let home = root.join(name);
    std::fs::create_dir_all(&home).unwrap();
    // SAFETY: this binary has one test, so nothing else reads the environment concurrently.
    unsafe {
        std::env::set_var("FACTR_HOME", &home);
        std::env::set_var("FACTR_CONFIG_HOME", &home);
    }
    factr_base::auth::AuthStatus::invalidate_cache();
    home
}

#[tokio::test]
async fn the_benchmark_start_resolves_as_on_main_whatever_config_yaml_says() {
    let root = std::env::temp_dir().join(format!("factr-bench-iso-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    // SAFETY: as above. The scrubbed benchmark environment has no API key.
    unsafe {
        for key in ["OPENAI_API_KEY", "FACTR_OPENAI_COMPAT_API_BASE", "FACTR_OPENAI_COMPAT_API_KEY_NAME", "FACTR_PROVIDER", "FACTR_MODEL"] {
            std::env::remove_var(key);
        }
    }
    factr_gateway::register_credentials();

    for (name, yaml) in [
        ("absent", None),
        ("openai-codex", Some("model:\n  provider: openai-codex\n  default: gpt-5.6-luna\n")),
        ("custom-base-url", Some("model:\n  provider: custom\n  default: stub-model\n  base_url: http://127.0.0.1:9/v1\n")),
        ("huggingface", Some("model:\n  provider: huggingface\n  default: some-org/some-model\n")),
    ] {
        let home = fresh_home(&root, name);
        std::fs::write(home.join("auth.json"), FAKE_VALID_LOGIN).unwrap();
        if let Some(yaml) = yaml {
            std::fs::write(home.join("config.yaml"), yaml).unwrap();
        }
        // `--provider openai --model gpt-6-luna`: explicit provider and model.
        let profile = current();
        let boot = boot_pick(true, Some("gpt-6-luna"), &profile);
        assert_eq!((boot.profile_provider, boot.model, boot.profile_model_applies), (None, Some("gpt-6-luna"), false), "{name}");
        // The served label the engine passes for that start (`OpenAIProvider::name()`).
        let config = config(&home, "OpenAI", "gpt-6-luna", boot.profile_model_applies);
        assert_eq!(effective_default(&config), ("gpt-6-luna".to_string(), "OpenAI".to_string()), "{name}");
        assert!(!chatgpt_saved_pick(&config) && !chatgpt_login_missing(&config), "{name}: the ChatGPT guard never fires");

        let (port, task) = serve(config).await;
        let status = get(port, "/api/status").await;
        let info = get(port, "/api/model/info").await;
        let options = get(port, "/api/model/options").await;
        task.abort();
        assert_eq!(status["missing_login"], Value::Null, "{name}: {status}");
        assert_eq!((info["model"].as_str(), info["provider"].as_str()), (Some("gpt-6-luna"), Some("OpenAI")), "{name}: {info}");
        assert_eq!((options["model"].as_str(), options["provider"].as_str()), (Some("gpt-6-luna"), Some("OpenAI")), "{name}: {options}");
        let openai = options["providers"].as_array().unwrap().iter().find(|r| r["slug"] == "openai").cloned().unwrap_or_default();
        assert_eq!((openai["authenticated"].as_bool(), openai["is_current"].as_bool()), (Some(true), Some(true)), "{name}: {options}");
        assert!(options.get("warning").is_none() && options.get("chatgpt_login_missing").is_none(), "{name}: {options}");
        assert!(std::env::var_os("FACTR_OPENAI_COMPAT_API_BASE").is_none(), "{name}: no custom endpoint applied");
    }

    // Fresh install: nothing saved, no credential anywhere. The desktop says "No AI provider is connected yet".
    let home = fresh_home(&root, "fresh");
    let (port, task) = serve(config(&home, "Claude", "claude-opus-5-5", true)).await;
    let fresh = get(port, "/api/status").await;
    // Once a pick is saved (the user chose a provider), it is no longer a fresh install.
    std::fs::write(home.join("config.yaml"), "model:\n  provider: ollama\n  default: qwen3.8:27b\n").unwrap();
    let saved = get(port, "/api/status").await;
    task.abort();
    assert_eq!(fresh["missing_login"].as_str(), Some(factr_gateway::NO_PROVIDER_CONNECTED), "{fresh}");
    assert_eq!(saved["missing_login"], Value::Null, "{saved}");

    let _ = std::fs::remove_dir_all(&root);
}
