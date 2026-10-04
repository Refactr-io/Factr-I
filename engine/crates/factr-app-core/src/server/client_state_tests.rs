use super::handle_get_history;
use super::handle_get_model_catalog;
use super::session_activity_snapshot;
use crate::agent::Agent;
use crate::message::{Message, ToolDefinition};
use crate::provider::{EventStream, Provider};
use crate::server::ClientConnectionInfo;
use crate::tool::Registry;
use anyhow::Result;
use async_trait::async_trait;
use std::collections::HashMap;
use std::io::BufRead as _;
use std::sync::Arc;
use std::time::Instant;
use tokio::io::AsyncReadExt;
use tokio::sync::{Mutex, RwLock, mpsc};

struct MockProvider(Option<&'static str>);

#[async_trait]
impl Provider for MockProvider {
    async fn complete(
        &self,
        _messages: &[Message],
        _tools: &[ToolDefinition],
        _system: &str,
        _resume_session_id: Option<&str>,
    ) -> Result<EventStream> {
        Err(anyhow::anyhow!(
            "mock provider complete should not be called in client_state tests"
        ))
    }

    fn name(&self) -> &str {
        "mock"
    }

    /// Deliberately different from `name()`: the provider class id and the
    /// profile label must not be interchangeable, or a fallback that reaches
    /// for `name()` where a display label belongs goes unnoticed (#1286).
    fn display_name(&self) -> String {
        "Mock Profile".to_string()
    }

    fn fork(&self) -> Arc<dyn Provider> {
        Arc::new(Self(self.0))
    }

    fn model(&self) -> String {
        "mock-model".to_string()
    }

    fn service_tier(&self) -> Option<String> {
        self.0.map(str::to_string)
    }

    fn reasoning_effort(&self) -> Option<String> {
        Some("high".to_string())
    }
}

#[tokio::test]
async fn session_activity_snapshot_prefers_live_tool_name_for_target_session() {
    let now = Instant::now();
    let client_connections = Arc::new(RwLock::new(HashMap::from([
        (
            "conn-idle".to_string(),
            ClientConnectionInfo {
                client_id: "conn-idle".to_string(),
                session_id: "other-session".to_string(),
                client_instance_id: None,
                connected_at: now,
                last_seen: now,
                is_processing: true,
                current_tool_name: Some("bash".to_string()),
                terminal_env: Vec::new(),
                disconnect_tx: mpsc::unbounded_channel().0,
            },
        ),
        (
            "conn-target".to_string(),
            ClientConnectionInfo {
                client_id: "conn-target".to_string(),
                session_id: "target-session".to_string(),
                client_instance_id: None,
                connected_at: now,
                last_seen: now,
                is_processing: true,
                current_tool_name: Some("batch".to_string()),
                terminal_env: Vec::new(),
                disconnect_tx: mpsc::unbounded_channel().0,
            },
        ),
    ])));

    let snapshot = session_activity_snapshot(&client_connections, "target-session", false)
        .await
        .expect("activity snapshot");

    assert!(snapshot.is_processing);
    assert_eq!(snapshot.current_tool_name.as_deref(), Some("batch"));
}

#[tokio::test]
async fn session_activity_snapshot_uses_fallback_when_no_live_connection_is_marked_busy() {
    let client_connections = Arc::new(RwLock::new(HashMap::<String, ClientConnectionInfo>::new()));

    let snapshot = session_activity_snapshot(&client_connections, "target-session", true)
        .await
        .expect("fallback snapshot");

    assert!(snapshot.is_processing);
    assert_eq!(snapshot.current_tool_name, None);
}

#[tokio::test]
async fn handle_get_history_falls_back_to_persisted_snapshot_when_agent_is_busy() {
    for tier in [Some("priority"), Some("flex"), None] {
        assert_history_service_tier(tier, true, false).await;
    }
}

#[tokio::test]
#[expect(
    clippy::await_holding_lock,
    reason = "test serializes storage environment and deliberately holds the busy agent"
)]
async fn handle_get_history_busy_fresh_session_returns_empty_without_waiting() {
    let _env_guard = crate::storage::lock_test_env();
    let temp_home = tempfile::TempDir::new().unwrap();
    let prev_home = std::env::var_os("FACTR_HOME");
    crate::env::set_var("FACTR_HOME", temp_home.path());

    let session_id = "session_fresh_busy_history";
    let session = crate::session::Session::create_with_id(session_id.into(), None, None);
    let provider: Arc<dyn Provider> = Arc::new(MockProvider(Some("priority")));
    let agent = Arc::new(Mutex::new(Agent::new_with_session(
        provider.clone(),
        Registry::empty(),
        session,
        None,
    )));
    let snapshot_path = crate::session::session_path(session_id).unwrap();
    assert!(
        !snapshot_path.exists(),
        "fresh empty sessions are not persisted"
    );
    let sessions = Arc::new(RwLock::new(HashMap::from([(
        session_id.to_string(),
        agent.clone(),
    )])));
    let connections = Arc::new(RwLock::new(HashMap::new()));
    let count = Arc::new(RwLock::new(1));
    let (stream, mut peer) = crate::transport::stream_pair().unwrap();
    let (_reader, write_half) = stream.into_split();
    let writer = Arc::new(Mutex::new(write_half));
    let busy_guard = agent.lock().await;

    // Keep the mutex held throughout, representing either idle prefetch or a
    // real turn. Concurrent requests must not queue behind either lock owner.
    let request = |id, processing| {
        handle_get_history(
            id,
            session_id,
            processing,
            &agent,
            &provider,
            &sessions,
            &connections,
            &count,
            &writer,
            "test-server",
            "test",
            None,
        )
    };
    for processing in [false, true] {
        let results = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            tokio::join!(
                request(1, processing),
                request(2, processing),
                request(3, processing)
            )
        })
        .await
        .expect("fresh history must not wait for busy agent");
        results.0.unwrap();
        results.1.unwrap();
        results.2.unwrap();
    }
    assert!(
        !snapshot_path.exists(),
        "fallback must not persist synthetic state"
    );

    // A nonexistent session is not the same as a registered, unsaved one.
    sessions.write().await.clear();
    assert!(request(4, false).await.is_err());
    sessions
        .write()
        .await
        .insert(session_id.into(), agent.clone());
    // Corrupt snapshots must not silently become empty history either.
    std::fs::create_dir_all(snapshot_path.parent().unwrap()).unwrap();
    std::fs::write(&snapshot_path, b"not valid session json").unwrap();
    assert!(request(5, false).await.is_err());

    drop(busy_guard);
    drop(writer);
    let mut bytes = Vec::new();
    peer.read_to_end(&mut bytes).await.unwrap();
    let events: Vec<crate::protocol::ServerEvent> = std::io::Cursor::new(bytes)
        .lines()
        .map(|line| serde_json::from_str(&line.unwrap()).unwrap())
        .collect();
    assert_eq!(events.len(), 6);
    for (index, event) in events.into_iter().enumerate() {
        match event {
            crate::protocol::ServerEvent::History {
                session_id: returned_id,
                messages,
                images,
                provider_name,
                provider_model,
                reasoning_effort,
                service_tier,
                activity,
                all_sessions,
                client_count,
                ..
            } => {
                assert_eq!(returned_id, session_id);
                assert!(messages.is_empty());
                assert!(images.is_empty());
                assert_eq!(provider_name.as_deref(), Some("Mock Profile"));
                assert_eq!(provider_model.as_deref(), Some("mock-model"));
                assert_eq!(reasoning_effort.as_deref(), Some("high"));
                assert_eq!(service_tier.as_deref(), Some("priority"));
                assert_eq!(
                    activity.is_some_and(|activity| activity.is_processing),
                    index >= 3
                );
                assert_eq!(all_sessions, vec![session_id.to_string()]);
                assert_eq!(client_count, Some(1));
            }
            other => panic!("expected history, got {other:?}"),
        }
    }
    if let Some(home) = prev_home {
        crate::env::set_var("FACTR_HOME", home);
    } else {
        crate::env::remove_var("FACTR_HOME");
    }
}

#[tokio::test]
async fn history_covers_live_and_persisted_paths() {
    for busy in [false, true] {
        assert_history_service_tier(None, busy, false).await;
    }
}

#[tokio::test]
async fn handle_get_history_uses_live_snapshot_when_agent_is_available() {
    assert_history_service_tier(None, false, false).await;
}

#[tokio::test]
async fn history_guard_survives_racing_turn_and_is_released_before_write() {
    assert_history_service_tier(None, false, true).await;
}

#[expect(
    clippy::await_holding_lock,
    reason = "test intentionally keeps the agent busy lock held to exercise persisted-history fallback"
)]
async fn assert_history_service_tier(
    tier: Option<&'static str>,
    busy: bool,
    racing_turn: bool,
) {
    let _guard = crate::storage::lock_test_env();
    let temp_home = tempfile::TempDir::new().expect("create temp home");
    let prev_home = std::env::var_os("FACTR_HOME");
    crate::env::set_var("FACTR_HOME", temp_home.path());

    let session_id = "session_busy_history_fallback";
    let mut session = crate::session::Session::create_with_id(
        session_id.to_string(),
        None,
        Some("busy fallback".to_string()),
    );
    session.model = Some("mock-model".to_string());
    session.append_stored_message(crate::session::StoredMessage {
        id: "msg-busy-fallback".to_string(),
        role: crate::message::Role::User,
        content: vec![crate::message::ContentBlock::Text {
            text: "persisted fallback history".to_string(),
            cache_control: None,
        }],
        display_role: None,
        timestamp: None,
        tool_duration_ms: None,
        token_usage: None,
    });
    session.save().expect("save session");

    let provider: Arc<dyn Provider> = Arc::new(MockProvider(tier));
    let registry = Registry::empty();
    let mut live_session = session.clone();
    live_session.title = Some("live agent".to_string());
    live_session.messages[0].content = vec![crate::message::ContentBlock::Text {
        text: "live unsaved history".to_string(),
        cache_control: None,
    }];
    let agent = Arc::new(Mutex::new(Agent::new_with_session(
        provider.clone(),
        registry,
        live_session,
        None,
    )));
    // Agent construction persists its session. Force a full snapshot of the
    // older transcript rather than a metadata-only journal update.
    session.replace_messages(session.messages.clone());
    session.save().expect("restore persisted history snapshot");
    let busy_guard = if busy { Some(agent.lock().await) } else { None };

    let sessions = Arc::new(RwLock::new(HashMap::from([(
        session_id.to_string(),
        Arc::clone(&agent),
    )])));
    let client_connections = Arc::new(RwLock::new(HashMap::<String, ClientConnectionInfo>::new()));
    let client_count = Arc::new(RwLock::new(1usize));

    let (stream_a, mut stream_b) = crate::transport::stream_pair().expect("stream pair");
    let (_reader_a, writer_a) = stream_a.into_split();
    let writer = Arc::new(Mutex::new(writer_a));

    if racing_turn {
        // Reproduce the exact decision/preparation boundary without scheduler
        // timing: a turn queues after the successful nonblocking acquisition.
        let history_guard = agent.try_lock().expect("idle agent fast path");
        let turn = agent.lock();
        tokio::pin!(turn);
        assert!(futures::poll!(&mut turn).is_pending());

        // Socket backpressure lets us inspect the lock lifetime after snapshot
        // preparation, while the history request is still in flight.
        let writer_guard = writer.lock().await;
        let history = super::send_history_with_guard(
            42,
            session_id,
            history_guard,
            &sessions,
            &client_count,
            &writer,
            "server-name",
            "🔥",
            None,
            None,
            super::HistoryPayloadMode::Full,
            true,
        );
        tokio::pin!(history);
        assert!(futures::poll!(&mut history).is_pending());
        let turn_guard = match futures::poll!(&mut turn) {
            std::task::Poll::Ready(guard) => guard,
            std::task::Poll::Pending => panic!("snapshot must release agent before socket write"),
        };
        drop(writer_guard);
        tokio::time::timeout(std::time::Duration::from_secs(2), &mut history)
            .await
            .expect("history must not reacquire the racing turn's lock")
            .expect("write live history");
        drop(turn_guard);
    } else {
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            handle_get_history(
                42,
                session_id,
                busy,
                &agent,
                &provider,
                &sessions,
                &client_connections,
                &client_count,
                &writer,
                "server-name",
                "🔥",
                None,
            ),
        )
        .await
        .expect("history must complete without waiting for the held turn lock")
        .expect("history should be written");
    }

    drop(busy_guard);
    drop(writer);

    let mut bytes = Vec::new();
    stream_b
        .read_to_end(&mut bytes)
        .await
        .expect("read history event bytes");
    let mut cursor = std::io::Cursor::new(bytes);
    let mut line = String::new();
    cursor.read_line(&mut line).expect("read first line");
    let event: crate::protocol::ServerEvent =
        serde_json::from_str(line.trim()).expect("decode history event");

    match event {
        crate::protocol::ServerEvent::History {
            id,
            session_id: returned_session_id,
            messages,
            activity,
            service_tier,
            ..
        } => {
            assert_eq!(id, 42);
            assert_eq!(returned_session_id, session_id);
            assert_eq!(messages.len(), 1);
            assert_eq!(
                messages[0].content,
                if busy {
                    "persisted fallback history"
                } else {
                    "live unsaved history"
                }
            );
            assert_eq!(service_tier.as_deref(), tier);
            if busy {
                let activity = activity.expect("fallback activity snapshot");
                assert!(activity.is_processing);
            }
        }
        other => panic!("expected history event, got {:?}", other),
    }

    if let Some(prev_home) = prev_home {
        crate::env::set_var("FACTR_HOME", prev_home);
    } else {
        crate::env::remove_var("FACTR_HOME");
    }
}

#[tokio::test]
async fn handle_get_model_catalog_does_not_wait_for_busy_agent_lock() {
    for tier in [Some("priority"), Some("flex"), None] {
        assert_model_catalog_service_tier(tier, true).await;
    }
}

#[tokio::test]
async fn handle_get_model_catalog_preserves_live_service_tier() {
    for tier in [Some("priority"), Some("flex"), None] {
        assert_model_catalog_service_tier(tier, false).await;
    }
}

#[expect(
    clippy::await_holding_lock,
    reason = "test intentionally keeps the agent busy lock held to exercise model-catalog fallback"
)]
async fn assert_model_catalog_service_tier(tier: Option<&'static str>, busy: bool) {
    let _guard = crate::storage::lock_test_env();
    let temp_home = tempfile::TempDir::new().expect("create temp home");
    let prev_home = std::env::var_os("FACTR_HOME");
    crate::env::set_var("FACTR_HOME", temp_home.path());

    let session_id = "session_busy_model_catalog_fallback";
    let mut session = crate::session::Session::create_with_id(
        session_id.to_string(),
        None,
        Some("busy model catalog".to_string()),
    );
    session.model = Some("persisted-model".to_string());
    session.save().expect("save session");

    let provider: Arc<dyn Provider> = Arc::new(MockProvider(tier));
    let agent = Arc::new(Mutex::new(Agent::new_with_session(
        provider.clone(),
        Registry::empty(),
        session.clone(),
        None,
    )));
    let busy_guard = if busy { Some(agent.lock().await) } else { None };

    let (stream_a, mut stream_b) = crate::transport::stream_pair().expect("stream pair");
    let (_reader_a, writer_a) = stream_a.into_split();
    let writer = Arc::new(Mutex::new(writer_a));

    tokio::time::timeout(
        std::time::Duration::from_millis(100),
        handle_get_model_catalog(43, session_id, &agent, &provider, &writer),
    )
    .await
    .expect("model catalog must not wait for busy agent mutex")
    .expect("model catalog fallback should write history event");

    drop(busy_guard);
    drop(writer);

    let mut bytes = Vec::new();
    stream_b
        .read_to_end(&mut bytes)
        .await
        .expect("read model catalog event bytes");
    let mut cursor = std::io::Cursor::new(bytes);
    let mut line = String::new();
    cursor.read_line(&mut line).expect("read first line");
    let event: crate::protocol::ServerEvent =
        serde_json::from_str(line.trim()).expect("decode model catalog event");

    match event {
        crate::protocol::ServerEvent::History {
            id,
            session_id: returned_session_id,
            provider_name,
            provider_model,
            service_tier,
            reasoning_effort,
            ..
        } => {
            assert_eq!(id, 43);
            assert_eq!(returned_session_id, session_id);
            assert_eq!(provider_name.as_deref(), Some("Mock Profile"));
            assert_eq!(
                provider_model.as_deref(),
                Some(if busy {
                    "persisted-model"
                } else {
                    "mock-model"
                })
            );
            assert_eq!(service_tier.as_deref(), tier);
            assert_eq!(reasoning_effort.as_deref(), Some("high"));
        }
        other => panic!("expected history event, got {:?}", other),
    }

    if let Some(prev_home) = prev_home {
        crate::env::set_var("FACTR_HOME", prev_home);
    } else {
        crate::env::remove_var("FACTR_HOME");
    }
}

struct ReloadHistoryEnvGuard {
    prev_home: Option<std::ffi::OsString>,
    prev_runtime: Option<std::ffi::OsString>,
}

impl ReloadHistoryEnvGuard {
    fn new(home: &std::path::Path, runtime: &std::path::Path) -> Self {
        let prev_home = std::env::var_os("FACTR_HOME");
        let prev_runtime = std::env::var_os("FACTR_RUNTIME_DIR");
        crate::env::set_var("FACTR_HOME", home);
        crate::env::set_var("FACTR_RUNTIME_DIR", runtime);
        Self {
            prev_home,
            prev_runtime,
        }
    }
}

impl Drop for ReloadHistoryEnvGuard {
    fn drop(&mut self) {
        if let Some(prev_home) = self.prev_home.take() {
            crate::env::set_var("FACTR_HOME", prev_home);
        } else {
            crate::env::remove_var("FACTR_HOME");
        }
        if let Some(prev_runtime) = self.prev_runtime.take() {
            crate::env::set_var("FACTR_RUNTIME_DIR", prev_runtime);
        } else {
            crate::env::remove_var("FACTR_RUNTIME_DIR");
        }
    }
}

fn write_pending_user_session(
    session_id: &str,
    status: crate::session::SessionStatus,
) -> Result<()> {
    let mut session = crate::session::Session::create_with_id(session_id.to_string(), None, None);
    session.status = status;
    session.add_message(
        crate::message::Role::User,
        vec![crate::message::ContentBlock::Text {
            text: "continue this after reload".to_string(),
            cache_control: None,
        }],
    );
    session.save()
}

#[test]
fn history_reload_recovery_does_not_infer_pending_user_turn_without_reload_marker() -> Result<()> {
    let _lock = crate::storage::lock_test_env();
    let home = tempfile::TempDir::new()?;
    let runtime = tempfile::TempDir::new()?;
    let _guard = ReloadHistoryEnvGuard::new(home.path(), runtime.path());
    let session_id = "session_history_no_reload_fallback";
    write_pending_user_session(session_id, crate::session::SessionStatus::Active)?;

    assert!(super::history_reload_recovery_snapshot(session_id, None).is_none());
    Ok(())
}

