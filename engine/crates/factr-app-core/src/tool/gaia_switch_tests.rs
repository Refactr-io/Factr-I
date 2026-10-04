//! GAIA-variant switches: pinned SearXNG, Wikipedia last resort, webfetch
//! allowed hosts and Wayback fallback. All against local mock servers.
use super::webfetch::WebFetchTool;
use super::webfetch_net::{check_host_allowed, fetch_resilient_with};
use super::websearch::WebSearchTool;
use super::{Tool, ToolContext, ToolExecutionMode};
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::time::Duration;

type Seen = Arc<Mutex<Vec<String>>>;

fn http(status: &str, ct: &str, body: &str, extra: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: {ct}\r\nContent-Length: {}\r\n{extra}Connection: close\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

/// Loopback server answering every connection via `handler(request_line)`.
fn mock(handler: impl Fn(&str) -> Vec<u8> + Send + 'static) -> (String, Seen) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let seen: Seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    std::thread::spawn(move || {
        while let Ok((mut s, _)) = listener.accept() {
            let mut buf = [0u8; 4096];
            let n = s.read(&mut buf).unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]).into_owned();
            let line = req.lines().next().unwrap_or("").to_string();
            log.lock().unwrap().push(line.clone());
            let _ = s.write_all(&handler(&line));
        }
    });
    (base, seen)
}

struct Env(Vec<(&'static str, Option<std::ffi::OsString>)>);
impl Env {
    fn set(vars: &[(&'static str, &str)]) -> Self {
        let mut prev = Vec::new();
        for (k, v) in vars {
            prev.push((*k, std::env::var_os(k)));
            crate::env::set_var(k, v);
        }
        crate::config::invalidate_config_cache();
        Env(prev)
    }
}
impl Drop for Env {
    fn drop(&mut self) {
        for (k, v) in self.0.drain(..) {
            match v {
                Some(v) => crate::env::set_var(k, v),
                None => crate::env::remove_var(k),
            }
        }
        crate::config::invalidate_config_cache();
    }
}

fn ctx() -> ToolContext {
    ToolContext {
        session_id: "s".into(),
        message_id: "m".into(),
        tool_call_id: "c".into(),
        working_dir: None,
        stdin_request_tx: None,
        graceful_shutdown_signal: None,
        execution_mode: ToolExecutionMode::AgentTurn,
    }
}

fn client() -> reqwest::Client {
    reqwest::Client::builder().no_proxy().build().unwrap()
}

fn search_service(line: &str) -> Vec<u8> {
    if line.contains("/search?") && line.contains("empty") {
        http("200 OK", "application/json", r#"{"results":[]}"#, "")
    } else if line.contains("/search?") {
        let body = r#"{"results":[{"title":"Mock hit","url":"https://example.org/a","content":"snippet"}]}"#;
        http("200 OK", "application/json", body, "")
    } else if line.contains("action=opensearch") {
        http("200 OK", "application/json", r#"["q",["Wiki Hit"],["d"],["https://en.wikipedia.org/wiki/Wiki_Hit"]]"#, "")
    } else {
        http("404 Not Found", "text/plain", "nope", "")
    }
}

fn searcher(base: &str) -> WebSearchTool {
    WebSearchTool { client: client(), wikipedia_api: format!("{base}/w/api.php") }
}

async fn search(t: &WebSearchTool, input: Value) -> anyhow::Result<String> {
    t.execute(input, ctx()).await.map(|o| o.output)
}

#[tokio::test]
async fn pinned_searxng_ignores_per_call_engine_and_fallbacks() {
    let _g = crate::storage::lock_test_env();
    let (base, seen) = mock(search_service);
    let url = format!("{base}/r/test");
    let _e = Env::set(&[
        ("FACTR_WEBSEARCH_ENGINE", "searxng"),
        ("FACTR_SEARXNG_URL", &url),
        ("FACTR_WEBSEARCH_FALLBACK_ENGINES", "bing,duckduckgo"),
    ]);
    let t = searcher(&base);

    let out = search(&t, json!({"query": "rust lang", "engine": "duckduckgo"})).await.unwrap();
    assert!(out.contains("Mock hit"), "{out}");
    let lines = seen.lock().unwrap().clone();
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].starts_with("GET /r/test/search?") && lines[0].contains("format=json"), "{lines:?}");

    // Empty result: nothing but searxng (and Wikipedia, still on) is contacted, never ddg/bing.
    seen.lock().unwrap().clear();
    let out = search(&t, json!({"query": "empty", "engine": "bing"})).await.unwrap();
    assert!(out.contains("Wiki Hit"), "{out}");
    let lines = seen.lock().unwrap().clone();
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(lines[0].contains("/r/test/search?") && lines[1].contains("action=opensearch"), "{lines:?}");

    // The schema tells the model the argument is ignored.
    let schema = t.parameters_schema().to_string();
    assert!(schema.contains("Ignored: every search goes to the one configured SearXNG"), "{schema}");
    assert!(!schema.contains("\"enum\""), "{schema}");
}

#[tokio::test]
async fn wikipedia_last_resort_switch() {
    let _g = crate::storage::lock_test_env();
    let (base, seen) = mock(search_service);
    let url = format!("{base}/r/test");
    let _e = Env::set(&[
        ("FACTR_WEBSEARCH_ENGINE", "searxng"),
        ("FACTR_SEARXNG_URL", &url),
        ("FACTR_WEBSEARCH_LAST_RESORT_WIKIPEDIA", "0"),
        ("FACTR_WEBSEARCH_FALLBACK_ENGINES", "bing"),
    ]);
    assert!(!crate::config::config().websearch.last_resort_wikipedia);
    let out = search(&searcher(&base), json!({"query": "empty"})).await.unwrap();
    assert_eq!(out, "No results found for: empty");
    let lines = seen.lock().unwrap().clone();
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].contains("/r/test/search?"), "{lines:?}");
}

#[tokio::test]
async fn defaults_keep_the_upstream_behaviour() {
    let _g = crate::storage::lock_test_env();
    let _e = Env::set(&[]);
    let c = crate::config::config();
    assert!(c.websearch.last_resort_wikipedia);
    assert!(c.webfetch.wayback_fallback && c.webfetch.allowed_hosts.is_empty());
    let schema = WebSearchTool::new().parameters_schema().to_string();
    assert!(schema.contains("\"enum\":[\"duckduckgo\",\"bing\",\"searxng\"]"), "{schema}");
    assert!(check_host_allowed("https://anything.example/", &[]).is_ok());
}

#[test]
fn host_check_is_case_insensitive_and_names_allowed_hosts() {
    let allowed = vec!["127.0.0.1".to_string(), "Svc.Local".to_string(), "[::1]".to_string()];
    assert!(check_host_allowed("http://127.0.0.1:8080/x", &allowed).is_ok());
    assert!(check_host_allowed("http://svc.local/x", &allowed).is_ok());
    assert!(check_host_allowed("http://[::1]:9/x", &allowed).is_ok());
    let err = check_host_allowed("https://example.com/", &allowed).unwrap_err();
    assert!(err.contains("example.com") && err.contains("127.0.0.1, Svc.Local, [::1]"), "{err}");
}

async fn fetch(url: &str) -> anyhow::Result<String> {
    let t = WebFetchTool { client: client() };
    t.execute(json!({"url": url, "timeout": 5}), ctx()).await.map(|o| o.output)
}

#[tokio::test]
async fn allowed_hosts_block_public_urls_and_redirects() {
    let _g = crate::storage::lock_test_env();
    let (base, seen) = mock(|line| {
        if line.contains("/r/test/fetch?") {
            http("200 OK", "text/html", "<html><body><p>service page</p></body></html>", "")
        } else if line.contains("/bounce") {
            http("302 Found", "text/plain", "", "Location: http://example.invalid/secret\r\n")
        } else if line.contains("/loop") {
            http("302 Found", "text/plain", "", "Location: /r/test/fetch?url=x\r\n")
        } else {
            http("404 Not Found", "text/plain", "nope", "")
        }
    });
    let _e = Env::set(&[("FACTR_WEBFETCH_ALLOWED_HOSTS", "127.0.0.1")]);

    let err = fetch("https://example.com/").await.unwrap_err().to_string();
    assert!(err.contains("not allowed") && err.contains("127.0.0.1"), "{err}");
    assert!(seen.lock().unwrap().is_empty());

    let ok = fetch(&format!("{base}/r/test/fetch?url=http%3A%2F%2Fexample.org")).await.unwrap();
    assert!(ok.contains("service page"), "{ok}");

    let err = fetch(&format!("{base}/bounce")).await.unwrap_err().to_string();
    assert!(err.contains("redirect blocked") && err.contains("example.invalid"), "{err}");

    // A redirect that stays on an allowed host is followed.
    let ok = fetch(&format!("{base}/loop")).await.unwrap();
    assert!(ok.contains("service page"), "{ok}");
}

#[tokio::test]
async fn wayback_switch_controls_archive_contact() {
    let _g = crate::storage::lock_test_env();
    let (site, site_seen) = mock(|_| http("404 Not Found", "text/plain", "gone", ""));
    let (api, api_seen) = mock(|_| http("200 OK", "application/json", r#"{"archived_snapshots":{}}"#, ""));
    let url = format!("{site}/page");
    let api_url = format!("{api}/wayback/available");

    let off = fetch_resilient_with(&client(), &url, Duration::from_secs(5), None, &[]).await;
    assert_eq!(off.err().unwrap().status, Some(404));
    assert!(api_seen.lock().unwrap().is_empty(), "wayback contacted although disabled");

    let on = fetch_resilient_with(&client(), &url, Duration::from_secs(5), Some(&api_url), &[]).await;
    assert!(on.err().unwrap().msg.contains("no archived copy"));
    assert_eq!(api_seen.lock().unwrap().len(), 1);

    // An allowed-hosts list that excludes the archive also skips it.
    let allowed = vec!["127.0.0.1".to_string()];
    let _ = fetch_resilient_with(&client(), &url, Duration::from_secs(5), Some("https://archive.org/wayback/available"), &allowed).await;
    assert!(site_seen.lock().unwrap().len() >= 3);

    // Tool level: FACTR_WEBFETCH_WAYBACK=0 means a 404 ends without any archive attempt.
    let _e = Env::set(&[("FACTR_WEBFETCH_WAYBACK", "0")]);
    let err = fetch(&url).await.unwrap_err().to_string();
    assert!(err.contains("HTTP 404") && !err.contains("archived"), "{err}");
}
