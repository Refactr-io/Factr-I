use super::*;
use anyhow::{Result, anyhow};

#[test]
fn detects_reload_interrupted_generation_text() {
    let agent = test_agent(vec![crate::session::StoredMessage {
        id: "msg_1".to_string(),
        role: crate::message::Role::Assistant,
        content: vec![ContentBlock::Text {
            text: "partial\n\n[generation interrupted - server reloading]".to_string(),
            cache_control: None,
        }],
        display_role: None,
        timestamp: None,
        tool_duration_ms: None,
        token_usage: None,
    }]);

    assert!(session_was_interrupted_by_reload(&agent));
}

#[test]
fn detects_reload_interrupted_tool_result() {
    let agent = test_agent(vec![crate::session::StoredMessage {
        id: "msg_2".to_string(),
        role: crate::message::Role::User,
        content: vec![ContentBlock::ToolResult {
            tool_use_id: "tool_1".to_string(),
            content: "[Tool 'bash' interrupted by server reload after 0.2s]".to_string(),
            is_error: Some(true),
        }],
        display_role: None,
        timestamp: None,
        tool_duration_ms: None,
        token_usage: None,
    }]);

    assert!(session_was_interrupted_by_reload(&agent));
}

#[test]
fn detects_reload_skipped_tool_result() {
    let agent = test_agent(vec![crate::session::StoredMessage {
        id: "msg_3".to_string(),
        role: crate::message::Role::User,
        content: vec![ContentBlock::ToolResult {
            tool_use_id: "tool_2".to_string(),
            content: "[Skipped - server reloading]".to_string(),
            is_error: Some(true),
        }],
        display_role: None,
        timestamp: None,
        tool_duration_ms: None,
        token_usage: None,
    }]);

    assert!(session_was_interrupted_by_reload(&agent));
}

#[test]
fn detects_selfdev_reload_tool_result_even_when_not_marked_error() {
    let agent = test_agent(vec![crate::session::StoredMessage {
        id: "msg_3b".to_string(),
        role: crate::message::Role::User,
        content: vec![ContentBlock::ToolResult {
            tool_use_id: "tool_2b".to_string(),
            content: "Reload initiated. Process restarting...".to_string(),
            is_error: Some(false),
        }],
        display_role: None,
        timestamp: None,
        tool_duration_ms: None,
        token_usage: None,
    }]);

    assert!(session_was_interrupted_by_reload(&agent));
}

#[test]
fn ignores_normal_tool_errors() {
    let agent = test_agent(vec![crate::session::StoredMessage {
        id: "msg_4".to_string(),
        role: crate::message::Role::User,
        content: vec![ContentBlock::ToolResult {
            tool_use_id: "tool_3".to_string(),
            content: "Error: file not found".to_string(),
            is_error: Some(true),
        }],
        display_role: None,
        timestamp: None,
        tool_duration_ms: None,
        token_usage: None,
    }]);

    assert!(!session_was_interrupted_by_reload(&agent));
}

#[test]
fn restored_closed_session_with_reload_marker_still_counts_as_interrupted() {
    let agent = test_agent(vec![crate::session::StoredMessage {
        id: "msg_5".to_string(),
        role: crate::message::Role::Assistant,
        content: vec![ContentBlock::Text {
            text: "partial\n\n[generation interrupted - server reloading]".to_string(),
            cache_control: None,
        }],
        display_role: None,
        timestamp: None,
        tool_duration_ms: None,
        token_usage: None,
    }]);

    assert!(restored_session_was_interrupted(
        "session_test_reload",
        &crate::session::SessionStatus::Closed,
        &agent,
    ));
}

#[test]
fn restored_closed_session_with_pending_user_message_without_reload_marker_is_not_interrupted() {
    let _guard = crate::storage::lock_test_env();
    let runtime = tempfile::TempDir::new().expect("runtime dir");
    let prev_runtime = std::env::var_os("FACTR_RUNTIME_DIR");
    crate::env::set_var("FACTR_RUNTIME_DIR", runtime.path());

    let agent = test_agent(vec![crate::session::StoredMessage {
        id: "msg_pending_normal_close".to_string(),
        role: crate::message::Role::User,
        content: vec![ContentBlock::Text {
            text: "normal pending user text".to_string(),
            cache_control: None,
        }],
        display_role: None,
        timestamp: None,
        tool_duration_ms: None,
        token_usage: None,
    }]);

    let interrupted = restored_session_was_interrupted(
        "session_test_reload",
        &crate::session::SessionStatus::Closed,
        &agent,
    );

    if let Some(prev_runtime) = prev_runtime {
        crate::env::set_var("FACTR_RUNTIME_DIR", prev_runtime);
    } else {
        crate::env::remove_var("FACTR_RUNTIME_DIR");
    }

    assert!(!interrupted);
}

#[test]
fn restored_closed_session_without_reload_marker_is_not_interrupted() {
    let agent = test_agent(vec![crate::session::StoredMessage {
        id: "msg_6".to_string(),
        role: crate::message::Role::Assistant,
        content: vec![ContentBlock::Text {
            text: "finished normally".to_string(),
            cache_control: None,
        }],
        display_role: None,
        timestamp: None,
        tool_duration_ms: None,
        token_usage: None,
    }]);

    assert!(!restored_session_was_interrupted(
        "session_test_reload",
        &crate::session::SessionStatus::Closed,
        &agent,
    ));
}

#[tokio::test]
async fn rename_shutdown_signal_moves_registration_to_restored_session() -> Result<()> {
    let signal = InterruptSignal::new();
    let shutdown_signals = Arc::new(RwLock::new(HashMap::from([(
        "session_old".to_string(),
        signal.clone(),
    )])));

    rename_shutdown_signal(&shutdown_signals, "session_old", "session_restored").await;

    let signals = shutdown_signals.read().await;
    assert!(!signals.contains_key("session_old"));
    let renamed = signals
        .get("session_restored")
        .ok_or_else(|| anyhow!("restored session should retain shutdown signal"))?;
    renamed.fire();
    assert!(signal.is_set());
    Ok(())
}
