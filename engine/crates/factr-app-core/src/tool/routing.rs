//! Deterministic tool routing (`tools.routing`, off by default).
//!
//! Most tool schemas are withheld until the model asks for them with `load_tools` (see `deferred`).
//! With routing on, a user message that plainly asks for something (a URL, a web search, the browser,
//! subagents, ...) loads the matching withheld tool up front, saving that round trip. It only ever
//! ADDS tools to the set the transcript already implies, so the cached prompt prefix changes at most
//! once per newly routed tool, exactly as with `load_tools`. No model call, no state.

use std::collections::BTreeSet;

/// (tool, phrases that route to it). Phrases are lower-case substrings of the user's text.
const ROUTES: &[(&str, &[&str])] = &[
    ("webfetch", &["http://", "https://", "www."]),
    ("websearch", &["search the web", "web search", "search online", "look it up online", "latest news", "search it online please"]),
    ("browser", &["in the browser", "open a browser", "headless browser", "click the button", "take a screenshot of the page"]),
    ("delegate", &["subagent", "sub-agent", "in parallel agents", "spawn an agent", "spawn agents"]),
    ("todo", &["todo list", "to-do list", "task list", "checklist"]),
    ("replace", &["replace all", "find and replace", "rename across", "replace every"]),
    ("session_search", &["earlier session", "previous session", "last time we", "we discussed", "past conversation"]),
    ("skill_manage", &["create a skill", "edit the skill", "my skills", "list skills"]),
    ("factr", &["cron job", "schedule a task", "generate an image", "generate image", "text to speech"]),
];

/// The withheld tools the given user messages route to. Unknown or empty input routes nothing.
pub(crate) fn routed_tools<'a>(user_texts: impl IntoIterator<Item = &'a str>) -> BTreeSet<&'static str> {
    let mut routed = BTreeSet::new();
    for text in user_texts {
        let lower = text.to_lowercase();
        for (tool, phrases) in ROUTES {
            if phrases.iter().any(|p| lower.contains(p)) {
                routed.insert(*tool);
            }
        }
    }
    routed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_requests_route_to_their_tool() {
        assert_eq!(routed_tools(["please read https://example.com/page"]), BTreeSet::from(["webfetch"]));
        assert!(routed_tools(["Search the web for rust news"]).contains("websearch"));
        assert!(routed_tools(["use subagents for this"]).contains("delegate"));
    }

    #[test]
    fn ordinary_messages_and_empty_input_route_nothing() {
        assert!(routed_tools(["fix the failing test in src/lib.rs"]).is_empty());
        assert!(routed_tools(std::iter::empty::<&str>()).is_empty());
        assert!(routed_tools([""]).is_empty());
    }

    #[test]
    fn every_routed_tool_is_a_deferred_one_and_never_memory() {
        for (tool, _) in ROUTES {
            assert!(crate::tool::deferred::is_deferred(tool), "{tool}");
            assert_ne!(*tool, "memory");
        }
    }
}
