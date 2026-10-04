//! Undo/redo bookkeeping that sits beside the engine's transcript: permanent row ids, one marker per
//! submitted turn (when it started and the file checkpoint taken then) and a redo stack.
//! Persisted per session at `<undo dir>/<session>.json` (`$FACTR_HOME/undo`), so ids and redo
//! survive a restart. The removed messages themselves live in the engine (`rewind_undo`); this
//! file holds what the engine cannot know: row ids and file state.
//!
//! Row ids: every user/assistant row gets a monotonic id from `next_row_id`, which only grows.
//! Rows are matched to the stored ids by position and a short fingerprint (role + start of the
//! text), so an undone row's id is never handed to a later message, a session without a file
//! is numbered 1..n (the old position scheme, so existing client caches stay valid), and any
//! drift (an external rewrite) gives the changed rows fresh ids rather than a wrong one.
//! Compaction only changes the provider's view of the stored messages, never the rows.

use factr_app_core::checkpoint::{self, Store};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

const MAX_REDO: usize = 10;
const MAX_TURNS: usize = 200;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Key {
    pub id: u64,
    pub role: String,
    h: String,
}

/// A submitted user turn: when it began, and the file checkpoint of that moment.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Turn {
    pub row_id: u64,
    pub at: String,
    pub dir: String,
    pub checkpoint: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Redo {
    /// Rows removed (empty for a files-only undo).
    pub keys: Vec<Key>,
    pub turns: Vec<Turn>,
    /// Rows left after the undo: the record only fits a transcript of exactly this length.
    pub after: usize,
    pub dir: String,
    /// Checkpoint of the files just before the undo (None: files were not touched).
    pub files_before: Option<String>,
}

#[derive(Default, Debug, Serialize, Deserialize)]
pub(crate) struct State {
    next_row_id: u64,
    pub rows: Vec<Key>,
    pub turns: Vec<Turn>,
    pub redo: Vec<Redo>,
}

fn key_of(row: &Value) -> (String, String) {
    let role = row["role"].as_str().unwrap_or_default().to_string();
    let start: String = row["content"].as_str().unwrap_or_default().chars().take(48).collect();
    let digest = Sha256::digest(format!("{role}\0{start}"));
    (role, digest.iter().take(6).map(|b| format!("{b:02x}")).collect())
}

fn file_of(dir: &Path, session: &str) -> PathBuf {
    checkpoint::undo_file(dir, session, "json")
}

type Locks = std::sync::Mutex<std::collections::HashMap<String, std::sync::Arc<tokio::sync::Mutex<()>>>>;
static LOCKS: std::sync::LazyLock<Locks> = std::sync::LazyLock::new(Default::default);

/// The session's undo-state lock. Every load-modify-save of its `State` runs under it (async
/// callers `lock().await`, blocking ones `blocking_lock()`), so two requests never read the same
/// `next_row_id` and an id is never issued twice.
pub(crate) fn session_lock(session: &str) -> std::sync::Arc<tokio::sync::Mutex<()>> {
    LOCKS.lock().unwrap_or_else(|p| p.into_inner()).entry(session.to_string()).or_default().clone()
}

/// Drop the lock entry of a deleted session.
pub(crate) fn forget_lock(session: &str) {
    LOCKS.lock().unwrap_or_else(|p| p.into_inner()).remove(session);
}

/// Delete everything undo keeps for `session`: the three `undo/<id>*` files, its pins in the
/// checkpoint store and its lock entry. Called whenever a chat is deleted (`forget_rows`).
/// Ids that are not already plain file names are ignored (a blank or path-like id must not reach
/// another session's files).
pub(crate) fn purge_session(session: &str) {
    if session.is_empty() || checkpoint::safe_name(session) != session {
        return;
    }
    purge_in(checkpoint::undo_dir().as_deref(), Store::default_store().as_ref(), session);
}

fn purge_in(dir: Option<&Path>, store: Option<&Store>, session: &str) {
    {
        // Best effort: a delete is refused mid-turn, so nothing normally holds the lock.
        let lock = session_lock(session);
        let _guard = lock.try_lock().ok();
        if let Some(dir) = dir {
            checkpoint::purge_undo_files(dir, session);
        }
    }
    if let Some(store) = store {
        store.sync_pins(session, &[]);
    }
    forget_lock(session);
}

/// Load the session's state under its lock, run `edit` (returns the result and whether it changed
/// anything), and save only when it did. Blocking: call from `spawn_blocking`.
pub(crate) fn with_state<R>(dir: &Path, session: &str, store: Option<&Store>, edit: impl FnOnce(&mut State) -> (R, bool)) -> R {
    let lock = session_lock(session);
    let _guard = lock.blocking_lock();
    let mut state = State::load(dir, session);
    let (out, changed) = edit(&mut state);
    if changed {
        state.save_pinned(dir, session, store);
    }
    out
}

impl State {
    pub fn next_row_id(&self) -> u64 {
        self.next_row_id
    }

    pub fn load(dir: &Path, session: &str) -> Self {
        std::fs::read_to_string(file_of(dir, session)).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save(&self, dir: &Path, session: &str) {
        let _ = std::fs::create_dir_all(dir);
        let file = file_of(dir, session);
        if let Ok(text) = serde_json::to_string(self) {
            let tmp = file.with_extension("tmp");
            if std::fs::write(&tmp, text).is_ok() {
                let _ = std::fs::rename(&tmp, &file);
            }
        }
    }

    /// Align the stored ids with the engine's current `rows` (user/assistant messages) and return
    /// one id per row. New rows take `next_row_id`; a shrunk transcript just drops the tail.
    pub fn sync(&mut self, rows: &[Value]) -> Vec<u64> {
        self.next_row_id = self.next_row_id.max(1);
        let mut aligned = true;
        let mut keys = Vec::with_capacity(rows.len());
        for (i, row) in rows.iter().enumerate() {
            let (role, h) = key_of(row);
            // An assistant row grows while it streams, so only its role has to match.
            let kept = self.rows.get(i).filter(|k| aligned && k.role == role && (role == "assistant" || k.h == h)).cloned();
            keys.push(match kept {
                Some(mut k) => {
                    k.h = h;
                    k
                }
                None => {
                    aligned = false;
                    self.next_row_id += 1;
                    Key { id: self.next_row_id - 1, role, h }
                }
            });
        }
        self.rows = keys;
        self.rows.iter().map(|k| k.id).collect()
    }

    /// Note that a user turn is about to be submitted (its row will take `next_row_id`). A new
    /// prompt ends the redo history, as in an editor.
    pub fn mark_turn(&mut self, dir: &str, checkpoint: Option<String>) {
        self.next_row_id = self.next_row_id.max(1);
        let row_id = self.next_row_id;
        self.turns.retain(|t| t.row_id != row_id);
        self.turns.push(Turn { row_id, at: checkpoint::now_stamp(), dir: dir.to_string(), checkpoint });
        let excess = self.turns.len().saturating_sub(MAX_TURNS);
        self.turns.drain(..excess);
        self.redo.clear();
    }

    pub fn turn(&self, row_id: u64) -> Option<&Turn> {
        self.turns.iter().find(|t| t.row_id == row_id)
    }

    /// Record an undo that cut the transcript at `index` (0 = everything; `rows.len()` for a
    /// files-only undo, which removes nothing).
    pub fn push_undo(&mut self, index: usize, dir: &str, files_before: Option<String>) {
        let index = index.min(self.rows.len());
        let keys = self.rows.split_off(index);
        let (gone, kept): (Vec<Turn>, Vec<Turn>) = std::mem::take(&mut self.turns).into_iter().partition(|t| keys.iter().any(|k| k.id == t.row_id));
        self.turns = kept;
        self.redo.push(Redo { keys, turns: gone, after: index, dir: dir.to_string(), files_before });
        let excess = self.redo.len().saturating_sub(MAX_REDO);
        self.redo.drain(..excess);
    }

    /// The newest redo record when it fits the transcript (`rows` rows now); a stale stack is dropped.
    pub fn take_redo(&mut self) -> Option<Redo> {
        let top = self.redo.pop()?;
        if top.after == self.rows.len() {
            return Some(top);
        }
        self.redo.clear();
        None
    }

    /// Every commit a turn marker or a redo record still needs, as `(project dir, commit)`.
    pub fn pinned(&self) -> Vec<(PathBuf, String)> {
        let turn = |t: &Turn| t.checkpoint.clone().map(|c| (PathBuf::from(&t.dir), c));
        let mut all: Vec<(PathBuf, String)> = self.turns.iter().filter_map(turn).collect();
        for r in &self.redo {
            all.extend(r.turns.iter().filter_map(turn));
            all.extend(r.files_before.clone().map(|c| (PathBuf::from(&r.dir), c)));
        }
        all.sort();
        all.dedup();
        all
    }

    /// The earliest start among the markers this state still holds (turns and redo records).
    pub fn oldest_turn_at(&self) -> Option<String> {
        self.turns.iter().chain(self.redo.iter().flat_map(|r| r.turns.iter())).map(|t| t.at.clone()).min()
    }

    /// Save, then make the session's pins in the checkpoint store match what the state references:
    /// a marker or redo record keeps its commit alive past pruning, and so does every snapshot a
    /// retained turn took before its first file change; a dropped one releases it.
    pub fn save_pinned(&self, dir: &Path, session: &str, store: Option<&Store>) {
        self.save(dir, session);
        checkpoint::retain_snapshots_since(dir, session, self.oldest_turn_at().as_deref());
        if let Some(store) = store {
            let mut wanted = self.pinned();
            wanted.extend(checkpoint::snapshots_of(dir, session).into_iter().map(|e| (PathBuf::from(e.dir), e.hash)));
            store.sync_pins(session, &wanted);
        }
    }

    /// Put a redone record's rows and turn markers back.
    pub fn apply_redo(&mut self, r: Redo) {
        self.rows.extend(r.keys);
        self.turns.extend(r.turns);
    }
}

/// The files as they were when `turn` began: its own marker (older sessions took one at submit), else
/// the first snapshot the engine's tool hooks took at or after the turn began. `None`: no turn since
/// then changed a file, so there is nothing to restore.
pub(crate) fn turn_checkpoint(dir: &Path, session: &str, turn: &Turn) -> Option<String> {
    turn.checkpoint.clone().or_else(|| checkpoint::first_snapshot_since(dir, session, Path::new(&turn.dir), &turn.at))
}

/// Snapshot `dir` (or reuse the newest checkpoint when nothing changed): the files exactly as
/// they are now. `None` when checkpoints are off or the folder is not eligible.
pub(crate) fn checkpoint_now(store: &Store, dir: &Path, reason: &str, session: Option<&str>) -> Option<String> {
    if !store.enabled_for(session) || !checkpoint::eligible(dir) {
        return None;
    }
    match store.snapshot_for(dir, reason, session) {
        Ok(Some(sha)) => Some(sha),
        Ok(None) => store.head(dir),
        Err(_) => None,
    }
}

#[derive(Debug, Default)]
pub(crate) struct FileReport {
    pub restored: Vec<String>,
    pub skipped: Vec<Value>,
    /// Why nothing (or not everything) could be restored.
    pub note: Option<String>,
}

/// Bring the files back to checkpoint `hash` in safe mode. Whatever the restore changed is
/// recorded as the agent-side state, so a later redo can reverse it without trampling edits
/// made in between.
pub(crate) fn restore_files(store: &Store, dir: &Path, hash: &str, session: Option<&str>) -> FileReport {
    let mut report = FileReport::default();
    if !store.has_agent_writes(dir) {
        report.note = Some("the agent has not written files here".into());
        return report;
    }
    let out = store.restore_for(dir, hash, None, true, session);
    if out["success"] != true {
        report.note = Some(out["error"].as_str().unwrap_or("restore failed").to_string());
        return report;
    }
    let names = |key: &str| -> Vec<String> { out[key].as_array().into_iter().flatten().filter_map(|v| v.as_str().map(str::to_string)).collect() };
    report.restored = names("restored_files");
    for (key, why) in [
        ("skipped_user_edits", "edited after the agent's last write"),
        ("skipped_oversize", "too large to checkpoint"),
        ("failed_deletes", "could not be deleted"),
    ] {
        report.skipped.extend(names(key).into_iter().map(|path| json!({ "path": path, "reason": why })));
    }
    let root = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    for rel in &report.restored {
        store.record_write(&root, &root.join(rel));
    }
    report
}

/// A fixed label for why files were not (all) restored, for spans (the note itself is prose).
pub(crate) fn note_class(note: &Option<String>) -> &'static str {
    match note.as_deref() {
        None => "ok",
        Some("file checkpoints are off") => "checkpoints_off",
        Some("no file changes in those turns") => "no_file_changes",
        Some("no record of when that turn began") => "no_marker",
        Some("the agent has not written files here") => "no_agent_writes",
        Some(_) => "restore_failed",
    }
}

/// `"restored 3 files, skipped 1 (a.txt: edited after ...)"` for the one-line notice.
pub(crate) fn files_line(report: &FileReport) -> String {
    let mut line = match report.restored.len() {
        0 => "no files restored".to_string(),
        1 => "restored 1 file".to_string(),
        n => format!("restored {n} files"),
    };
    if !report.skipped.is_empty() {
        let names: Vec<String> = report.skipped.iter().take(3).map(|s| s["path"].as_str().unwrap_or_default().to_string()).collect();
        line.push_str(&format!(", skipped {} ({}, edited or not restorable)", report.skipped.len(), names.join(", ")));
    }
    if let Some(note) = &report.note {
        line.push_str(&format!(" ({note})"));
    }
    line
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_forgotten_session_lock_is_dropped_and_recreated_on_demand() {
        let first = super::session_lock("lock-forget");
        super::forget_lock("lock-forget");
        assert!(!super::LOCKS.lock().unwrap().contains_key("lock-forget"));
        assert!(!std::sync::Arc::ptr_eq(&first, &super::session_lock("lock-forget")));
        super::forget_lock("lock-forget");
    }

    use super::*;

    struct TempDir(PathBuf);
    impl TempDir {
        fn new() -> Self {
            static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            let p = std::env::temp_dir().join(format!("undo-{}-{}", std::process::id(), N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).unwrap();
            Self(p)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn rows(spec: &[(&str, &str)]) -> Vec<Value> {
        spec.iter().map(|(r, c)| json!({ "role": r, "content": c })).collect()
    }

    #[test]
    fn row_ids_are_monotonic_and_never_reused_after_an_undo() {
        let mut s = State::default();
        let all = rows(&[("user", "one"), ("assistant", "a1"), ("user", "two"), ("assistant", "a2")]);
        assert_eq!(s.sync(&all), [1, 2, 3, 4], "a session without ids is numbered by position");
        s.push_undo(2, "/w", None);
        let after_undo = rows(&all.iter().map(|m| (m["role"].as_str().unwrap(), m["content"].as_str().unwrap())).take(2).collect::<Vec<_>>());
        assert_eq!(s.sync(&after_undo), [1, 2]);
        let mut next = after_undo.clone();
        next.push(json!({ "role": "user", "content": "brand new" }));
        next.push(json!({ "role": "assistant", "content": "reply" }));
        assert_eq!(s.sync(&next), [1, 2, 5, 6], "ids 3 and 4 belonged to the undone rows and are not reissued");
    }

    #[test]
    fn row_ids_survive_a_restart_and_streaming_text() {
        let tmp = TempDir::new();
        let mut s = State::default();
        let mut all = rows(&[("user", "one"), ("assistant", "par")]);
        s.sync(&all);
        s.save(tmp.path(), "sess/1");
        let mut back = State::load(tmp.path(), "sess/1");
        all[1]["content"] = json!("partial answer, now longer");
        all.push(json!({ "role": "user", "content": "two" }));
        assert_eq!(back.sync(&all), [1, 2, 3]);
        // a different transcript at the same positions (external rewrite) never reuses an id
        let other = rows(&[("user", "changed"), ("assistant", "x")]);
        assert_eq!(back.sync(&other), [4, 5]);
    }

    #[test]
    fn undo_then_redo_restores_ids_and_turn_markers_in_order() {
        let mut s = State::default();
        let mut all = rows(&[]);
        for text in ["one", "two", "three"] {
            s.sync(&all);
            s.mark_turn("/w", Some(format!("ck-{text}")));
            all.push(json!({ "role": "user", "content": text }));
            all.push(json!({ "role": "assistant", "content": "r" }));
        }
        assert_eq!(s.sync(&all), [1, 2, 3, 4, 5, 6]);
        assert_eq!(s.turn(5).unwrap().checkpoint.as_deref(), Some("ck-three"));
        // undo twice
        s.push_undo(4, "/w", Some("files-a".into()));
        s.push_undo(2, "/w", Some("files-b".into()));
        assert_eq!((s.rows.len(), s.turns.len(), s.redo.len()), (2, 1, 2));
        // redo twice, newest undo first
        let first = s.take_redo().unwrap();
        assert_eq!((first.files_before.as_deref(), first.keys.iter().map(|k| k.id).collect::<Vec<_>>()), (Some("files-b"), vec![3, 4]));
        s.apply_redo(first);
        let second = s.take_redo().unwrap();
        assert_eq!(second.files_before.as_deref(), Some("files-a"));
        s.apply_redo(second);
        assert_eq!(s.rows.iter().map(|k| k.id).collect::<Vec<_>>(), [1, 2, 3, 4, 5, 6]);
        assert_eq!(s.turns.len(), 3);
        assert!(s.take_redo().is_none());
    }

    #[test]
    fn a_new_prompt_or_a_changed_transcript_clears_redo() {
        let mut s = State::default();
        s.sync(&rows(&[("user", "one"), ("assistant", "a")]));
        s.push_undo(0, "/w", None);
        assert_eq!(s.redo.len(), 1);
        s.mark_turn("/w", None);
        assert!(s.redo.is_empty(), "a new prompt ends the redo history");
        s.sync(&rows(&[("user", "one"), ("assistant", "a")]));
        s.push_undo(0, "/w", None);
        s.sync(&rows(&[("user", "other")]));
        assert!(s.take_redo().is_none(), "a transcript that no longer fits drops the record");
    }

    #[test]
    fn markers_keep_their_commits_through_pruning_and_undo_to_the_oldest_restores_files() {
        let e = env();
        let undo_dir = e._tmp.path().join("undo");
        let mut s = State::default();
        let mut all = rows(&[]);
        let mut first_marker = None;
        for i in 0..25 {
            s.sync(&all);
            let marker = checkpoint_now(&e.store, &e.work, "turn start", None).unwrap();
            e.store.pin(&e.work, "sess", &marker);
            first_marker.get_or_insert(marker.clone());
            s.mark_turn(e.work.to_str().unwrap(), Some(marker));
            agent_write(&e, "f.txt", &format!("v{i}"));
            all.push(json!({ "role": "user", "content": format!("turn {i}") }));
            all.push(json!({ "role": "assistant", "content": "r" }));
            s.save_pinned(&undo_dir, "sess", Some(&e.store));
        }
        let first_marker = first_marker.unwrap();
        assert!(!e.store.list(&e.work).iter().any(|c| c.hash == first_marker), "20 kept, the oldest marker left the list");
        e.store.gc_now();
        let ids = s.sync(&all);
        let turn = s.turn(ids[0]).expect("oldest marker still recorded");
        assert_eq!(turn.checkpoint.as_deref(), Some(first_marker.as_str()));
        let report = restore_files(&e.store, &e.work, turn.checkpoint.as_deref().unwrap(), None);
        assert!(report.note.is_none(), "{report:?}");
        assert_eq!(std::fs::read_to_string(e.work.join("f.txt")).unwrap_or_default(), "", "first turn started before f.txt existed: undo removes it");
        // dropping every marker releases the pins
        let mut empty = State::default();
        empty.sync(&[]);
        empty.save_pinned(&undo_dir, "sess", Some(&e.store));
        assert_eq!(e.store.pin_count("sess"), 0);
    }

    #[test]
    fn with_state_saves_only_when_something_changed() {
        let tmp = TempDir::new();
        with_state(tmp.path(), "quiet", None, |s| {
            s.sync(&[]);
            ((), false)
        });
        assert!(!tmp.path().join("quiet.json").exists(), "an unchanged state is not written");
        with_state(tmp.path(), "quiet", None, |s| {
            s.sync(&rows(&[("user", "hi")]));
            ((), true)
        });
        assert!(tmp.path().join("quiet.json").exists());
    }

    #[test]
    fn two_hundred_concurrent_updates_never_reuse_a_row_id() {
        let tmp = TempDir::new();
        let dir = tmp.path().to_path_buf();
        let handles: Vec<_> = (0..200)
            .map(|i| {
                let dir = dir.clone();
                std::thread::spawn(move || {
                    with_state(&dir, "race", None, |s| {
                        // each call brings one row that is new to the stored state
                        let ids = s.sync(&rows(&[("user", &format!("message {i}"))]));
                        s.mark_turn("/w", None);
                        (ids[0], true)
                    })
                })
            })
            .collect();
        let mut ids: Vec<u64> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 200, "every id is unique");
        assert_eq!(State::load(&dir, "race").next_row_id, 201);
    }

    #[test]
    fn purging_a_session_removes_its_undo_files_and_pins_and_spares_others() {
        let e = env();
        let undo_dir = e._tmp.path().join("undo");
        std::fs::write(e.work.join("a.txt"), "1").unwrap();
        let hash = checkpoint_now(&e.store, &e.work, "turn start", None).unwrap();
        for session in ["gone", "gone2", "kept"] {
            let mut s = State::default();
            s.sync(&rows(&[("user", "x")]));
            s.mark_turn(e.work.to_str().unwrap(), Some(hash.clone()));
            s.save_pinned(&undo_dir, session, Some(&e.store));
            checkpoint::note_effect_in(&undo_dir, session, "git push");
            std::fs::write(checkpoint::undo_file(&undo_dir, session, "engine.json"), "[]").unwrap();
        }
        purge_in(Some(&undo_dir), Some(&e.store), "gone");
        let left: Vec<String> = std::fs::read_dir(&undo_dir).unwrap().map(|f| f.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert!(left.iter().all(|n| !n.starts_with("gone.")), "{left:?}");
        assert_eq!(left.iter().filter(|n| n.starts_with("gone2.")).count(), 3, "{left:?}");
        assert_eq!((e.store.pin_count("gone"), e.store.pin_count("kept")), (0, 1));
        purge_session("");
        purge_session("a/b");
    }

    #[test]
    fn undo_cap_keeps_the_newest_ten() {
        let mut s = State::default();
        s.sync(&rows(&[("user", "x")]));
        for _ in 0..12 {
            s.push_undo(1, "/w", None);
        }
        assert_eq!(s.redo.len(), MAX_REDO);
    }

    struct Env {
        _tmp: TempDir,
        store: Store,
        work: PathBuf,
    }

    fn env() -> Env {
        let tmp = TempDir::new();
        let work = tmp.path().join("work");
        std::fs::create_dir_all(&work).unwrap();
        let work = work.canonicalize().unwrap();
        let store = Store::at(tmp.path().join("store")).with_config_dir(tmp.path().join("factr"));
        Env { _tmp: tmp, store, work }
    }

    fn agent_write(e: &Env, file: &str, text: &str) {
        std::fs::write(e.work.join(file), text).unwrap();
        e.store.record_write(&e.work, &e.work.join(file));
    }

    #[test]
    fn undo_restores_agent_files_skips_a_hand_edited_one_and_redo_brings_them_back() {
        let e = env();
        std::fs::write(e.work.join("a.txt"), "a0").unwrap();
        std::fs::write(e.work.join("b.txt"), "b0").unwrap();
        let start = checkpoint_now(&e.store, &e.work, "turn start", None).unwrap();
        agent_write(&e, "a.txt", "a1");
        agent_write(&e, "b.txt", "b1");
        agent_write(&e, "new.txt", "created");
        std::fs::write(e.work.join("b.txt"), "b1 + my own edit").unwrap();
        // undo
        let before = checkpoint_now(&e.store, &e.work, "before undo", None).unwrap();
        let undo = restore_files(&e.store, &e.work, &start, None);
        assert_eq!(std::fs::read_to_string(e.work.join("a.txt")).unwrap(), "a0");
        assert!(!e.work.join("new.txt").exists());
        assert_eq!(std::fs::read_to_string(e.work.join("b.txt")).unwrap(), "b1 + my own edit", "hand edit kept");
        let mut restored = undo.restored.clone();
        restored.sort();
        assert_eq!(restored, ["a.txt", "new.txt"]);
        assert_eq!(undo.skipped.len(), 1);
        assert_eq!(undo.skipped[0]["path"], "b.txt");
        assert!(files_line(&undo).contains("restored 2 files, skipped 1 (b.txt"));
        // redo re-applies exactly what the undo removed
        let redo = restore_files(&e.store, &e.work, &before, None);
        assert_eq!(std::fs::read_to_string(e.work.join("a.txt")).unwrap(), "a1");
        assert_eq!(std::fs::read_to_string(e.work.join("new.txt")).unwrap(), "created");
        assert_eq!(std::fs::read_to_string(e.work.join("b.txt")).unwrap(), "b1 + my own edit");
        assert!(redo.restored.contains(&"a.txt".to_string()) && redo.restored.contains(&"new.txt".to_string()), "{redo:?}");
    }

    #[test]
    fn a_file_edited_after_the_undo_is_not_trampled_by_redo() {
        let e = env();
        std::fs::write(e.work.join("a.txt"), "a0").unwrap();
        let start = checkpoint_now(&e.store, &e.work, "turn start", None).unwrap();
        agent_write(&e, "a.txt", "a1");
        let before = checkpoint_now(&e.store, &e.work, "before undo", None).unwrap();
        restore_files(&e.store, &e.work, &start, None);
        std::fs::write(e.work.join("a.txt"), "a0, edited by me").unwrap();
        let redo = restore_files(&e.store, &e.work, &before, None);
        assert_eq!(std::fs::read_to_string(e.work.join("a.txt")).unwrap(), "a0, edited by me");
        assert_eq!(redo.skipped.len(), 1);
    }

    #[test]
    fn undo_twice_then_redo_twice_walks_the_file_states_in_order() {
        let e = env();
        std::fs::write(e.work.join("f.txt"), "v0").unwrap();
        let t1 = checkpoint_now(&e.store, &e.work, "turn 1", None).unwrap();
        agent_write(&e, "f.txt", "v1");
        let t2 = checkpoint_now(&e.store, &e.work, "turn 2", None).unwrap();
        agent_write(&e, "f.txt", "v2");
        let read = || std::fs::read_to_string(e.work.join("f.txt")).unwrap();
        let before_u1 = checkpoint_now(&e.store, &e.work, "undo 1", None).unwrap();
        restore_files(&e.store, &e.work, &t2, None);
        assert_eq!(read(), "v1");
        let before_u2 = checkpoint_now(&e.store, &e.work, "undo 2", None).unwrap();
        restore_files(&e.store, &e.work, &t1, None);
        assert_eq!(read(), "v0");
        restore_files(&e.store, &e.work, &before_u2, None);
        assert_eq!(read(), "v1");
        restore_files(&e.store, &e.work, &before_u1, None);
        assert_eq!(read(), "v2");
    }

    #[test]
    fn no_agent_writes_means_no_file_restore() {
        let e = env();
        std::fs::write(e.work.join("mine.txt"), "x").unwrap();
        let start = checkpoint_now(&e.store, &e.work, "turn start", None).unwrap();
        std::fs::write(e.work.join("mine.txt"), "y").unwrap();
        let report = restore_files(&e.store, &e.work, &start, None);
        assert!(report.restored.is_empty() && report.note.is_some());
        assert_eq!(std::fs::read_to_string(e.work.join("mine.txt")).unwrap(), "y");
    }
}
