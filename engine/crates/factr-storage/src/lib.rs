use anyhow::Result;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::io::Write;
use std::path::{Path, PathBuf};

#[cfg(windows)]
use std::collections::{HashMap, HashSet};
#[cfg(windows)]
use std::sync::{LazyLock, Mutex};
#[cfg(windows)]
use std::time::{Duration, Instant};

#[cfg(windows)]
const SECRET_HARDEN_CACHE_TTL: Duration = Duration::from_secs(60);
#[cfg(windows)]
const SECRET_HARDEN_FAILURE_BACKOFF: Duration = Duration::from_secs(5);
#[cfg(windows)]
const SECRET_HARDEN_DEFER_DELAY: Duration = Duration::from_secs(30);

#[cfg(windows)]
#[derive(Clone, Copy)]
enum SecretHardenAttempt {
    InFlight,
    Succeeded(Instant),
    Failed(Instant),
}

#[cfg(windows)]
#[derive(Default)]
struct SecretHardenState {
    directories: HashMap<PathBuf, SecretHardenAttempt>,
    files: HashMap<PathBuf, SecretHardenAttempt>,
    pending_directories: HashSet<PathBuf>,
    pending_files: HashSet<PathBuf>,
    worker_running: bool,
}

#[cfg(windows)]
impl SecretHardenState {
    /// Queue a path for best-effort hardening. Returns true when the caller
    /// should start the single worker for this process.
    fn enqueue(&mut self, path: &Path, directory: bool, now: Instant) -> bool {
        let attempted = if directory {
            &self.directories
        } else {
            &self.files
        };
        let should_suppress = match attempted.get(path) {
            Some(SecretHardenAttempt::InFlight) => true,
            Some(SecretHardenAttempt::Succeeded(attempted_at)) => {
                now.saturating_duration_since(*attempted_at) < SECRET_HARDEN_CACHE_TTL
            }
            Some(SecretHardenAttempt::Failed(attempted_at)) => {
                now.saturating_duration_since(*attempted_at) < SECRET_HARDEN_FAILURE_BACKOFF
            }
            None => false,
        };
        if should_suppress {
            return false;
        }

        if directory {
            self.pending_directories.insert(path.to_path_buf());
        } else {
            self.pending_files.insert(path.to_path_buf());
        }
        if self.worker_running {
            false
        } else {
            self.worker_running = true;
            true
        }
    }
}

#[cfg(windows)]
static SECRET_HARDEN_STATE: LazyLock<Mutex<SecretHardenState>> =
    LazyLock::new(|| Mutex::new(SecretHardenState::default()));

mod active_pids;
pub use active_pids::{
    SessionCounts, SessionPresence, StreamingGuard, active_pids_dir, active_session_ids,
    find_active_session_id_by_pid, internal_pids_dir, mark_streaming, prune_active_pids_owned_by,
    register_active_pid, session_counts, sweep_dead_active_pids, session_is_internal, session_presence,
    set_session_internal, streaming_pids_dir, streaming_session_ids, unmark_streaming,
    unregister_active_pid, user_session_counts, user_session_presence,
};

/// Platform-aware runtime directory for sockets and ephemeral state.
///
/// - Linux: `$XDG_RUNTIME_DIR` (typically `/run/user/<uid>`)
/// - macOS: `$TMPDIR` (per-user, e.g. `/var/folders/xx/.../T/`)
/// - Fallback: `std::env::temp_dir()`
///
/// Can be overridden with `$FACTR_RUNTIME_DIR`.
pub fn runtime_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("FACTR_RUNTIME_DIR") {
        return PathBuf::from(dir);
    }
    if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR") {
        return PathBuf::from(dir);
    }
    #[cfg(target_os = "macos")]
    {
        if let Ok(dir) = std::env::var("TMPDIR") {
            return PathBuf::from(dir);
        }
    }

    let dir = fallback_runtime_dir();
    ensure_private_runtime_dir(&dir);
    dir
}

fn fallback_runtime_dir() -> PathBuf {
    std::env::temp_dir().join(format!("factr-{}", runtime_user_discriminator()))
}

#[cfg(unix)]
fn runtime_user_discriminator() -> String {
    unsafe { libc::geteuid() }.to_string()
}

#[cfg(not(unix))]
fn runtime_user_discriminator() -> String {
    let raw = std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_else(|_| "user".to_string());
    let sanitized: String = raw
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
        .take(64)
        .collect();
    if sanitized.is_empty() {
        "user".to_string()
    } else {
        sanitized
    }
}

fn ensure_private_runtime_dir(path: &Path) {
    let _ = std::fs::create_dir_all(path);
    #[cfg(unix)]
    {
        let _ = factr_core::fs::set_directory_permissions_owner_only(path);
    }
}

/// The login user's home from the passwd database, not `$HOME` (a test may repoint that).
pub fn passwd_home() -> Option<PathBuf> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let mut buf = vec![0u8; 8192];
        // SAFETY: zeroed passwd is a valid out-parameter for getpwuid_r.
        let mut pwd: libc::passwd = unsafe { std::mem::zeroed() };
        let mut out: *mut libc::passwd = std::ptr::null_mut();
        // SAFETY: buffers outlive the call; getpwuid_r fills `pwd` and points `out` at it.
        let rc = unsafe { libc::getpwuid_r(libc::getuid(), &mut pwd, buf.as_mut_ptr().cast(), buf.len(), &mut out) };
        if rc != 0 || out.is_null() || pwd.pw_dir.is_null() {
            return None;
        }
        // SAFETY: pw_dir is a NUL-terminated string inside `buf`.
        let dir = unsafe { std::ffi::CStr::from_ptr(pwd.pw_dir) };
        return Some(PathBuf::from(std::ffi::OsStr::from_bytes(dir.to_bytes())));
    }
    #[allow(unreachable_code)]
    dirs::home_dir()
}

/// `path` with its deepest existing ancestor canonicalised (symlinks resolved), the rest appended.
fn resolve_path(path: &Path) -> PathBuf {
    let mut tail = Vec::new();
    let mut cur = path.to_path_buf();
    loop {
        if let Ok(real) = cur.canonicalize() {
            return tail.iter().rev().fold(real, |acc, part| acc.join(part));
        }
        match (cur.file_name().map(|n| n.to_os_string()), cur.parent().map(Path::to_path_buf)) {
            (Some(name), Some(parent)) => {
                tail.push(name);
                cur = parent;
            }
            _ => return path.to_path_buf(),
        }
    }
}

/// Whether `path` is inside the real `~/.factr` or `~/.factr/engine` of the login user.
pub fn is_real_home_path(path: &Path) -> bool {
    let Some(home) = passwd_home() else { return false };
    let target = resolve_path(path);
    [".factr", ".factr/engine"].iter().any(|dir| target.starts_with(resolve_path(&home.join(dir))))
}

/// Whether this process is a cargo test binary (`target/<profile>/deps/<name>-<hash>`).
pub fn in_test_binary() -> bool {
    std::env::current_exe().ok().and_then(|exe| exe.parent().and_then(Path::file_name).map(|d| d == "deps")).unwrap_or(false)
}

/// The process-lifetime temp home a test binary gets instead of the real `~/.factr/engine`.
fn test_home() -> PathBuf {
    static HOME: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    HOME.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("factr-test-home-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        dir
    })
    .clone()
}

pub fn factr_dir() -> Result<PathBuf> {
    resolve_factr_dir(std::env::var_os("FACTR_HOME").map(PathBuf::from), in_test_binary())
}

/// [`factr_dir`] for a given `FACTR_HOME` and test-binary flag. A cargo test binary never gets the real
/// home, whether `FACTR_HOME` is unset (the developer's own dir) or inherited from a dev shell that
/// exports it: it gets a process-lifetime temp dir instead.
fn resolve_factr_dir(configured: Option<PathBuf>, test_binary: bool) -> Result<PathBuf> {
    match configured {
        Some(path) if test_binary && is_real_home_path(&path) => Ok(test_home()),
        Some(path) => Ok(path),
        None => {
            let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("No home directory"))?;
            let dir = home.join(".factr/engine");
            // A test that points HOME at a temp dir keeps its own `$HOME/.factr/engine`.
            if test_binary && is_real_home_path(&dir) { Ok(test_home()) } else { Ok(dir) }
        }
    }
}

/// Whether `FACTR_HOME` redirects this process away from the user's real
/// `~/.factr/engine` directory.
///
/// Sandboxed runs must not consult machine-global resources, such as the macOS
/// Keychain, that cannot be redirected beneath `FACTR_HOME`.
pub fn running_with_sandboxed_home() -> bool {
    let Some(configured) = std::env::var_os("FACTR_HOME").map(PathBuf::from) else {
        return false;
    };
    let Some(default) = dirs::home_dir().map(|home| home.join(".factr/engine")) else {
        return true;
    };

    match (configured.canonicalize(), default.canonicalize()) {
        (Ok(configured), Ok(default)) => configured != default,
        _ => configured != default,
    }
}

pub fn logs_dir() -> Result<PathBuf> {
    Ok(factr_dir()?.join("logs"))
}

/// Durable state directory for state that must survive reboots.
///
/// [`runtime_dir`] typically resolves to a tmpfs (for example
/// `/run/user/<uid>` on Linux) that is wiped on reboot, so it must only hold
/// sockets and truly ephemeral state. State that has to outlive a reboot,
/// such as swarm plans and member records, belongs here instead: it resolves
/// to `~/.factr/engine/state` (respecting `FACTR_HOME`).
///
/// When `FACTR_RUNTIME_DIR` is set (tests and sandboxed temp servers), it
/// takes precedence so isolated runs never touch the real factr home.
pub fn durable_state_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("FACTR_RUNTIME_DIR") {
        return PathBuf::from(dir).join("durable-state");
    }
    match factr_dir() {
        Ok(dir) => dir.join("state"),
        Err(_) => runtime_dir().join("durable-state"),
    }
}

/// Resolve factr's app-owned config directory.
///
/// Default location is the platform config dir + `factr` (for example
/// `~/.config/factr` on Linux). When `FACTR_HOME` is set, sandbox this under
/// `$FACTR_HOME/config/factr` so self-dev/tests do not leak into the user's
/// real config directory.
pub fn app_config_dir() -> Result<PathBuf> {
    if let Ok(path) = std::env::var("FACTR_HOME") {
        return Ok(PathBuf::from(path).join("config").join("factr"));
    }

    let config_dir =
        dirs::config_dir().ok_or_else(|| anyhow::anyhow!("No config directory found"))?;
    Ok(config_dir.join("factr"))
}

/// Resolve a path under the user's home directory, but sandbox it under
/// `$FACTR_HOME/external/` when `FACTR_HOME` is set.
///
/// This keeps external provider auth files isolated during tests and sandboxed
/// runs without changing default on-disk locations for normal users.
pub fn user_home_path(relative: impl AsRef<Path>) -> Result<PathBuf> {
    let relative = relative.as_ref();
    if relative.is_absolute() {
        anyhow::bail!(
            "user_home_path expects a relative path, got {}",
            relative.display()
        );
    }

    if let Ok(path) = std::env::var("FACTR_HOME") {
        return Ok(PathBuf::from(path).join("external").join(relative));
    }

    let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("No home directory"))?;
    Ok(home.join(relative))
}

/// Best-effort startup hardening for local config dirs that may store credentials.
///
/// This intentionally ignores failures so startup does not fail on exotic
/// filesystems, but it narrows exposure on typical Unix systems.
pub fn harden_user_config_permissions() {
    #[cfg(windows)]
    {
        if let Some(config_dir) = dirs::config_dir() {
            let factr_config_dir = config_dir.join("factr");
            if factr_config_dir.exists() {
                schedule_windows_path_hardening(&factr_config_dir, true);
            }
        }

        if let Ok(factr_home) = factr_dir()
            && factr_home.exists()
        {
            schedule_windows_path_hardening(&factr_home, true);
        }
        return;
    }

    #[cfg(not(windows))]
    {
        if let Some(config_dir) = dirs::config_dir() {
            let factr_config_dir = config_dir.join("factr");
            if factr_config_dir.exists() {
                let _ = factr_core::fs::set_directory_permissions_owner_only(&factr_config_dir);
            }
        }

        if let Ok(factr_home) = factr_dir()
            && factr_home.exists()
        {
            let _ = factr_core::fs::set_directory_permissions_owner_only(&factr_home);
        }
    }
}

/// Best-effort hardening for a secret-bearing file and its parent directory.
///
/// This is used before reading credential files so legacy permissive modes can
/// be tightened opportunistically.
pub fn harden_secret_file_permissions(path: &Path) {
    #[cfg(windows)]
    {
        harden_secret_file_permissions_windows(path);
        return;
    }

    #[cfg(not(windows))]
    {
        if let Some(parent) = path.parent() {
            let _ = factr_core::fs::set_directory_permissions_owner_only(parent);
        }
        if path.exists() {
            let _ = factr_core::fs::set_permissions_owner_only(path);
        }
    }
}

#[cfg(windows)]
fn harden_secret_file_permissions_windows(path: &Path) {
    // Windows ACL replacement is substantially more expensive than chmod and
    // security products can amplify it into seconds. Credential readers call
    // this helper frequently, including on the startup and TUI render paths.
    // Read-time hardening is opportunistic, while Factr's own secret writes
    // harden synchronously below. Defer the opportunistic repair so first-frame
    // latency does not inherit multi-second SetNamedSecurityInfoW calls. The
    // worker coalesces repeated probes and retries paths after a short TTL.
    if let Some(parent) = path.parent() {
        schedule_windows_path_hardening(parent, true);
    }
    if path.exists() {
        schedule_windows_path_hardening(path, false);
    }
}

#[cfg(windows)]
fn schedule_windows_path_hardening(path: &Path, directory: bool) {
    let should_spawn = {
        let Ok(mut state) = SECRET_HARDEN_STATE.lock() else {
            return;
        };
        state.enqueue(path, directory, Instant::now())
    };

    if !should_spawn {
        return;
    }

    if std::thread::Builder::new()
        .name("factr-windows-acl-harden".to_string())
        .spawn(|| {
            std::thread::sleep(SECRET_HARDEN_DEFER_DELAY);
            run_windows_hardening_worker();
        })
        .is_err()
        && let Ok(mut state) = SECRET_HARDEN_STATE.lock()
    {
        state.worker_running = false;
    }
}

#[cfg(windows)]
fn run_windows_hardening_worker() {
    loop {
        let (directories, files) = {
            let Ok(mut state) = SECRET_HARDEN_STATE.lock() else {
                return;
            };
            if state.pending_directories.is_empty() && state.pending_files.is_empty() {
                state.worker_running = false;
                return;
            }
            let directories = std::mem::take(&mut state.pending_directories);
            let files = std::mem::take(&mut state.pending_files);
            // Mark attempts before releasing the lock. Otherwise render-time
            // probes can requeue the same paths while a slow ACL call is in
            // flight, keeping the worker in an endless hardening loop.
            for path in &directories {
                state
                    .directories
                    .insert(path.clone(), SecretHardenAttempt::InFlight);
            }
            for path in &files {
                state
                    .files
                    .insert(path.clone(), SecretHardenAttempt::InFlight);
            }
            (directories, files)
        };

        let mut directory_results = Vec::with_capacity(directories.len());
        for path in &directories {
            let succeeded = factr_core::fs::set_directory_permissions_owner_only(path).is_ok();
            directory_results.push((path.clone(), succeeded));
        }
        let mut file_results = Vec::with_capacity(files.len());
        for path in &files {
            let succeeded =
                !path.exists() || factr_core::fs::set_permissions_owner_only(path).is_ok();
            file_results.push((path.clone(), succeeded));
        }

        let Ok(mut state) = SECRET_HARDEN_STATE.lock() else {
            return;
        };
        let completed_at = Instant::now();
        for (path, succeeded) in directory_results {
            state.directories.insert(
                path,
                if succeeded {
                    SecretHardenAttempt::Succeeded(completed_at)
                } else {
                    SecretHardenAttempt::Failed(completed_at)
                },
            );
        }
        for (path, succeeded) in file_results {
            state.files.insert(
                path,
                if succeeded {
                    SecretHardenAttempt::Succeeded(completed_at)
                } else {
                    SecretHardenAttempt::Failed(completed_at)
                },
            );
        }
        if state.pending_directories.is_empty() && state.pending_files.is_empty() {
            state.worker_running = false;
            return;
        }
        // New paths arrived while the ACL calls were running. Process them in
        // this worker without another startup delay.
        drop(state);
    }
}

/// Validate an external auth file managed by another tool before reading it.
///
/// factr intentionally avoids mutating these files. We also reject obvious risky
/// cases like symlinks so a remembered trust decision stays bound to a real file
/// path rather than an arbitrary redirect.
pub fn validate_external_auth_file(path: &Path) -> Result<PathBuf> {
    let metadata = std::fs::symlink_metadata(path).map_err(|e| {
        anyhow::anyhow!(
            "Failed to inspect external auth file {}: {}",
            path.display(),
            e
        )
    })?;
    if metadata.file_type().is_symlink() {
        anyhow::bail!(
            "Refusing to read external auth file via symlink: {}",
            path.display()
        );
    }
    if !metadata.is_file() {
        anyhow::bail!(
            "External auth path is not a regular file: {}",
            path.display()
        );
    }
    std::fs::canonicalize(path).map_err(|e| {
        anyhow::anyhow!(
            "Failed to canonicalize external auth file {}: {}",
            path.display(),
            e
        )
    })
}

pub fn ensure_dir(path: &Path) -> Result<()> {
    if !path.exists() {
        std::fs::create_dir_all(path)?;
        factr_core::fs::set_directory_permissions_owner_only(path)?;
    }
    Ok(())
}

pub fn write_text_secret(path: &Path, content: &str) -> Result<()> {
    write_bytes_inner(path, content.as_bytes(), true, true)
}

pub fn write_json<T: Serialize + ?Sized>(path: &Path, value: &T) -> Result<()> {
    write_json_inner(path, value, true, false)
}

pub fn write_json_secret<T: Serialize + ?Sized>(path: &Path, value: &T) -> Result<()> {
    write_json_inner(path, value, true, true)
}

/// Fast JSON write: atomic rename but no fsync. Good for frequent saves where
/// durability on power loss is not critical (e.g., session saves during tool execution).
/// Data is still safe against process crashes (atomic rename protects against partial writes).
pub fn write_json_fast<T: Serialize + ?Sized>(path: &Path, value: &T) -> Result<()> {
    write_json_inner(path, value, false, false)
}

/// Atomically write raw bytes to `path` (temp file + rename), fsync'd for
/// durability. Used for editing user config files where a torn write would be
/// catastrophic.
pub fn write_bytes(path: &Path, bytes: &[u8]) -> Result<()> {
    write_bytes_inner(path, bytes, true, false)
}

fn write_json_inner<T: Serialize + ?Sized>(
    path: &Path,
    value: &T,
    durable: bool,
    secret: bool,
) -> Result<()> {
    let bytes = serde_json::to_vec(value)?;
    write_bytes_inner(path, &bytes, durable, secret)
}

fn write_bytes_inner(path: &Path, bytes: &[u8], durable: bool, secret: bool) -> Result<()> {
    if let Some(parent) = path.parent() {
        ensure_dir(parent)?;
        if secret {
            // Writes remain strict even though read-time legacy repair is
            // deferred on Windows. Harden the container before any secret
            // bytes are created so a permissive inherited ACL is never
            // published, even briefly.
            factr_core::fs::set_directory_permissions_owner_only(parent)?;
        }
    }

    let pid = std::process::id();
    let nonce: u64 = rand::random();
    let tmp_path = path.with_extension(format!("tmp.{}.{}", pid, nonce));

    let result = (|| -> Result<()> {
        let file = std::fs::File::create(&tmp_path)?;
        if secret {
            factr_core::fs::set_permissions_owner_only(&tmp_path)?;
        }
        let mut writer = std::io::BufWriter::new(file);
        writer.write_all(bytes)?;
        let file = writer
            .into_inner()
            .map_err(|e| anyhow::anyhow!("flush failed: {}", e))?;

        if durable {
            file.sync_all()?;
        }

        if path.exists() {
            let bak_path = path.with_extension("bak");
            if secret {
                factr_core::fs::set_permissions_owner_only(path)?;
            }
            // Preserve the previous version as .bak without ever leaving the
            // primary path missing. On Unix, rename(tmp, path) atomically
            // replaces the destination, so the backup can be a hard link to
            // the old inode: concurrent readers always see either the old or
            // the new content, never ENOENT. (The old rename-away approach
            // opened a window where the primary did not exist, which made
            // concurrent load-all style readers silently drop entries, e.g.
            // self-dev build requests "disappearing" from the queue.)
            #[cfg(unix)]
            {
                let _ = std::fs::remove_file(&bak_path);
                let _ = std::fs::hard_link(path, &bak_path);
            }
            // On Windows, rename fails when the destination exists, so the
            // primary must be moved away first; the brief missing window is
            // unavoidable without platform-specific replace APIs.
            #[cfg(not(unix))]
            {
                let _ = std::fs::remove_file(&bak_path);
                let _ = std::fs::rename(path, &bak_path);
            }
            if secret && bak_path.exists() {
                factr_core::fs::set_permissions_owner_only(&bak_path)?;
            }
        }

        std::fs::rename(&tmp_path, path)?;
        if secret {
            factr_core::fs::set_permissions_owner_only(path)?;
        }

        #[cfg(unix)]
        if durable
            && let Some(parent) = path.parent()
            && let Ok(dir) = std::fs::File::open(parent)
        {
            let _ = dir.sync_all();
        }

        Ok(())
    })();

    if result.is_err() {
        let _ = std::fs::remove_file(&tmp_path);
    }

    result
}

pub enum StorageRecoveryEvent<'a> {
    CorruptPrimary {
        path: &'a Path,
        error: &'a serde_json::Error,
    },
    RecoveredFromBackup {
        backup_path: &'a Path,
    },
}

pub fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    read_json_with_recovery_handler(path, |event| match event {
        StorageRecoveryEvent::CorruptPrimary { path, error } => {
            eprintln!(
                "Corrupt JSON at {}, trying backup: {}",
                path.display(),
                error
            );
        }
        StorageRecoveryEvent::RecoveredFromBackup { backup_path } => {
            eprintln!("Recovered from backup: {}", backup_path.display());
        }
    })
}

pub fn read_json_with_recovery_handler<T, F>(path: &Path, mut on_recovery: F) -> Result<T>
where
    T: DeserializeOwned,
    F: FnMut(StorageRecoveryEvent<'_>),
{
    let data = std::fs::read_to_string(path)?;
    match serde_json::from_str(&data) {
        Ok(val) => Ok(val),
        Err(e) => {
            let bak_path = path.with_extension("bak");
            if bak_path.exists() {
                on_recovery(StorageRecoveryEvent::CorruptPrimary { path, error: &e });
                let bak_data = std::fs::read_to_string(&bak_path)?;
                match serde_json::from_str(&bak_data) {
                    Ok(val) => {
                        on_recovery(StorageRecoveryEvent::RecoveredFromBackup {
                            backup_path: &bak_path,
                        });
                        let _ = std::fs::copy(&bak_path, path);
                        Ok(val)
                    }
                    Err(bak_err) => Err(anyhow::anyhow!(
                        "Corrupt JSON at {} ({}), backup also corrupt ({})",
                        path.display(),
                        e,
                        bak_err
                    )),
                }
            } else {
                Err(anyhow::anyhow!("Corrupt JSON at {}: {}", path.display(), e))
            }
        }
    }
}

/// Fast append of a single JSON value followed by a newline.
/// Intended for append-only journals where per-write fsync is not required.
///
/// The entire line (value + trailing newline) is serialized into one buffer
/// and appended with a single `write_all`. Streaming the serializer straight
/// into the file issued many small writes, so a concurrent reader (or a
/// process killed mid-append) could observe a torn half-line, and two
/// concurrent appenders could interleave fragments. A single `O_APPEND` write
/// of the complete line keeps each journal line intact.
pub fn append_json_line_fast<T: Serialize + ?Sized>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        ensure_dir(parent)?;
    }

    let mut line = serde_json::to_vec(value)?;
    line.push(b'\n');
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    file.write_all(&line)?;
    Ok(())
}

#[cfg(all(test, windows))]
mod windows_hardening_tests {
    use super::*;

    #[test]
    fn first_path_starts_one_worker_and_repeated_paths_are_coalesced() {
        let mut state = SecretHardenState::default();
        let now = Instant::now();
        let directory = Path::new(r"C:\Users\test\.factr/engine");
        let file = directory.join("auth.json");

        assert!(state.enqueue(directory, true, now));
        assert!(!state.enqueue(directory, true, now));
        assert!(!state.enqueue(&file, false, now));
        assert!(state.worker_running);
        assert_eq!(state.pending_directories.len(), 1);
        assert_eq!(state.pending_files.len(), 1);
    }

    #[test]
    fn recently_attempted_paths_are_not_requeued() {
        let mut state = SecretHardenState::default();
        let attempted_at = Instant::now();
        let file = PathBuf::from(r"C:\Users\test\.factr/engine\auth.json");
        state
            .files
            .insert(file.clone(), SecretHardenAttempt::Succeeded(attempted_at));

        assert!(!state.enqueue(&file, false, attempted_at));
        assert!(!state.worker_running);
        assert!(state.pending_files.is_empty());
    }

    #[test]
    fn failed_paths_retry_after_shorter_backoff() {
        let mut state = SecretHardenState::default();
        let attempted_at = Instant::now();
        let file = PathBuf::from(r"C:\Users\test\.factr/engine\auth.json");
        state
            .files
            .insert(file.clone(), SecretHardenAttempt::Failed(attempted_at));

        assert!(!state.enqueue(&file, false, attempted_at));
        let retry_at = attempted_at + SECRET_HARDEN_FAILURE_BACKOFF;
        assert!(state.enqueue(&file, false, retry_at));
    }

    #[test]
    fn in_flight_paths_are_not_requeued() {
        let mut state = SecretHardenState::default();
        let now = Instant::now();
        let file = PathBuf::from(r"C:\Users\test\.factr/engine\auth.json");
        state
            .files
            .insert(file.clone(), SecretHardenAttempt::InFlight);

        assert!(!state.enqueue(&file, false, now));
        assert!(state.pending_files.is_empty());
    }
}

#[cfg(test)]
mod real_home_tests {
    use super::*;

    #[test]
    fn a_test_binary_never_resolves_the_real_factr_home() {
        let real = passwd_home().unwrap().join(".factr/engine");
        let sandbox = test_home();
        assert!(in_test_binary(), "this is a cargo test binary");
        assert_eq!(resolve_factr_dir(Some(real.clone()), true).unwrap(), sandbox, "an exported real FACTR_HOME");
        assert_eq!(resolve_factr_dir(Some(real.join("sessions")), true).unwrap(), sandbox, "a path inside it");
        assert!(!is_real_home_path(&resolve_factr_dir(None, true).unwrap()), "unset");
        // Anything else is taken as given, and a real run still gets the real home.
        let temp = std::env::temp_dir().join("elsewhere");
        assert_eq!(resolve_factr_dir(Some(temp.clone()), true).unwrap(), temp);
        assert_eq!(resolve_factr_dir(Some(real.clone()), false).unwrap(), real);
        assert!(!is_real_home_path(&sandbox));
    }
}
