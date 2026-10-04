//! The unified-diff `patch` tool takes a checkpoint before it changes files, like edit/write/apply_patch.

use factr_app_core::checkpoint::Store;
use factr_app_core::tool::{Tool, ToolContext, ToolExecutionMode, patch::PatchTool};
use serde_json::json;

#[tokio::test]
async fn patch_snapshots_the_folder_before_changing_it() {
    let base = std::env::temp_dir().join(format!("patch-ckpt-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let (jhome, work) = (base.join("factr"), base.join("work"));
    std::fs::create_dir_all(&work).unwrap();
    std::fs::create_dir_all(&jhome).unwrap();
    let work = work.canonicalize().unwrap();
    factr_base::env::set_var("FACTR_HOME", &jhome);
    std::fs::write(work.join("a.txt"), "one\ntwo\n").unwrap();

    let ctx = ToolContext {
        session_id: "patch-session".into(),
        message_id: "m".into(),
        tool_call_id: "t".into(),
        working_dir: Some(work.clone()),
        stdin_request_tx: None,
        graceful_shutdown_signal: None,
        execution_mode: ToolExecutionMode::Direct,
    };
    let diff = "--- a/a.txt\n+++ b/a.txt\n@@ -1,2 +1,2 @@\n one\n-two\n+three\n";
    let out = PatchTool::new().execute(json!({ "patch_text": diff }), ctx).await.unwrap();
    assert!(out.output.contains('✓'), "{}", out.output);
    assert_eq!(std::fs::read_to_string(work.join("a.txt")).unwrap(), "one\nthree\n");

    let store = Store::default_store().expect("factr home");
    let list = store.list(&work);
    assert_eq!(list.len(), 1, "one checkpoint taken before the patch");
    assert!(list[0].message.starts_with("before patch: a.txt"), "{}", list[0].message);
    let restored = store.restore(&work, &list[0].hash, None, false);
    assert_eq!(restored["success"], true, "{restored}");
    assert_eq!(std::fs::read_to_string(work.join("a.txt")).unwrap(), "one\ntwo\n", "the checkpoint holds the pre-patch file");
}
