use super::*;
use crate::provider::{EventStream, Provider};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::Notify;

struct MockSummaryProvider;

struct BlockingSummaryProvider {
    started: Arc<Notify>,
}

#[async_trait::async_trait]
impl Provider for BlockingSummaryProvider {
    async fn complete(
        &self,
        _messages: &[Message],
        _tools: &[crate::message::ToolDefinition],
        _system: &str,
        _resume_session_id: Option<&str>,
    ) -> Result<EventStream> {
        Ok(Box::pin(futures::stream::empty()))
    }

    fn name(&self) -> &str {
        "blocking-summary"
    }

    fn model(&self) -> String {
        "blocking-summary-model".into()
    }

    fn fork(&self) -> Arc<dyn Provider> {
        Arc::new(Self {
            started: self.started.clone(),
        })
    }

    async fn complete_simple(&self, _prompt: &str, _system: &str) -> Result<String> {
        std::future::pending().await
    }

    async fn complete_simple_with_usage(
        &self,
        _prompt: &str,
        _system: &str,
    ) -> Result<factr_provider_core::SimpleCompletion> {
        self.started.notify_one();
        std::future::pending().await
    }
}

#[async_trait::async_trait]
impl Provider for MockSummaryProvider {
    async fn complete(
        &self,
        _messages: &[Message],
        _tools: &[crate::message::ToolDefinition],
        _system: &str,
        _resume_session_id: Option<&str>,
    ) -> Result<EventStream> {
        Ok(Box::pin(futures::stream::empty()))
    }

    fn name(&self) -> &str {
        "mock-summary"
    }

    fn model(&self) -> String {
        "mock-summary-model".to_string()
    }

    fn fork(&self) -> Arc<dyn Provider> {
        Arc::new(MockSummaryProvider)
    }

    async fn complete_simple(&self, prompt: &str, _system: &str) -> Result<String> {
        Ok(format!("summary({} chars)", prompt.len()))
    }

    async fn complete_simple_with_usage(
        &self,
        prompt: &str,
        _system: &str,
    ) -> Result<factr_provider_core::SimpleCompletion> {
        Ok(factr_provider_core::SimpleCompletion {
            text: format!("summary({} chars)", prompt.len()),
            usage: Some(factr_provider_core::SimpleUsage {
                input: 17,
                output: 5,
                cache_read: 3,
                cache_write: 0,
            }),
        })
    }
}

fn make_text_message(role: Role, text: &str) -> Message {
    Message {
        role,
        content: vec![ContentBlock::Text {
            text: text.to_string(),
            cache_control: None,
        }],
        timestamp: None,
        tool_duration_ms: None,
    }
}

#[test]
fn test_new_manager() {
    let manager = CompactionManager::new();
    assert_eq!(manager.compacted_count, 0);
    assert!(manager.active_summary.is_none());
    assert!(!manager.is_compacting());
}

#[test]
fn test_notify_message_added() {
    let mut manager = CompactionManager::new();
    manager.notify_message_added();
    manager.notify_message_added();
    assert_eq!(manager.total_turns, 2);
}

#[test]
fn test_restored_messages_do_not_trigger_compaction_immediately() {
    let mut manager = CompactionManager::new().with_budget(1_000);
    let mut messages = Vec::new();
    for i in 0..20 {
        messages.push(make_text_message(Role::User, &format!("restored {}", i)));
    }
    manager.seed_restored_messages(messages.len());
    manager.update_observed_input_tokens(900);

    assert!(
        !manager.should_compact_with(&messages),
        "restored history should not compact until a new message is added"
    );
}

#[test]
fn test_new_message_after_restore_reenables_compaction() {
    let mut manager = CompactionManager::new().with_budget(1_000);
    let mut messages = Vec::new();
    for i in 0..20 {
        messages.push(make_text_message(Role::User, &format!("restored {}", i)));
    }
    manager.seed_restored_messages(messages.len());
    manager.update_observed_input_tokens(900);
    assert!(!manager.should_compact_with(&messages));

    messages.push(make_text_message(Role::User, "new turn after restore"));
    manager.notify_message_added();

    assert!(
        manager.should_compact_with(&messages),
        "compaction should resume once a genuinely new message is added"
    );
}

#[test]
fn test_token_estimate() {
    let manager = CompactionManager::new();
    // 100 chars = 25 tokens. System/tool overhead is included only for a
    // context budget large enough to use the full-window estimate.
    let messages = vec![make_text_message(Role::User, &"x".repeat(100))];
    let estimate = manager.token_estimate_with(&messages);
    let overhead = if manager.token_budget() >= DEFAULT_TOKEN_BUDGET / 2 {
        SYSTEM_OVERHEAD_TOKENS
    } else {
        0
    };
    assert_eq!(estimate, 25 + overhead);
}

#[test]
fn test_should_compact() {
    let mut manager = CompactionManager::new().with_budget(100); // Very small budget

    let mut messages = Vec::new();
    for i in 0..20 {
        messages.push(make_text_message(
            Role::User,
            &format!("Message {} with some content", i),
        ));
        manager.notify_message_added();
    }

    assert!(manager.should_compact_with(&messages));
}

#[test]
fn test_context_usage_prefers_observed_tokens() {
    let mut manager = CompactionManager::new().with_budget(1_000);
    let messages = vec![make_text_message(Role::User, "short message")];
    manager.notify_message_added();
    manager.update_observed_input_tokens(900);

    assert!(manager.context_usage_with(&messages) >= 0.90);
    assert!(manager.effective_token_count_with(&messages) >= 900);
}

#[test]
fn test_should_compact_uses_observed_tokens() {
    let mut manager = CompactionManager::new().with_budget(1_000);

    let mut messages = Vec::new();
    for _ in 0..12 {
        messages.push(make_text_message(Role::User, "x"));
        manager.notify_message_added();
    }
    manager.update_observed_input_tokens(850);

    assert!(manager.should_compact_with(&messages));
}

#[test]
fn test_messages_for_api_no_summary() {
    let mut manager = CompactionManager::new();
    let messages = vec![
        make_text_message(Role::User, "Hello"),
        make_text_message(Role::Assistant, "Hi!"),
    ];
    manager.notify_message_added();
    manager.notify_message_added();

    let msgs = manager.messages_for_api_with(&messages);
    assert_eq!(msgs.len(), 2);
}

#[tokio::test]
async fn test_force_compact_applies_summary() {
    let mut manager = CompactionManager::new().with_budget(1_000);
    let mut messages = Vec::new();
    for i in 0..30 {
        messages.push(make_text_message(
            Role::User,
            &format!("Turn {} {}", i, "x".repeat(120)),
        ));
        manager.notify_message_added();
    }

    let provider: Arc<dyn Provider> = Arc::new(MockSummaryProvider);
    manager
        .force_compact_with(&messages, provider)
        .expect("manual compaction should start");

    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        manager.check_and_apply_compaction();
        if manager.stats().has_summary {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    assert!(
        manager.stats().has_summary,
        "summary should be applied after compaction task completes"
    );
    let calls = manager.take_model_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].provider, "mock-summary");
    assert_eq!(calls[0].model, "mock-summary-model");
    assert_eq!(calls[0].usage.unwrap().input, 17);

    // After compaction, compacted_count should be > 0
    assert!(manager.compacted_count > 0);

    let msgs = manager.messages_for_api_with(&messages);
    assert!(msgs.len() < 30);
    let first = msgs.first().expect("summary message missing");
    assert_eq!(first.role, Role::User);
    match &first.content[0] {
        ContentBlock::Text { text, .. } => {
            assert!(text.contains("Previous Conversation Summary"));
        }
        _ => panic!("expected text summary block"),
    }
}

#[tokio::test]
async fn test_aborted_compaction_keeps_unknown_usage_model_call() {
    let mut manager = CompactionManager::new().with_budget(1_000);
    let mut messages = Vec::new();
    for i in 0..30 {
        messages.push(make_text_message(
            Role::User,
            &format!("Turn {} {}", i, "x".repeat(120)),
        ));
        manager.notify_message_added();
    }

    let blocking = Arc::new(BlockingSummaryProvider {
        started: Arc::new(Notify::new()),
    });
    let started_notify = blocking.started.clone();
    let started = started_notify.notified();
    let provider: Arc<dyn Provider> = blocking.clone();
    manager
        .force_compact_with(&messages, provider)
        .expect("manual compaction should start");
    started.await;

    manager
        .hard_compact_with(&messages)
        .expect("hard compaction should supersede the pending summary");
    let calls = manager.take_model_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].provider, "blocking-summary");
    assert_eq!(calls[0].model, "blocking-summary-model");
    assert!(calls[0].usage.is_none());
    assert!(
        calls[0]
            .error
            .as_deref()
            .unwrap()
            .contains("usage unavailable")
    );
}

// ── ensure_context_fits tests ──────────────────────────────

#[tokio::test]
async fn test_guard_below_80_does_nothing() {
    let mut manager = CompactionManager::new().with_budget(10_000);
    let mut messages = Vec::new();
    for i in 0..15 {
        messages.push(make_text_message(Role::User, &format!("msg {}", i)));
        manager.notify_message_added();
    }
    // Char estimate is tiny, observed tokens well below 80%
    manager.update_observed_input_tokens(5_000);

    let provider: Arc<dyn Provider> = Arc::new(MockSummaryProvider);
    let action = manager.ensure_context_fits(&messages, provider);
    assert_eq!(
        action,
        CompactionAction::None,
        "should do nothing below 80%"
    );
    assert!(
        !manager.is_compacting(),
        "should NOT start background compaction below 80%"
    );
    assert_eq!(manager.compacted_count, 0);
}

#[tokio::test]
async fn test_guard_between_80_and_95_starts_background_only() {
    let mut manager = CompactionManager::new().with_budget(1_000);
    let mut messages = Vec::new();
    for i in 0..20 {
        messages.push(make_text_message(Role::User, &format!("msg {}", i)));
        manager.notify_message_added();
    }
    // 85% usage — above 80% threshold but below 95% critical
    manager.update_observed_input_tokens(850);

    let provider: Arc<dyn Provider> = Arc::new(MockSummaryProvider);
    let action = manager.ensure_context_fits(&messages, provider);
    assert_eq!(
        action,
        CompactionAction::BackgroundStarted {
            trigger: "reactive".to_string()
        },
        "should start background compaction at 85%"
    );
    assert!(
        manager.is_compacting(),
        "SHOULD start background compaction at 85%"
    );
    assert_eq!(
        manager.compacted_count, 0,
        "compacted_count should stay 0 (no hard compact)"
    );
}

/// Regression: a hard compact that runs while a background (reactive)
/// compaction is in flight must abort the background task and discard its
/// stale `pending_cutoff`. Otherwise, when the background task completes,
/// `check_and_apply_compaction_with` adds the stale cutoff on top of the
/// already-advanced `compacted_count`, double-compacting and wiping out all
/// live messages (observed as "kept 0 recent messages").
#[tokio::test]
async fn test_hard_compact_aborts_inflight_background_compaction() {
    let mut manager = CompactionManager::new().with_budget(1_000);
    let mut messages = Vec::new();
    for i in 0..30 {
        messages.push(make_text_message(
            Role::User,
            &format!("turn {} content {}", i, "z".repeat(60)),
        ));
        manager.notify_message_added();
    }

    // Start a background reactive compaction (85% usage, below critical).
    manager.update_observed_input_tokens(850);
    let provider: Arc<dyn Provider> = Arc::new(MockSummaryProvider);
    manager.maybe_start_compaction_with(&messages, provider);
    assert!(
        manager.is_compacting(),
        "background compaction should be in flight"
    );
    let inflight_cutoff = manager.pending_cutoff;
    assert!(inflight_cutoff > 0, "background task should have a cutoff");

    // Now pressure spikes to critical and we hard-compact synchronously while
    // the background task is still pending.
    let dropped = manager
        .hard_compact_with(&messages)
        .expect("hard compact should succeed");
    assert!(dropped > 0);

    // The in-flight background compaction must have been aborted/discarded.
    assert!(
        !manager.is_compacting(),
        "hard compact must abort the in-flight background compaction"
    );
    assert_eq!(
        manager.pending_cutoff, 0,
        "stale pending_cutoff must be reset"
    );

    let compacted_after_hard = manager.compacted_count;

    // Simulate the (now-aborted) background task completion path. With the fix
    // there is no pending task, so this is a no-op and must NOT advance
    // compacted_count again.
    manager.check_and_apply_compaction_with(&messages);
    assert_eq!(
        manager.compacted_count, compacted_after_hard,
        "completing after abort must not double-advance compacted_count"
    );

    // Live messages must survive: active_messages_count stays positive.
    assert!(
        manager.active_messages_count() > 0,
        "must keep recent messages live, not wipe everything to 0"
    );
    let active = manager.active_messages(&messages);
    assert!(
        !active.is_empty(),
        "active message slice must not be empty after hard compact"
    );
}

/// Defense-in-depth: if `compacted_count` advances while a background
/// compaction is in flight (so its `pending_cutoff` becomes stale), applying
/// the completed result must NOT over-advance `compacted_count` and wipe the
/// live tail. The stale result should be discarded instead.
#[tokio::test]
async fn test_stale_background_result_discarded_when_context_shrinks() {
    let mut manager = CompactionManager::new().with_budget(1_000);
    let mut messages = Vec::new();
    for i in 0..30 {
        messages.push(make_text_message(
            Role::User,
            &format!("turn {} content {}", i, "q".repeat(60)),
        ));
        manager.notify_message_added();
    }

    manager.update_observed_input_tokens(850);
    let provider: Arc<dyn Provider> = Arc::new(MockSummaryProvider);
    manager.maybe_start_compaction_with(&messages, provider);
    assert!(manager.is_compacting());
    let pending = manager.pending_cutoff;
    assert!(pending > 0);

    // Simulate an interleaving mutation that advances compacted_count out from
    // under the in-flight task (e.g. a hard compact via a different path),
    // leaving only a small active tail.
    manager.compacted_count = messages.len() - 3;

    // Drain the background task to completion, then apply.
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        manager.check_and_apply_compaction_with(&messages);
        if !manager.is_compacting() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(!manager.is_compacting(), "task should have been drained");

    // The stale result must have been discarded: compacted_count stays where
    // the interleaving mutation left it, and the live tail survives.
    assert_eq!(
        manager.compacted_count,
        messages.len() - 3,
        "stale pending_cutoff must not advance compacted_count further"
    );
    assert!(
        manager.active_messages(&messages).len() >= 3,
        "live tail must survive a discarded stale compaction"
    );
    assert_eq!(manager.pending_cutoff, 0, "pending_cutoff must be reset");
}

#[tokio::test]
async fn test_guard_at_95_triggers_hard_compact() {
    let mut manager = CompactionManager::new().with_budget(1_000);
    let mut messages = Vec::new();
    for i in 0..20 {
        messages.push(make_text_message(
            Role::User,
            &format!("message {} with padding {}", i, "x".repeat(50)),
        ));
        manager.notify_message_added();
    }
    // 96% usage — above critical threshold
    manager.update_observed_input_tokens(960);

    let provider: Arc<dyn Provider> = Arc::new(MockSummaryProvider);
    let action = manager.ensure_context_fits(&messages, provider);
    assert!(
        matches!(action, CompactionAction::HardCompacted(_)),
        "SHOULD hard-compact at 96%"
    );
    assert!(
        manager.compacted_count > 0,
        "compacted_count should increase after hard compact"
    );
    assert!(
        manager.active_summary.is_some(),
        "should have an emergency summary"
    );
}

#[tokio::test]
async fn test_guard_at_100_percent_drops_messages() {
    let mut manager = CompactionManager::new().with_budget(1_000);
    let mut messages = Vec::new();
    for i in 0..30 {
        messages.push(make_text_message(
            Role::User,
            &format!("turn {} content {}", i, "y".repeat(80)),
        ));
        manager.notify_message_added();
    }
    // Over 100% — simulates the exact bug scenario
    manager.update_observed_input_tokens(1_050);

    let provider: Arc<dyn Provider> = Arc::new(MockSummaryProvider);
    let action = manager.ensure_context_fits(&messages, provider);
    assert!(
        matches!(action, CompactionAction::HardCompacted(_)),
        "MUST hard-compact when over 100%"
    );

    let api_messages = manager.messages_for_api_with(&messages);
    assert!(
        api_messages.len() < messages.len(),
        "API messages should be fewer after hard compact"
    );
    // First message should be the emergency summary
    match &api_messages[0].content[0] {
        ContentBlock::Text { text, .. } => {
            assert!(text.contains("Previous Conversation Summary"));
            assert!(text.contains("Emergency compaction"));
        }
        _ => panic!("expected text summary block"),
    }
}

// ── hard_compact_with edge cases ────────────────────────────────

#[test]
fn test_hard_compact_too_few_messages() {
    let mut manager = CompactionManager::new().with_budget(100);
    let messages = vec![
        make_text_message(Role::User, "hello"),
        make_text_message(Role::Assistant, "hi"),
    ];
    manager.notify_message_added();
    manager.notify_message_added();

    let result = manager.hard_compact_with(&messages);
    assert!(
        result.is_err(),
        "should fail with only 2 messages (MIN_TURNS_TO_KEEP)"
    );
}

#[test]
fn test_hard_compact_preserves_recent_turns() {
    let mut manager = CompactionManager::new().with_budget(1_000);
    let mut messages = Vec::new();
    for i in 0..25 {
        messages.push(make_text_message(Role::User, &format!("turn {}", i)));
        manager.notify_message_added();
    }
    manager.update_observed_input_tokens(950);

    let dropped = manager
        .hard_compact_with(&messages)
        .expect("should compact");
    assert!(dropped > 0, "should drop some messages");
    assert!(dropped < 25, "should not drop ALL messages");

    let api_messages = manager.messages_for_api_with(&messages);
    // Should have summary + recent turns
    assert!(
        api_messages.len() >= 2,
        "should keep at least MIN_TURNS_TO_KEEP + summary"
    );
    assert!(
        api_messages.len() <= 15,
        "should have dropped a significant number"
    );
}

// ── safe_compaction_cutoff: tool call/result pair integrity ─────────

#[test]
fn test_safe_cutoff_preserves_tool_pairs() {
    // Messages: [user, assistant(tool_use), user(tool_result), assistant, user]
    // If cutoff tries to split between tool_use and tool_result, it should back up
    let messages = vec![
        make_text_message(Role::User, "do something"),
        Message {
            role: Role::Assistant,
            content: vec![ContentBlock::ToolUse {
                id: "tool_1".to_string(),
                name: "bash".to_string(),
                input: serde_json::json!({"command": "ls"}),
                thought_signature: None,
            }],
            timestamp: None,
            tool_duration_ms: None,
        },
        Message {
            role: Role::User,
            content: vec![ContentBlock::ToolResult {
                tool_use_id: "tool_1".to_string(),
                content: "file1.txt\nfile2.txt".to_string(),
                is_error: Some(false),
            }],
            timestamp: None,
            tool_duration_ms: None,
        },
        make_text_message(Role::Assistant, "I see the files"),
        make_text_message(Role::User, "thanks"),
    ];

    // Try to cut between tool_use (index 1) and tool_result (index 2)
    let cutoff = safe_compaction_cutoff(&messages, 2);
    // Should move back to include the tool_use at index 1
    assert!(
        cutoff <= 1,
        "cutoff should back up to include tool_use (got {})",
        cutoff
    );
}

#[test]
fn test_safe_cutoff_no_tool_pairs() {
    let messages = vec![
        make_text_message(Role::User, "hello"),
        make_text_message(Role::Assistant, "hi"),
        make_text_message(Role::User, "how are you"),
        make_text_message(Role::Assistant, "fine"),
    ];

    let cutoff = safe_compaction_cutoff(&messages, 2);
    assert_eq!(cutoff, 2, "no tool pairs, cutoff should stay unchanged");
}

#[test]
fn test_safe_cutoff_handles_chained_tool_dependencies_without_rescan() {
    let messages = vec![
        Message {
            role: Role::Assistant,
            content: vec![ContentBlock::ToolUse {
                id: "tool_a".to_string(),
                name: "read".to_string(),
                input: serde_json::json!({"file": "a.txt"}),
                thought_signature: None,
            }],
            timestamp: None,
            tool_duration_ms: None,
        },
        make_text_message(Role::User, "intermediate"),
        Message {
            role: Role::Assistant,
            content: vec![
                ContentBlock::ToolResult {
                    tool_use_id: "tool_a".to_string(),
                    content: "a contents".to_string(),
                    is_error: Some(false),
                },
                ContentBlock::ToolUse {
                    id: "tool_b".to_string(),
                    name: "grep".to_string(),
                    input: serde_json::json!({"pattern": "foo"}),
                    thought_signature: None,
                },
            ],
            timestamp: None,
            tool_duration_ms: None,
        },
        Message {
            role: Role::User,
            content: vec![ContentBlock::ToolResult {
                tool_use_id: "tool_b".to_string(),
                content: "foo".to_string(),
                is_error: Some(false),
            }],
            timestamp: None,
            tool_duration_ms: None,
        },
        make_text_message(Role::Assistant, "done"),
    ];

    let cutoff = safe_compaction_cutoff(&messages, 3);
    assert_eq!(
        cutoff, 0,
        "cutoff should walk back through nested tool dependencies until the kept suffix is self-contained"
    );
}

// ── emergency_truncate_with ─────────────────────────────────────

#[test]
fn test_emergency_truncate_large_tool_results() {
    let mut manager = CompactionManager::new().with_budget(1_000);
    let big_result = "x".repeat(10_000); // Way over EMERGENCY_TOOL_RESULT_MAX_CHARS (4000)
    let mut messages = vec![
        make_text_message(Role::User, "run something"),
        Message {
            role: Role::Assistant,
            content: vec![ContentBlock::ToolUse {
                id: "tool_1".to_string(),
                name: "bash".to_string(),
                input: serde_json::json!({"command": "cat bigfile"}),
                thought_signature: None,
            }],
            timestamp: None,
            tool_duration_ms: None,
        },
        Message {
            role: Role::User,
            content: vec![ContentBlock::ToolResult {
                tool_use_id: "tool_1".to_string(),
                content: big_result.clone(),
                is_error: Some(false),
            }],
            timestamp: None,
            tool_duration_ms: None,
        },
        make_text_message(Role::Assistant, "that's a big file"),
    ];
    for _ in &messages {
        manager.notify_message_added();
    }

    let truncated = manager.emergency_truncate_with(&mut messages);
    assert_eq!(truncated, 1, "should truncate exactly 1 tool result");

    // Check the truncated content
    if let ContentBlock::ToolResult { content, .. } = &messages[2].content[0] {
        assert!(
            content.len() < big_result.len(),
            "content should be shorter"
        );
        assert!(
            content.contains("truncated for context recovery"),
            "should have truncation marker"
        );
    } else {
        panic!("expected tool result");
    }
}

#[test]
fn test_emergency_truncate_skips_small_results() {
    let mut manager = CompactionManager::new().with_budget(1_000);
    let mut messages = vec![Message {
        role: Role::User,
        content: vec![ContentBlock::ToolResult {
            tool_use_id: "tool_1".to_string(),
            content: "small output".to_string(),
            is_error: Some(false),
        }],
        timestamp: None,
        tool_duration_ms: None,
    }];
    manager.notify_message_added();

    let truncated = manager.emergency_truncate_with(&mut messages);
    assert_eq!(truncated, 0, "should not truncate small results");
}

// ── Double compaction ───────────────────────────────────────────

#[test]
fn test_hard_compact_twice() {
    let mut manager = CompactionManager::new().with_budget(500);
    let mut messages = Vec::new();
    for i in 0..30 {
        messages.push(make_text_message(
            Role::User,
            &format!("turn {} {}", i, "z".repeat(40)),
        ));
        manager.notify_message_added();
    }
    manager.update_observed_input_tokens(480);

    // First hard compact
    let dropped1 = manager
        .hard_compact_with(&messages)
        .expect("first compact should work");
    assert!(dropped1 > 0);
    let count_after_first = manager.compacted_count;

    // Simulate more messages arriving after first compact
    for i in 30..45 {
        messages.push(make_text_message(
            Role::User,
            &format!("turn {} {}", i, "z".repeat(40)),
        ));
        manager.notify_message_added();
    }
    manager.update_observed_input_tokens(490);

    // Second hard compact
    let dropped2 = manager
        .hard_compact_with(&messages)
        .expect("second compact should work");
    assert!(dropped2 > 0);
    assert!(
        manager.compacted_count > count_after_first,
        "compacted_count should increase"
    );

    // Summary should mention both compactions
    let api_messages = manager.messages_for_api_with(&messages);
    assert!(api_messages.len() < messages.len());
    match &api_messages[0].content[0] {
        ContentBlock::Text { text, .. } => {
            assert!(text.contains("Emergency compaction"));
        }
        _ => panic!("expected summary"),
    }
}

#[test]
fn test_hard_compact_clamps_pathological_compacted_count() {
    let mut manager = CompactionManager::new().with_budget(1_000);
    let mut messages = Vec::new();
    for i in 0..30 {
        messages.push(make_text_message(
            Role::User,
            &format!("turn {} content {}", i, "x".repeat(200)),
        ));
        manager.notify_message_added();
    }

    // Reproduce the #175 bad state: bookkeeping says more messages were
    // compacted than exist in the current message vector. Before the fix,
    // active_messages() returned the full transcript in this state, so each
    // hard compaction appended another emergency marker and increased
    // compacted_count even further past messages.len().
    manager.compacted_count = 100;
    manager.active_summary = Some(Summary {
        text: "# Existing summary".to_string(),
        openai_encrypted_content: None,
        covers_up_to_turn: 100,
        original_turn_count: 100,
    });
    manager.active_chars.invalidate();

    for _ in 0..3 {
        let _ = manager.hard_compact_with(&messages);
    }

    assert_eq!(
        manager.compacted_count,
        messages.len(),
        "hard compaction must clamp compacted_count to the available messages"
    );
    let summary_markers = manager
        .active_summary
        .as_ref()
        .map(|summary| summary.text.matches("[Emergency compaction]").count())
        .unwrap_or(0);
    assert_eq!(
        summary_markers, 0,
        "pathological state should not append repeated emergency markers"
    );

    let api_messages = manager.messages_for_api_with(&messages);
    assert_eq!(
        api_messages.len(),
        1,
        "all current messages should remain covered by the existing summary until new turns arrive"
    );
}

#[test]
fn test_hard_compact_reduces_api_payload_and_reports_saved_tokens() {
    let mut manager = CompactionManager::new().with_budget(1_000);
    let mut messages = Vec::new();
    for i in 0..40 {
        messages.push(make_text_message(
            Role::User,
            &format!("turn {} {}", i, "payload ".repeat(80)),
        ));
        manager.notify_message_added();
    }

    let pre_api_messages = manager.messages_for_api_with(&messages);
    let pre_chars: usize = pre_api_messages.iter().map(message_char_count).sum();
    let pre_tokens = manager.effective_token_count_with(&messages);

    manager
        .hard_compact_with(&messages)
        .expect("hard compaction should recover oversized context");

    let post_api_messages = manager.messages_for_api_with(&messages);
    let post_chars: usize = post_api_messages.iter().map(message_char_count).sum();
    let post_tokens = manager.effective_token_count_with(&messages);
    let event = manager
        .take_compaction_event()
        .expect("hard compaction should publish an event");

    assert!(
        post_api_messages.len() < pre_api_messages.len(),
        "hard compaction should send fewer messages"
    );
    assert!(
        post_chars < pre_chars,
        "hard compaction should reduce outgoing payload chars: pre={pre_chars}, post={post_chars}"
    );
    assert!(
        post_tokens <= pre_tokens,
        "hard compaction must not increase effective tokens: pre={pre_tokens}, post={post_tokens}"
    );
    assert!(
        event.tokens_saved.unwrap_or(0) > 0,
        "event should attribute positive token savings: {event:?}"
    );
}

#[test]
fn test_invalid_compacted_count_does_not_resurrect_full_transcript_after_new_turn() {
    let mut manager = CompactionManager::new().with_budget(1_000);
    let mut messages = Vec::new();
    for i in 0..30 {
        messages.push(make_text_message(
            Role::User,
            &format!("old turn {} {}", i, "x".repeat(120)),
        ));
        manager.notify_message_added();
    }

    manager.compacted_count = 500;
    manager.active_summary = Some(Summary {
        text: "# Existing summary".to_string(),
        openai_encrypted_content: None,
        covers_up_to_turn: 500,
        original_turn_count: 500,
    });
    manager.active_chars.invalidate();

    let before_new_turn = manager.messages_for_api_with(&messages);
    assert_eq!(before_new_turn.len(), 1);
    assert_eq!(manager.compacted_count(), messages.len());

    messages.push(make_text_message(Role::User, "new turn after restore"));
    manager.notify_message_added();

    let after_new_turn = manager.messages_for_api_with(&messages);
    assert_eq!(
        after_new_turn.len(),
        2,
        "request should contain summary plus only the new active turn"
    );
    match &after_new_turn[1].content[0] {
        ContentBlock::Text { text, .. } => assert_eq!(text, "new turn after restore"),
        _ => panic!("expected new active text turn"),
    }
}

// ── messages_for_api_with after compaction ──────────────────────

#[test]
fn test_messages_for_api_with_summary_prepended() {
    let mut manager = CompactionManager::new().with_budget(500);
    let mut messages = Vec::new();
    for i in 0..20 {
        messages.push(make_text_message(Role::User, &format!("turn {}", i)));
        manager.notify_message_added();
    }
    manager.update_observed_input_tokens(490);

    manager
        .hard_compact_with(&messages)
        .expect("should compact");

    let api_msgs = manager.messages_for_api_with(&messages);
    // First message should be the summary
    assert_eq!(api_msgs[0].role, Role::User);
    match &api_msgs[0].content[0] {
        ContentBlock::Text { text, .. } => {
            assert!(text.starts_with("## Previous Conversation Summary"));
        }
        _ => panic!("expected text"),
    }
    // Remaining should be recent turns from original messages
    assert!(api_msgs.len() < messages.len());
}

#[test]
fn test_persisted_state_round_trip_preserves_compacted_view() {
    let mut manager = CompactionManager::new().with_budget(500);
    let mut messages = Vec::new();
    for i in 0..20 {
        messages.push(make_text_message(
            Role::User,
            &format!("turn {} {}", i, "x".repeat(40)),
        ));
        manager.notify_message_added();
    }
    manager.update_observed_input_tokens(490);
    manager
        .hard_compact_with(&messages)
        .expect("should compact before persisting");

    let persisted = manager
        .persisted_state()
        .expect("compaction state should be exportable");
    let expected = manager.messages_for_api_with(&messages);

    let mut restored = CompactionManager::new().with_budget(500);
    restored.restore_persisted_state(&persisted, messages.len());
    let restored_msgs = restored.messages_for_api_with(&messages);

    assert_eq!(restored.compacted_count, persisted.compacted_count);
    assert_eq!(restored_msgs.len(), expected.len());
    match &restored_msgs[0].content[0] {
        ContentBlock::Text { text, .. } => {
            assert!(text.contains("Previous Conversation Summary"));
            assert!(text.contains("Emergency compaction"));
        }
        _ => panic!("expected restored summary block"),
    }
}

// ── context_usage accuracy ──────────────────────────────────────

#[test]
fn test_context_usage_with_both_estimate_and_observed() {
    let mut manager = CompactionManager::new().with_budget(200_000);
    // Build messages totalling ~50k chars = ~12.5k token estimate
    let mut messages = Vec::new();
    for i in 0..50 {
        messages.push(make_text_message(
            Role::User,
            &format!("{} {}", i, "a".repeat(1000)),
        ));
        manager.notify_message_added();
    }

    // Without observed tokens, usage should be based on char estimate
    let usage_no_observed = manager.context_usage_with(&messages);
    let estimate = manager.effective_token_count_with(&messages);
    let budget = manager.token_budget();
    assert_eq!(usage_no_observed, estimate as f32 / budget as f32);

    // With observed tokens at 160k, should use observed (higher) value
    manager.update_observed_input_tokens(160_000);
    let usage_with_observed = manager.context_usage_with(&messages);
    assert!(
        usage_with_observed > usage_no_observed,
        "observed usage should exceed the character estimate: {usage_with_observed} <= {usage_no_observed}"
    );
}

#[test]
fn test_context_usage_after_compaction_resets_observed() {
    let mut manager = CompactionManager::new().with_budget(1_000);
    let mut messages = Vec::new();
    for i in 0..20 {
        messages.push(make_text_message(
            Role::User,
            &format!("msg {} pad {}", i, "x".repeat(50)),
        ));
        manager.notify_message_added();
    }
    manager.update_observed_input_tokens(960);

    // Hard compact should reset observed_input_tokens
    manager
        .hard_compact_with(&messages)
        .expect("should compact");
    assert!(
        manager.observed_input_tokens.is_none(),
        "observed_input_tokens should be cleared after hard compact"
    );

    // After compaction, usage should be based on char estimate of remaining messages only
    let post_usage = manager.context_usage_with(&messages);
    // The remaining messages are small, so usage should be well below the critical threshold
    assert!(
        post_usage < CRITICAL_THRESHOLD,
        "post-compaction usage should be below critical: {}",
        post_usage
    );
}

#[test]
fn test_recover_within_budget_drops_messages_without_truncation() {
    let mut manager = CompactionManager::new().with_budget(1_000);
    let mut messages = Vec::new();
    for i in 0..30 {
        messages.push(make_text_message(
            Role::User,
            &format!("msg {} pad {}", i, "x".repeat(40)),
        ));
        manager.notify_message_added();
    }
    // Push well over budget so recovery triggers.
    manager.update_observed_input_tokens(2_000);

    let recovery = manager.recover_within_budget(&mut messages);
    assert!(
        recovery.dropped.unwrap_or(0) > 0,
        "should drop old messages"
    );
    // Dropping turns alone should fit the small remaining tail, so no
    // truncation escalation is needed.
    assert_eq!(
        recovery.truncated, 0,
        "should not truncate when dropping turns fits the budget"
    );
    assert!(recovery.did_anything());
    assert!(
        manager.context_usage_with(&messages) <= 1.0,
        "context should be back under budget after recovery"
    );
}

#[test]
fn test_recover_within_budget_truncates_when_tail_still_too_large() {
    let mut manager = CompactionManager::new().with_budget(1_000);
    let mut messages = Vec::new();
    // Build tool-use/tool-result pairs whose results are each individually
    // larger than the whole budget. After hard compaction drops down to the
    // minimum kept tail, the surviving tool result is still far over budget, so
    // recovery must escalate to truncation (which only acts on tool results).
    for i in 0..10 {
        let id = format!("tool_{i}");
        messages.push(Message {
            role: Role::Assistant,
            content: vec![ContentBlock::ToolUse {
                id: id.clone(),
                name: "bash".to_string(),
                input: serde_json::json!({ "command": "cat big.log" }),
                thought_signature: None,
            }],
            timestamp: None,
            tool_duration_ms: None,
        });
        manager.notify_message_added();
        messages.push(Message {
            role: Role::User,
            content: vec![ContentBlock::ToolResult {
                tool_use_id: id,
                content: format!("huge {} {}", i, "y".repeat(20_000)),
                is_error: Some(false),
            }],
            timestamp: None,
            tool_duration_ms: None,
        });
        manager.notify_message_added();
    }
    manager.update_observed_input_tokens(50_000);

    let recovery = manager.recover_within_budget(&mut messages);
    assert!(recovery.did_anything());
    assert!(
        recovery.truncated > 0,
        "should escalate to truncation when the remaining tail is still too large"
    );
}

#[test]
fn test_recover_within_budget_summary_line_variants() {
    let dropped_only = EmergencyRecovery {
        pre_usage: 1.6,
        dropped: Some(7),
        truncated: 0,
    };
    let line = dropped_only.summary_line(dropped_only.pre_usage);
    assert!(line.contains("dropped 7 old messages"));
    assert!(line.contains("160%"));
    assert!(!line.contains("truncated"));

    let dropped_and_truncated = EmergencyRecovery {
        pre_usage: 2.0,
        dropped: Some(3),
        truncated: 2,
    };
    let line = dropped_and_truncated.summary_line(dropped_and_truncated.pre_usage);
    assert!(line.contains("dropped 3 old messages"));
    assert!(line.contains("truncated 2 tool result(s)"));

    let truncate_only = EmergencyRecovery {
        pre_usage: 1.2,
        dropped: None,
        truncated: 5,
    };
    let line = truncate_only.summary_line(truncate_only.pre_usage);
    assert!(line.contains("shortened 5 large tool result(s)"));
    assert!(!line.contains("dropped"));
}

#[test]
fn threshold_tokens_lowers_the_soft_trigger_on_large_windows_only() {
    let limits = CompactionLimits::default();
    // A 1M window at the default 0.80 would fold at 800k; the 256k cap folds it at 25.6%, floored at 0.30.
    assert_eq!(with_token_threshold(limits, Some(256_000), 1_000_000).soft, 0.30);
    assert_eq!(with_token_threshold(limits, Some(300_000), 1_000_000).soft, 0.30);
    assert!((with_token_threshold(limits, Some(400_000), 1_000_000).soft - 0.40).abs() < 1e-6);
    // A window smaller than the cap keeps the ratio; unset, zero or a negative count change nothing.
    assert_eq!(with_token_threshold(limits, Some(256_000), 200_000), limits);
    assert_eq!(with_token_threshold(limits, None, 1_000_000), limits);
    assert_eq!(with_token_threshold(limits, Some(0), 1_000_000), limits);
    assert_eq!(with_token_threshold(limits, Some(-5), 1_000_000), limits);
    assert_eq!(with_token_threshold(limits, Some(100), 0), limits);
}

#[test]
fn live_plan_is_appended_and_replaced_not_duplicated() {
    let todo = |content: &str, status: &str, id: &str| factr_task_types::TodoItem {
        content: content.into(),
        status: status.into(),
        priority: "medium".into(),
        id: id.into(),
        ..Default::default()
    };
    let todos = vec![
        todo("ship guard", "in_progress", "1"),
        todo("old work", "completed", "2"),
    ];
    let once = attach_live_plan("## Goal\nx".to_string(), &todos);
    assert!(once.contains("ship guard"));
    assert!(!once.contains("old work"));
    let twice = attach_live_plan(once, &[todo("next", "pending", "3")]);
    assert_eq!(twice.matches(LIVE_PLAN_HEADING).count(), 1);
    assert!(twice.contains("next") && !twice.contains("ship guard"));
    assert_eq!(attach_live_plan("s".into(), &[]), "s");
}

// ── Factr compression.* overrides ─────────────────────────

fn manager_with_20_messages(limits: Option<CompactionLimits>) -> (CompactionManager, Vec<Message>) {
    let mut manager = CompactionManager::new().with_budget(1_000);
    manager.set_limits(limits);
    let mut messages = Vec::new();
    for i in 0..20 {
        messages.push(make_text_message(Role::User, &format!("msg {}", i)));
        manager.notify_message_added();
    }
    (manager, messages)
}

#[test]
fn factr_compression_block_maps_onto_clamped_limits() {
    let cfg = crate::factr_config::FactrConfig::parse(
        "compression:\n  enabled: true\n  threshold: 0.5\n  protect_last_n: 4\n",
    );
    let limits = limits_from(&cfg.compression);
    assert_eq!((limits.soft, limits.keep_recent, limits.enabled), (0.5, 4, true));
    assert_eq!(limits_from(&Default::default()), CompactionLimits::default(), "unset keys leave the defaults");
}

#[tokio::test]
async fn a_lower_configured_threshold_starts_compaction_below_the_default_80_percent() {
    // 60% usage: the default 80% trigger stays quiet, threshold 0.5 compacts.
    let (mut quiet, messages) = manager_with_20_messages(None);
    quiet.update_observed_input_tokens(600);
    assert_eq!(quiet.ensure_context_fits(&messages, Arc::new(MockSummaryProvider)), CompactionAction::None);

    let limits = CompactionLimits::with_overrides(None, Some(0.5), None, Some(4));
    let (mut eager, messages) = manager_with_20_messages(Some(limits));
    eager.update_observed_input_tokens(600);
    assert!(matches!(
        eager.ensure_context_fits(&messages, Arc::new(MockSummaryProvider)),
        CompactionAction::BackgroundStarted { .. }
    ));
    assert_eq!(eager.pending_cutoff, 16, "keeps the configured 4 recent messages");
}

#[tokio::test]
async fn a_larger_target_ratio_keeps_a_longer_verbatim_tail() {
    // Each message is ~6 chars (2 tokens); budget 1000 x soft 0.8 x ratio 0.2 = 160 tokens fits them all,
    // so the tail grows past protect_last_n and nothing is left to compact.
    let limits = CompactionLimits::with_overrides(None, None, Some(0.2), Some(4));
    let (mut manager, messages) = manager_with_20_messages(Some(limits));
    manager.update_observed_input_tokens(850);
    let active = manager.active_messages(&messages);
    assert_eq!(manager.keep_turns(&limits, active), 20);
    assert_eq!(manager.ensure_context_fits(&messages, Arc::new(MockSummaryProvider)), CompactionAction::None);
}

#[tokio::test]
async fn compression_disabled_stops_automatic_compaction_but_not_hard_or_manual_recovery() {
    let off = CompactionLimits::with_overrides(Some(false), None, None, None);
    let (mut manager, messages) = manager_with_20_messages(Some(off));
    // Past both the soft (80%) and hard (95%) thresholds: still nothing automatic.
    manager.update_observed_input_tokens(990);
    assert_eq!(manager.ensure_context_fits(&messages, Arc::new(MockSummaryProvider)), CompactionAction::None);
    assert!(!manager.is_compacting() && manager.compacted_count == 0);
    // The emergency path (context-limit error) drops old messages regardless.
    let dropped = manager.hard_compact_with(&messages).expect("emergency hard compact still works");
    assert!(dropped > 0 && manager.compacted_count > 0);
}

struct SwitchableProvider {
    model: Arc<Mutex<String>>,
    accepts: bool,
}

#[async_trait::async_trait]
impl Provider for SwitchableProvider {
    async fn complete(
        &self,
        _messages: &[Message],
        _tools: &[crate::message::ToolDefinition],
        _system: &str,
        _resume_session_id: Option<&str>,
    ) -> Result<EventStream> {
        Ok(Box::pin(futures::stream::empty()))
    }
    fn name(&self) -> &str {
        "switchable"
    }
    fn model(&self) -> String {
        self.model.lock().unwrap().clone()
    }
    fn set_model(&self, model: &str) -> Result<()> {
        if !self.accepts {
            anyhow::bail!("no credentials");
        }
        *self.model.lock().unwrap() = model.to_string();
        Ok(())
    }
    fn fork(&self) -> Arc<dyn Provider> {
        Arc::new(Self { model: Arc::new(Mutex::new(self.model.lock().unwrap().clone())), accepts: self.accepts })
    }
}

#[test]
fn the_summary_model_comes_from_auxiliary_compression_on_a_fork_and_a_bad_one_is_ignored() {
    let session: Arc<dyn Provider> = Arc::new(SwitchableProvider { model: Arc::new(Mutex::new("main".into())), accepts: true });
    assert_eq!(summarizer_with(session.clone(), None).model(), "main");
    let summarizer = summarizer_with(session.clone(), Some("openrouter:cheap"));
    assert_eq!(summarizer.model(), "openrouter:cheap");
    assert_eq!(session.model(), "main", "the session's own model is never switched");
    let refusing: Arc<dyn Provider> = Arc::new(SwitchableProvider { model: Arc::new(Mutex::new("main".into())), accepts: false });
    assert_eq!(summarizer_with(refusing, Some("openrouter:cheap")).model(), "main", "a setting that cannot apply falls back");
}
