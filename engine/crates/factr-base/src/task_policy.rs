//! What a task's own text forbids. One detector shared by the stop-nudge chain, the turn deadline
//! reminders and the environment snapshot, so every guard respects "do not use tools or code".

use regex::Regex;
use std::sync::OnceLock;

const NEG: &str = r"do not|don't|never|without|no|not allowed to|must not|may not|cannot|can't";
const VERB: &str = r"use|using|run|running|call|calling|execute|executing";
const OBJ: &str = r"tool calls?|tool use|tools?|code|python|shell|bash|terminal|calculator|repl|scripts?";
/// A word right after the object that makes it a noun phrase about something else ("no code changes").
const NOT_A_BAN: [&str; 7] = ["change", "changes", "duplication", "review", "directory", "dir", "folder"];

fn ban_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(&format!(
            r"\b(?:{NEG})\b\s+(?:[\w']+\s+){{0,4}}?(?:(?:{VERB})\s+(?:[\w']+\s+){{0,3}}?)?(?:{OBJ})\b"
        ))
        .unwrap()
    })
}

fn standalone_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(r"\b(?:by hand|in your head|mentally|from memory)\b|\btools? (?:are|is) not (?:allowed|permitted)\b").unwrap()
    })
}

/// Lowercase, one apostrophe form, `dont` spelled `don't`, single spaces.
fn normalise(text: &str) -> String {
    static DONT: OnceLock<Regex> = OnceLock::new();
    let t = text.to_lowercase().replace(['\u{2019}', '\u{2018}', '\u{02bc}'], "'");
    let t = DONT.get_or_init(|| Regex::new(r"\bdont\b").unwrap()).replace_all(&t, "don't");
    t.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// True when the task text tells the agent not to use tools or code (by hand, in its head, no tool calls ...).
/// Pass the first user message only.
pub fn forbids_tools(task: &str) -> bool {
    let t = normalise(task);
    if standalone_re().is_match(&t) {
        return true;
    }
    ban_re().find_iter(&t).any(|m| {
        let rest = &t[m.end()..];
        if rest.starts_with('/') {
            return false;
        }
        !rest
            .split_whitespace()
            .take(2)
            .any(|w| NOT_A_BAN.contains(&w.trim_matches(|c: char| !c.is_alphanumeric())) || w.starts_with("installed"))
    })
}

/// The gating on top of the detector (`FACTR_GUARD_NOTOOLS=0` turns it off for ablation).
pub fn gating_on() -> bool {
    std::env::var("FACTR_GUARD_NOTOOLS").map_or(true, |v| v != "0")
}

#[cfg(test)]
mod tests {
    use super::forbids_tools;

    #[test]
    fn detector_corpus() {
        let yes = [
            "Do not use any tools or code.",
            "Please don't use any tools.",
            "dont use tools",
            "Don\u{2019}t use tools here",
            "Don\u{2018}t use tools here",
            "no tool use",
            "No tool calls please",
            "do not run any code",
            "Answer without running code.",
            "do not call any tools",
            "Answer without using any tools or code.",
            "Tools are not allowed.",
            "The tool is not permitted",
            "no code or tools",
            "Use no tools",
            "Do NOT use tools",
            "without tools",
            "without a calculator",
            "Work it out by hand.",
            "Do it in your head",
            "solve it mentally",
            "answer from memory",
            "You must not use the shell",
            "never   use\nbash",
        ];
        let no = [
            "Fix the bug by handling the error case.",
            "Add a field in your header file.",
            "no code changes needed",
            "Refactor without code duplication.",
            "There is no tools/ directory",
            "no tools directory here",
            "Write a function and run the tests.",
            "Use the tools and code available to solve it.",
            "Install it, no review needed.",
            "",
        ];
        for t in yes {
            assert!(forbids_tools(t), "should forbid: {t}");
        }
        for t in no {
            assert!(!forbids_tools(t), "should allow: {t}");
        }
    }
}
