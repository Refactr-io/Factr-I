#![cfg_attr(test, allow(clippy::await_holding_lock))]

use super::socket::sibling_socket_path;
#[cfg(unix)]
use super::socket::{
    daemon_lock_path, server_start_matches_existing_server, try_acquire_daemon_lock,
};
use super::{cleanup_socket_files, cleanup_socket_pair};
#[cfg(unix)]
use super::{connect_socket, reap_stale_socket_if_dead};
#[cfg(unix)]
use crate::transport::Listener;

#[test]
fn sibling_socket_path_roundtrip() {
    let main = std::path::PathBuf::from("/tmp/factr.sock");
    let debug = std::path::PathBuf::from("/tmp/factr-debug.sock");

    assert_eq!(sibling_socket_path(&main), Some(debug.clone()));
    assert_eq!(sibling_socket_path(&debug), Some(main));
}

#[test]
fn cleanup_socket_pair_removes_main_and_debug_files() {
    let stamp = format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    let dir = std::env::temp_dir();
    let main = dir.join(format!("factr-test-{}.sock", stamp));
    let debug = dir.join(format!("factr-test-{}-debug.sock", stamp));

    std::fs::write(&main, b"").expect("create main socket placeholder");
    std::fs::write(&debug, b"").expect("create debug socket placeholder");

    cleanup_socket_pair(&main);

    assert!(!main.exists(), "main socket file should be removed");
    assert!(!debug.exists(), "debug socket file should be removed");
}

#[cfg(unix)]
#[tokio::test]
async fn connect_socket_preserves_refused_socket_path() {
    let temp = tempfile::tempdir().expect("tempdir");
    let socket_path = temp.path().join("factr.sock");

    {
        let _listener = Listener::bind(&socket_path).expect("bind listener");
    }

    assert!(
        socket_path.exists(),
        "listener drop should leave the socket path behind for stale-socket checks"
    );

    let err = connect_socket(&socket_path)
        .await
        .expect_err("connect should fail once the listener is gone");
    assert!(
        err.to_string().contains("refused the connection"),
        "unexpected error: {err:#}"
    );
    assert!(
        socket_path.exists(),
        "connect_socket should not unlink the socket path on connection refusal"
    );
}

#[cfg(unix)]
#[test]
fn daemon_lock_serializes_server_processes() {
    let _guard = crate::storage::lock_test_env();
    let temp = tempfile::tempdir().expect("tempdir");
    let prev_runtime = std::env::var_os("FACTR_RUNTIME_DIR");
    crate::env::set_var("FACTR_RUNTIME_DIR", temp.path());

    let lock_path = daemon_lock_path();
    let first = try_acquire_daemon_lock(&lock_path)
        .expect("acquire first daemon lock")
        .expect("first daemon lock should succeed");
    let second = try_acquire_daemon_lock(&lock_path).expect("acquire second daemon lock");
    assert!(second.is_none(), "second daemon lock should fail");
    drop(first);

    let third = try_acquire_daemon_lock(&lock_path)
        .expect("acquire third daemon lock")
        .expect("third daemon lock should succeed after release");
    drop(third);

    if let Some(prev_runtime) = prev_runtime {
        crate::env::set_var("FACTR_RUNTIME_DIR", prev_runtime);
    } else {
        crate::env::remove_var("FACTR_RUNTIME_DIR");
    }
}

#[cfg(unix)]
#[tokio::test]
async fn reap_stale_socket_removes_dead_socket_pair_and_lock() {
    let _guard = crate::storage::lock_test_env();
    let temp = tempfile::tempdir().expect("tempdir");
    let prev_runtime = std::env::var_os("FACTR_RUNTIME_DIR");
    crate::env::set_var("FACTR_RUNTIME_DIR", temp.path());

    let socket = temp.path().join("factr.sock");
    let debug = temp.path().join("factr-debug.sock");
    let lock = daemon_lock_path();

    // Simulate the post-upgrade/crash state: socket + debug + lock files left
    // behind, but no process is listening on the socket.
    std::fs::write(&socket, b"").expect("write stale socket");
    std::fs::write(&debug, b"").expect("write stale debug socket");
    std::fs::write(&lock, b"").expect("write stale lock");

    let reaped = reap_stale_socket_if_dead(&socket).await;
    assert!(reaped, "a dead socket with no listener should be reaped");
    assert!(!socket.exists(), "stale socket should be removed");
    assert!(!debug.exists(), "stale debug socket should be removed");
    assert!(!lock.exists(), "stale daemon lock should be removed");

    if let Some(prev_runtime) = prev_runtime {
        crate::env::set_var("FACTR_RUNTIME_DIR", prev_runtime);
    } else {
        crate::env::remove_var("FACTR_RUNTIME_DIR");
    }
}

#[cfg(unix)]
#[tokio::test]
async fn reap_stale_socket_spares_live_listener() {
    let _guard = crate::storage::lock_test_env();
    let temp = tempfile::tempdir().expect("tempdir");
    let prev_runtime = std::env::var_os("FACTR_RUNTIME_DIR");
    crate::env::set_var("FACTR_RUNTIME_DIR", temp.path());

    let socket = temp.path().join("factr.sock");
    // A live listener means a daemon is bound; reaping must be a no-op.
    let listener = Listener::bind(&socket).expect("bind listener");

    let reaped = reap_stale_socket_if_dead(&socket).await;
    assert!(!reaped, "a live listener must never be reaped");
    assert!(socket.exists(), "live socket must be left intact");

    drop(listener);
    if let Some(prev_runtime) = prev_runtime {
        crate::env::set_var("FACTR_RUNTIME_DIR", prev_runtime);
    } else {
        crate::env::remove_var("FACTR_RUNTIME_DIR");
    }
}

#[cfg(unix)]
#[tokio::test]
async fn reap_stale_socket_spares_socket_when_lock_is_held() {
    let _guard = crate::storage::lock_test_env();
    let temp = tempfile::tempdir().expect("tempdir");
    let prev_runtime = std::env::var_os("FACTR_RUNTIME_DIR");
    crate::env::set_var("FACTR_RUNTIME_DIR", temp.path());

    let socket = temp.path().join("factr.sock");
    std::fs::write(&socket, b"").expect("write stale-looking socket");

    // Hold the daemon lock, emulating a live daemon whose socket probe happens
    // to be momentarily unanswerable. The reaper must not unlink the socket.
    let lock_path = daemon_lock_path();
    let held = try_acquire_daemon_lock(&lock_path)
        .expect("acquire lock")
        .expect("lock should be free");

    let reaped = reap_stale_socket_if_dead(&socket).await;
    assert!(
        !reaped,
        "socket must be spared while the daemon lock is held"
    );
    assert!(
        socket.exists(),
        "socket must be left intact while lock is held"
    );

    drop(held);
    if let Some(prev_runtime) = prev_runtime {
        crate::env::set_var("FACTR_RUNTIME_DIR", prev_runtime);
    } else {
        crate::env::remove_var("FACTR_RUNTIME_DIR");
    }
}

#[cfg(unix)]
#[test]
fn existing_server_start_errors_are_detected() {
    assert!(server_start_matches_existing_server(
        "Error: Another factr server process is already running for runtime dir /run/user/1000"
    ));
    assert!(server_start_matches_existing_server(
        "Error: Refusing to replace active server socket at /run/user/1000/factr.sock"
    ));
    assert!(!server_start_matches_existing_server(
        "Error: failed to bind socket: permission denied"
    ));
}

#[test]
fn cleanup_socket_files_removes_sockets_and_hash_files_in_a_temp_runtime_dir() {
    let runtime = tempfile::tempdir().expect("runtime dir");
    let main = runtime.path().join("factr-4242.sock");
    let debug = runtime.path().join("factr-4242-debug.sock");
    let main_hash = runtime.path().join("factr-4242.sock.hash");
    let debug_hash = runtime.path().join("factr-4242-debug.sock.hash");
    let other = runtime.path().join("factr-9999.sock");
    for path in [&main, &debug, &main_hash, &debug_hash, &other] {
        std::fs::write(path, b"").expect("create placeholder");
    }

    cleanup_socket_files(&main);

    for path in [&main, &debug, &main_hash, &debug_hash] {
        assert!(!path.exists(), "{} should be removed", path.display());
    }
    assert!(other.exists(), "another process's socket must be left alone");
}
