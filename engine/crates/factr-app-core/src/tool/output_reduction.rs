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

/// The rtk binary shipped with the app, next to the engine executable (`resources/factr/rtk-bin/rtk`). An explicit
/// `FACTR_RTK_BIN` replaces it in debug and test builds only. The PATH is never searched: release builds
/// run only the pinned, verified copy.
fn rtk_binary() -> Option<std::path::PathBuf> {
    #[cfg(any(test, debug_assertions))]
    if let Some(explicit) = factr_base::factr_config::env_text("FACTR_RTK_BIN") {
        return Some(explicit.into());
    }
    let exe = if cfg!(windows) { "rtk.exe" } else { "rtk" };
    // A folder of its own: the rewritten command puts it on PATH and must not expose the engine binary next to it.
    std::env::current_exe().ok()?.parent().map(|dir| dir.join("rtk-bin").join(exe)).filter(|p| p.is_file())
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
    let spawned = rtk_binary().and_then(|program| {
        let child = Command::new(&program)
            .arg("rewrite")
            .arg(trimmed)
            .envs(rtk_env())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .ok()?;
        Some((child, program))
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

/// rtk's own switches: no telemetry, and its usage history, recall store and tee files stay in Factr's
/// own data folder rather than the user's profile.
fn rtk_env() -> Vec<(&'static str, String)> {
    let state = factr_base::factr_config::home().map(|h| h.join("rtk"));
    let at = |name: &str| state.as_ref().map(|d| d.join(name).to_string_lossy().into_owned());
    // RTK_NO_TOML: a checked-out project's own rtk filters never change what the bundled rtk does.
    let mut env = vec![("RTK_TELEMETRY_DISABLED", "1".to_string()), ("RTK_NO_TOML", "1".to_string())];
    env.extend(at("history.db").map(|v| ("RTK_DB_PATH", v)));
    env.extend(at("recall.db").map(|v| ("RTK_RECALL_DB", v)));
    env.extend(at("tee").map(|v| ("RTK_TEE_DIR", v)));
    env
}

/// The rewritten command calls `rtk` by name. Define `rtk` as a function for the bundled absolute path (it
/// wins over the PATH and over any inherited `rtk` function, and leaves the PATH alone), and give it the
/// same switches as the rewrite call.
fn with_rtk_on_path(rewritten: &str, program: &std::path::Path) -> String {
    let quote = |v: &str| format!("'{}'", v.replace('\'', "'\"'\"'"));
    if !program.is_absolute() {
        return rewritten.to_string();
    }
    let vars: String = rtk_env().iter().map(|(k, v)| format!(" {k}={}", quote(v))).collect();
    format!("rtk() {{ {} \"$@\"; }}; export{vars}; {rewritten}", quote(&program.to_string_lossy()))
}

/// True when a reduced command failed because the shell could not start `rtk` itself (missing, not
/// executable, blocked by the OS): the original command then runs once.
pub(crate) fn rtk_missing_at_run(output: &str) -> bool {
    ["rtk: command not found", "rtk: not found", "rtk: Permission denied", "rtk: cannot execute", "rtk: Operation not permitted", "rtk: No such file or directory"].iter().any(|m| output.contains(m))
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
            assert!(rewrite("git status", Duration::from_secs(5)).await.is_some_and(|r| r.starts_with("rtk() {") && r.ends_with("; rtk git status")));
        })
        .await;
    }

    #[tokio::test]
    async fn rewritten_with_the_ask_exit_code_is_still_used_but_deny_and_no_equivalent_are_not() {
        with_fake(r#"echo "rtk $2"; exit 3"#, || async {
            assert!(rewrite("git status", Duration::from_secs(5)).await.is_some_and(|r| r.starts_with("rtk() {") && r.ends_with("; rtk git status")));
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
    fn only_the_bundled_or_explicit_binary_is_ever_used() {
        let _env = factr_base::storage::lock_test_env();
        factr_base::env::remove_var("FACTR_RTK_BIN");
        // Next to the test executable there is no rtk, and the PATH is not consulted.
        assert_eq!(rtk_binary(), None);
        factr_base::env::set_var("FACTR_RTK_BIN", "/custom/rtk");
        assert_eq!(rtk_binary(), Some(std::path::PathBuf::from("/custom/rtk")));
        factr_base::env::remove_var("FACTR_RTK_BIN");
    }

    #[test]
    fn the_command_calls_the_bundled_rtk_by_absolute_path_with_telemetry_off() {
        let r = with_rtk_on_path("rtk git status", std::path::Path::new("/app/Resources/factr/rtk-bin/rtk"));
        assert!(r.starts_with("rtk() { '/app/Resources/factr/rtk-bin/rtk' \"$@\"; }; export RTK_TELEMETRY_DISABLED='1' RTK_NO_TOML='1'"), "{r}");
        assert!(r.ends_with("; rtk git status"));
        assert_eq!(with_rtk_on_path("rtk git status", std::path::Path::new("rtk")), "rtk git status");
        assert!(with_rtk_on_path("rtk ls", std::path::Path::new("/tmp/it's/rtk")).contains(r#"'/tmp/it'"'"'s/rtk'"#));
    }

    #[test]
    fn rtk_keeps_its_history_inside_the_factr_home() {
        let _env = factr_base::storage::lock_test_env();
        let home = tempfile::tempdir().unwrap();
        factr_base::env::set_var("FACTR_CONFIG_HOME", home.path());
        let env = rtk_env();
        factr_base::env::remove_var("FACTR_CONFIG_HOME");
        let get = |k: &str| env.iter().find(|(n, _)| *n == k).map(|(_, v)| v.clone());
        assert_eq!(get("RTK_TELEMETRY_DISABLED").as_deref(), Some("1"));
        for k in ["RTK_DB_PATH", "RTK_RECALL_DB", "RTK_TEE_DIR"] {
            assert!(get(k).is_some_and(|v| v.contains(&*home.path().to_string_lossy())), "{k}: {env:?}");
        }
    }

    #[test]
    fn a_missing_rtk_at_run_time_is_recognised() {
        assert!(rtk_missing_at_run("bash: rtk: command not found\n\nExit code: 127"));
        assert!(rtk_missing_at_run("sh: 1: rtk: not found"));
        assert!(rtk_missing_at_run("sh: /app/rtk-bin/rtk: Permission denied"));
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

    /// The shipped rtk itself, staged by `desktop/app/scripts/stage-rtk.mjs`; CI points `FACTR_TEST_REAL_RTK` at it.
    #[tokio::test]
    async fn the_real_bundled_rtk_rewrites_runs_and_keeps_its_data_in_the_factr_home() {
        let Ok(real) = std::env::var("FACTR_TEST_REAL_RTK") else { return };
        let _env = factr_base::storage::lock_test_env();
        let (profile, factr_home, repo) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let real_home = std::env::var_os("HOME");
        factr_base::env::set_var("HOME", profile.path());
        factr_base::env::set_var("XDG_DATA_HOME", profile.path().join("xdg"));
        factr_base::env::set_var("FACTR_RTK_BIN", &real);
        factr_base::env::set_var("FACTR_CONFIG_HOME", factr_home.path());
        reset_missing_cache();
        let git = |args: &[&str]| std::process::Command::new("git").args(args).current_dir(repo.path()).output().unwrap();
        git(&["init", "-q"]);
        let rewritten = rewrite("git status", Duration::from_secs(10)).await.expect("rtk rewrites git status");
        assert!(rewritten.ends_with("; rtk git status"), "{rewritten}");
        assert_eq!(rewrite("definitely-not-a-known-tool --x", Duration::from_secs(10)).await, None, "no equivalent: run unchanged");
        let run = std::process::Command::new("sh").arg("-c").arg(&rewritten).current_dir(repo.path()).output().unwrap();
        assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
        assert!(factr_home.path().join("rtk").join("history.db").is_file(), "history stays in the Factr home");
        assert_eq!(std::fs::read_dir(profile.path()).unwrap().count(), 0, "nothing is written to the user's profile");
        factr_base::env::remove_var("FACTR_RTK_BIN");
        factr_base::env::remove_var("FACTR_CONFIG_HOME");
        factr_base::env::remove_var("XDG_DATA_HOME");
        if let Some(h) = real_home {
            factr_base::env::set_var("HOME", h);
        }
        reset_missing_cache();
    }
}
