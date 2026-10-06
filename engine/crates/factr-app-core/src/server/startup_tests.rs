#![cfg_attr(test, allow(clippy::await_holding_lock))]

use super::runtime::ServerRuntime;
use super::socket::wait_for_existing_server;
use super::{Client, Server, is_server_ready};
use crate::message::{Message, ToolDefinition};
use crate::provider::{EventStream, Provider};
use crate::transport::Listener;
use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;
use std::time::Duration;

struct TestProvider;

#[async_trait]
impl Provider for TestProvider {
    async fn complete(
        &self,
        _messages: &[Message],
        _tools: &[ToolDefinition],
        _system: &str,
        _resume_session_id: Option<&str>,
    ) -> Result<EventStream> {
        Err(anyhow::anyhow!(
            "test provider complete should not be called in startup tests"
        ))
    }

    fn name(&self) -> &str {
        "test"
    }

    fn fork(&self) -> Arc<dyn Provider> {
        Arc::new(TestProvider)
    }
}

#[tokio::test]
async fn server_run_refuses_to_replace_live_socket() {
    let _guard = crate::storage::lock_test_env();
    let temp = tempfile::tempdir().expect("tempdir");
    let prev_runtime = std::env::var_os("FACTR_RUNTIME_DIR");
    crate::env::set_var("FACTR_RUNTIME_DIR", temp.path());
    let socket_path = temp.path().join("factr.sock");
    let debug_socket_path = temp.path().join("factr-debug.sock");
    let _listener = Listener::bind(&socket_path).expect("bind existing live socket");
    let provider: Arc<dyn Provider> = Arc::new(TestProvider);
    let server = Server::new_with_paths(provider, socket_path, debug_socket_path);

    let error = server
        .run()
        .await
        .expect_err("should refuse live socket takeover");
    assert!(
        error
            .to_string()
            .contains("Refusing to replace active server socket"),
        "unexpected error: {error:#}"
    );

    if let Some(prev_runtime) = prev_runtime {
        crate::env::set_var("FACTR_RUNTIME_DIR", prev_runtime);
    } else {
        crate::env::remove_var("FACTR_RUNTIME_DIR");
    }
}

#[tokio::test]
async fn is_server_ready_returns_false_immediately_for_missing_socket() {
    let _guard = crate::storage::lock_test_env();
    let temp = tempfile::tempdir().expect("tempdir");
    let socket_path = temp.path().join("missing.sock");

    let ready = tokio::time::timeout(Duration::from_millis(50), is_server_ready(&socket_path))
        .await
        .expect("missing socket probe should return quickly");

    assert!(!ready, "missing socket should not report ready");
}

#[tokio::test]
async fn wait_for_existing_server_tolerates_delayed_listener() {
    let _guard = crate::storage::lock_test_env();
    let temp = tempfile::tempdir().expect("tempdir");
    let socket_path = temp.path().join("factr.sock");
    let bind_path = socket_path.clone();

    let bind_task = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let listener = Listener::bind(&bind_path).expect("bind delayed listener");
        tokio::time::sleep(Duration::from_millis(200)).await;
        drop(listener);
    });

    let ready = wait_for_existing_server(&socket_path, Duration::from_secs(1)).await;
    assert!(ready, "delayed live listener should be detected");

    bind_task.await.expect("bind task should complete");
}

#[tokio::test]
async fn debug_accept_loop_responds_to_ping_without_affecting_client_count() {
    let _guard = crate::storage::lock_test_env();
    let temp = tempfile::tempdir().expect("tempdir");
    let socket_path = temp.path().join("factr.sock");
    let debug_socket_path = temp.path().join("factr-debug.sock");
    let provider: Arc<dyn Provider> = Arc::new(TestProvider);
    let server = Server::new_with_paths(provider, socket_path, debug_socket_path.clone());
    let runtime = ServerRuntime::from_server(&server);
    let debug_listener = Listener::bind(&debug_socket_path).expect("bind debug socket");
    let debug_handle = runtime.spawn_debug_accept_loop(debug_listener, std::time::Instant::now());

    let mut client = tokio::time::timeout(
        Duration::from_secs(1),
        Client::connect_debug_with_path(debug_socket_path),
    )
    .await
    .expect("debug connect should complete")
    .expect("debug client should connect");

    assert!(client.ping().await.expect("debug ping should succeed"));
    assert_eq!(*server.client_count.read().await, 0);

    tokio::time::timeout(Duration::from_secs(1), runtime.shutdown())
        .await
        .expect("runtime shutdown should join debug connection tasks");
    tokio::time::timeout(Duration::from_secs(1), debug_handle)
        .await
        .expect("debug accept loop should observe runtime cancellation")
        .expect("debug accept loop should exit cleanly");
}

/// Sends one debug command and returns `(ok, output)`.
async fn debug_command(client: &mut Client, command: &str) -> (bool, String) {
    let id = client.debug_command(command, None).await.expect("send debug command");
    loop {
        let event = tokio::time::timeout(Duration::from_secs(2), client.read_event())
            .await
            .expect("debug reply in time")
            .expect("debug reply");
        if let crate::protocol::ServerEvent::DebugResponse { id: reply, ok, output } = event
            && reply == id
        {
            return (ok, output);
        }
    }
}

#[tokio::test]
async fn debug_socket_answers_server_info_and_clients_map_and_rejects_everything_else() {
    let _guard = crate::storage::lock_test_env();
    let temp = tempfile::tempdir().expect("tempdir");
    let debug_socket_path = temp.path().join("factr-debug.sock");
    let provider: Arc<dyn Provider> = Arc::new(TestProvider);
    let server = Server::new_with_paths(provider, temp.path().join("factr.sock"), debug_socket_path.clone());
    let runtime = ServerRuntime::from_server(&server);
    let debug_handle = runtime.spawn_debug_accept_loop(
        Listener::bind(&debug_socket_path).expect("bind debug socket"),
        std::time::Instant::now(),
    );
    let mut client = Client::connect_debug_with_path(debug_socket_path).await.expect("debug client");

    let (ok, info) = debug_command(&mut client, "server:info").await;
    assert!(ok, "{info}");
    let info: serde_json::Value = serde_json::from_str(&info).expect("server:info is json");
    assert_eq!(info["id"], server.identity().id.as_str());
    assert_eq!(info["session_count"], 0);

    let (ok, map) = debug_command(&mut client, "clients:map").await;
    assert!(ok, "{map}");
    let map: serde_json::Value = serde_json::from_str(&map).expect("clients:map is json");
    assert_eq!(map["count"], 0);

    // The debug commands that drove a TUI, swarms, jobs, memory dumps and self-dev are gone.
    for gone in ["client:screen", "swarm:list", "jobs", "server:memory", "reload"] {
        let (ok, output) = debug_command(&mut client, gone).await;
        assert!(!ok, "{gone} must be rejected: {output}");
        assert!(output.contains("unknown debug command"), "{gone}: {output}");
    }
    let (ok, output) = debug_command(&mut client, "state").await;
    assert!(!ok && output.contains("No active session"), "{output}");

    runtime.shutdown().await;
    let _ = debug_handle.await;
}

#[tokio::test]
async fn the_debug_socket_is_bound_only_in_test_mode() {
    let _guard = crate::storage::lock_test_env();
    let temp = tempfile::tempdir().expect("tempdir");
    let previous = std::env::var_os("FACTR_DEBUG_CONTROL");
    let socket_path = temp.path().join("factr.sock");
    let debug_socket_path = temp.path().join("factr-debug.sock");
    let (mut bound_when_off, mut bound_when_on) = (true, false);
    for (enabled, bound) in [(false, &mut bound_when_off), (true, &mut bound_when_on)] {
        if enabled {
            crate::env::set_var("FACTR_DEBUG_CONTROL", "1");
        } else {
            crate::env::remove_var("FACTR_DEBUG_CONTROL");
        }
        let provider: Arc<dyn Provider> = Arc::new(TestProvider);
        let server = Server::new_with_paths(provider, socket_path.clone(), debug_socket_path.clone());
        let run = tokio::spawn(async move { server.run().await });
        assert!(wait_for_existing_server(&socket_path, Duration::from_secs(5)).await, "server socket should come up");
        // Windows binds a named pipe, which `Path::exists` never sees; on Unix this is `exists`.
        *bound = crate::transport::is_socket_path(&debug_socket_path);
        run.abort();
        let _ = run.await;
        let _ = std::fs::remove_file(&socket_path);
        let _ = std::fs::remove_file(&debug_socket_path);
    }
    match previous {
        Some(value) => crate::env::set_var("FACTR_DEBUG_CONTROL", value),
        None => crate::env::remove_var("FACTR_DEBUG_CONTROL"),
    }
    assert!(!bound_when_off, "a normal run must not open the debug socket");
    assert!(bound_when_on, "test mode binds the debug socket");
}
