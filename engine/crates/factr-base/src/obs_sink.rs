//! Span hook for memory and learning steps.
//!
//! The lower crates (memory store, extraction, recall) cannot depend on the gateway, so they call
//! [`emit`] and the runtime installs the recorder (Observability's `Observer`) once at startup. With no
//! recorder installed (tests, CLI tools) [`emit`] does nothing. Attributes carry ids, counts and
//! reasons; memory text never goes in.

use serde_json::{Value, json};
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone)]
pub struct Span {
    /// `memory.write`, `memory.recall`, `memory.inject`, `memory.extract`, `memory.skip`,
    /// `learning.gate`, `learning.refine`, `learning.apply`, `learning.skip`, `loop.guard`,
    /// and the undo/checkpoint kinds below (counts, durations and fixed labels only: never a path
    /// or message text):
    ///
    /// - `undo.turn`: one `/undo`. Attrs `mode` (both|chat|files), `turns`, `messages_removed`,
    ///   `files_restored`, `files_skipped`, `tasks_cancelled`, `irreversible_effects`,
    ///   `redo_available`, `reason` (ok|no_file_changes|no_marker|no_agent_writes|checkpoints_off|
    ///   restore_failed); duration in `duration_ms`.
    /// - `undo.redo`: one `/redo`. Attrs `messages_restored`, `files_restored`, `files_skipped`,
    ///   `redo_available`, `reason` (as above).
    /// - `checkpoint.snapshot`: one store snapshot. Attrs `reason` (write|edit|patch|command|
    ///   pre_rollback|goal|undo|turn_start|other), `made` (false when the tree matched the newest
    ///   checkpoint); error "snapshot failed".
    /// - `checkpoint.restore`: one restore. Attrs `scope` (tree|file), `safe`, `restored`,
    ///   `skipped_user_edits`, `skipped_oversize`, `failed`; error "restore failed".
    /// - `memory.rewind`: an undo deactivated memories extracted from the undone messages (only
    ///   emitted when some were). Attr `deactivated`.
    /// - `memory.restore`: a redo put them back (only emitted when the undo had deactivated some).
    ///   Attrs `restored`, `skipped_changed` (rows edited or audited since the undo, left alone).
    /// - `rollback.list`: one checkpoint listing. Attrs `checkpoints`, `goal_pinned`, `goal_legacy`.
    pub kind: &'static str,
    pub session_id: Option<String>,
    pub error: Option<String>,
    pub attributes: Value,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub duration_ms: u64,
}

impl Span {
    pub fn new(kind: &'static str) -> Self {
        Self { kind, session_id: None, error: None, attributes: json!({}), input_tokens: 0, output_tokens: 0, duration_ms: 0 }
    }
    pub fn session(mut self, id: impl Into<String>) -> Self {
        self.session_id = Some(id.into());
        self
    }
    pub fn attr(mut self, key: &str, value: impl Into<Value>) -> Self {
        self.attributes[key] = value.into();
        self
    }
    pub fn error(mut self, message: impl Into<String>) -> Self {
        self.error = Some(message.into());
        self
    }
    pub fn tokens(mut self, input: u64, output: u64) -> Self {
        self.input_tokens = input;
        self.output_tokens = output;
        self
    }
    pub fn took_ms(mut self, ms: u64) -> Self {
        self.duration_ms = ms;
        self
    }
}

type Recorder = Arc<dyn Fn(Span) + Send + Sync>;
static RECORDER: RwLock<Option<Recorder>> = RwLock::new(None);

/// Install the process-wide recorder, replacing any earlier one.
pub fn install(recorder: impl Fn(Span) + Send + Sync + 'static) {
    if let Ok(mut slot) = RECORDER.write() {
        *slot = Some(Arc::new(recorder));
    }
}

pub fn emit(span: Span) {
    let recorder = RECORDER.read().ok().and_then(|slot| slot.clone());
    if let Some(recorder) = recorder {
        recorder(span);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn emit_reaches_the_installed_recorder_and_is_silent_without_one() {
        emit(Span::new("memory.write"));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = seen.clone();
        install(move |s| sink.lock().unwrap().push(s));
        emit(Span::new("memory.write").session("s1").attr("outcome", "merged"));
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].attributes["outcome"], "merged");
        assert_eq!(seen[0].session_id.as_deref(), Some("s1"));
    }
}
