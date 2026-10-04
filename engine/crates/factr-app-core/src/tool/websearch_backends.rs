//! Key-based web search backends Factr supports (Tavily, Exa, Firecrawl, Brave), over plain HTTPS
//! REST. Chosen when `FACTR_WEB_BACKEND` or Factr's `config.yaml` (`web.search_backend`, else
//! `web.backend`) names one, else Factr's autodetect order (tavily, exa, firecrawl, brave) by which
//! key is present. Keys are read through the engine's credential reader (Factr `.env`, then the
//! environment; the environment only in private deployment) and are never logged. Any failure makes
//! the caller fall back to the keyless chain.

use super::websearch::SearchResult;
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Backend {
    Tavily,
    Exa,
    Firecrawl,
    Brave,
}

fn parse(name: &str) -> Option<Backend> {
    match name.trim().to_ascii_lowercase().as_str() {
        "tavily" => Some(Backend::Tavily),
        "exa" => Some(Backend::Exa),
        "firecrawl" => Some(Backend::Firecrawl),
        "brave" | "brave-free" | "brave_free" => Some(Backend::Brave),
        _ => None,
    }
}

/// `secret(name)`: a key or URL variable if set. `configured`: the `web.backend` value, if any.
/// Returns the backend and its key (empty for a self-hosted Firecrawl with only `FIRECRAWL_API_URL`).
pub(super) fn choose(configured: Option<&str>, secret: &dyn Fn(&str) -> Option<String>) -> Option<(Backend, String)> {
    let key = |b: Backend| -> Option<String> {
        let get = |n: &str| secret(n).filter(|v| !v.trim().is_empty());
        match b {
            Backend::Tavily => get("TAVILY_API_KEY"),
            Backend::Exa => get("EXA_API_KEY"),
            Backend::Brave => get("BRAVE_SEARCH_API_KEY").or_else(|| get("BRAVE_API_KEY")),
            Backend::Firecrawl => get("FIRECRAWL_API_KEY").or_else(|| get("FIRECRAWL_API_URL").map(|_| String::new())),
        }
    };
    if let Some(name) = configured.filter(|c| !c.trim().is_empty()) {
        // A stored backend is used as is (Factr does the same); a non-keyed one means keyless chain.
        let b = parse(name)?;
        return key(b).map(|k| (b, k));
    }
    [Backend::Tavily, Backend::Exa, Backend::Firecrawl, Backend::Brave].into_iter().find_map(|b| key(b).map(|k| (b, k)))
}

fn row(title: Option<&str>, url: Option<&str>, snippet: String) -> Option<SearchResult> {
    let url = url.filter(|u| !u.is_empty())?;
    Some(SearchResult { title: title.filter(|t| !t.is_empty()).unwrap_or(url).to_string(), url: url.to_string(), snippet })
}

fn str_of<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    v.get(k).and_then(Value::as_str)
}

pub(super) fn parse_tavily(v: &Value) -> Vec<SearchResult> {
    v["results"].as_array().into_iter().flatten().filter_map(|r| row(str_of(r, "title"), str_of(r, "url"), str_of(r, "content").unwrap_or("").to_string())).collect()
}

pub(super) fn parse_exa(v: &Value) -> Vec<SearchResult> {
    v["results"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| {
            let highlights: Vec<&str> = r["highlights"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
            let snippet = if highlights.is_empty() { str_of(r, "text").unwrap_or("").to_string() } else { highlights.join(" ") };
            row(str_of(r, "title"), str_of(r, "url"), snippet)
        })
        .collect()
}

pub(super) fn parse_brave(v: &Value) -> Vec<SearchResult> {
    v["web"]["results"].as_array().into_iter().flatten().filter_map(|r| row(str_of(r, "title"), str_of(r, "url"), str_of(r, "description").unwrap_or("").to_string())).collect()
}

/// Firecrawl v2 answers `{data: {web: [...]}}`; older shapes `{data: [...]}` or `{web: [...]}`.
pub(super) fn parse_firecrawl(v: &Value) -> Vec<SearchResult> {
    let list = [&v["data"], &v["data"]["web"], &v["data"]["results"], &v["web"], &v["results"]]
        .into_iter()
        .find_map(|c| c.as_array().filter(|a| !a.is_empty()));
    list.into_iter()
        .flatten()
        .filter_map(|r| row(str_of(r, "title"), str_of(r, "url"), str_of(r, "description").or_else(|| str_of(r, "snippet")).unwrap_or("").to_string()))
        .collect()
}

/// One search. `base` overrides the vendor host (self-hosted Firecrawl, `TAVILY_BASE_URL`, tests).
pub(super) async fn search(client: &reqwest::Client, backend: Backend, key: &str, base: Option<&str>, query: &str, n: usize) -> Result<Vec<SearchResult>> {
    let trim = |s: &str| s.trim_end_matches('/').to_string();
    let n = n.clamp(1, 20);
    let request = match backend {
        Backend::Tavily => client
            .post(format!("{}/search", base.map(trim).unwrap_or_else(|| "https://api.tavily.com".into())))
            .bearer_auth(key)
            .header("X-Client-Name", "factr-backend")
            .json(&json!({ "query": query, "max_results": n, "include_raw_content": false, "include_images": false })),
        Backend::Exa => client
            .post(format!("{}/search", base.map(trim).unwrap_or_else(|| "https://api.exa.ai".into())))
            .header("x-api-key", key)
            .json(&json!({ "query": query, "numResults": n, "contents": { "highlights": true } })),
        Backend::Firecrawl => {
            let r = client
                .post(format!("{}/v2/search", base.map(trim).unwrap_or_else(|| "https://api.firecrawl.dev".into())))
                .json(&json!({ "query": query, "limit": n }));
            if key.is_empty() { r } else { r.bearer_auth(key) }
        }
        Backend::Brave => client
            .get(format!("{}/res/v1/web/search", base.map(trim).unwrap_or_else(|| "https://api.search.brave.com".into())))
            .query(&[("q", query), ("count", &n.to_string())])
            .header("X-Subscription-Token", key)
            .header("Accept", "application/json"),
    };
    let response = request.timeout(std::time::Duration::from_secs(20)).send().await.context("web backend request failed")?;
    let status = response.status();
    if !status.is_success() {
        bail!("web backend {backend:?} answered {status}");
    }
    let v: Value = response.json().await.context("web backend returned a non-JSON body")?;
    Ok(match backend {
        Backend::Tavily => parse_tavily(&v),
        Backend::Exa => parse_exa(&v),
        Backend::Firecrawl => parse_firecrawl(&v),
        Backend::Brave => parse_brave(&v),
    })
}

/// The backend to use for this call, if any, with the host override it needs.
pub(super) fn configured() -> Option<(Backend, String, Option<String>)> {
    let secret = |n: &str| crate::provider_catalog::env_secret(n);
    let private = crate::factr_env::deployment_private();
    let mut name = factr_base::factr_config::env_text("FACTR_WEB_BACKEND");
    if name.is_none() && !private {
        // Factr's own selection (`factr tools` writes web.backend to config.yaml).
        name = factr_base::factr_config::current().web.search().map(str::to_string);
    }
    let (backend, key) = choose(name.as_deref(), &secret)?;
    let base = match backend {
        Backend::Tavily => secret("TAVILY_BASE_URL"),
        Backend::Firecrawl => secret("FIRECRAWL_API_URL"),
        _ => None,
    };
    Some((backend, key, base))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// One-shot HTTP server: records the request head+body and answers `body` as JSON.
    async fn mock(status: u16, body: &str) -> (String, tokio::task::JoinHandle<String>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let body = body.to_string();
        let handle = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 16384];
            let mut got = 0;
            loop {
                let n = sock.read(&mut buf[got..]).await.unwrap();
                got += n;
                let text = String::from_utf8_lossy(&buf[..got]).to_string();
                if let Some(h) = text.find("\r\n\r\n") {
                    let len = text[..h].to_ascii_lowercase().split("content-length:").nth(1).and_then(|r| r.lines().next()?.trim().parse::<usize>().ok()).unwrap_or(0);
                    if got >= h + 4 + len || n == 0 {
                        break;
                    }
                } else if n == 0 {
                    break;
                }
            }
            let resp = format!("HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len());
            sock.write_all(resp.as_bytes()).await.unwrap();
            String::from_utf8_lossy(&buf[..got]).to_string()
        });
        (url, handle)
    }

    fn client() -> reqwest::Client {
        reqwest::Client::new()
    }

    #[tokio::test]
    async fn tavily_request_and_shape() {
        let (url, req) = mock(200, r#"{"results":[{"title":"T","url":"https://a.test/x","content":"snip"},{"title":"","url":""}]}"#).await;
        let r = search(&client(), Backend::Tavily, "FAKE-KEY", Some(&url), "rust", 5).await.unwrap();
        assert_eq!((r.len(), r[0].title.as_str(), r[0].snippet.as_str()), (1, "T", "snip"));
        let req = req.await.unwrap().to_ascii_lowercase();
        assert!(req.starts_with("post /search") && req.contains("bearer fake-key") && req.contains("\"max_results\":5"), "{req}");
    }

    #[tokio::test]
    async fn exa_request_and_shape() {
        let (url, req) = mock(200, r#"{"results":[{"title":"E","url":"https://e.test","highlights":["one","two"]},{"title":"F","url":"https://f.test","text":"body"}]}"#).await;
        let r = search(&client(), Backend::Exa, "FAKE-KEY", Some(&url), "q", 5).await.unwrap();
        assert_eq!((r[0].snippet.as_str(), r[1].snippet.as_str()), ("one two", "body"));
        let req = req.await.unwrap().to_ascii_lowercase();
        assert!(req.contains("x-api-key: fake-key") && req.contains("\"highlights\":true") && req.contains("\"numresults\":5"), "{req}");
    }

    #[tokio::test]
    async fn brave_request_and_shape() {
        let (url, req) = mock(200, r#"{"web":{"results":[{"title":"B","url":"https://b.test","description":"d"}]}}"#).await;
        let r = search(&client(), Backend::Brave, "FAKE-KEY", Some(&url), "hello world", 3).await.unwrap();
        assert_eq!((r[0].title.as_str(), r[0].snippet.as_str()), ("B", "d"));
        let req = req.await.unwrap().to_ascii_lowercase();
        assert!(req.starts_with("get /res/v1/web/search?") && req.contains("q=hello") && req.contains("count=3") && req.contains("x-subscription-token: fake-key"), "{req}");
    }

    #[tokio::test]
    async fn firecrawl_request_and_both_shapes() {
        let (url, req) = mock(200, r#"{"success":true,"data":{"web":[{"title":"W","url":"https://w.test","description":"wd"}]}}"#).await;
        let r = search(&client(), Backend::Firecrawl, "FAKE-KEY", Some(&url), "q", 5).await.unwrap();
        assert_eq!((r[0].title.as_str(), r[0].snippet.as_str()), ("W", "wd"));
        let req = req.await.unwrap().to_ascii_lowercase();
        assert!(req.starts_with("post /v2/search") && req.contains("bearer fake-key") && req.contains("\"limit\":5"), "{req}");
        assert_eq!(parse_firecrawl(&json!({"data": [{"url": "https://l.test", "title": "L"}]})).len(), 1);
        // Self-hosted with no key sends no Authorization header.
        let (url, req) = mock(200, r#"{"data":[]}"#).await;
        assert!(search(&client(), Backend::Firecrawl, "", Some(&url), "q", 5).await.unwrap().is_empty());
        assert!(!req.await.unwrap().to_ascii_lowercase().contains("authorization"));
    }

    #[tokio::test]
    async fn an_error_status_is_an_error_without_the_key() {
        let (url, _) = mock(429, r#"{"error":"slow down"}"#).await;
        let e = search(&client(), Backend::Exa, "FAKE-SECRET-KEY", Some(&url), "q", 5).await.unwrap_err();
        assert!(e.to_string().contains("429") && !format!("{e:#}").contains("FAKE-SECRET-KEY"));
    }

    #[test]
    fn selection_follows_config_then_factr_autodetect_order() {
        let env = |pairs: &'static [(&'static str, &'static str)]| move |n: &str| pairs.iter().find(|(k, _)| *k == n).map(|(_, v)| v.to_string());
        let both = env(&[("EXA_API_KEY", "e"), ("BRAVE_SEARCH_API_KEY", "b")]);
        assert_eq!(choose(None, &both), Some((Backend::Exa, "e".into())));
        assert_eq!(choose(Some("brave-free"), &both), Some((Backend::Brave, "b".into())));
        assert_eq!(choose(Some("tavily"), &both), None, "configured but no key: keyless chain");
        assert_eq!(choose(Some("ddgs"), &both), None);
        assert_eq!(choose(None, &env(&[("TAVILY_API_KEY", "t"), ("EXA_API_KEY", "e")])), Some((Backend::Tavily, "t".into())));
        assert_eq!(choose(None, &env(&[("BRAVE_API_KEY", "b2")])), Some((Backend::Brave, "b2".into())));
        assert_eq!(choose(None, &env(&[("FIRECRAWL_API_URL", "http://x")])), Some((Backend::Firecrawl, String::new())));
        assert_eq!(choose(None, &env(&[("EXA_API_KEY", "  ")])), None);
    }

    #[test]
    fn factr_config_yaml_backend() {
        let web = |yaml: &str| factr_base::factr_config::FactrConfig::parse(yaml).web.search().map(str::to_string);
        assert_eq!(web("model: x\nweb:\n  backend: exa\n  other: 1\nterminal:\n  backend: docker\n").as_deref(), Some("exa"));
        assert_eq!(web("web:\n  backend: exa\n  search_backend: \"tavily\"\n").as_deref(), Some("tavily"));
        assert_eq!(web("terminal:\n  backend: docker\n"), None);
    }
}
