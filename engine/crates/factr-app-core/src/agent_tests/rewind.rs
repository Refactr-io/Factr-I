//! `rewind_to_message` / `undo_rewind`: compaction reset and the session-context message.

use super::*;

fn text(role: Role, t: &str) -> (Role, Vec<ContentBlock>) {
    (role, vec![ContentBlock::Text { text: t.into(), cache_control: None }])
}

fn temp_home() -> (std::sync::MutexGuard<'static, ()>, tempfile::TempDir, Option<std::ffi::OsString>) {
    let lock = crate::storage::lock_test_env();
    let home = tempfile::tempdir().unwrap();
    let prev = std::env::var_os("FACTR_HOME");
    crate::env::set_var("FACTR_HOME", home.path());
    crate::config::Config::invalidate_cache();
    (lock, home, prev)
}

fn restore_home(prev: Option<std::ffi::OsString>) {
    match prev {
        Some(h) => crate::env::set_var("FACTR_HOME", h),
        None => crate::env::remove_var("FACTR_HOME"),
    }
    crate::config::Config::invalidate_cache();
}

fn all_text(messages: &[Message]) -> String {
    messages
        .iter()
        .flat_map(|m| m.content.iter())
        .filter_map(|b| match b {
            ContentBlock::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[tokio::test]
async fn rewinding_past_a_compaction_drops_its_summary_and_redo_brings_it_back() {
    let (_lock, _home, prev) = temp_home();
    let provider: Arc<dyn Provider> = Arc::new(NativeAutoCompactionProvider);
    let registry = Registry::new(provider.clone()).await;
    let mut agent = Agent::new(provider, registry);
    for i in 0..30 {
        let (role, content) = text(Role::User, &format!("turn {i} {}", "x".repeat(120)));
        agent.add_message(role, content);
    }
    let (_, started) = agent.request_manual_compaction();
    assert!(started);
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        let (messages, event) = agent.messages_for_provider();
        if event.is_some() {
            assert!(all_text(&messages).contains("manual summary from native-auto provider"));
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let covered = agent.session.compaction.as_ref().expect("compacted").compacted_count;
    assert!(covered > 2);

    let removed = agent.rewind_to_message(2).unwrap();
    assert_eq!(removed, 28);
    assert!(agent.session.compaction.is_none(), "the summary covers undone turns");
    let (after, _) = agent.messages_for_provider();
    let seen = all_text(&after);
    assert!(!seen.contains("manual summary") && !seen.contains("Previous Conversation Summary"), "{seen}");
    let kept = agent.session.messages.len();
    assert_eq!(after.len(), kept, "only the surviving messages (context + 2) reach the model: {seen}");
    assert_eq!(kept, 3);

    // A new message after the undo is the only addition.
    let (role, content) = text(Role::User, "brand new");
    agent.add_message(role, content);
    let (next, _) = agent.messages_for_provider();
    assert_eq!(next.len(), kept + 1);
    assert!(all_text(&next).ends_with("brand new") && !all_text(&next).contains("manual summary"));

    // Redo of the rewind (transcript is back to the rewound length first) restores the summary.
    agent.session.truncate_messages(kept);
    assert_eq!(agent.undo_rewind().unwrap(), 28);
    assert_eq!(agent.session.compaction.as_ref().map(|c| c.compacted_count), Some(covered));
    restore_home(prev);
}

#[tokio::test]
async fn rewinding_to_nothing_keeps_the_session_context_message() {
    let (_lock, _home, prev) = temp_home();
    let provider: Arc<dyn Provider> = Arc::new(NativeAutoCompactionProvider);
    let registry = Registry::new(provider.clone()).await;
    let mut agent = Agent::new(provider, registry);
    agent.session.ensure_initial_session_context_message();
    assert!(agent.session.has_session_context_message());
    for (role, t) in [(Role::User, "hello"), (Role::Assistant, "hi")] {
        let (role, content) = text(role, t);
        agent.add_message(role, content);
    }
    assert_eq!(agent.rewind_to_message(0).unwrap(), 2);
    assert!(agent.session.has_session_context_message(), "the first-turn undo keeps the context message");
    assert_eq!(agent.session.messages.len(), 1);
    let (role, content) = text(Role::User, "again");
    agent.add_message(role, content);
    let (sent, _) = agent.messages_for_provider();
    assert_eq!(sent.len(), 2);
    assert!(all_text(&sent).starts_with("<system-reminder>") && all_text(&sent).ends_with("again"));
    // and the removed turn can come back
    agent.session.truncate_messages(1);
    assert_eq!(agent.undo_rewind().unwrap(), 2);
    assert_eq!(agent.session.messages.len(), 3);

    // a session that never had one gets it on rewind(0), like a fresh session
    let provider: Arc<dyn Provider> = Arc::new(NativeAutoCompactionProvider);
    let registry = Registry::new(provider.clone()).await;
    let mut bare = Agent::new(provider, registry);
    let (role, content) = text(Role::User, "solo");
    bare.add_message(role, content);
    bare.rewind_to_message(0).unwrap();
    assert!(bare.session.has_session_context_message() && bare.session.messages.len() == 1);
    restore_home(prev);
}

fn marker_of(home: &std::path::Path, session: &str) -> Option<String> {
    let out = std::process::Command::new("sqlite3")
        .arg(home.join("factr.db"))
        .arg(format!("SELECT value FROM memory_meta WHERE key='extracted_through:{session}'"))
        .output()
        .ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

#[tokio::test]
async fn rewinding_below_the_extraction_marker_lowers_it_at_once() {
    if std::process::Command::new("sqlite3").arg("-version").output().is_err() {
        return; // no sqlite3 CLI to read the marker back with
    }
    let (_lock, home, prev) = temp_home();
    let provider: Arc<dyn Provider> = Arc::new(NativeAutoCompactionProvider);
    let registry = Registry::new(provider.clone()).await;
    let mut agent = Agent::new(provider, registry);
    agent.memory_enabled = true;
    for i in 0..10 {
        let (role, content) = text(if i % 2 == 0 { Role::User } else { Role::Assistant }, &format!("message {i}"));
        agent.add_message(role, content);
    }
    let id = agent.session_id().to_string();
    let total = agent.session.messages.len();
    crate::memory_extract::adopt_session(&id, total);
    assert_eq!(marker_of(home.path(), &id), Some(total.to_string()), "marker starts at the end of the transcript");
    agent.rewind_to_message(3).unwrap();
    let kept = agent.session.messages.len();
    assert!(kept < total);
    assert_eq!(marker_of(home.path(), &id), Some(kept.to_string()), "lowered at rewind time, not at the next extraction");
    restore_home(prev);
}
