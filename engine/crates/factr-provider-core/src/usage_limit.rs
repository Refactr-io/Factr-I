//! Usage-limit classification: when a provider error says the plan's quota is spent and says when it
//! resets, how long until then. The agent parks the turn for that long instead of failing it.
//!
//! Shapes read (all from text the provider runtimes already put in the error):
//! - OpenAI/Codex `usage_limit_reached`: `"resets_in_seconds":N`, or the formatted `Resets in 1d 2h`
//!   / `Resets at 2026-09-01 10:00 UTC` / `Retry after 5m`.
//! - Anthropic `rate_limit_error`: the `retry-after` header, appended by the runtime as `[retry-after: Ns]`
//!   (the provider retry loop itself caps that hint at one minute).
//! - Gemini `RESOURCE_EXHAUSTED`: `"retryDelay": "3600s"` or `quota will reset after 1h2m3s`.

use std::time::Duration;

const LIMIT_WORDS: &[&str] = &[
    "usage_limit", "usage limit", "quota", "rate_limit", "rate limit", "resource_exhausted", "too many requests", "429",
];

/// A limit error with a reset time, as the wait until the reset. `None`: not a usage limit, or no reset time given.
pub fn reset_wait(message: &str, now_unix: i64) -> Option<Duration> {
    let lower = message.to_ascii_lowercase();
    if !LIMIT_WORDS.iter().any(|w| lower.contains(w)) {
        return None;
    }
    let after = |key: &str| lower.find(key).map(|i| &lower[i + key.len()..]);
    if let Some(rest) = after("resets_in_seconds") {
        let digits: String = rest.chars().skip_while(|c| !c.is_ascii_digit()).take_while(char::is_ascii_digit).collect();
        if let Ok(n) = digits.parse::<u64>() {
            return Some(Duration::from_secs(n));
        }
    }
    if let Some(d) = after("resets in ").and_then(compact) {
        return Some(d);
    }
    if let Some(ts) = after("resets at ").and_then(utc_stamp) {
        return Some(Duration::from_secs((ts - now_unix).max(0) as u64));
    }
    if let Some(rest) = after("retry-after:") {
        let digits: String = rest.trim_start().chars().take_while(char::is_ascii_digit).collect();
        if let Ok(n) = digits.parse::<u64>() {
            return Some(Duration::from_secs(n));
        }
    }
    if let Some(d) = after("retry after ").and_then(compact) {
        return Some(d);
    }
    if let Some(rest) = after("retrydelay") {
        let quoted = rest.trim_start_matches(|c: char| c == '"' || c == ':' || c == ' ' || c == '\\');
        if let Some(d) = compact(quoted) {
            return Some(d);
        }
    }
    after("reset after ").and_then(compact)
}

/// `1d 2h 3m 4s`, `1h2m3s`, `38s`, `0.5s`: leading duration of the text.
fn compact(text: &str) -> Option<Duration> {
    let (mut total, mut number, mut any) = (0f64, String::new(), false);
    for c in text.chars() {
        match c {
            '0'..='9' | '.' => number.push(c),
            'd' | 'h' | 'm' | 's' => {
                let n: f64 = number.parse().ok()?;
                total += n * match c { 'd' => 86_400.0, 'h' => 3_600.0, 'm' => 60.0, _ => 1.0 };
                number.clear();
                any = true;
            }
            ' ' if number.is_empty() => {}
            _ => break,
        }
    }
    (any && total.is_finite()).then(|| Duration::from_secs_f64(total))
}

/// `2026-09-01 10:00 utc` -> unix seconds.
fn utc_stamp(text: &str) -> Option<i64> {
    let b = text.get(..16)?.as_bytes();
    let num = |r: std::ops::Range<usize>| std::str::from_utf8(&b[r]).ok()?.parse::<i64>().ok();
    if b[4] != b'-' || b[7] != b'-' || b[10] != b' ' || b[13] != b':' {
        return None;
    }
    let (y, m, d, hh, mm) = (num(0..4)?, num(5..7)?, num(8..10)?, num(11..13)?, num(14..16)?);
    // Days from civil (Howard Hinnant).
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some((era * 146_097 + doe - 719_468) * 86_400 + hh * 3_600 + mm * 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    const S: fn(u64) -> Option<Duration> = |n| Some(Duration::from_secs(n));

    #[test]
    fn openai_raw_body_and_formatted_message() {
        let raw = r#"{"error":{"type":"usage_limit_reached","message":"The usage limit has been reached","resets_at":1787286694,"resets_in_seconds":2608165}}"#;
        assert_eq!(reset_wait(raw, 0), S(2_608_165));
        assert_eq!(reset_wait("Rate limited: The usage limit has been reached. Plan: team. Resets in 1d 2h 3m (2026-09-01 10:00 UTC).", 0), S(86_400 + 7_200 + 180));
        assert_eq!(reset_wait("Rate limited: usage limit. Resets at 1970-01-02 00:00 UTC.", 1000), S(86_400 - 1000));
        assert_eq!(reset_wait("Rate limited: usage limit. Resets at 1970-01-02 00:00 UTC.", 999_999), S(0));
        assert_eq!(reset_wait("Rate limited (retry after 5m): too many requests", 0), S(300));
    }

    #[test]
    fn anthropic_retry_after_marker() {
        assert_eq!(reset_wait("Anthropic API error (429 Too Many Requests): {\"type\":\"rate_limit_error\"} [retry-after: 3600s]", 0), S(3600));
    }

    #[test]
    fn gemini_retry_delay_and_reset_after() {
        let body = r#"Gemini request generateContent failed (HTTP 429): {"error":{"status":"RESOURCE_EXHAUSTED","details":[{"retryDelay":"3600s"}]}}"#;
        assert_eq!(reset_wait(body, 0), S(3600));
        assert_eq!(reset_wait("RESOURCE_EXHAUSTED: Your quota will reset after 1h2m3s.", 0), S(3723));
        assert_eq!(reset_wait("429 RESOURCE_EXHAUSTED quota will reset after 0s", 0), S(0));
    }

    #[test]
    fn not_a_limit_or_no_reset_time() {
        assert_eq!(reset_wait("connection reset by peer, resets in 5m", 0), None);
        assert_eq!(reset_wait("Rate limited: usage limit reached", 0), None);
        assert_eq!(reset_wait("500 internal server error", 0), None);
    }
}
