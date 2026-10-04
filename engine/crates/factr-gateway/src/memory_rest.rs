//! Desktop memory REST over factr's memory store: the one memory the agent
//! recalls from. Serves `GET /api/memory` + `POST /api/memory/reset` (the
//! Command Center maintenance rows) and list/add/edit/delete under
//! `/api/memory/entries`. Factr memory providers would be a second store, so
//! `/api/memory/providers/*` is refused instead of forwarded to Python.

use super::{Request, read_body, respond};
use anyhow::{Context, Result};
use factr_base::memory::{MemoryCategory, MemoryEntry, MemoryManager};
use serde_json::{Value, json};
use tokio::net::TcpStream;

/// "user" memories are preferences; everything else is the agent's notes.
fn is_user(entry: &MemoryEntry) -> bool {
    matches!(entry.category, MemoryCategory::Preference)
}

/// Every memory in every scope (global and each project), as recall sees them,
/// tagged with its scope (`global` | `project:<hash>`).
fn scoped() -> Result<Vec<(String, MemoryEntry)>> {
    let mut rows: Vec<_> = MemoryManager::new()
        .every_scope_graph()?
        .into_iter()
        .flat_map(|(scope, graph)| graph.all_memories().map(|m| (scope.clone(), m.clone())).collect::<Vec<_>>())
        .collect();
    rows.sort_by(|a, b| b.1.updated_at.cmp(&a.1.updated_at));
    Ok(rows)
}

fn all() -> Result<Vec<MemoryEntry>> {
    Ok(scoped()?.into_iter().map(|(_, entry)| entry).collect())
}

pub(crate) fn find(id: &str) -> Option<MemoryEntry> {
    all().ok()?.into_iter().find(|m| m.id == id)
}

/// Remove `id` from whichever scope holds it (one row; no other memory is rewritten).
pub(crate) fn forget(id: &str) -> Result<bool> {
    MemoryManager::new().forget_anywhere(id)
}

/// The answer to a typed `/memory`: Factr's write-approval queue does not exist here, because the
/// code quality gate decides what is stored. One line: what is saved, and where to manage it.
pub(crate) fn summary_line() -> String {
    let (mut user, mut notes) = (0, 0);
    for entry in all().unwrap_or_default() {
        if is_user(&entry) { user += 1 } else { notes += 1 }
    }
    format!(
        "Memory is one store with a quality gate, so nothing waits for approval: {user} preferences and {notes} notes are saved. Review, edit or remove them in Settings > Memory."
    )
}

fn status() -> Result<Value> {
    let (mut memory, mut user) = (0, 0);
    for entry in all()? {
        let bytes = entry.content.len();
        if is_user(&entry) { user += bytes } else { memory += bytes }
    }
    Ok(json!({
        "active": "engine memory", "providers": [],
        "builtin_files": { "memory": memory, "user": user },
    }))
}

/// Forget every memory in `target` (`memory` | `user` | `all`); returns the ids cleared. factr-learn's
/// learned entries (prompt, skill, subagent) are never touched: they reset through the learning REST,
/// with a changeset. One scoped DELETE per scope.
fn reset(target: &str) -> Result<Value> {
    let only_preferences = match target {
        "all" => None,
        "user" => Some(true),
        _ => Some(false),
    };
    Ok(json!({ "ok": true, "deleted": MemoryManager::new().reset_all(only_preferences)? }))
}

/// One row for the desktop. factr-learn's learned entries (category `prompt`, `skill` or `subagent`) also
/// carry a `learned` object: its title, where it applies (`global`, `project` or `session` plus the
/// chat's id) and the changeset that made it, which `/refine rollback <changeset id>` undoes.
fn row(scope: &str, entry: &MemoryEntry, changesets: &std::collections::HashMap<String, String>) -> Value {
    let mut row = json!({
        "scope": scope, "id": entry.id, "content": entry.content, "category": entry.category.to_string(),
        "source": entry.source, "active": entry.active, "updated_at": entry.updated_at.timestamp(),
    });
    if entry.category.is_learned() {
        let meta = entry.learned.clone().unwrap_or_default();
        let (applies_to, session) = match scope.strip_prefix("session:") {
            Some(session) => ("session", Some(session)),
            None if scope.starts_with("project:") => ("project", None),
            None => ("global", None),
        };
        row["learned"] = json!({
            "kind": entry.category.to_string(), "title": meta.title, "path": meta.path, "version": meta.version,
            "applies_to": applies_to, "session": session, "changeset": changesets.get(&entry.id),
        });
    }
    row
}

/// `GET /api/memory/entries`: every memory in every scope, learned entries included.
fn entries_json() -> Result<Value> {
    let changesets = factr_base::storage::factr_dir()
        .ok()
        .and_then(|home| factr_learn::entries::EntryStore::open_cached(&home).ok())
        .map(|store| store.changeset_index())
        .unwrap_or_default();
    Ok(json!({ "entries": scoped()?.iter().map(|(scope, m)| row(scope, m, &changesets)).collect::<Vec<_>>() }))
}

pub(crate) fn add(content: &str, category: &str, source: &str) -> Result<String> {
    let mut entry = MemoryEntry::new(category.parse().unwrap_or(MemoryCategory::Fact), content);
    entry.source = Some(source.to_string());
    // A desktop add is a user action: high trust; the content filters still apply (an error reads
    // "not stored: ...").
    entry.trust = factr_base::memory::TrustLevel::High;
    MemoryManager::new().remember_global(entry)
}

/// Replace one memory's text (its row only); `false` when the id is unknown.
pub(crate) fn edit(id: &str, content: &str) -> Result<bool> {
    MemoryManager::new().edit_content(id, content)
}

/// Memories (any scope) no harness entry points at (the model's own `memory` tool
/// writes), for the learning graph.
pub(crate) fn unreferenced(referenced: &std::collections::HashSet<String>) -> Result<Vec<MemoryEntry>> {
    Ok(all()?.into_iter().filter(|m| !referenced.contains(&m.id)).collect())
}

async fn blocking<T: Send + 'static>(work: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(work).await.context("memory worker failed")?
}

pub(super) async fn route(stream: &mut TcpStream, req: &Request) -> Option<Result<()>> {
    let rest = req.path.strip_prefix("/api/memory")?;
    let segments: Vec<&str> = rest.split('/').filter(|s| !s.is_empty()).collect();
    let (method, segments) = (req.method.as_str(), segments.as_slice());
    let body: Value = if matches!(method, "POST" | "PUT") {
        match read_body(stream, req).await {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or(Value::Null),
            Err(err) => return Some(Err(err)),
        }
    } else {
        Value::Null
    };
    let text = body["content"].as_str().unwrap_or_default().trim().to_string();
    let result: Result<Option<Value>> = match (method, segments) {
        ("GET", []) => blocking(|| status().map(Some)).await,
        ("POST", ["reset"]) => {
            let target = body["target"].as_str().unwrap_or("all").to_string();
            blocking(move || reset(&target).map(Some)).await
        }
        ("GET", ["entries"]) => blocking(|| entries_json().map(Some)).await,
        ("POST", ["entries"]) if !text.is_empty() => {
            let category = body["category"].as_str().unwrap_or("fact").to_string();
            blocking(move || Ok(Some(json!({ "ok": true, "id": add(&text, &category, "desktop")? })))).await
        }
        ("PUT", ["entries", id]) if !text.is_empty() => {
            let id = id.to_string();
            blocking(move || Ok(edit(&id, &text)?.then(|| json!({ "ok": true })))).await
        }
        ("POST", ["entries", id, "expire"]) => {
            let id = id.to_string();
            let reason = body["reason"].as_str().unwrap_or_default().to_string();
            blocking(move || {
                let found = MemoryManager::new().expire(&id, &reason)?;
                if found {
                    factr_base::obs_sink::emit(
                        factr_base::obs_sink::Span::new("memory.write").attr("action", "expired").attr("id", id.as_str()),
                    );
                }
                Ok(found.then(|| json!({ "ok": true })))
            })
            .await
        }
        ("DELETE", ["entries", id]) => {
            let id = id.to_string();
            blocking(move || Ok(forget(&id)?.then(|| json!({ "ok": true })))).await
        }
        ("POST" | "PUT", ["entries", ..]) => {
            return Some(respond(stream, "400 Bad Request", &json!({"detail": "content is required"})).await);
        }
        (_, ["providers", ..]) | ("PUT", ["provider"]) => {
            return Some(respond(stream, "404 Not Found", &json!({"detail": "not supported by engine: it has one memory"})).await);
        }
        _ => return None,
    };
    Some(match result {
        Ok(Some(value)) => respond(stream, "200 OK", &value).await,
        Ok(None) => respond(stream, "404 Not Found", &json!({"detail": "memory not found"})).await,
        Err(err) if err.to_string().starts_with("not stored:") => respond(stream, "422 Unprocessable Entity", &json!({"detail": err.to_string()})).await,
        Err(err) => respond(stream, "503 Service Unavailable", &json!({"detail": err.to_string()})).await,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rest_add_and_edit_run_the_quality_gate() {
        let _env = crate::factr_env::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let home = std::env::temp_dir().join(format!("memory-rest-gate-{}", std::process::id()));
        std::fs::create_dir_all(&home).unwrap();
        // SAFETY: FACTR_HOME is only touched under ENV_LOCK.
        unsafe { std::env::set_var("FACTR_HOME", &home) };
        let err = add("the config lives in /Users/someone/app/config.toml", "fact", "desktop").unwrap_err();
        assert_eq!(err.to_string(), "not stored: contains an absolute path");
        assert!(add("hi", "fact", "desktop").unwrap_err().to_string().contains("too short"));
        let id = add("releases are cut from the main branch", "fact", "desktop").unwrap();
        assert_eq!(find(&id).unwrap().trust, factr_base::memory::TrustLevel::High);
        assert!(edit(&id, "api key is sk-abcdefgh12345678").unwrap_err().to_string().contains("secret"));
        assert_eq!(find(&id).unwrap().content, "releases are cut from the main branch");
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn expiring_a_memory_deactivates_it_and_records_why() {
        let _env = crate::factr_env::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let home = std::env::temp_dir().join(format!("memory-rest-expire-{}", std::process::id()));
        std::fs::create_dir_all(&home).unwrap();
        // SAFETY: FACTR_HOME is only touched under ENV_LOCK.
        unsafe { std::env::set_var("FACTR_HOME", &home) };
        let id = add("deploys need two approvals", "fact", "test").unwrap();
        assert!(MemoryManager::new().expire(&id, "policy changed").unwrap());
        let entry = find(&id).expect("row is kept");
        assert!(!entry.active);
        assert!(entry.source.unwrap().contains("policy changed"));
        assert!(!MemoryManager::new().expire("nope", "").unwrap());
        forget(&id).unwrap();
    }

    #[test]
    fn adding_the_same_memory_twice_stores_it_once() {
        let _env = crate::factr_env::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let home = std::env::temp_dir().join(format!("memory-rest-dedup-{}", std::process::id()));
        std::fs::create_dir_all(&home).unwrap();
        // SAFETY: FACTR_HOME is only touched under ENV_LOCK.
        unsafe { std::env::set_var("FACTR_HOME", &home) };
        let first = add("staging deploys need two approvals", "fact", "desktop").unwrap();
        let second = add("Staging deploys need two approvals.", "fact", "desktop").unwrap();
        assert_eq!(first, second);
        assert_eq!(all().unwrap().iter().filter(|m| m.active).count(), 1);
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn reset_leaves_learned_entries_alone_and_forget_keeps_other_rows() {
        let _env = crate::factr_env::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let home = std::env::temp_dir().join(format!("memory-rest-reset-{}", std::process::id()));
        std::fs::create_dir_all(&home).unwrap();
        // SAFETY: FACTR_HOME is only touched under ENV_LOCK.
        unsafe { std::env::set_var("FACTR_HOME", &home) };
        let mut prompt = MemoryEntry::new(MemoryCategory::Custom("prompt".into()), "Always run the linter");
        prompt.id = "learned-1".into();
        factr_base::memory::learned::put(&home.join("factr.db"), "global", prompt).unwrap();
        let (keep, drop_me) = (add("keep this note around", "fact", "test").unwrap(), add("forget this note soon", "fact", "test").unwrap());
        assert!(forget(&drop_me).unwrap());
        assert!(find(&keep).is_some(), "forgetting one row leaves the rest");
        let pref = add("likes terse replies", "preference", "test").unwrap();
        assert_eq!(reset("memory").unwrap()["deleted"], json!([keep]));
        assert_eq!(reset("all").unwrap()["deleted"], json!([pref]));
        let learned = factr_base::memory::learned::get(&home.join("factr.db"), "learned-1").unwrap();
        assert!(learned.is_some(), "factr-learn's learned entry survives every reset");
        factr_base::memory::learned::close(&home.join("factr.db"));
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn engine_memory_is_the_single_store_behind_the_screen() {
        let _env = crate::factr_env::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let home = std::env::temp_dir().join(format!("memory-rest-{}", std::process::id()));
        std::fs::create_dir_all(&home).unwrap();
        // SAFETY: FACTR_HOME is only touched under ENV_LOCK.
        unsafe { std::env::set_var("FACTR_HOME", &home) };
        assert_eq!(status().unwrap()["builtin_files"], json!({"memory": 0, "user": 0}));
        let note = add("repo uses cargo", "fact", "test").unwrap();
        let pref = add("likes terse replies", "preference", "test").unwrap();
        assert_eq!(status().unwrap()["builtin_files"], json!({"memory": 15, "user": 19}));
        assert!(edit(&note, "repo uses cargo workspaces").unwrap());
        assert!(!edit("nope", "a long enough edit").unwrap());
        assert!(all().unwrap().iter().any(|m| m.content == "repo uses cargo workspaces"));
        // the learning graph lists the model's own memories and edits/deletes them by node id
        let g = crate::learning_rest::graph(&home).unwrap();
        let node = g["nodes"].as_array().unwrap().iter().find(|n| n["id"] == json!(format!("mem:{note}"))).unwrap().clone();
        assert_eq!(node["kind"], "memory");
        assert_eq!(g["stats"]["memory_nodes"], 2);
        assert_eq!(crate::learning_rest::node(&home, node["id"].as_str().unwrap()).unwrap().unwrap()["content"], "repo uses cargo workspaces");
        let refs = std::collections::HashSet::from([pref.clone()]);
        assert_eq!(unreferenced(&refs).unwrap().len(), 1);
        assert_eq!(reset("user").unwrap()["deleted"], json!([pref]));
        assert_eq!(all().unwrap().len(), 1);
        assert_eq!(reset("all").unwrap()["deleted"].as_array().unwrap().len(), 1);
        // a project memory shows up, edits, and forgets like a global one
        let project = MemoryManager::new().with_project_dir("/tmp/some-project");
        let id = project.remember_project(MemoryEntry::new(MemoryCategory::Fact, "project uses pnpm")).unwrap();
        let listed = scoped().unwrap();
        let (scope, _) = listed.iter().find(|(_, m)| m.id == id).unwrap();
        assert!(scope.starts_with("project:"));
        assert_eq!(status().unwrap()["builtin_files"]["memory"], json!(17));
        assert!(edit(&id, "project uses yarn").unwrap());
        assert!(all().unwrap().iter().any(|m| m.content == "project uses yarn"));
        assert!(forget(&id).unwrap());
        assert!(!forget(&id).unwrap());
        let id = project.remember_project(MemoryEntry::new(MemoryCategory::Fact, "again and again here")).unwrap();
        assert_eq!(reset("all").unwrap()["deleted"], json!([id]));
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn learned_rows_list_with_their_category_scope_and_changeset_id() {
        use factr_learn::entries::{Action, AppliedEdit, EntryKind, EntryStore, NewEntry, Scope};
        let _env = crate::factr_env::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let home = std::env::temp_dir().join(format!("memory-rest-learned-{}", std::process::id()));
        std::fs::create_dir_all(&home).unwrap();
        // SAFETY: FACTR_HOME is only touched under ENV_LOCK.
        unsafe { std::env::set_var("FACTR_HOME", &home) };
        let store = EntryStore::open_cached(&home).unwrap();
        let skill = store.create(NewEntry::new(EntryKind::Skill, Scope::Local, "Codeword", "The codeword is MARIGOLD").with_session("chat-1")).unwrap();
        let note = store.create(NewEntry::new(EntryKind::Prompt, Scope::Global, "Rule", "Run tests")).unwrap();
        let cs = store
            .record_changeset(Some("chat-1"), Scope::Local, "learned", "r", "e",
                &[AppliedEdit { action: Action::Create, id: skill.id.clone(), before: None, after: Some(skill.clone()) }], None, "refine")
            .unwrap();
        add("plain fact about the repo", "fact", "test").unwrap();
        let rows = entries_json().unwrap()["entries"].as_array().unwrap().clone();
        let by_id = |id: &str| rows.iter().find(|r| r["id"] == id).unwrap_or_else(|| panic!("{id} not listed")).clone();
        let s = by_id(&skill.id);
        assert_eq!((s["category"].as_str(), s["scope"].as_str()), (Some("skill"), Some("session:chat-1")));
        assert_eq!(s["learned"]["applies_to"], "session");
        assert_eq!(s["learned"]["session"], "chat-1");
        assert_eq!(s["learned"]["changeset"], json!(cs));
        assert_eq!(s["learned"]["title"], "Codeword");
        let n = by_id(&note.id);
        assert_eq!((n["category"].as_str(), n["learned"]["applies_to"].as_str()), (Some("prompt"), Some("global")));
        assert!(n["learned"]["changeset"].is_null());
        assert!(rows.iter().any(|r| r["category"] == "fact" && r.get("learned").is_none()), "plain memories have no learned block");
        let _ = std::fs::remove_dir_all(home);
    }
}
