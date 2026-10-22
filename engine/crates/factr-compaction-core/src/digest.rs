//! Deterministic first-pass compaction: a condensed, model-free account of older messages.
//!
//! Every summarized message becomes one line carrying its absolute position `#N` in the session, so the
//! exact text is never lost: `session_search` with `scope: current` and `turns: {start: N, end: N+1}`
//! returns the original. Pure function of its input (no clocks, no ordering by hash), so the same history
//! always yields the same bytes. Tool results are not replayed; the assistant's call line plus a flag on
//! errors say what happened.

use crate::truncate_str_boundary;
use factr_message_types::{ContentBlock, Message, Role};

const TEXT_MAX_BYTES: usize = 200;
const ARG_MAX_BYTES: usize = 80;
const ERROR_MAX_BYTES: usize = 120;
const MAX_FILES: usize = 40;
const PATH_KEYS: [&str; 5] = ["path", "file_path", "filepath", "file", "target"];

fn one_line(text: &str, max: usize) -> String {
    let flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.len() <= max { flat } else { format!("{}...", truncate_str_boundary(&flat, max)) }
}

fn call_gist(name: &str, input: &serde_json::Value, files: &mut Vec<String>) -> String {
    let mut gist = String::new();
    if let Some(obj) = input.as_object() {
        for key in PATH_KEYS {
            if let Some(path) = obj.get(key).and_then(|v| v.as_str()) {
                if !files.iter().any(|f| f == path) && files.len() < MAX_FILES {
                    files.push(path.to_string());
                }
                gist = one_line(path, ARG_MAX_BYTES);
                break;
            }
        }
        if gist.is_empty() {
            if let Some(cmd) = obj.get("command").and_then(|v| v.as_str()) {
                gist = one_line(cmd, ARG_MAX_BYTES);
            } else if let Some(q) = obj.get("query").or_else(|| obj.get("url")).and_then(|v| v.as_str()) {
                gist = one_line(q, ARG_MAX_BYTES);
            }
        }
    }
    if gist.is_empty() { format!("{name}()") } else { format!("{name}({gist})") }
}

/// Condensed lines for `messages`, the first of which is session message `first_index`. `previous` is the
/// earlier digest (kept in front, so digests chain). The result is cut to `max_chars` by dropping the
/// oldest message lines, never the header. `None` when there is nothing to say.
pub fn digest(messages: &[Message], first_index: usize, previous: Option<&str>, max_chars: usize) -> Option<String> {
    if messages.is_empty() {
        return None;
    }
    let mut lines: Vec<String> = Vec::with_capacity(messages.len());
    let mut files: Vec<String> = Vec::new();
    for (offset, message) in messages.iter().enumerate() {
        let n = first_index + offset;
        let mut parts: Vec<String> = Vec::new();
        for block in &message.content {
            match block {
                ContentBlock::Text { text, .. } if !text.trim().is_empty() => parts.push(one_line(text, TEXT_MAX_BYTES)),
                ContentBlock::ToolUse { name, input, .. } => parts.push(call_gist(name, input, &mut files)),
                ContentBlock::ToolResult { content, is_error: Some(true), .. } => {
                    parts.push(format!("error: {}", one_line(content, ERROR_MAX_BYTES)));
                }
                ContentBlock::Image { .. } => parts.push("[image]".to_string()),
                _ => {}
            }
        }
        if parts.is_empty() {
            continue;
        }
        let role = match message.role {
            Role::User => "user",
            Role::Assistant => "assistant",
        };
        lines.push(format!("[#{n}] {role}: {}", parts.join(" | ")));
    }
    if lines.is_empty() {
        return None;
    }
    let last = first_index + messages.len() - 1;
    let header = format!(
        "Earlier conversation, messages #{first_index}-#{last}, condensed without a model. Each line starts with the \\
         message number; the exact text of message N is session_search (load_tools first if needed) with scope current \\
         and turns {{start: N, end: N+1}}."
    );
    let files_line = (!files.is_empty()).then(|| format!("Files touched: {}", files.join(", ")));
    let mut tail = String::new();
    if let Some(line) = &files_line {
        tail.push_str("\n\n");
        tail.push_str(line);
    }
    let previous = previous.map(str::trim).filter(|p| !p.is_empty());
    let fixed = header.len() + tail.len() + previous.map_or(0, |p| p.len() + 2) + 2;
    let mut budget = max_chars.saturating_sub(fixed);
    // Newest lines win: walk from the end and keep what fits.
    let mut kept: Vec<&String> = Vec::new();
    for line in lines.iter().rev() {
        if line.len() + 1 > budget && !kept.is_empty() {
            break;
        }
        budget = budget.saturating_sub(line.len() + 1);
        kept.push(line);
    }
    kept.reverse();
    let dropped = lines.len() - kept.len();
    let mut out = String::new();
    if let Some(prev) = previous {
        out.push_str(prev);
        out.push_str("\n\n");
    }
    out.push_str(&header);
    out.push('\n');
    if dropped > 0 {
        out.push_str(&format!("({dropped} older lines omitted)\n"));
    }
    out.push_str(&kept.iter().map(|l| l.as_str()).collect::<Vec<_>>().join("\n"));
    out.push_str(&tail);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn text(role: Role, t: &str) -> Message {
        Message { role, content: vec![ContentBlock::Text { text: t.into(), cache_control: None }], timestamp: None, tool_duration_ms: None }
    }

    fn call(name: &str, input: serde_json::Value) -> Message {
        Message {
            role: Role::Assistant,
            content: vec![ContentBlock::ToolUse { id: "c1".into(), name: name.into(), input, thought_signature: None }],
            timestamp: None,
            tool_duration_ms: None,
        }
    }

    fn result(content: &str, is_error: bool) -> Message {
        Message {
            role: Role::User,
            content: vec![ContentBlock::ToolResult { tool_use_id: "c1".into(), content: content.into(), is_error: Some(is_error) }],
            timestamp: None,
            tool_duration_ms: None,
        }
    }

    fn sample() -> Vec<Message> {
        vec![
            text(Role::User, "fix the login bug"),
            call("read", json!({"path": "src/auth.rs"})),
            result("ok contents", false),
            call("bash", json!({"command": "cargo test auth"})),
            result("test failed: expected 3", true),
            text(Role::Assistant, "found it, patching"),
        ]
    }

    #[test]
    fn the_same_history_gives_the_same_bytes_and_every_message_is_linked() {
        let a = digest(&sample(), 10, None, 10_000).unwrap();
        let b = digest(&sample(), 10, None, 10_000).unwrap();
        assert_eq!(a, b);
        assert!(a.contains("messages #10-#15"));
        assert!(a.contains("[#10] user: fix the login bug"));
        assert!(a.contains("[#11] assistant: read(src/auth.rs)"));
        assert!(a.contains("[#13] assistant: bash(cargo test auth)"));
        assert!(a.contains("[#14] user: error: test failed: expected 3"));
        assert!(!a.contains("ok contents"), "successful results are not replayed");
        assert!(a.contains("Files touched: src/auth.rs"));
    }

    #[test]
    fn empty_or_contentless_history_has_no_digest() {
        assert_eq!(digest(&[], 0, None, 1000), None);
        assert_eq!(digest(&[result("", false)], 0, None, 1000), None);
    }

    #[test]
    fn the_cap_drops_the_oldest_lines_and_keeps_the_header() {
        let msgs: Vec<Message> = (0..50).map(|i| text(Role::User, &format!("message number {i} with some words"))).collect();
        let out = digest(&msgs, 0, None, 900).unwrap();
        assert!(out.len() <= 900 + 64, "{}", out.len());
        assert!(out.contains("messages #0-#49"));
        assert!(out.contains("[#49]") && !out.contains("[#0]"));
        assert!(out.contains("older lines omitted"));
    }

    #[test]
    fn digests_chain_and_multibyte_text_is_cut_on_a_char_boundary() {
        let long = "é".repeat(300);
        let first = digest(&[text(Role::User, &long)], 0, None, 10_000).unwrap();
        assert!(first.contains("..."));
        let second = digest(&[text(Role::User, "next")], 1, Some(&first), 10_000).unwrap();
        assert!(second.starts_with(first.trim()));
        assert!(second.contains("[#1] user: next"));
    }
}
