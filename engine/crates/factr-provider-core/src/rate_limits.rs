//! The provider's own view of how much room is left, read ONLY from the rate-limit headers of the
//! responses the app itself receives (Anthropic `anthropic-ratelimit-*`, OpenAI-style `x-ratelimit-*`).
//! Nothing here reads another tool's credentials or calls a quota endpoint.
//!
//! The runtimes call [`record`] once per response; the UI shows the most recent snapshot.

use reqwest::header::HeaderMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How much of one limit window is left.
#[derive(Debug, Clone, PartialEq)]
pub struct Window {
    /// `"requests"` or `"tokens"`.
    pub kind: &'static str,
    pub limit: u64,
    pub remaining: u64,
    /// Seconds until the window resets, when the provider said.
    pub reset_in_secs: Option<u64>,
}

impl Window {
    pub fn remaining_pct(&self) -> f64 {
        if self.limit == 0 { 100.0 } else { (self.remaining as f64 / self.limit as f64 * 100.0).clamp(0.0, 100.0) }
    }
}

/// The most constrained window from one response, with where it came from.
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub provider: String,
    pub window: Window,
    pub captured: Instant,
}

static LATEST: Mutex<Option<Snapshot>> = Mutex::new(None);

fn header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name)?.to_str().ok().map(str::trim)
}

fn number(headers: &HeaderMap, name: &str) -> Option<u64> {
    header(headers, name)?.parse().ok()
}

/// `"1s"`, `"6m0s"`, `"1h2m"`, `"250ms"`, a bare number of seconds, or an RFC 3339 instant is too rich to
/// trust: only the duration forms and bare seconds are read.
fn duration_secs(text: &str) -> Option<u64> {
    let text = text.trim();
    if let Ok(secs) = text.parse::<f64>() {
        return (secs.is_finite() && secs >= 0.0).then(|| secs.ceil() as u64);
    }
    let mut total = 0f64;
    let mut number = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_ascii_digit() || c == '.' {
            number.push(c);
            continue;
        }
        let value: f64 = number.parse().ok()?;
        number.clear();
        match c {
            'h' => total += value * 3600.0,
            'm' if chars.peek() == Some(&'s') => {
                chars.next();
                total += value / 1000.0;
            }
            'm' => total += value * 60.0,
            's' => total += value,
            _ => return None,
        }
    }
    number.is_empty().then(|| total.ceil() as u64)
}

/// The most constrained window the headers describe, `None` when they describe none.
pub fn parse(headers: &HeaderMap) -> Option<Window> {
    let mut windows = Vec::new();
    // Anthropic: requests, tokens, and the split input/output token windows.
    for (kind, prefix) in [
        ("requests", "anthropic-ratelimit-requests"),
        ("tokens", "anthropic-ratelimit-tokens"),
        ("tokens", "anthropic-ratelimit-input-tokens"),
        ("tokens", "anthropic-ratelimit-output-tokens"),
    ] {
        if let (Some(limit), Some(remaining)) = (number(headers, &format!("{prefix}-limit")), number(headers, &format!("{prefix}-remaining"))) {
            // The reset is an RFC 3339 instant for these; the relative form is not offered, so none is shown.
            windows.push(Window { kind, limit, remaining, reset_in_secs: None });
        }
    }
    // OpenAI and the many servers that copy its headers.
    for (kind, suffix) in [("requests", "requests"), ("tokens", "tokens")] {
        if let (Some(limit), Some(remaining)) =
            (number(headers, &format!("x-ratelimit-limit-{suffix}")), number(headers, &format!("x-ratelimit-remaining-{suffix}")))
        {
            let reset_in_secs = header(headers, &format!("x-ratelimit-reset-{suffix}")).and_then(duration_secs);
            windows.push(Window { kind, limit, remaining, reset_in_secs });
        }
    }
    windows.into_iter().filter(|w| w.limit > 0).min_by(|a, b| a.remaining_pct().total_cmp(&b.remaining_pct()))
}

/// Remember what `provider`'s response headers say. No-op when they carry no rate-limit data.
pub fn record(provider: &str, headers: &HeaderMap) {
    if let Some(window) = parse(headers)
        && let Ok(mut latest) = LATEST.lock()
    {
        *latest = Some(Snapshot { provider: provider.to_string(), window, captured: Instant::now() });
    }
}

/// The latest snapshot, unless it is older than `max_age`.
pub fn latest(max_age: Duration) -> Option<Snapshot> {
    LATEST.lock().ok()?.clone().filter(|s| s.captured.elapsed() <= max_age)
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::{HeaderName, HeaderValue};

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (k, v) in pairs {
            map.insert(HeaderName::from_bytes(k.as_bytes()).unwrap(), HeaderValue::from_str(v).unwrap());
        }
        map
    }

    #[test]
    fn the_most_constrained_openai_window_wins_and_its_reset_is_read() {
        let w = parse(&headers(&[
            ("x-ratelimit-limit-requests", "500"),
            ("x-ratelimit-remaining-requests", "499"),
            ("x-ratelimit-reset-requests", "120ms"),
            ("x-ratelimit-limit-tokens", "30000"),
            ("x-ratelimit-remaining-tokens", "3000"),
            ("x-ratelimit-reset-tokens", "6m0s"),
        ]))
        .unwrap();
        assert_eq!((w.kind, w.limit, w.remaining, w.reset_in_secs), ("tokens", 30000, 3000, Some(360)));
        assert_eq!(w.remaining_pct(), 10.0);
    }

    #[test]
    fn anthropic_headers_are_read() {
        let w = parse(&headers(&[
            ("anthropic-ratelimit-requests-limit", "50"),
            ("anthropic-ratelimit-requests-remaining", "49"),
            ("anthropic-ratelimit-input-tokens-limit", "40000"),
            ("anthropic-ratelimit-input-tokens-remaining", "10000"),
        ]))
        .unwrap();
        assert_eq!((w.kind, w.remaining), ("tokens", 10000));
    }

    #[test]
    fn responses_without_rate_limit_headers_and_malformed_values_give_nothing() {
        assert_eq!(parse(&HeaderMap::new()), None);
        assert_eq!(parse(&headers(&[("x-ratelimit-limit-requests", "abc"), ("x-ratelimit-remaining-requests", "1")])), None);
        assert_eq!(parse(&headers(&[("x-ratelimit-limit-requests", "0"), ("x-ratelimit-remaining-requests", "0")])), None);
    }

    #[test]
    fn durations_parse_and_garbage_does_not() {
        assert_eq!(duration_secs("1s"), Some(1));
        assert_eq!(duration_secs("1h2m3s"), Some(3723));
        assert_eq!(duration_secs("250ms"), Some(1));
        assert_eq!(duration_secs("90"), Some(90));
        assert_eq!(duration_secs("soon"), None);
        assert_eq!(duration_secs("5x"), None);
    }

    #[test]
    fn record_keeps_the_latest_and_ignores_headerless_responses() {
        record("p", &headers(&[("x-ratelimit-limit-requests", "10"), ("x-ratelimit-remaining-requests", "1")]));
        record("p", &HeaderMap::new());
        let s = latest(Duration::from_secs(60)).expect("kept");
        assert_eq!((s.provider.as_str(), s.window.remaining), ("p", 1));
    }
}
