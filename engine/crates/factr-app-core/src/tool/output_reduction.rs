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

/// Where to look for the rtk binary: an explicit `FACTR_RTK_BIN`, else the PATH, else the usual install
/// folders (a desktop app launched from the Dock does not inherit the shell's PATH).
fn rtk_candidates(home: Option<&std::path::Path>) -> Vec<String> {
    if let Some(explicit) = factr_base::factr_config::env_text("FACTR_RTK_BIN") {
        return vec![explicit];
    }
    let mut found = vec!["rtk".to_string()];
    let exe = if cfg!(windows) { "rtk.exe" } else { "rtk" };
    let folders = [
        home.map(|h| h.join(".local").join("bin")),
        home.map(|h| h.join(".cargo").join("bin")),
        Some(std::path::PathBuf::from("/opt/homebrew/bin")),
        Some(std::path::PathBuf::from("/usr/local/bin")),
    ];
    for folder in folders.into_iter().flatten() {
        let path = folder.join(exe);
        if path.is_file() {
            found.push(path.to_string_lossy().into_owned());
        }
    }
    found
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
    let spawned = rtk_candidates(factr_base::platform::user_home_dir().as_deref()).into_iter().find_map(|program| {
        Command::new(&program)
            .arg("rewrite")
            .arg(trimmed)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .ok()
            .map(|child| (child, program))
    });
    let Some((mut child, program)) = spawned else {
        if let Ok(mut guard) = MISSING_UNTIL.lock() {
            *guard = Some(Instant::now() + MISSING_BACKOFF);
        }
        return None;
    };
    let mut stdout = child.stdout.take()?;
    let run = async {
        let mut bytes = Vec::new();
        tokio::io::AsyncReadExt::read_to_end(&mut stdout, &mut bytes).await.ok()?;
        let status = child.wait().await.ok()?;
        // 0: rewritten. 3: rewritten, and rtk's own permission rules say "ask": the approval gate already
        // judged the original command, so it is still usable. 1 (no equivalent), 2 (deny) and the rest: run as is.
        matches!(status.code(), Some(0 | 3)).then_some(bytes)
    };
    let bytes = tokio::time::timeout(timeout, run).await.ok().flatten()?;
    let rewritten = String::from_utf8(bytes).ok()?.trim().to_string();
    (!rewritten.is_empty() && rewritten != trimmed && rewritten.len() <= MAX_COMMAND_BYTES * 2)
        .then(|| with_rtk_on_path(&rewritten, &program))
}

/// The rewritten command calls `rtk` by name. When rtk was found by absolute path (a desktop app does not
/// inherit the shell's PATH), put its folder on the command's PATH so the shell finds it too.
fn with_rtk_on_path(rewritten: &str, program: &str) -> String {
    match std::path::Path::new(program).parent().filter(|dir| dir.is_absolute()) {
        Some(dir) => format!("export PATH='{}':\"$PATH\"; {rewritten}", dir.to_string_lossy().replace('\'', "'\"'\"'")),
        None => rewritten.to_string(),
    }
}

/// True when a reduced command failed because the shell could not find `rtk` itself.
pub(crate) fn rtk_missing_at_run(output: &str) -> bool {
    output.contains("rtk: command not found") || output.contains("rtk: not found")
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
            assert!(rewrite("git status", Duration::from_secs(5)).await.is_some_and(|r| r.starts_with("export PATH=") && r.ends_with("; rtk git status")));
        })
        .await;
    }

    #[tokio::test]
    async fn rewritten_with_the_ask_exit_code_is_still_used_but_deny_and_no_equivalent_are_not() {
        with_fake(r#"echo "rtk $2"; exit 3"#, || async {
            assert!(rewrite("git status", Duration::from_secs(5)).await.is_some_and(|r| r.starts_with("export PATH=") && r.ends_with("; rtk git status")));
        })
        .await;
        for code in [1, 2, 4] {
            with_fake(&format!(r#"echo "rtk $2"; exit {code}"#), || async { assert_eq!(rewrite("git status", Duration::from_secs(5)).await, None) }).await;
        }
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

    #[test]
    fn candidates_put_the_path_first_then_the_usual_install_folders_and_honour_the_override() {
        let _env = factr_base::storage::lock_test_env();
        factr_base::env::remove_var("FACTR_RTK_BIN");
        let home = tempfile::tempdir().unwrap();
        let local = home.path().join(".local").join("bin");
        std::fs::create_dir_all(&local).unwrap();
        std::fs::write(local.join("rtk"), "").unwrap();
        let found = rtk_candidates(Some(home.path()));
        assert_eq!(found[0], "rtk");
        assert!(found.iter().any(|c| c.ends_with(".local/bin/rtk")), "{found:?}");
        assert!(!found.iter().any(|c| c.contains(".cargo")), "only folders that hold the binary are listed");
        factr_base::env::set_var("FACTR_RTK_BIN", "/custom/rtk");
        assert_eq!(rtk_candidates(Some(home.path())), vec!["/custom/rtk".to_string()]);
        factr_base::env::remove_var("FACTR_RTK_BIN");
    }

    #[test]
    fn an_absolute_rtk_gets_its_folder_on_the_commands_path_and_a_plain_one_does_not() {
        assert_eq!(with_rtk_on_path("rtk git status", "rtk"), "rtk git status");
        assert_eq!(with_rtk_on_path("rtk git status", "/home/u/.local/bin/rtk"), "export PATH='/home/u/.local/bin':\"$PATH\"; rtk git status");
        assert!(with_rtk_on_path("rtk ls", "/tmp/it's/rtk").contains(r#"'/tmp/it'"'"'s'"#));
    }

    #[test]
    fn a_missing_rtk_at_run_time_is_recognised() {
        assert!(rtk_missing_at_run("bash: rtk: command not found\n\nExit code: 127"));
        assert!(rtk_missing_at_run("sh: 1: rtk: not found"));
        assert!(!rtk_missing_at_run("fatal: not a git repository"));
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
