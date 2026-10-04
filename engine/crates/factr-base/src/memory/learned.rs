//! factr-learn's learned entries (`prompt`, `skill`, `subagent`) as memories.
//!
//! They live in the same `memories` table as every other memory and are written through
//! `memory_store::remember` like any other writer; what sets them apart is that they are keyed
//! by id alone (never merged by similarity) and that recall treats them specially (see
//! [`is_kept_out_of_recall`]). These functions take the database file, because factr-learn's store is
//! opened on an explicit home rather than on `FACTR_HOME`.

use super::{MemoryEntry, forget_graph};
use crate::memory_store;
use anyhow::Result;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

/// `project:<hash of the project dir>`, the scope of memories written from that directory.
pub fn project_scope(dir: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    std::path::PathBuf::from(dir).hash(&mut hasher);
    format!("project:{:016x}", hasher.finish())
}

/// Counts every change to a learned row (put, delete, drop of a scope). A prompt snapshot built at
/// one count is stale at another; it is the only thing that tells a running session a note changed.
static GENERATION: AtomicU64 = AtomicU64::new(0);

pub fn generation() -> u64 {
    GENERATION.load(Ordering::Relaxed)
}

fn invalidate(db: &Path, scope: &str) {
    GENERATION.fetch_add(1, Ordering::Relaxed);
    forget_graph(&format!("{}#{scope}", db.display()));
}

/// Write (or replace, by id) a learned memory in `scope`.
pub fn put(db: &Path, scope: &str, entry: MemoryEntry) -> Result<()> {
    crate::memory_quality::gate_entry(&entry)?;
    memory_store::remember(db, scope, entry)?;
    invalidate(db, scope);
    Ok(())
}

pub fn get(db: &Path, id: &str) -> Result<Option<(String, MemoryEntry)>> {
    memory_store::get_learned(db, id)
}

/// Active learned memories of these categories in `scopes` (every scope when `None`).
pub fn list(db: &Path, categories: &[&str], scopes: Option<&[String]>) -> Result<Vec<(String, MemoryEntry)>> {
    memory_store::list_learned(db, categories, scopes)
}

pub fn delete(db: &Path, id: &str) -> Result<Option<(String, MemoryEntry)>> {
    let gone = memory_store::delete_learned(db, id)?;
    if let Some((scope, _)) = &gone {
        invalidate(db, scope);
    }
    Ok(gone)
}

/// Drop every memory of `scope` (a deleted session's own rows).
pub fn drop_scope(db: &Path, scope: &str) -> Result<usize> {
    let n = memory_store::drop_scope(db, scope)?;
    invalidate(db, scope);
    Ok(n)
}

/// Close this process's cached connection to `db` (before a test deletes the file).
pub fn close(db: &Path) {
    memory_store::close(db);
}

/// Prompt notes are always in the cached system prompt, and a skill with a generated `SKILL.md`
/// is already offered by the skill list: recall shows neither again.
pub fn is_kept_out_of_recall(entry: &MemoryEntry) -> bool {
    match entry.category.to_string().as_str() {
        "prompt" => entry.category.is_learned(),
        "skill" => entry.category.is_learned() && entry.learned.as_ref().is_some_and(|l| l.listed),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::{MemoryCategory, MemoryEntry};

    #[test]
    fn every_change_to_a_learned_row_moves_the_generation() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("memories.db");
        let entry = MemoryEntry::new(MemoryCategory::Custom("prompt".into()), "Write release scripts in Nim");
        let id = entry.id.clone();
        let g = generation();
        put(&db, "global", entry).unwrap();
        let after_put = generation();
        assert!(after_put > g, "put");
        delete(&db, &id).unwrap();
        let after_delete = generation();
        assert!(after_delete > after_put, "delete");
        drop_scope(&db, "session:s1").unwrap();
        assert!(generation() > after_delete, "drop_scope");
        close(&db);
    }
}
