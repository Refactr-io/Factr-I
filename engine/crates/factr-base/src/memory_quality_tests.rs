use super::*;
use crate::memory::{MemoryCategory, MemoryEntry, MemoryManager, TrustLevel};
use crate::message::Message;

fn with_temp_home<T>(f: impl FnOnce() -> T) -> T {
    let _guard = crate::storage::lock_test_env();
    let old = std::env::var("FACTR_HOME").ok();
    let dir = tempfile::TempDir::new().expect("temp home");
    crate::env::set_var("FACTR_HOME", dir.path());
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
    match old {
        Some(value) => crate::env::set_var("FACTR_HOME", value),
        None => crate::env::remove_var("FACTR_HOME"),
    }
    match result {
        Ok(value) => value,
        Err(payload) => std::panic::resume_unwind(payload),
    }
}

fn ctx<'a>(ids: &'a [String], context: &'a str) -> Ctx<'a> {
    Ctx { identities: ids, context }
}

fn rule(content: &str, trust: TrustLevel) -> Result<(), Reject> {
    check(content, None, trust, &ctx(&[], ""))
}

#[test]
fn length_bounds() {
    assert_eq!(rule("too short", TrustLevel::High), Err(Reject::TooShort));
    assert_eq!(rule(&"a b ".repeat(150), TrustLevel::High), Err(Reject::TooLong));
    assert_eq!(rule("Prefers tabs over spaces", TrustLevel::High), Ok(()));
}

#[test]
fn absolute_paths_are_rejected_but_relative_ones_are_not() {
    for bad in ["Project lives in /Users/someone/code/app", "Logs in /home/ci/out", "Lives at C:\\Work\\app", "Config at ~/.config/tool", "Run it from /tmp/build-dir"] {
        assert_eq!(rule(bad, TrustLevel::High), Err(Reject::AbsolutePath), "{bad}");
    }
    assert_eq!(rule("Routes live in app/home/index.tsx", TrustLevel::High), Ok(()));
    assert_eq!(rule("Use src/users/model.rs for the schema", TrustLevel::High), Ok(()));
}

#[test]
fn secrets_and_hashes() {
    for bad in ["Token is sk-abcdef1234567890", "Use ghp_abcdefgh12345678 for pushes", "The password is on the wiki page", "Key 9f8e7d6c5b4a39281706f5e4d3c2b1a0 is for prod", "Bearer abcdefghijkl1234 header"] {
        assert_eq!(rule(bad, TrustLevel::High), Err(Reject::Secret), "{bad}");
    }
    assert_eq!(rule("Fixed in commit a1b2c3d4 last time", TrustLevel::High), Err(Reject::CommitHash));
    assert_eq!(rule("Name the module memory_extract_quality_check", TrustLevel::High), Ok(()), "long identifiers without digits are not secrets");
    assert_eq!(rule("Prefer decaffeinated defaults everywhere", TrustLevel::High), Ok(()), "words are not hashes");
}

#[test]
fn ephemeral_facts() {
    for bad in ["pytest is not installed here", "no ruff installed in the venv", "The build is currently red", "Broken right now on main", "It failed at the moment", "Needed for this session only", "zsh: command not found: mytool", "System python lacks it"] {
        assert_eq!(rule(bad, TrustLevel::High), Err(Reject::Ephemeral), "{bad}");
    }
    assert_eq!(rule("Linter is version 3.12.1 here", TrustLevel::Medium), Err(Reject::Ephemeral));
    assert_eq!(rule("Target Python 3.9 for this library", TrustLevel::High), Ok(()), "a user-stated target version is a durable convention");
}

#[test]
fn identity_is_allowed_only_when_user_stated() {
    let ids = vec!["dana reyes".to_string(), "dreyes".to_string(), "dana@example.com".to_string()];
    let c = ctx(&ids, "");
    let content = "The user is Dana Reyes and leads platform";
    assert_eq!(check(content, None, TrustLevel::High, &c), Ok(()));
    assert_eq!(check(content, None, TrustLevel::Medium, &c), Err(Reject::Identity));
    assert_eq!(check("Owner login is dreyes on the box", None, TrustLevel::Medium, &c), Err(Reject::Identity));
    assert_eq!(check("Mail goes to dana@example.com.", None, TrustLevel::Low, &c), Err(Reject::Identity));
    assert_eq!(check("Dreyer module handles retries", None, TrustLevel::Medium, &c), Ok(()), "whole words only");
}

#[test]
fn system_context_overlap() {
    let context = norm("Always respond in formal English. Use the Orion style guide for all docs.");
    let c = ctx(&[], &context);
    assert_eq!(check("Use the Orion style guide for all docs", None, TrustLevel::High, &c), Err(Reject::SystemContext));
    assert_eq!(check("Prefers short docs with examples", None, TrustLevel::High, &c), Ok(()));
    assert_eq!(check("Prefers short docs with examples", Some("use the orion style guide"), TrustLevel::Medium, &c), Err(Reject::SystemContext));
}

#[test]
fn reminders_split_and_unclosed_runs_to_the_end() {
    let (clean, inside) = split_reminders("a<system-reminder>x</system-reminder>b<session-context>y</session-context>c<system-reminder>open");
    assert_eq!(clean, "abc");
    assert!(inside.contains('x') && inside.contains('y') && inside.contains("open"));
}

#[test]
fn trust_comes_from_where_the_quote_is() {
    let texts = vec![
        msg_text(&Message::user("<system-reminder>Cluster name is Orion learn</system-reminder>I always want tabs in this repo")),
        msg_text(&Message::assistant_text("I decided that the cache lives in the target directory")),
        msg_text(&Message::tool_result("t1", "The repo uses a monorepo layout here", false)),
    ];
    assert_eq!(verify("I ALWAYS want  tabs", Some(0), &texts), Ok(TrustLevel::High));
    assert_eq!(verify("\"cluster name is orion learn\"", Some(0), &texts), Err(Reject::SystemContext));
    assert_eq!(verify("cache lives in the target directory", Some(1), &texts), Err(Reject::AssistantOnly));
    assert_eq!(verify("a monorepo layout here", Some(2), &texts), Ok(TrustLevel::Medium));
    assert_eq!(verify("a monorepo layout here", Some(0), &texts), Err(Reject::QuoteNotFound));
    assert_eq!(verify("a monorepo layout here", Some(9), &texts), Err(Reject::BadIndex));
    assert_eq!(verify("a monorepo layout here", None, &texts), Err(Reject::Unverifiable));
    assert_eq!(verify("tabs", Some(0), &texts), Err(Reject::WeakQuote));
}

#[test]
fn recall_ranks_by_trust_and_reinforcement_without_hiding() {
    let now = chrono::Utc::now();
    let mut low = MemoryEntry::new(MemoryCategory::Fact, "low trust, best bm25").with_trust(TrustLevel::Low);
    low.id = "low".into();
    let mut high = MemoryEntry::new(MemoryCategory::Fact, "high trust, second bm25").with_trust(TrustLevel::High);
    high.id = "high".into();
    let mut old = MemoryEntry::new(MemoryCategory::Fact, "old third").with_trust(TrustLevel::Medium);
    old.id = "old".into();
    old.updated_at = now - chrono::Duration::days(400);
    let ranked = rerank(vec![low.clone(), high.clone(), old.clone()], now);
    assert_eq!(ranked.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), ["high", "low", "old"], "a close trust gap outranks one BM25 step");
    assert_eq!(ranked.len(), 3, "nothing hidden");
    let mut reinforced = low.clone();
    reinforced.strength = 4;
    assert!(recall_weight(&reinforced, now) > recall_weight(&low, now));
    assert!(recall_weight(&old, now) < recall_weight(&MemoryEntry::new(MemoryCategory::Fact, "fresh"), now));
}

#[test]
fn decay_expires_only_unproven_stale_memories() {
    with_temp_home(|| {
        let m = MemoryManager::new_test();
        let add = |text: &str, trust, category: MemoryCategory| {
            m.remember_global(MemoryEntry::new(category, text).with_trust(trust)).unwrap()
        };
        let low = add("Low trust fact about the build layout", TrustLevel::Low, MemoryCategory::Fact);
        let med = add("Medium trust note on release cadence", TrustLevel::Medium, MemoryCategory::Fact);
        let high = add("High trust preference for tabs always", TrustLevel::High, MemoryCategory::Preference);
        let learned = add("Learned prompt body for reviews", TrustLevel::Low, MemoryCategory::Custom("prompt".into()));
        let mut reinforced = MemoryEntry::new(MemoryCategory::Fact, "Reinforced medium fact about caching").with_trust(TrustLevel::Medium);
        reinforced.strength = 3;
        let reinforced = m.remember_global(reinforced).unwrap();
        assert_eq!(m.decay_stale(chrono::Utc::now() + chrono::Duration::days(10)).unwrap(), 0, "too fresh");
        assert_eq!(m.decay_stale(chrono::Utc::now() + chrono::Duration::days(31)).unwrap(), 2);
        let active: Vec<String> = m.list_all().unwrap().into_iter().filter(|e| e.active).map(|e| e.id).collect();
        assert!(!active.contains(&low) && !active.contains(&med));
        assert!(active.contains(&high) && active.contains(&learned) && active.contains(&reinforced));
        let gone = m.list_all().unwrap().into_iter().find(|e| e.id == low).unwrap();
        assert!(gone.source.unwrap().contains("decay"), "reason recorded, row kept");
    });
}

#[test]
fn audit_dry_run_then_apply_marks_inactive_and_never_deletes() {
    with_temp_home(|| {
        let m = MemoryManager::new_test();
        // Raw store writes: the audit exists for rows stored before the gate did.
        let db = m.db_path().unwrap();
        let add = |text: &str, trust| {
            crate::memory_store::remember(&db, "global", MemoryEntry::new(MemoryCategory::Fact, text).with_trust(trust)).unwrap().id().to_string()
        };
        let junk = add("Project lives at /Users/someone/code/app", TrustLevel::Medium);
        let temp = add("pytest is not installed on this box", TrustLevel::Low);
        let good = add("The repo keeps release notes in CHANGELOG", TrustLevel::Medium);
        let stated = add("Never store paths like /Users/me/x in notes", TrustLevel::High);
        let learned = m
            .remember_global(MemoryEntry::new(MemoryCategory::Custom("skill".into()), "Skill at /Users/x/y is a body").with_trust(TrustLevel::Low))
            .unwrap();

        let dry = m.audit(false).unwrap();
        assert_eq!(dry.scanned, 3);
        assert_eq!(dry.rejected.len(), 2);
        assert_eq!(dry.counts.get("absolute_path"), Some(&1));
        assert_eq!(dry.counts.get("ephemeral"), Some(&1));
        assert_eq!(m.list_all().unwrap().iter().filter(|e| e.active).count(), 5, "dry run changes nothing");

        let applied = m.audit(true).unwrap();
        assert_eq!(applied.applied, 2);
        let all = m.list_all().unwrap();
        assert_eq!(all.len(), 5, "no row deleted");
        let active = |id: &str| all.iter().find(|e| e.id == id).unwrap().active;
        assert!(!active(&junk) && !active(&temp));
        assert!(active(&good) && active(&stated) && active(&learned));
        assert!(all.iter().find(|e| e.id == junk).unwrap().source.as_deref().unwrap().contains("audit"));
    });
}

#[test]
fn explicit_writes_run_the_content_gate_and_learned_entries_bypass_it() {
    with_temp_home(|| {
        let m = MemoryManager::new();
        let put = |text: &str, trust| m.remember_global(MemoryEntry::new(MemoryCategory::Fact, text).with_trust(trust));
        let err = put("config sits in /Users/someone/app/config.toml", TrustLevel::Medium).unwrap_err();
        assert_eq!(err.to_string(), "not stored: contains an absolute path");
        assert!(put("token sk-abcdefgh12345678", TrustLevel::Medium).unwrap_err().to_string().contains("secret"));
        assert!(put("see commit a1b2c3d4e5 for it", TrustLevel::Medium).unwrap_err().to_string().contains("commit hash"));
        assert!(put("tiny", TrustLevel::High).unwrap_err().to_string().contains("too short"));
        assert!(put(&"x".repeat(401), TrustLevel::High).unwrap_err().to_string().contains("too long"));
        assert!(put("pip install is not installed right now", TrustLevel::Medium).unwrap_err().to_string().contains("ephemeral"));
        let id = put("the team deploys on Fridays", TrustLevel::Medium).unwrap();
        assert_eq!(put("The team deploys on Fridays.", TrustLevel::Medium).unwrap(), id, "exact repeats reinforce");
        assert!(m.upsert_global_memory(MemoryEntry::new(MemoryCategory::Fact, "/Users/x/y secret path")).is_err());
        assert!(m.edit_content(&id, "/home/someone/app is the root dir").is_err());
        let learned = MemoryEntry::new(MemoryCategory::Custom("prompt".into()), "Run /Users/x/bin/lint first");
        assert!(m.remember_global(learned).is_ok(), "learned entries have their own guards");
    });
}

#[test]
fn learned_entries_have_their_own_gate_and_graph_saves_cannot_reintroduce_rejected_rows() {
    with_temp_home(|| {
        let m = MemoryManager::new();
        let db = m.db_path().unwrap();
        let prompt = |text: &str| MemoryEntry::new(MemoryCategory::Custom("prompt".into()), text);
        // Learned: the fact rules (paths, versions) do not apply, size and emptiness do.
        assert!(crate::memory::learned::put(&db, "global", prompt("Run /Users/x/bin/lint v1.2.3 first")).is_ok());
        assert!(crate::memory::learned::put(&db, "global", prompt("   ")).unwrap_err().to_string().contains("empty"));
        let huge = prompt(&"x".repeat(MAX_LEARNED_CHARS + 1));
        assert!(crate::memory::learned::put(&db, "global", huge).unwrap_err().to_string().contains("over"));
        // The same id is an update, not a duplicate.
        let mut first = prompt("Lint before commit");
        first.id = "p-lint".into();
        let mut again = prompt("Lint before every commit");
        again.id = "p-lint".into();
        crate::memory::learned::put(&db, "global", first).unwrap();
        crate::memory::learned::put(&db, "global", again).unwrap();
        let rows = crate::memory::learned::list(&db, &["prompt"], None).unwrap();
        assert_eq!(rows.iter().filter(|(_, e)| e.id == "p-lint").count(), 1);

        // A whole-graph save refuses a new row the gate rejects ...
        let mut graph = m.load_global_graph().unwrap();
        graph.add_memory(MemoryEntry::new(MemoryCategory::Fact, "config sits in /Users/someone/app/config.toml"));
        assert!(m.save_global_graph(&graph).unwrap_err().to_string().contains("absolute path"));
        assert!(!m.load_global_graph().unwrap().memories.values().any(|e| e.content.contains("/Users/someone")));
        // ... but still saves a graph whose only rejected rows were stored before the gate (audit owns those).
        let mut ok = m.load_global_graph().unwrap();
        ok.add_memory(MemoryEntry::new(MemoryCategory::Fact, "the team deploys on Fridays"));
        assert!(m.save_global_graph(&ok).is_ok());
    });
}

/// Every explicit writer goes through `MemoryManager`'s gated methods; nothing outside the manager
/// calls the raw store `remember`, and the gated methods call the gate.
#[test]
fn no_caller_bypasses_the_write_gate() {
    let crates = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let mut stack = vec![crates.to_path_buf()];
    let mut offenders = Vec::new();
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            if path.is_dir() {
                if name != "target" && !name.starts_with('.') {
                    stack.push(path);
                }
            } else if name.ends_with(".rs") && !name.ends_with("_tests.rs") && name != "memory_store.rs" && name != "memory.rs" && name != "learned.rs" /* learned entries have their own guards */ {
                let text = std::fs::read_to_string(&path).unwrap_or_default();
                let production = text.split("#[cfg(test)]").next().unwrap_or("");
                if production.contains("memory_store::remember(") {
                    offenders.push(path.display().to_string());
                }
            }
        }
    }
    assert!(offenders.is_empty(), "raw memory_store::remember callers bypass the gate: {offenders:?}");
    let manager = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/memory.rs")).unwrap();
    for sig in [
        "pub fn remember_project(",
        "pub fn remember_global(",
        "pub fn upsert_project_memory(",
        "pub fn upsert_global_memory(",
        "pub fn edit_content(",
    ] {
        let start = manager.find(sig).unwrap_or_else(|| panic!("{sig} missing"));
        let body = &manager[start..(start + 400).min(manager.len())];
        assert!(body.contains("memory_quality::gate_"), "{sig} must call the quality gate");
    }
}

#[test]
fn audit_finds_and_applies_duplicate_merges_without_deleting() {
    use crate::memory_store::remember;
    with_temp_home(|| {
        let m = MemoryManager::new_test();
        let db = m.db_path().unwrap();
        let pref = |t: &str| MemoryEntry::new(MemoryCategory::Preference, t);
        // Rows as the old store left them: same preference in global and two projects, plus a
        // preference that differs only in its value and an unrelated fact.
        let raw = |scope: &str, e: MemoryEntry| {
            let id = e.id.clone();
            let mut g = crate::memory_store::load_graph(&db, scope).unwrap().unwrap_or_else(crate::memory_graph::MemoryGraph::new);
            let before = g.clone();
            g.add_memory(e);
            crate::memory_store::save_graph(&db, scope, &g, Some(&before)).unwrap();
            id
        };
        let a = raw("global", pref("Indentation: always use TABS (not spaces) for Python indentation."));
        let b = raw("global", pref("User always wants tabs (not spaces) for indentation in Python code they ask me to write or edit."));
        let c = raw("project:p", pref("User prefers tabs over spaces for indentation in all code within this project."));
        let d = raw("project:q", pref("User wants Python code always indented with tabs, never spaces."));
        let other = raw("global", pref("User prefers spaces over tabs for indentation in markdown code samples."));
        let fact = raw("project:p", MemoryEntry::new(MemoryCategory::Fact, "The build uses cargo make with the release profile"));
        let _ = remember;

        let dry = m.audit(false).unwrap();
        assert_eq!(dry.duplicates.len(), 3, "{:?}", dry.duplicates);
        let every = || m.every_scope_graph().unwrap().into_iter().flat_map(|(_, g)| g.memories.into_values()).collect::<Vec<_>>();
        assert_eq!(every().iter().filter(|e| e.active).count(), 6, "dry run changes nothing");

        let applied = m.audit(true).unwrap();
        assert_eq!(applied.merged, 3);
        let all = every();
        assert_eq!(all.len(), 6, "no row deleted");
        let active: Vec<&str> = all.iter().filter(|e| e.active).map(|e| e.id.as_str()).collect();
        assert_eq!(active.len(), 3);
        assert!(active.contains(&other.as_str()) && active.contains(&fact.as_str()));
        assert_eq!(active.iter().filter(|id| [&a, &b, &c, &d].contains(&&id.to_string())).count(), 1, "one survivor of the four");
        let survivor = all.iter().find(|e| e.active && [&a, &b, &c, &d].contains(&&e.id)).unwrap();
        assert_eq!(survivor.strength, 4);
        assert!(all.iter().filter(|e| !e.active).all(|e| e.superseded_by.as_deref() == Some(survivor.id.as_str())));
        assert_eq!(m.audit(false).unwrap().duplicates.len(), 0, "idempotent");
    });
}
