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

/// What the user said about checking the result of the work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CheckBan {
    /// Nothing forbidden (silent, optional wording, or an explicit request to run or check).
    #[default]
    None,
    /// Running the result is forbidden ("do not run it"); reading it back is still allowed.
    Run,
    /// Any check of the result is forbidden ("do not verify", "no review").
    Check,
}

const BAN_NEG: &str = r"do not|don't|never|must not|should not|shouldn't|please don't";

fn run_ban_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(&format!(
            r"\b(?:(?:{BAN_NEG})\s+(?:run|execute|test)\b|without (?:running|executing|testing)\b)"
        ))
        .unwrap()
    })
}

fn check_ban_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(&format!(
            r"\b(?:(?:{BAN_NEG})\s+(?:verify|review|check(\s+(?:in|out)\b)?)\b|without (?:verifying|checking|reviewing)\b|\bno (?:verification|checking|review)\b)"
        ))
        .unwrap()
    })
}

/// "run it", "go ahead and test it": an explicit request that overrides an earlier ban.
fn allow_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(r"\b(?:run|execute|test|verify|check)\s+(?:it|this|that|them|the (?:file|script|code|program|result|tests?))\b").unwrap()
    })
}

/// Text in double quotes, backticks or code blocks is content, not an instruction.
fn strip_quoted(text: &str) -> String {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r#"(?s)```.*?```|`[^`]*`|"[^"]*"|\u{201C}[^\u{201D}]*\u{201D}"#).unwrap())
        .replace_all(text, " ")
        .into_owned()
}

/// One user message's stance: `Some(ban)` when it forbids or explicitly asks for a run or check
/// (`Some(CheckBan::None)`), `None` when it says nothing about it. "no need to test" and
/// "you may skip tests" are optional, not a ban, so they say nothing.
pub fn check_stance(text: &str) -> Option<CheckBan> {
    let t = normalise(&strip_quoted(&text.replace(['\u{2019}', '\u{2018}', '\u{02bc}'], "'")));
    // "do not check in / check out" is about version control, not about checking the result.
    if check_ban_re().captures_iter(&t).any(|c| c.get(1).is_none()) {
        Some(CheckBan::Check)
    } else if run_ban_re().is_match(&t) {
        Some(CheckBan::Run)
    } else if allow_re().is_match(&t) {
        Some(CheckBan::None)
    } else {
        None
    }
}

/// The effective ban for the turn: user messages newest first; the newest one with a stance
/// decides, so a later correction wins over the first task text.
pub fn check_ban<'a>(newest_first: impl IntoIterator<Item = &'a str>) -> CheckBan {
    newest_first.into_iter().find_map(check_stance).unwrap_or_default()
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

#[cfg(test)]
mod check_ban_tests {
    use super::{check_ban, check_stance, CheckBan};

    #[test]
    fn check_ban_phrases() {
        let cases = [
            ("Write one file with the file tool, do not run it, reply DONE", Some(CheckBan::Run)),
            ("Don't execute anything, just write the script.", Some(CheckBan::Run)),
            ("Don\u{2019}t run it.", Some(CheckBan::Run)),
            ("Write the parser without running it.", Some(CheckBan::Run)),
            ("Do not test it yet.", Some(CheckBan::Run)),
            ("Do not verify the output.", Some(CheckBan::Check)),
            ("Do not check or review it, just write it.", Some(CheckBan::Check)),
            ("No verification needed.", Some(CheckBan::Check)),
            // Optional wording is not a ban ("test it" reads as allowing a test, which bans nothing).
            ("No need to test it.", Some(CheckBan::None)),
            ("No need to test.", None),
            ("You may skip tests.", None),
            // Must not match as a ban.
            ("Do not stop until it runs.", None),
            ("Run it without arguments.", Some(CheckBan::None)),
            ("Never mind, run it.", Some(CheckBan::None)),
            ("Do not check in the build folder.", None),
            ("Write the code and run the tests.", Some(CheckBan::None)),
            ("Fix the bug.", None),
            // Quoted text and code are content, not instructions.
            ("Add a README line that says \"do not run it as root\".", None),
            ("Print `don't execute` on error.", None),
            ("Use this banner:\n```\nDo not verify\n```", None),
            ("", None),
        ];
        for (text, want) in cases {
            assert_eq!(check_stance(text), want, "{text:?}");
        }
    }

    #[test]
    fn the_newest_message_with_a_stance_decides() {
        assert_eq!(check_ban(["Also add a docstring.", "Write it, do not run it."]), CheckBan::Run);
        assert_eq!(check_ban(["Actually, run it now.", "Write it, do not run it."]), CheckBan::None);
        assert_eq!(check_ban(["Do not verify anything.", "Write it and run it."]), CheckBan::Check);
        assert_eq!(check_ban(Vec::<&str>::new()), CheckBan::None);
    }
}
