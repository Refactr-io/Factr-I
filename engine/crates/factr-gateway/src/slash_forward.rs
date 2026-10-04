//! The one slash-command table: canonical names, aliases and who serves each command. The popup
//! (`commands.catalog`, `complete.slash`), `slash.exec` and `command.dispatch` all read it, so a
//! listed command is always one that is handled and no name is defined twice.
//!
//! Names and aliases follow Factr's registry (`factr_backend/commands.py`), the source of truth:
//! `/compress` (alias `/compact`), `/branch` (alias `/fork`). Where the desktop client's own table
//! names an extra alias for a command (`/commands`, `/sessions`, `/switch`, `/learning`,
//! `/memory-graph`) it is carried here once, so the client keeps no table of names of its own.
//! Engine-only commands (`/redo`, `/harness`, `/journey`) have no Factr twin. A command the table
//! does not name is forwarded to the Factr backend (skill, quick, plugin and bundle commands).

use serde_json::{Value, json};

/// Who handles a command.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Route {
    /// `Conn::engine_slash`: history rewind, checkpoints, compaction, branching, steering.
    Engine,
    /// `Conn::harness_command`: goals, refine, heartbeat (the Continual Harness).
    Harness,
    /// The desktop client runs it (an action, picker or dedicated RPC the engine answers).
    Desktop,
    /// A Factr prompt builder, forwarded to the Python backend (needs no Python session).
    Factr,
}

pub(crate) struct Cmd {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub desc: &'static str,
    pub args: &'static str,
    pub subs: &'static [&'static str],
    pub route: Route,
    /// Needs the Factr Python backend to do its job; unlisted when none is configured.
    pub python: bool,
    /// In Factr's registry under this name and aliases (false: engine-only).
    pub factr: bool,
    pub category: &'static str,
}

const fn cmd(name: &'static str, aliases: &'static [&'static str], desc: &'static str, args: &'static str, route: Route, category: &'static str) -> Cmd {
    Cmd { name, aliases, desc, args, subs: &[], route, python: false, factr: true, category }
}

impl Cmd {
    const fn subs(mut self, subs: &'static [&'static str]) -> Self {
        self.subs = subs;
        self
    }
    const fn python(mut self) -> Self {
        self.python = true;
        self
    }
    const fn engine_only(mut self) -> Self {
        self.factr = false;
        self
    }
}

use Route::{Desktop, Engine, Harness, Factr};

pub(crate) const COMMANDS: &[Cmd] = &[
    // Session
    cmd("new", &["reset"], "Start a new session (fresh session ID + history)", "[name]", Desktop, "Session"),
    cmd("stop", &[], "Stop the active turn and background processes", "", Desktop, "Session"),
    cmd("retry", &[], "Rewind the last user turn and send it again", "", Engine, "Session"),
    cmd("undo", &[], "Back up N user turns (default 1): chat and the files the agent changed", "[N|chat|files]", Engine, "Session").subs(&["chat", "files"]),
    cmd("redo", &[], "Re-apply the last undo (chat and files)", "", Engine, "Session").engine_only(),
    cmd("rollback", &[], "List file checkpoints; diff N previews, N [file] restores", "[diff N | N [file]]", Engine, "Session").subs(&["diff"]),
    cmd("title", &[], "Set a title for the current session", "[name]", Desktop, "Session"),
    cmd("branch", &["fork"], "Branch the current session into a new one", "[name]", Engine, "Session"),
    cmd("compress", &["compact"], "Compress conversation context", "", Engine, "Session"),
    cmd("steer", &[], "Inject a message after the next tool call without interrupting", "<prompt>", Engine, "Session"),
    cmd("btw", &[], "Ask a side question about the conversation without interrupting it", "<question>", Desktop, "Session"),
    cmd("save", &[], "Export the current conversation", "<json|md|html> [filename]", Desktop, "Session"),
    cmd("status", &[], "Show session, model, token, and context info", "", Desktop, "Session"),
    cmd("resume", &["sessions", "switch"], "Resume a previously-named session", "[name]", Desktop, "Session"),
    cmd("queue", &["q"], "Queue a prompt for the next turn", "<prompt>", Factr, "Session").python(),
    cmd("plan", &[], "Write a markdown implementation plan without executing anything", "[task]", Factr, "Session").python(),
    // Harness (Continual Harness and unattended work)
    cmd("goal", &[], "Set a standing goal worked on across turns until achieved", "[text | draft <text> | show | gate add <cmd> | pause | resume | clear | status | wait <pid> | unwait] [--budget N] [--turns N] [--gate <cmd>]", Harness, "Harness")
        .subs(&["draft", "show", "gate", "pause", "resume", "clear", "status", "wait", "unwait"]),
    cmd("subgoal", &[], "Add, remove or clear extra criteria on the active goal", "[text | remove N | clear]", Harness, "Harness").subs(&["remove", "clear"]),
    cmd("loop", &["proactive", "heartbeat", "hb"], "Re-run a prompt on a recurring interval in this session", "[every] <interval> <prompt> [--times N] [--until <condition>] | status | pause | resume | stop", Harness, "Harness")
        .subs(&["status", "pause", "resume", "stop"]),
    cmd("refine", &[], "Propose evidence-backed Continual Harness edits from this session", "[--global | status | rollback [id]]", Harness, "Harness")
        .subs(&["--global", "status", "rollback"]),
    cmd("harness", &[], "Show the learned instructions", "", Harness, "Harness").engine_only(),
    cmd("init", &[], "Generate or update AGENTS.md project instructions from a repo scan", "[notes]", Factr, "Tools & Skills").python(),
    // Configuration and info
    cmd("yolo", &[], "Toggle YOLO mode (skip all dangerous command approvals)", "", Desktop, "Configuration"),
    cmd("reasoning", &[], "Manage reasoning effort and display", "[level|show|hide|full|clamp] [--global]", Desktop, "Configuration"),
    cmd("focus", &[], "Toggle focus view: show only your prompt and the final response", "[on|off|status]", Engine, "Configuration").subs(&["on", "off", "status"]),
    cmd("skin", &[], "Switch between white and black appearance", "[name]", Desktop, "Configuration"),
    cmd("help", &["commands"], "Show available commands", "", Desktop, "Info"),
    cmd("journey", &["learning", "memory-graph"], "Open the memory graph: skills and memories over time", "", Desktop, "Tools & Skills").engine_only(),
    // Desktop actions backed by the Factr Python backend
    cmd("handoff", &[], "Hand off this session to a messaging platform", "<platform>", Desktop, "Session").python(),
    cmd("wake", &[], "Control the wake-word listener", "[on|off|status]", Desktop, "Configuration").python().subs(&["on", "off", "status"]),
    cmd("profile", &[], "Switch the active Factr-I profile", "", Desktop, "Info").python(),
    cmd("browser", &[], "Manage the browser CDP connection", "[connect|disconnect|status]", Desktop, "Tools & Skills").python().subs(&["connect", "disconnect", "status"]),
    cmd("pet", &[], "Toggle or adopt a petdex mascot", "[list|<slug>]", Desktop, "Tools & Skills").python(),
    cmd("hatch", &["generate-pet"], "Generate a new petdex pet from a description", "[description]", Desktop, "Tools & Skills").python(),
];

/// Factr built-ins this engine does not serve yet. Typed, they get one clear line instead of a
/// round trip to a backend that cannot run them against engine sessions.
const UNSERVED: &[&str] = &["moa", "curator", "skills", "snapshot", "snap"];

/// The reply for a typed command that is deliberately not served, `None` for anything else.
pub(crate) fn unserved(name: &str) -> Option<String> {
    let n = norm(name);
    if n == "learn" {
        return Some("Learning is automatic; see /harness.".into());
    }
    // Factr's memory write-approval queue (/memory pending|approve|reject|approval) is replaced by
    // the code quality gate on the one memory store: nothing waits for approval.
    if n == "memory" {
        return Some(crate::memory_rest::summary_line());
    }
    UNSERVED.contains(&n.as_str()).then(|| format!("/{n} is not available yet."))
}

fn norm(name: &str) -> String {
    name.trim().trim_start_matches('/').to_ascii_lowercase()
}

/// The table row for `/name` or any of its aliases.
pub(crate) fn lookup(name: &str) -> Option<&'static Cmd> {
    let n = norm(name);
    COMMANDS.iter().find(|c| c.name == n || c.aliases.contains(&n.as_str()))
}

/// Commands the engine itself must answer (never handed to Factr): its own history, checkpoints
/// and harness, none of which Python's store has seen.
pub(crate) fn engine_serves(name: &str) -> bool {
    lookup(name).is_some_and(|c| matches!(c.route, Route::Engine | Route::Harness))
}

fn args_mode(c: &Cmd) -> Value {
    match (c.args.is_empty(), c.subs.is_empty()) {
        (true, _) => Value::Null,
        (false, true) => json!("text"),
        (false, false) => json!("mixed"),
    }
}

fn describe(c: &Cmd) -> String {
    if c.args.is_empty() { c.desc.to_string() } else { format!("{} (usage: /{} {})", c.desc, c.name, c.args) }
}

fn listed(c: &Cmd, python: bool) -> bool {
    python || !c.python
}

/// `commands.catalog`, answered natively. `skills` are `(name, description)` rows offered as
/// `/name` skill commands; pass none when no Factr backend can run them.
pub(crate) fn catalog(python: bool, skills: &[(String, String)]) -> Value {
    let mut pairs: Vec<Value> = Vec::new();
    let mut categories: Vec<(&str, Vec<Value>)> = Vec::new();
    let (mut canon, mut commands, mut sub) = (serde_json::Map::new(), serde_json::Map::new(), serde_json::Map::new());
    for c in COMMANDS.iter().filter(|c| listed(c, python)) {
        let key = format!("/{}", c.name);
        let row = json!([key, describe(c)]);
        pairs.push(row.clone());
        match categories.iter_mut().find(|(n, _)| *n == c.category) {
            Some((_, rows)) => rows.push(row),
            None => categories.push((c.category, vec![row])),
        }
        canon.insert(key.clone(), json!(key));
        for a in c.aliases {
            canon.insert(format!("/{a}"), json!(key));
        }
        commands.insert(key.clone(), json!({ "argument_mode": args_mode(c), "desktop": null }));
        if !c.subs.is_empty() {
            sub.insert(key, json!(c.subs));
        }
    }
    let mut skill_meta = serde_json::Map::new();
    for (name, desc) in skills {
        let key = format!("/{name}");
        if lookup(name).is_some() || skill_meta.contains_key(&key) {
            continue;
        }
        pairs.push(json!([key, desc]));
        skill_meta.insert(key, json!({ "usage": 0, "origin": null }));
    }
    json!({
        "pairs": pairs, "sub": sub, "canon": canon, "commands": commands,
        "categories": categories.into_iter().map(|(name, pairs)| json!({ "name": name, "pairs": pairs })).collect::<Vec<_>>(),
        "skills": skill_meta, "skill_count": skills.len(), "warning": "",
    })
}

/// Append what only a Factr backend knows (quick commands, plugin commands, skill and bundle
/// commands) to the native `catalog`. Registry rows are never taken from it: the table decides
/// which built-ins are listed. Rows whose name or alias the table already has are skipped.
pub(crate) fn merge_backend(cat: &mut Value, backend: &Value) {
    let extra_cat = |name: &str| matches!(name, "User commands" | "Plugin commands");
    let mut taken: Vec<(String, String, String)> = Vec::new(); // (key, description, category)
    for c in backend["categories"].as_array().into_iter().flatten().filter(|c| extra_cat(c["name"].as_str().unwrap_or_default())) {
        for row in c["pairs"].as_array().into_iter().flatten() {
            if let (Some(k), Some(d)) = (row[0].as_str(), row[1].as_str()) {
                taken.push((k.to_string(), d.to_string(), c["name"].as_str().unwrap_or_default().to_string()));
            }
        }
    }
    let skills = backend["skills"].as_object();
    for row in backend["pairs"].as_array().into_iter().flatten() {
        if let (Some(k), Some(d)) = (row[0].as_str(), row[1].as_str()) {
            if skills.is_some_and(|s| s.contains_key(k)) && !taken.iter().any(|t| t.0 == k) {
                taken.push((k.to_string(), d.to_string(), String::new()));
            }
        }
    }
    for (key, desc, category) in taken {
        let bare = key.trim_start_matches('/');
        let have = |v: &Value| v["pairs"].as_array().is_some_and(|p| p.iter().any(|r| r[0] == key.as_str()));
        if lookup(bare).is_some() || have(cat) {
            continue;
        }
        let row = json!([key, desc]);
        cat["pairs"].as_array_mut().unwrap().push(row.clone());
        cat["canon"][&key] = json!(key);
        if !category.is_empty() {
            let cats = cat["categories"].as_array_mut().unwrap();
            match cats.iter_mut().find(|c| c["name"] == category.as_str()) {
                Some(c) => c["pairs"].as_array_mut().unwrap().push(row),
                None => cats.push(json!({ "name": category, "pairs": [row] })),
            }
        }
        if let Some(meta) = backend["commands"].get(&key).filter(|m| !m.is_null()) {
            cat["commands"][&key] = meta.clone();
        }
        if let Some(meta) = skills.and_then(|s| s.get(&key)) {
            cat["skills"][&key] = meta.clone();
            let n = cat["skills"].as_object().map_or(0, |s| s.len());
            cat["skill_count"] = json!(n);
        }
    }
}

/// Skill command name Factr derives from a skill's name.
pub(crate) fn skill_command(name: &str) -> String {
    name.trim().to_ascii_lowercase().replace([' ', '_'], "-")
}

/// `complete.slash`, answered natively: command names (prefix first, then name or description
/// containing the query), then subcommands once the command is typed and a space follows.
pub(crate) fn complete(text: &str, python: bool, skills: &[(String, String)]) -> Value {
    let Some(body) = text.strip_prefix('/') else { return json!({ "items": [] }) };
    let item = |text: String, meta: &str, kind: &str| json!({ "text": text, "display": text, "meta": meta, "kind": kind });
    if let Some((head, tail)) = body.split_once(char::is_whitespace) {
        let from = text.len() - tail.len();
        let tail_lower = tail.trim_start().to_ascii_lowercase();
        let items: Vec<Value> = lookup(head)
            .filter(|c| listed(c, python))
            .into_iter()
            .flat_map(|c| c.subs.iter())
            .filter(|s| !tail_lower.contains(char::is_whitespace) && s.starts_with(tail_lower.as_str()))
            .map(|s| item((*s).to_string(), "", "command"))
            .collect();
        return json!({ "items": items, "replace_from": from + (tail.len() - tail.trim_start().len()) });
    }
    let q = body.to_ascii_lowercase();
    let rank = |names: &[&str], desc: &str| -> Option<u8> {
        if names.iter().any(|n| n.starts_with(&q)) {
            Some(0)
        } else if names.iter().any(|n| n.contains(&q)) {
            Some(1)
        } else if desc.to_ascii_lowercase().contains(&q) {
            Some(2)
        } else {
            None
        }
    };
    let mut hits: Vec<(u8, usize, Value)> = Vec::new();
    for (i, c) in COMMANDS.iter().enumerate().filter(|(_, c)| listed(c, python)) {
        let names: Vec<&str> = std::iter::once(c.name).chain(c.aliases.iter().copied()).collect();
        if let Some(r) = rank(&names, c.desc) {
            hits.push((r, i, item(format!("/{}", c.name), &describe(c), "command")));
        }
    }
    for (i, (name, desc)) in skills.iter().enumerate().filter(|(_, (n, _))| lookup(n).is_none()) {
        if let Some(r) = rank(&[name.as_str()], desc) {
            hits.push((r, COMMANDS.len() + i, item(format!("/{name}"), desc, "skill")));
        }
    }
    hits.sort_by_key(|(r, i, _)| (*r, *i));
    json!({ "items": hits.into_iter().map(|(_, _, v)| v).collect::<Vec<_>>(), "replace_from": 1 })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn canonical(name: &str) -> String {
        lookup(name).map_or_else(|| norm(name), |c| c.name.to_string())
    }

    fn names_and_aliases() -> Vec<&'static str> {
        COMMANDS.iter().flat_map(|c| std::iter::once(c.name).chain(c.aliases.iter().copied())).collect()
    }

    #[test]
    fn no_duplicate_names_or_aliases() {
        let all = names_and_aliases();
        let unique: HashSet<&str> = all.iter().copied().collect();
        assert_eq!(all.len(), unique.len(), "a name or alias is defined twice: {all:?}");
        for n in &all {
            assert!(!n.is_empty() && !n.contains(char::is_whitespace) && !n.starts_with('/') && *n == n.to_ascii_lowercase(), "{n}");
        }
    }

    #[test]
    fn aliases_resolve_to_one_canonical_name() {
        for (alias, name) in [
            ("compact", "compress"), ("/compress", "compress"), ("fork", "branch"), ("reset", "new"), ("q", "queue"), ("hb", "loop"), ("heartbeat", "loop"), ("proactive", "loop"),
            ("commands", "help"), ("sessions", "resume"), ("switch", "resume"), ("learning", "journey"), ("memory-graph", "journey"),
        ] {
            assert_eq!(canonical(alias), name);
        }
        for gone in ["clone", "autonomous", "autonomous-loop", "snapshot", "snap", "moa", "curator", "skills", "memory"] {
            assert!(lookup(gone).is_none(), "/{gone} has no engine or desktop handler and must not be listed");
        }
        assert_eq!(canonical("my-skill"), "my-skill");
    }

    #[test]
    fn engine_and_harness_names_have_a_handler() {
        let engine = include_str!("rpc/undo_turn.rs");
        let arms = &engine[engine.find("fn engine_slash").expect("engine_slash")..];
        let arms = &arms[..arms.find("\n    }\n").expect("end of engine_slash")];
        let rpc = include_str!("rpc.rs");
        let harness = &rpc[rpc.find("async fn harness_command").expect("harness_command")..];
        let harness = &harness[..harness.find("_ => return None").expect("harness fallthrough")];
        for c in COMMANDS {
            match c.route {
                Route::Engine => assert!(arms.contains(&format!("\"{}\"", c.name)), "/{} is Engine but engine_slash has no arm", c.name),
                Route::Harness => assert!(harness.contains(&format!("[\"{}\"", c.name)) || harness.contains(&format!("\"{}\" |", c.name)), "/{} is Harness but harness_command has no arm", c.name),
                Route::Desktop | Route::Factr => {}
            }
            assert!(!(c.python && matches!(c.route, Route::Engine | Route::Harness)), "/{}", c.name);
        }
        for dead in ["\"autonomous\"", "\"snapshot\"", "\"moa\""] {
            assert!(!harness.contains(dead) && !arms.contains(dead), "{dead} must not have a handler alias");
        }
    }

    /// Aliases the engine catalog owns outright: the desktop no longer carries its own table (it resolves
    /// every alias through `commands.catalog` `canon`), so these exist nowhere else.
    const ENGINE_OWNED_ALIASES: &[&str] = &["heartbeat", "hb", "proactive", "sessions", "switch", "learning", "memory-graph", "commands", "compact", "fork", "reset", "generate-pet"];

    #[test]
    fn names_match_the_factr_registry_and_desktop() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../backend");
        let Ok(registry) = std::fs::read_to_string(root.join("factr_backend/commands.py")) else {
            eprintln!("skipping: factr-backend checkout not found");
            return;
        };
        let desktop = std::fs::read_to_string(root.join("apps/desktop/src/lib/desktop-slash-commands.ts")).unwrap_or_default();
        for c in COMMANDS {
            if c.factr {
                let at = registry.find(&format!("CommandDef(\"{}\"", c.name)).unwrap_or_else(|| panic!("/{} is not in the Factr registry", c.name));
                let def = &registry[at..at + registry[at..].find("),\n").unwrap_or(400).min(600)];
                // An alias is Factr's own, or one the desktop client's table adds (carried here once).
                for a in c.aliases {
                    assert!(def.contains(&format!("\"{a}\"")) || desktop.contains(&format!("'/{a}'")) || ENGINE_OWNED_ALIASES.contains(a), "/{a} is not a Factr or desktop alias of /{}", c.name);
                }
            }
            if c.route == Route::Desktop && !desktop.is_empty() {
                assert!(desktop.contains(&format!("'/{}'", c.name)) || registry.contains(&format!("CommandDef(\"{}\"", c.name)), "/{} is not a desktop command", c.name);
            }
        }
    }

    #[test]
    fn catalog_lists_only_handled_commands_once() {
        let skills = vec![("my-skill".to_string(), "s".to_string()), ("undo".to_string(), "clash".to_string())];
        let cat = catalog(true, &skills);
        let keys: Vec<&str> = cat["pairs"].as_array().unwrap().iter().map(|p| p[0].as_str().unwrap()).collect();
        assert_eq!(keys.len(), keys.iter().collect::<HashSet<_>>().len(), "duplicate catalog entry: {keys:?}");
        for k in &keys {
            assert!(lookup(k).is_some() || *k == "/my-skill", "{k} is listed but nothing handles it");
        }
        assert!(!keys.contains(&"/moa") && !keys.contains(&"/snapshot") && keys.contains(&"/steer"));
        assert_eq!(cat["canon"]["/compact"], "/compress");
        assert_eq!(cat["canon"]["/fork"], "/branch");
        assert_eq!(cat["sub"]["/undo"], json!(["chat", "files"]));
        let cat_total: usize = cat["categories"].as_array().unwrap().iter().map(|c| c["pairs"].as_array().unwrap().len()).sum();
        assert_eq!(cat_total + 1, keys.len(), "categories hold every command; the skill is the uncategorised row");
        let offline = catalog(false, &[]);
        let offline_keys: Vec<&str> = offline["pairs"].as_array().unwrap().iter().map(|p| p[0].as_str().unwrap()).collect();
        assert!(offline_keys.contains(&"/compress") && !offline_keys.contains(&"/handoff") && !offline_keys.contains(&"/pet"));
    }

    #[test]
    fn unserved_built_ins_answer_one_line_and_are_never_listed() {
        for n in UNSERVED {
            assert!(lookup(n).is_none(), "/{n}");
            assert_eq!(unserved(&format!("/{}", n.to_ascii_uppercase())).unwrap(), format!("/{n} is not available yet."));
        }
        assert!(lookup("memory").is_none() && unserved("/memory").unwrap().contains("Settings > Memory"));
        assert_eq!(unserved("/learn").unwrap(), "Learning is automatic; see /harness.");
        assert!(lookup("learn").is_none(), "/learn is not in the table, so not in the popup");
        assert!(unserved("plan").is_none() && unserved("focus").is_none() && unserved("my-skill").is_none());
    }

    #[test]
    fn backend_rows_are_appended_without_duplicates_or_unserved_built_ins() {
        let mut cat = catalog(true, &[("my-skill".to_string(), "own".to_string())]);
        let backend = json!({
            "pairs": [["/plan", "dup"], ["/moa", "built-in"], ["/deploy", "Ship it"], ["/q", "alias"], ["/hello", "plugin"], ["/my-skill", "dup"], ["/bundle", "b"]],
            "categories": [
                { "name": "Session", "pairs": [["/moa", "built-in"]] },
                { "name": "User commands", "pairs": [["/deploy", "Ship it"], ["/q", "alias"], ["/plan", "dup"]] },
                { "name": "Plugin commands", "pairs": [["/hello", "plugin"]] },
            ],
            "commands": { "/hello": { "argument_mode": "text", "desktop": null } },
            "skills": { "/bundle": { "usage": 2, "origin": null }, "/my-skill": { "usage": 1, "origin": null } },
        });
        merge_backend(&mut cat, &backend);
        let keys: Vec<&str> = cat["pairs"].as_array().unwrap().iter().map(|p| p[0].as_str().unwrap()).collect();
        assert_eq!(keys.len(), keys.iter().collect::<HashSet<_>>().len(), "{keys:?}");
        for k in ["/deploy", "/hello", "/bundle", "/my-skill", "/plan", "/queue"] {
            assert!(keys.contains(&k), "{k}");
        }
        assert!(!keys.contains(&"/moa") && !keys.contains(&"/q"), "{keys:?}");
        assert_eq!(cat["commands"]["/hello"]["argument_mode"], "text");
        assert_eq!(cat["skills"]["/bundle"]["usage"], 2);
        let names: Vec<&str> = cat["categories"].as_array().unwrap().iter().map(|c| c["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"User commands") && names.contains(&"Plugin commands"));
    }

    #[test]
    fn complete_ranks_prefix_before_description_and_offers_subcommands() {
        let texts = |v: &Value| -> Vec<String> { v["items"].as_array().unwrap().iter().map(|i| i["text"].as_str().unwrap().to_string()).collect() };
        let c = complete("/co", false, &[]);
        assert_eq!(texts(&c)[0], "/compress");
        assert_eq!(c["replace_from"], 1);
        assert_eq!(texts(&complete("/compa", false, &[])), ["/compress"], "alias prefix lists the canonical name");
        assert_eq!(texts(&complete("/", false, &[])).len(), catalog(false, &[])["pairs"].as_array().unwrap().len());
        let sub = complete("/undo ch", false, &[]);
        assert_eq!(texts(&sub), ["chat"]);
        assert_eq!(sub["replace_from"], 6);
        assert_eq!(texts(&complete("/undo ", false, &[])), ["chat", "files"]);
        assert!(texts(&complete("/wake ", false, &[])).is_empty() && !texts(&complete("/wake ", true, &[])).is_empty());
        assert!(texts(&complete("hello", false, &[])).is_empty());
        let skill = complete("/my", true, &[("my-skill".into(), "d".into())]);
        assert_eq!(skill["items"][0]["kind"], "skill");
    }

    #[test]
    fn engine_owned_names_are_never_forwardable() {
        for name in ["rollback", "/rollback", "undo", "redo", "retry", "compact", "fork", "steer", "goal", "hb", "loop", "proactive"] {
            assert!(engine_serves(name), "{name}");
        }
        assert!(!engine_serves("plan") && !engine_serves("learn") && !engine_serves("rollbacks") && !engine_serves("moa"));
    }
}
