//! Optional shell-output reduction through the `rtk` CLI.
//!
//! `terminal.reduce_output` (default off) asks `rtk rewrite <command>` for a shorter-output equivalent of
//! a foreground shell command. Fail-open by construction: a missing binary, a timeout, a non-zero exit,
//! an empty or unchanged answer all mean the original command runs untouched. The approval gate always
//! looks at the original command; only the executed text changes.

use std::process::Stdio;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tokio::process::Command;

/// Appended to a reduced command's output so the model and the UI can both see it happened.
pub(crate) const REDUCED_MARKER: &str = "[output reduced by rtk]";

const DEFAULT_TIMEOUT_MS: u64 = 1_500;
const MAX_COMMAND_BYTES: usize = 8_192;
/// After a failed spawn (rtk not installed) the lookup is skipped for this long.
const MISSING_BACKOFF: Duration = Duration::from_secs(30);

static MISSING_UNTIL: Mutex<Option<Instant>> = Mutex::new(None);

fn rtk_program() -> String {
    factr_base::factr_config::env_text("FACTR_RTK_BIN").unwrap_or_else(|| "rtk".to_string())
}

/// The command to run instead of `command`, or `None` to run it unchanged.
pub(crate) async fn reduced_command(command: &str) -> Option<String> {
    // rtk rewrites POSIX shell commands; the Windows shell tool runs cmd.exe syntax.
    let terminal = factr_base::factr_config::current().terminal.clone();
    if cfg!(windows) || !terminal.reduce_output {
        return None;
    }
    rewrite(command, Duration::from_millis(terminal.reduce_output_timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS))).await
}

async fn rewrite(command: &str, timeout: Duration) -> Option<String> {
    let trimmed = command.trim();
    if trimmed.is_empty() || trimmed.len() > MAX_COMMAND_BYTES || trimmed.starts_with("rtk ") {
        return None;
    }
    if MISSING_UNTIL.lock().ok().and_then(|g| *g).is_some_and(|until| Instant::now() < until) {
        return None;
    }
    let mut child = match Command::new(rtk_program())
        .arg("rewrite")
        .arg(trimmed)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(child) => child,
        Err(_) => {
            if let Ok(mut guard) = MISSING_UNTIL.lock() {
                *guard = Some(Instant::now() + MISSING_BACKOFF);
            }
            return None;
        }
    };
    let mut stdout = child.stdout.take()?;
    let run = async {
        let mut bytes = Vec::new();
        tokio::io::AsyncReadExt::read_to_end(&mut stdout, &mut bytes).await.ok()?;
        let status = child.wait().await.ok()?;
        status.success().then_some(bytes)
    };
    let bytes = tokio::time::timeout(timeout, run).await.ok().flatten()?;
    let rewritten = String::from_utf8(bytes).ok()?.trim().to_string();
    (!rewritten.is_empty() && rewritten != trimmed && rewritten.len() <= MAX_COMMAND_BYTES * 2).then_some(rewritten)
}

/// Forget a recorded "rtk is missing" (tests, and a settings change).
#[cfg(test)]
pub(crate) fn reset_missing_cache() {
    if let Ok(mut guard) = MISSING_UNTIL.lock() {
        *guard = None;
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn fake_rtk(dir: &std::path::Path, body: &str) -> String {
        let path = dir.join("rtk-fake");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path.to_string_lossy().into_owned()
    }

    async fn with_fake<F: std::future::Future<Output = ()>>(body: &str, check: impl FnOnce() -> F) {
        let _env = factr_base::storage::lock_test_env();
        let dir = tempfile::tempdir().unwrap();
        let program = fake_rtk(dir.path(), body);
        factr_base::env::set_var("FACTR_RTK_BIN", &program);
        reset_missing_cache();
        check().await;
        factr_base::env::remove_var("FACTR_RTK_BIN");
        reset_missing_cache();
    }

    #[tokio::test]
    async fn a_successful_rewrite_is_returned() {
        with_fake(r#"[ "$1" = rewrite ] && echo "rtk $2" && exit 0; exit 1"#, || async {
            assert_eq!(rewrite("git status", Duration::from_secs(5)).await.as_deref(), Some("rtk git status"));
        })
        .await;
    }

    #[tokio::test]
    async fn every_failure_runs_the_command_unchanged() {
        with_fake("exit 1", || async { assert_eq!(rewrite("git status", Duration::from_secs(5)).await, None) }).await;
        with_fake("exit 0", || async { assert_eq!(rewrite("git status", Duration::from_secs(5)).await, None) }).await;
        with_fake(r#"echo "$2""#, || async { assert_eq!(rewrite("git status", Duration::from_secs(5)).await, None, "unchanged answer") }).await;
        with_fake("sleep 5; echo rtk x", || async { assert_eq!(rewrite("git status", Duration::from_millis(150)).await, None, "timeout") }).await;
        with_fake("echo rtk x", || async {
            assert_eq!(rewrite("rtk git status", Duration::from_secs(5)).await, None, "already rtk");
            assert_eq!(rewrite("   ", Duration::from_secs(5)).await, None);
        })
        .await;
    }

    #[tokio::test]
    async fn a_missing_binary_is_remembered_and_never_an_error() {
        let _env = factr_base::storage::lock_test_env();
        factr_base::env::set_var("FACTR_RTK_BIN", "/nonexistent/rtk-binary");
        reset_missing_cache();
        assert_eq!(rewrite("git status", Duration::from_secs(1)).await, None);
        assert!(MISSING_UNTIL.lock().unwrap().is_some(), "the miss is cached");
        factr_base::env::remove_var("FACTR_RTK_BIN");
        reset_missing_cache();
    }
}
