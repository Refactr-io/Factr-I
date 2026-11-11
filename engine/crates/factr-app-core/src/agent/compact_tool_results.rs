//! The model-facing copy of edit-tool results (`LoopShape::compact_write_result`). The tools
//! append the full unified diff (`file_diff::attach`) for clients; for a new file that is the
//! whole file a second time. Applied to the owned outbound copy of each request only: the
//! session, the `ToolDone` event and every parser of the result text keep the full text.

use super::stop_nudge::EDIT_TOOLS;
use crate::message::{ContentBlock, Message};
use std::collections::HashMap;

/// Exactly what `file_diff::attach` appends around the diff.
const DIFF_OPEN: &str = "\n\nFile diff:\n```diff\n";
const DIFF_CLOSE: &str = "```\n";
/// True for `write`: it writes the bytes it was given (`tokio::fs::write`, no rewriting).
pub(super) const SAVED_NOTE: &str = "\n\nSaved as sent; no need to read it back just to confirm the save.";

/// Pure and deterministic: the same history always gives the same text, so the cached
/// prefix does not shift from one call to the next.
pub(super) fn compact_write_results(messages: &mut [Message]) {
    // ToolResult blocks carry no tool name: join each to its ToolUse by call id.
    let calls: HashMap<String, (String, bool)> = messages
        .iter()
        .flat_map(|m| &m.content)
        .filter_map(|b| match b {
            ContentBlock::ToolUse { id, name, input, .. } => {
                Some((id.clone(), (name.clone(), input["dry_run"].as_bool() == Some(true))))
            }
            _ => None,
        })
        .collect();
    for block in messages.iter_mut().flat_map(|m| &mut m.content) {
        let ContentBlock::ToolResult { tool_use_id, content, is_error } = block else { continue };
        let Some((name, dry_run)) = calls.get(tool_use_id.as_str()) else { continue };
        // Failed results, previews (replace dry_run) and every other tool are left as they are.
        if *is_error == Some(true) || *dry_run || !EDIT_TOOLS.contains(&name.as_str()) {
            continue;
        }
        if let Some(cut) = without_diff(content) {
            content.truncate(cut);
        }
        if name == "write" && (content.starts_with("Created ") || content.starts_with("Updated ")) {
            content.push_str(SAVED_NOTE);
        }
    }
}

/// Where the appended diff section starts, when the text ends with exactly that section.
/// A unified diff has no empty lines, so the last `DIFF_OPEN` is the one `attach` wrote.
fn without_diff(text: &str) -> Option<usize> {
    let start = text.rfind(DIFF_OPEN)?;
    let diff = text[start + DIFF_OPEN.len()..].strip_suffix(DIFF_CLOSE)?;
    diff.starts_with("--- ").then_some(start)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::Role;

    fn call(id: &str, name: &str, input: serde_json::Value) -> Message {
        Message {
            role: Role::Assistant,
            content: vec![ContentBlock::ToolUse { id: id.into(), name: name.into(), input, thought_signature: None }],
            timestamp: None,
            tool_duration_ms: None,
        }
    }

    fn result(id: &str, text: &str, is_error: Option<bool>) -> Message {
        Message {
            role: Role::User,
            content: vec![ContentBlock::ToolResult { tool_use_id: id.into(), content: text.into(), is_error }],
            timestamp: None,
            tool_duration_ms: None,
        }
    }

    fn text_of(m: &Message) -> &str {
        match &m.content[0] {
            ContentBlock::ToolResult { content, .. } => content,
            _ => unreachable!(),
        }
    }

    const DIFF: &str = "\n\nFile diff:\n```diff\n--- /dev/null\n+++ b/a.py\n@@ -0,0 +1 @@\n+print()\n```\n";

    #[test]
    fn only_the_appended_diff_of_a_successful_edit_tool_result_goes() {
        let created = "Created a.py (1 lines):\n1+ print()";
        let edited = "Edited a.py: replaced 1 occurrence\n2- x\n2+ y";
        let batch = format!("--- [1] write ---\n{created}{DIFF}--- [2] bash ---\nok");
        let mut messages = vec![
            call("w", "write", serde_json::json!({"file_path": "a.py"})),
            result("w", &format!("{created}{DIFF}"), None),
            call("e", "edit", serde_json::json!({})),
            result("e", &format!("{edited}{DIFF}"), None),
            call("d", "replace", serde_json::json!({"dry_run": true})),
            result("d", &format!("Would replace 1 match{DIFF}"), None),
            call("f", "write", serde_json::json!({})),
            result("f", &format!("Error: disk full{DIFF}"), Some(true)),
            call("b", "batch", serde_json::json!({})),
            result("b", &batch, None),
            result("unknown", &format!("{created}{DIFF}"), None),
            call("t", "write", serde_json::json!({})),
            result("t", &format!("{created}\n\nFile diff:\n```diff\n--- /dev/null\n+++ b/a.py\n@@ -0,0 +1 @@\n+pri"), None),
        ];
        let original = messages.clone();
        compact_write_results(&mut messages);
        assert_eq!(text_of(&messages[1]), format!("{created}{SAVED_NOTE}"));
        assert_eq!(text_of(&messages[3]), edited, "no save note on edit");
        assert_eq!(text_of(&messages[5]), text_of(&original[5]), "a dry run's diff is the point");
        assert_eq!(text_of(&messages[7]), text_of(&original[7]), "failed result untouched");
        assert_eq!(text_of(&messages[9]), text_of(&original[9]), "batch untouched");
        assert_eq!(text_of(&messages[10]), text_of(&original[10]), "no matching call");
        // A cut-off diff (history cap) is not the exact section: only the note is added.
        assert_eq!(text_of(&messages[12]), format!("{}{SAVED_NOTE}", text_of(&original[12])));
        // Deterministic: a second pass over the original gives the same text.
        let mut again = original.clone();
        compact_write_results(&mut again);
        assert_eq!(
            serde_json::to_string(&again).unwrap(),
            serde_json::to_string(&messages).unwrap()
        );
    }
}
