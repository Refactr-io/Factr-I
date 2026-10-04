//! The edit, write, apply_patch and bash tools each call `checkpoint::before_change` before they
//! touch files. Unit tests never use the real checkpoint store, so this injects one per session.

use super::*;
use crate::checkpoint::{self, Store};
use serde_json::json;

struct Rig {
    _tmp: tempfile::TempDir,
    store: Store,
    work: std::path::PathBuf,
    session: String,
}

fn rig(name: &str) -> Rig {
    let tmp = tempfile::tempdir().unwrap();
    let work = tmp.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    let work = work.canonicalize().unwrap();
    let session = format!("hooks-{name}-{}", std::process::id());
    checkpoint::use_store_for_test(&session, tmp.path().join("store"));
    Rig { store: Store::at(tmp.path().join("store")), _tmp: tmp, work, session }
}

impl Rig {
    fn ctx(&self) -> ToolContext {
        ToolContext {
            session_id: self.session.clone(),
            message_id: "m".into(),
            tool_call_id: "c".into(),
            working_dir: Some(self.work.clone()),
            stdin_request_tx: None,
            graceful_shutdown_signal: None,
            execution_mode: ToolExecutionMode::Direct,
        }
    }

    /// The reasons of the snapshots the tool took, newest first.
    fn reasons(&self) -> Vec<String> {
        self.store.list(&self.work).into_iter().map(|c| c.message).collect()
    }
}

#[tokio::test]
async fn write_edit_apply_patch_and_bash_snapshot_before_changing_files() {
    let _lock = crate::storage::lock_test_env();

    let r = rig("write");
    std::fs::write(r.work.join("a.txt"), "old").unwrap();
    write::WriteTool::new().execute(json!({ "file_path": "a.txt", "content": "new" }), r.ctx()).await.unwrap();
    assert!(r.reasons().first().is_some_and(|m| m.starts_with("before write: a.txt")), "{:?}", r.reasons());
    let before = r.store.list(&r.work)[0].hash.clone();
    assert!(r.store.diff(&r.work, &before).unwrap()["diff"].as_str().unwrap().contains("-old"), "the snapshot holds the pre-write content");
    assert!(r.store.has_agent_writes(&r.work), "after_write recorded the ledger entry");

    let r = rig("edit");
    std::fs::write(r.work.join("b.txt"), "alpha beta").unwrap();
    edit::EditTool::new().execute(json!({ "file_path": "b.txt", "old_string": "alpha", "new_string": "gamma" }), r.ctx()).await.unwrap();
    assert!(r.reasons().first().is_some_and(|m| m.starts_with("before edit: b.txt")), "{:?}", r.reasons());

    let r = rig("patch");
    std::fs::write(r.work.join("c.txt"), "seed").unwrap();
    let patch = "*** Begin Patch\n*** Add File: made.txt\n+hello\n*** End Patch";
    apply_patch::ApplyPatchTool::new().execute(json!({ "patch_text": patch }), r.ctx()).await.unwrap();
    assert!(r.reasons().first().is_some_and(|m| m.starts_with("before patch: made.txt")), "{:?}", r.reasons());

    let r = rig("bash");
    std::fs::write(r.work.join("d.txt"), "keep me").unwrap();
    bash::BashTool::new().execute(json!({ "command": "sed -i.bak s/keep/lose/ d.txt" }), r.ctx()).await.unwrap();
    assert!(r.reasons().first().is_some_and(|m| m.starts_with("before command: sed -i.bak")), "{:?}", r.reasons());
    assert_eq!(std::fs::read_to_string(r.work.join("d.txt")).unwrap(), "lose me");
    // a harmless command takes no snapshot
    let r = rig("bash-safe");
    bash::BashTool::new().execute(json!({ "command": "ls" }), r.ctx()).await.unwrap();
    assert!(r.reasons().is_empty());
}
