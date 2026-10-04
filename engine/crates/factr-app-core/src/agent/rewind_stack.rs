//! Redo stack for conversation rewinds: the stored messages each rewind removed, newest last,
//! kept in `$FACTR_HOME/undo/<session>.engine.json` so `undo_rewind` survives a restart and can
//! be chained (rewind twice, undo twice restores in order). An entry only applies while the
//! transcript is exactly the length the rewind left: a new message makes it stale.

use factr_session_types::StoredMessage;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const MAX_ENTRIES: usize = 10;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct Entry {
    /// Stored messages left after the rewind.
    pub stored_len: usize,
    pub removed: Vec<StoredMessage>,
    /// The compaction state the rewind discarded because it covered removed messages.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compaction: Option<factr_session_types::StoredCompactionState>,
    /// Memories the rewind deactivated and the extraction marker it replaced, for redo.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memories: Option<crate::memory_extract::UndoneMemories>,
}

fn base() -> Option<PathBuf> {
    crate::checkpoint::undo_dir()
}

fn path(base: &Path, session_id: &str) -> PathBuf {
    crate::checkpoint::undo_file(base, session_id, "engine.json")
}

fn load(base: &Path, session_id: &str) -> Vec<Entry> {
    std::fs::read_to_string(path(base, session_id)).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn save(base: &Path, session_id: &str, entries: &[Entry]) {
    let p = path(base, session_id);
    if entries.is_empty() {
        let _ = std::fs::remove_file(p);
        return;
    }
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(text) = serde_json::to_string(entries) {
        let tmp = p.with_extension("tmp");
        if std::fs::write(&tmp, text).is_ok() {
            let _ = std::fs::rename(&tmp, &p);
        }
    }
}

pub(super) fn push(session_id: &str, entry: Entry) {
    if let Some(base) = base() {
        push_in(&base, session_id, entry);
    }
}

fn push_in(base: &Path, session_id: &str, entry: Entry) {
    let mut entries = load(base, session_id);
    entries.push(entry);
    let excess = entries.len().saturating_sub(MAX_ENTRIES);
    entries.drain(..excess);
    save(base, session_id, &entries);
}

/// The newest entry when it still fits a transcript of `stored_len` messages; otherwise the whole
/// stack is stale and dropped.
pub(super) fn pop_if_current(session_id: &str, stored_len: usize) -> Option<Entry> {
    pop_in(&base()?, session_id, stored_len)
}

fn pop_in(base: &Path, session_id: &str, stored_len: usize) -> Option<Entry> {
    let mut entries = load(base, session_id);
    let top = entries.pop()?;
    if top.stored_len != stored_len {
        save(base, session_id, &[]);
        return None;
    }
    save(base, session_id, &entries);
    Some(top)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(stored_len: usize, n: usize) -> Entry {
        let message = |i: usize| serde_json::from_value(serde_json::json!({ "id": format!("m{i}"), "role": "user", "content": [] })).unwrap();
        Entry { stored_len, removed: (0..n).map(message).collect(), compaction: None, memories: None }
    }

    #[test]
    fn rewinds_chain_in_order_persist_and_go_stale_when_the_transcript_moves() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        push_in(dir, "s/1", entry(4, 2));
        push_in(dir, "s/1", entry(2, 2));
        // a fresh process reads the same stack
        assert_eq!(pop_in(dir, "s/1", 2).unwrap().removed.len(), 2);
        assert_eq!(pop_in(dir, "s/1", 4).unwrap().stored_len, 4);
        assert!(pop_in(dir, "s/1", 4).is_none());
        // a message arrived after the rewind: the entry no longer fits and the stack is dropped
        push_in(dir, "s/1", entry(4, 1));
        push_in(dir, "s/1", entry(2, 1));
        assert!(pop_in(dir, "s/1", 3).is_none());
        assert!(pop_in(dir, "s/1", 2).is_none(), "stale stack is gone");
        for i in 0..12 {
            push_in(dir, "s/2", entry(i, 1));
        }
        assert_eq!(load(dir, "s/2").len(), MAX_ENTRIES);
    }
}
