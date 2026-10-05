//! Resilient HTTP for `webfetch`: browser UA, retry with jittered backoff,
//! Wayback fallback. Pure helpers are unit-tested.
use futures::StreamExt;
use std::time::Duration;

pub const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 \
(KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";
/// Hard cap on any downloaded body (PDFs and office files can be large).
pub const MAX_BODY: usize = 25 * 1024 * 1024;
pub const WAYBACK_API: &str = "https://archive.org/wayback/available";
const MAX_RETRY_AFTER: Duration = Duration::from_secs(10);

pub struct Fetched {
    pub content_type: String,
    pub bytes: Vec<u8>,
}

pub struct FetchFail {
    pub status: Option<u16>,
    pub msg: String,
    transient: bool,
    retry_after: Option<Duration>,
}

pub fn backoff(attempt: u32, retry_after: Option<Duration>) -> Duration {
    if let Some(d) = retry_after {
        return d.min(MAX_RETRY_AFTER);
    }
    let base = 500u64 << attempt.min(4);
    Duration::from_millis(base + rand::random_range(0..300))
}

async fn get_once(client: &reqwest::Client, url: &str, timeout: Duration) -> Result<Fetched, FetchFail> {
    let fail = |status, msg: String, transient, retry_after| FetchFail { status, msg, transient, retry_after };
    let resp = client
        .get(url)
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .header(reqwest::header::ACCEPT, "text/html,application/xhtml+xml,application/pdf,*/*;q=0.8")
        .header(reqwest::header::ACCEPT_LANGUAGE, "en-US,en;q=0.9")
        .timeout(timeout)
        .send()
        .await
        .map_err(|e| {
            let transient = e.is_connect() || e.is_timeout();
            let redirect_src = if e.is_redirect() { std::error::Error::source(&e).map(|s| s.to_string()) } else { None };
            let mut msg = format!("request failed: {}", e.without_url());
            if let Some(src) = redirect_src {
                msg = format!("{msg}: {src}");
            }
            fail(None, msg, transient, None)
        })?;
    let status = resp.status();
    if !status.is_success() {
        let retry_after = resp
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<u64>().ok())
            .map(Duration::from_secs);
        let transient = status.as_u16() == 429 || status.is_server_error();
        return Err(fail(Some(status.as_u16()), format!("HTTP {status}"), transient, retry_after));
    }
    if resp.content_length().is_some_and(|l| l as usize > MAX_BODY) {
        return Err(fail(None, format!("response larger than {} MB", MAX_BODY >> 20), false, None));
    }
    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let mut bytes = Vec::new();
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| fail(None, format!("read failed: {}", e.without_url()), true, None))?;
        let room = MAX_BODY - bytes.len();
        bytes.extend_from_slice(&chunk[..chunk.len().min(room)]);
        if chunk.len() > room {
            break;
        }
    }
    Ok(Fetched { content_type, bytes })
}

/// GET with up to two retries on 429/5xx/connect/timeout errors.
pub async fn fetch_retry(client: &reqwest::Client, url: &str, timeout: Duration) -> Result<Fetched, FetchFail> {
    let mut attempt = 0;
    loop {
        match get_once(client, url, timeout).await {
            Err(e) if e.transient && attempt < 2 => {
                tokio::time::sleep(backoff(attempt, e.retry_after)).await;
                attempt += 1;
            }
            other => return other,
        }
    }
}

/// Fetch, then on 403/404/410 try the closest Wayback snapshot. The bool is
/// true when the archived copy was used.
/// `wayback_api` None disables the archive.org fallback entirely. A non-empty
/// `allowed_hosts` refuses any host outside the list (Wayback included).
pub async fn fetch_resilient_with(
    client: &reqwest::Client,
    url: &str,
    timeout: Duration,
    wayback_api: Option<&str>,
    allowed_hosts: &[String],
) -> Result<(Fetched, bool), FetchFail> {
    if let Err(msg) = check_host_allowed(url, allowed_hosts) {
        return Err(FetchFail { status: None, msg, transient: false, retry_after: None });
    }
    let mut err = match fetch_retry(client, url, timeout).await {
        Ok(f) => return Ok((f, false)),
        Err(e) => e,
    };
    if matches!(err.status, Some(403 | 404 | 410))
        && let Some(api) = wayback_api
        && check_host_allowed(api, allowed_hosts).is_ok()
        && let Some(snap) = wayback_snapshot(client, api, url).await
        && check_host_allowed(&snap, allowed_hosts).is_ok()
        && let Ok(f) = fetch_retry(client, &snap, timeout).await
    {
        return Ok((f, true));
    }
    if matches!(err.status, Some(403 | 404 | 410)) && wayback_api.is_some() {
        err.msg.push_str(" (no archived copy)");
    }
    Err(err)
}

fn norm_host(h: &str) -> String {
    h.trim().trim_start_matches('[').trim_end_matches(']').to_ascii_lowercase()
}

/// Ok when `allowed` is empty or the URL matches an entry (case-insensitive). An entry
/// `host` matches that host on any port; an entry `host:port` matches host AND port
/// (the URL's scheme default port counts, so `example.com:443` matches `https://example.com`).
pub fn check_host_allowed(url: &str, allowed: &[String]) -> Result<(), String> {
    if allowed.is_empty() {
        return Ok(());
    }
    let parsed = reqwest::Url::parse(url).ok();
    let host = parsed.as_ref().and_then(|u| u.host_str().map(norm_host)).unwrap_or_default();
    let port = parsed.as_ref().and_then(|u| u.port_or_known_default());
    let ok = allowed.iter().any(|entry| {
        let (entry_host, entry_port) = split_host_port(entry);
        entry_host == host && (entry_port.is_none() || entry_port == port)
    });
    if ok {
        return Ok(());
    }
    Err(format!(
        "host `{host}` is not allowed; webfetch is restricted to: {}",
        allowed.join(", ")
    ))
}

/// Split an allow-list entry into (normalised host, optional port). Bracketed IPv6
/// (`[::1]:8080`) is handled; a bare IPv6 literal or a non-numeric suffix has no port.
fn split_host_port(entry: &str) -> (String, Option<u16>) {
    let e = entry.trim();
    if let Some(rest) = e.strip_prefix('[')
        && let Some((h, tail)) = rest.split_once(']')
    {
        return (norm_host(h), tail.strip_prefix(':').and_then(|p| p.parse().ok()));
    }
    if e.matches(':').count() == 1
        && let Some((h, p)) = e.split_once(':')
        && let Ok(port) = p.parse::<u16>()
    {
        return (norm_host(h), Some(port));
    }
    (norm_host(e), None)
}

/// A client whose redirect hops are re-checked against `allowed`.
pub fn restricted_client(allowed: &[String]) -> reqwest::Client {
    let allowed = allowed.to_vec();
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(Duration::from_secs(15))
        .no_proxy()
        .redirect(reqwest::redirect::Policy::custom(move |attempt| {
            if attempt.previous().len() >= 10 {
                return attempt.error("too many redirects");
            }
            match check_host_allowed(attempt.url().as_str(), &allowed) {
                Ok(()) => attempt.follow(),
                Err(msg) => attempt.error(format!("redirect blocked: {msg}")),
            }
        }))
        .build()
        .unwrap_or_default()
}

pub fn wayback_query_url(api: &str, url: &str) -> String {
    format!("{api}?url={}", urlencoding::encode(url))
}

/// Closest snapshot from a Wayback availability response, as a raw-content
/// (`id_`) https URL without the Wayback toolbar.
pub fn parse_wayback(json: &serde_json::Value) -> Option<String> {
    let closest = json.pointer("/archived_snapshots/closest")?;
    if !closest.get("available")?.as_bool()? {
        return None;
    }
    let url = closest.get("url")?.as_str()?.replacen("http://", "https://", 1);
    let (head, rest) = url.split_once("/web/")?;
    let (ts, orig) = rest.split_once('/')?;
    Some(format!("{head}/web/{ts}id_/{orig}"))
}

async fn wayback_snapshot(client: &reqwest::Client, api: &str, url: &str) -> Option<String> {
    let resp = client
        .get(wayback_query_url(api, url))
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .ok()?;
    parse_wayback(&resp.json().await.ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wayback_builder_and_parser() {
        assert_eq!(
            wayback_query_url("https://archive.org/wayback/available", "https://a.b/c?d=e"),
            "https://archive.org/wayback/available?url=https%3A%2F%2Fa.b%2Fc%3Fd%3De"
        );
        let hit = serde_json::json!({"archived_snapshots":{"closest":{"available":true,
            "url":"http://web.archive.org/web/20200101000000/http://a.b/c"}}});
        assert_eq!(
            parse_wayback(&hit).as_deref(),
            Some("https://web.archive.org/web/20200101000000id_/http://a.b/c")
        );
        assert!(parse_wayback(&serde_json::json!({"archived_snapshots":{}})).is_none());
    }

    #[test]
    fn backoff_honours_retry_after_with_cap() {
        assert_eq!(backoff(0, Some(Duration::from_secs(3))), Duration::from_secs(3));
        assert_eq!(backoff(0, Some(Duration::from_secs(99))), MAX_RETRY_AFTER);
        assert!(backoff(1, None) >= Duration::from_millis(1000));
    }
}
