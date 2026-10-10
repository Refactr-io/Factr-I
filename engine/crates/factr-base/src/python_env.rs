//! The ONE Python environment of a session.
//!
//! The REPL, the environment snapshot and `bash` must all see the same interpreter and therefore the
//! same installed packages. The interpreter is `FACTR_REPL_PYTHON` when set (explicit), else the first
//! real `python3` on `PATH`. Bash gets it through the process `PATH` it inherits: [`install_python_shim`]
//! adds a `python` launcher under `$FACTR_HOME/bin` only when no `python` exists yet (it never shadows
//! `python3`).

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

fn is_executable_file(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata().is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

fn on_path(name: &str, path: &OsStr) -> Option<PathBuf> {
    std::env::split_paths(path).map(|dir| dir.join(name)).find(|p| is_executable_file(p))
}

/// The first real `python3` in `path`. The macOS `/usr/bin/python3` stub only opens an installer
/// prompt when no developer tools are installed, so it is replaced by the tools' interpreter, or skipped.
pub fn python3_in(path: &OsStr) -> Option<PathBuf> {
    let found = on_path("python3", path)?;
    #[cfg(target_os = "macos")]
    if found == Path::new("/usr/bin/python3") {
        return [
            "/Library/Developer/CommandLineTools/usr/bin/python3",
            "/Applications/Xcode.app/Contents/Developer/Library/Frameworks/Python3.framework/Versions/Current/bin/python3",
        ]
        .into_iter()
        .map(PathBuf::from)
        .find(|p| is_executable_file(p));
    }
    Some(found)
}

/// `explicit` (`FACTR_REPL_PYTHON`) when non-empty, else the first real `python3` in `path`.
pub fn interpreter_from(explicit: Option<OsString>, path: Option<OsString>) -> Option<PathBuf> {
    match explicit.filter(|p| !p.is_empty()) {
        Some(python) => Some(PathBuf::from(python)),
        None => python3_in(&path?),
    }
}

/// The interpreter of this process's environment.
pub fn interpreter() -> Option<PathBuf> {
    ensure_session_venv(VENV_WAIT);
    interpreter_from(std::env::var_os("FACTR_REPL_PYTHON"), std::env::var_os("PATH"))
}

/// Make `python` resolvable for bash: when `path` has no `python`, write `<home>/bin/python` and
/// return the directory to put first on `PATH`. `None` when `python` already exists or the shim
/// cannot be written. The shim is a two-line launcher, not a symlink: a symlink gives the interpreter
/// the shim's own path, and a virtualenv interpreter finds its packages through `pyvenv.cfg` next to
/// the path it was started by, so a symlinked venv python silently loses every installed package.
pub fn python_shim_dir(home: &Path, interpreter: &Path, path: &OsStr) -> Option<PathBuf> {
    if on_path("python", path).is_some() {
        return None;
    }
    let dir = home.join("bin");
    std::fs::create_dir_all(&dir).ok()?;
    let quoted = interpreter.to_string_lossy().replace('\'', "'\\''");
    let script = format!("#!/bin/sh\nexec '{quoted}' \"$@\"\n");
    let file = dir.join("python");
    if std::fs::read_to_string(&file).is_ok_and(|current| current == script) {
        return Some(dir);
    }
    let staged = dir.join(format!(".python.{}", std::process::id()));
    std::fs::write(&staged, script).ok()?;
    #[cfg(unix)]
    std::fs::set_permissions(&staged, std::os::unix::fs::PermissionsExt::from_mode(0o755)).ok()?;
    std::fs::rename(&staged, &file).ok()?;
    Some(dir)
}

/// Once at engine start (before any thread): prepend the shim directory to this process's `PATH`,
/// which every spawned shell inherits.
pub fn install_python_shim(home: &Path) {
    let Some(path) = std::env::var_os("PATH") else { return };
    // Resolved against the original PATH, so a `python` the session venv will provide is not shadowed:
    // the venv's `bin` goes first, the shim (used only when the venv is absent or failed) second.
    let shim = interpreter_from(std::env::var_os("FACTR_REPL_PYTHON"), Some(path.clone()))
        .and_then(|python| python_shim_dir(home, &python, &path));
    let venv_bin = install_session_venv(&path);
    let mut dirs: Vec<PathBuf> = venv_bin.into_iter().chain(shim).collect();
    if dirs.is_empty() {
        return;
    }
    dirs.extend(std::env::split_paths(&path));
    if let Ok(joined) = std::env::join_paths(dirs) {
        crate::env::set_var("PATH", joined);
    }
}

const VENV_PREFIX: &str = "factr-session-venv-";

/// The creator's private TMPDIR (pip's `ensurepip` leaves a `tmp*cacert.pem` behind in the one it
/// is given), a sibling of the venv; removed with it.
fn build_scratch(dir: &Path) -> PathBuf {
    let mut name = dir.file_name().unwrap_or_default().to_os_string();
    name.push("-build");
    dir.with_file_name(name)
}

fn remove_venv_dirs(dir: &Path) {
    let _ = std::fs::remove_dir_all(dir);
    let _ = std::fs::remove_dir_all(build_scratch(dir));
}

/// How long a caller waits for the session environment before going on without it.
pub const VENV_WAIT: std::time::Duration = std::time::Duration::from_secs(15);

#[derive(PartialEq, Clone, Copy)]
enum Phase {
    Pending,
    Ready,
    Failed,
}

/// The session environment being built on a background thread. Everything that reads the venv (the
/// first command, the REPL worker, the environment line, the hints) goes through [`VenvGate::wait`].
pub struct VenvGate {
    phase: std::sync::Mutex<Phase>,
    changed: std::sync::Condvar,
    /// Set on shutdown or when a waiter gave up: the creator must stop and leave nothing behind.
    cancelled: std::sync::atomic::AtomicBool,
    /// Process group of the creator's child, so cancelling kills it and its own children.
    child_group: std::sync::atomic::AtomicI32,
    dir: PathBuf,
    on_fail: Box<dyn Fn() + Send + Sync>,
    logged: std::sync::Once,
}

impl VenvGate {
    /// Run `create` on a background thread. It returns whether the venv is usable.
    pub fn start(dir: PathBuf, on_fail: Box<dyn Fn() + Send + Sync>, create: impl FnOnce(&VenvGate) -> bool + Send + 'static) -> std::sync::Arc<Self> {
        let gate = std::sync::Arc::new(Self {
            phase: std::sync::Mutex::new(Phase::Pending),
            changed: std::sync::Condvar::new(),
            cancelled: std::sync::atomic::AtomicBool::new(false),
            child_group: std::sync::atomic::AtomicI32::new(0),
            dir,
            on_fail,
            logged: std::sync::Once::new(),
        });
        let worker = gate.clone();
        let spawned = std::thread::Builder::new().name("session-venv".into()).spawn(move || {
            let made = create(&worker);
            let mut phase = worker.phase.lock().unwrap_or_else(|e| e.into_inner());
            let usable = made && !worker.cancelled.load(std::sync::atomic::Ordering::SeqCst);
            // A waiter that gave up already set `Failed`; a late result never overrides it.
            if *phase == Phase::Pending {
                *phase = if usable { Phase::Ready } else { Phase::Failed };
            }
            let failed = *phase == Phase::Failed;
            drop(phase);
            worker.changed.notify_all();
            if failed {
                remove_venv_dirs(&worker.dir);
                (worker.on_fail)();
            }
        });
        if spawned.is_err() {
            *gate.phase.lock().unwrap_or_else(|e| e.into_inner()) = Phase::Failed;
            (gate.on_fail)();
        }
        gate
    }

    pub fn pending(&self) -> bool {
        *self.phase.lock().unwrap_or_else(|e| e.into_inner()) == Phase::Pending
    }

    /// Wait for the venv: `true` when it is usable. Returns at once when it is finished. On timeout
    /// the build is cancelled and removed (logged once), so the caller proceeds without the venv.
    pub fn wait(&self, timeout: std::time::Duration) -> bool {
        let guard = self.phase.lock().unwrap_or_else(|e| e.into_inner());
        let (guard, _) = self.changed.wait_timeout_while(guard, timeout, |p| *p == Phase::Pending).unwrap_or_else(|e| e.into_inner());
        match *guard {
            Phase::Ready => true,
            Phase::Failed => {
                drop(guard);
                self.log_once("the session environment could not be created; using the system interpreter");
                false
            }
            Phase::Pending => {
                drop(guard);
                self.log_once("the session environment was not ready in time; using the system interpreter");
                self.cancel();
                *self.phase.lock().unwrap_or_else(|e| e.into_inner()) = Phase::Failed;
                self.changed.notify_all();
                (self.on_fail)();
                false
            }
        }
    }

    fn log_once(&self, message: &str) {
        self.logged.call_once(|| crate::logging::warn(message));
    }

    /// Stop the build: mark it cancelled and kill the creator's process group.
    fn cancel(&self) {
        self.cancelled.store(true, std::sync::atomic::Ordering::SeqCst);
        #[cfg(unix)]
        {
            let group = self.child_group.load(std::sync::atomic::Ordering::SeqCst);
            if group > 0 {
                // SAFETY: SIGKILL to the process group this gate's creator started.
                unsafe { libc::kill(-group, libc::SIGKILL) };
            }
        }
    }

    /// Shutdown: cancel, give the creator thread a moment to reap its child, remove the directory.
    pub fn shutdown(&self) {
        self.cancel();
        {
            let guard = self.phase.lock().unwrap_or_else(|e| e.into_inner());
            let _ = self.changed.wait_timeout_while(guard, std::time::Duration::from_millis(300), |p| *p == Phase::Pending);
        }
        let mut phase = self.phase.lock().unwrap_or_else(|e| e.into_inner());
        if *phase == Phase::Pending {
            *phase = Phase::Failed;
        }
        drop(phase);
        remove_venv_dirs(&self.dir);
    }
}

static GATE: std::sync::RwLock<Option<std::sync::Arc<VenvGate>>> = std::sync::RwLock::new(None);

fn gate() -> Option<std::sync::Arc<VenvGate>> {
    GATE.read().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Tests only: swap the session gate (for example one built on a slow fake creator).
#[doc(hidden)]
pub fn set_gate_for_test(gate: Option<std::sync::Arc<VenvGate>>) {
    *GATE.write().unwrap_or_else(|e| e.into_inner()) = gate;
}

/// Block until the session environment is built (at most `timeout`); `true` when it is usable.
/// `false` at once when there is none to wait for. Call it before anything reads the venv path or
/// interpreter, or starts a command that should see it on `PATH`.
pub fn ensure_session_venv(timeout: std::time::Duration) -> bool {
    gate().is_some_and(|gate| gate.wait(timeout))
}

/// Whether the session environment is still being built (a cheap check before an async caller moves
/// the blocking wait off its thread).
pub fn session_venv_pending() -> bool {
    gate().is_some_and(|gate| gate.pending())
}

/// Stop building and remove the venv this process made (never one it inherited). Best-effort, for
/// the shutdown paths; a half-built directory and the creator's child process are gone afterwards.
pub fn remove_session_venv() {
    if let Some(gate) = gate() {
        gate.shutdown();
    }
}

/// Whether `pid` is a running process (signal 0).
#[cfg(unix)]
fn pid_alive(pid: u32) -> bool {
    // SAFETY: signal 0 only checks existence and permission.
    let r = unsafe { libc::kill(pid as libc::pid_t, 0) };
    r == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}
#[cfg(not(unix))]
fn pid_alive(_pid: u32) -> bool {
    true
}

/// Delete `factr-session-venv-<pid>` directories in `tmp` whose process is gone. Only the exact name
/// pattern, real directories (no symlinks) owned by the current user. Returns how many were removed.
pub fn sweep_stale_session_venvs(tmp: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(tmp) else { return 0 };
    let mut removed = 0;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(pid) = name.to_str().and_then(|n| n.strip_prefix(VENV_PREFIX)).map(|n| n.strip_suffix("-build").unwrap_or(n)).and_then(|n| n.parse::<u32>().ok()) else { continue };
        if pid == std::process::id() || pid_alive(pid) {
            continue;
        }
        let Ok(meta) = std::fs::symlink_metadata(entry.path()) else { continue };
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            // SAFETY: getuid has no preconditions.
            if meta.uid() != unsafe { libc::getuid() } {
                continue;
            }
        }
        if meta.is_dir() && std::fs::remove_dir_all(entry.path()).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// Whether the per-session environment applies: not switched off (`FACTR_GUARD_SESSION_VENV=0`) and no
/// explicit interpreter (`FACTR_REPL_PYTHON` is the operator's choice and is never replaced).
pub fn session_venv_enabled() -> bool {
    std::env::var_os("FACTR_GUARD_SESSION_VENV").is_none_or(|v| v != "0")
        && std::env::var_os("FACTR_REPL_PYTHON").is_none_or(|v| v.is_empty())
}

/// Whether `python` already runs in a virtualenv (it is then the user's own writable environment).
fn in_virtualenv(python: &Path) -> bool {
    std::env::var_os("VIRTUAL_ENV").is_some_and(|v| !v.is_empty())
        || python.parent().and_then(Path::parent).is_some_and(|v| v.join("pyvenv.cfg").is_file())
}

/// Arguments for creating the session venv. Never `--seed` / ensurepip unless `seed` (the base
/// interpreter has no pip): the venv sees the base's site packages, and `uv pip install --python`
/// needs no pip inside the venv.
fn venv_args(uv: bool, base: &Path, dir: &Path, seed: bool) -> Vec<OsString> {
    let mut a: Vec<OsString> = if uv { vec!["venv".into(), "--system-site-packages".into(), "--python".into(), base.into()] } else { vec!["-m".into(), "venv".into(), "--system-site-packages".into()] };
    if seed && uv {
        a.insert(1, "--seed".into());
    } else if !uv && !seed {
        a.push("--without-pip".into());
    }
    a.push(dir.into());
    a
}

/// Whether the base interpreter can import pip. Spawns python once; only run on the no-uv path.
fn base_has_pip(base: &Path) -> bool {
    std::process::Command::new(base)
        .args(["-c", "import importlib.util;print(importlib.util.find_spec(\"pip\") is not None)"])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).trim() == "True")
}

/// A venv made without pip has no `bin/pip`: a bare `pip install` in bash would reach the system pip
/// and its unwritable site packages. Tiny shims route `pip`/`pip3` to the venv python's `-m pip`.
fn write_pip_shims(bin: &Path) {
    for name in ["pip", "pip3"] {
        let f = bin.join(name);
        if f.exists() {
            continue;
        }
        if std::fs::write(&f, "#!/bin/sh\nexec \"$(dirname \"$0\")/python\" -m pip \"$@\"\n").is_ok() {
            #[cfg(unix)]
            let _ = std::fs::set_permissions(&f, std::os::unix::fs::PermissionsExt::from_mode(0o755));
        }
    }
}

/// Build the session venv in `dir`: sweep dead engines' leftovers, then `uv venv` (else
/// `python3 -m venv`) with the base interpreter's site packages, in its own process group.
fn create_session_venv(gate: &VenvGate, base: &Path, uv: Option<PathBuf>, dir: &Path) -> bool {
    sweep_stale_session_venvs(&std::env::temp_dir());
    let _ = std::fs::remove_dir_all(dir);
    let mut cmd = match uv {
        Some(uv) => {
            let mut c = std::process::Command::new(&uv);
            c.args(venv_args(true, base, dir, false));
            c
        }
        None => {
            // `ensurepip` is the costly part of `python -m venv`: skip it whenever the base interpreter
            // already ships pip, which the venv then reaches through the system site packages.
            let mut c = std::process::Command::new(base);
            c.args(venv_args(false, base, dir, !base_has_pip(base)));
            c
        }
    };
    let scratch = build_scratch(dir);
    let _ = std::fs::remove_dir_all(&scratch);
    let _ = std::fs::create_dir_all(&scratch);
    cmd.env("TMPDIR", &scratch).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    let Ok(mut child) = cmd.spawn() else {
        let _ = std::fs::remove_dir_all(&scratch);
        return false;
    };
    gate.child_group.store(child.id() as i32, std::sync::atomic::Ordering::SeqCst);
    if gate.cancelled.load(std::sync::atomic::Ordering::SeqCst) {
        gate.cancel(); // cancelled between the check and the spawn: kill what was just started
    }
    let finished = child.wait().is_ok_and(|s| s.success());
    gate.child_group.store(0, std::sync::atomic::Ordering::SeqCst);
    let _ = std::fs::remove_dir_all(&scratch);
    let bin = dir.join("bin");
    if finished {
        write_pip_shims(&bin);
    }
    finished && (is_executable_file(&bin.join("python3")) || is_executable_file(&bin.join("python")))
}

/// Start building (once) the session's writable virtualenv, seeded with the base interpreter's site
/// packages, and return its `bin` directory right away; the directory is deterministic
/// (`<tmp>/factr-session-venv-<pid>`) so `PATH` can be set before it exists. The build runs on a
/// background thread; [`ensure_session_venv`] is the gate that waits for it. Reused through
/// `FACTR_SESSION_VENV` by anything this process starts. The model installs packages here (`pip`,
/// `uv pip install`) and the REPL imports from it: one environment for bash and the REPL, never the
/// system interpreter's unwritable site-packages. `None` when disabled, no `python3`, or the base is
/// already a virtualenv. If the build fails nothing is mutated (the variables stay as they were, the
/// directory is gone); readers fall back to the system interpreter ([`session_venv_active`]) and
/// spawned commands get the variables dropped per command ([`withdrawn_path`]).
fn install_session_venv(path: &OsStr) -> Option<PathBuf> {
    if !session_venv_enabled() {
        return None;
    }
    let base = python3_in(path)?;
    if in_virtualenv(&base) {
        return None;
    }
    if let Some(existing) = std::env::var_os("FACTR_SESSION_VENV").map(PathBuf::from).filter(|d| d.join("pyvenv.cfg").is_file()) {
        return Some(existing.join("bin")); // inherited from the engine that started this process
    }
    let dir = std::env::temp_dir().join(format!("{VENV_PREFIX}{}", std::process::id()));
    let uv = on_path("uv", path);
    let (build_base, build_dir) = (base.clone(), dir.clone());
    let gate = VenvGate::start(
        dir.clone(),
        // No environment mutation from the build thread: a failed build is handled by readers, see
        // [`withdrawn_path`].
        Box::new(|| {}),
        move |gate| create_session_venv(gate, &build_base, uv, &build_dir),
    );
    *GATE.write().unwrap_or_else(|e| e.into_inner()) = Some(gate);
    crate::env::set_var("FACTR_SESSION_VENV", &dir);
    // `uv pip install` with no `--python` targets this environment too.
    crate::env::set_var("VIRTUAL_ENV", &dir);
    Some(dir.join("bin"))
}

impl VenvGate {
    /// Finished and unusable: the build failed, was abandoned after the wait bound, or was shut down.
    fn withdrawn(&self) -> bool {
        match *self.phase.lock().unwrap_or_else(|e| e.into_inner()) {
            Phase::Pending => false,
            Phase::Failed => true,
            Phase::Ready => !["python3", "python"].iter().any(|n| is_executable_file(&self.dir.join("bin").join(n))),
        }
    }
}

/// `Some(PATH without the venv's `bin`)` when this process's build of the session venv is over and
/// unusable. A command the engine starts then drops `FACTR_SESSION_VENV` and `VIRTUAL_ENV` and uses
/// this `PATH` (`cmd.env_remove(..)`, `cmd.env("PATH", ..)`), instead of the engine rewriting its own
/// environment from a background thread. `None` when the venv is usable, still building, or inherited.
pub fn withdrawn_path() -> Option<OsString> {
    withdrawn_path_from(&std::env::var_os("PATH")?)
}

fn withdrawn_path_from(path: &OsStr) -> Option<OsString> {
    let gate = gate().filter(|g| g.withdrawn())?;
    let bin = gate.dir.join("bin");
    std::env::join_paths(std::env::split_paths(path).filter(|d| *d != bin)).ok()
}

/// Whether a usable session venv is what `FACTR_SESSION_VENV` points at.
pub fn session_venv_active() -> bool {
    std::env::var_os("FACTR_SESSION_VENV").is_some() && gate().is_none_or(|g| !g.withdrawn())
}

/// The session venv's interpreter when one was installed by this process.
pub fn session_venv_python() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("FACTR_SESSION_VENV")?);
    let bin = dir.join("bin");
    ["python3", "python"].into_iter().map(|n| bin.join(n)).find(|p| is_executable_file(p))
}

/// Directories the interpreter imports from (its `sys.path` after `site` ran: site-packages, the
/// user site, `.pth` additions such as editable installs), resolved to real paths, plus the virtualenv
/// root when `python` lives in one. A sandboxed worker needs read access to exactly these.
pub fn read_roots(python: &Path) -> Vec<PathBuf> {
    let code = "import sys, site, json\nprint(json.dumps([sys.path + [sys.prefix, sys.base_prefix], site.getusersitepackages() if site.ENABLE_USER_SITE else ''])) ";
    let output = std::process::Command::new(python)
        .args(["-E", "-c", code])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output();
    let (listed, user): (Vec<String>, String) = output
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| serde_json::from_slice(&o.stdout).ok())
        .unwrap_or_default();
    let mut roots: Vec<PathBuf> = listed
        .into_iter()
        .filter(|p| !p.is_empty())
        .filter_map(|p| std::fs::canonicalize(p).ok())
        .filter(|p| p.is_dir())
        .collect();
    // The user site is readable even before it exists: `pip install --user` creates it mid-session.
    if !user.is_empty() {
        roots.extend(canonical_lenient(Path::new(&user)));
    }
    if let Some(venv) = python.parent().and_then(Path::parent).filter(|v| v.join("pyvenv.cfg").is_file()) {
        roots.extend(std::fs::canonicalize(venv).ok());
    }
    roots.sort();
    roots.dedup();
    roots
}

/// `path` with its longest existing ancestor resolved to a real path and the rest appended as is.
fn canonical_lenient(path: &Path) -> Option<PathBuf> {
    let mut rest = Vec::new();
    let mut cur = path;
    loop {
        if let Ok(real) = std::fs::canonicalize(cur) {
            return Some(rest.iter().rev().fold(real, |acc, part| acc.join(part)));
        }
        rest.push(cur.file_name()?.to_owned());
        cur = cur.parent()?;
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn sweep_removes_only_dead_pid_session_venvs() {
        let tmp = tempfile::tempdir().unwrap();
        // A pid that is certainly not running: spawn and reap a child.
        let mut child = std::process::Command::new("true").spawn().unwrap();
        let dead = child.id();
        child.wait().unwrap();
        let stale = tmp.path().join(format!("{VENV_PREFIX}{dead}"));
        let stale_build = tmp.path().join(format!("{VENV_PREFIX}{dead}-build"));
        let live = tmp.path().join(format!("{VENV_PREFIX}{}", std::process::id()));
        let other = tmp.path().join("factr-session-venv-notapid");
        let unrelated = tmp.path().join("something-else");
        for d in [&stale, &stale_build, &live, &other, &unrelated] {
            std::fs::create_dir_all(d.join("bin")).unwrap();
        }
        assert_eq!(sweep_stale_session_venvs(tmp.path()), 2);
        assert!(!stale.exists() && !stale_build.exists() && live.exists() && other.exists() && unrelated.exists());
        // A live foreign pid (the parent shell/test runner) is kept too.
        let parent = tmp.path().join(format!("{VENV_PREFIX}{}", std::os::unix::process::parent_id()));
        std::fs::create_dir_all(&parent).unwrap();
        assert_eq!(sweep_stale_session_venvs(tmp.path()), 0);
        assert!(parent.exists());
    }

    fn gate_in(dir: &Path, on_fail: Box<dyn Fn() + Send + Sync>, create: impl FnOnce(&VenvGate) -> bool + Send + 'static) -> std::sync::Arc<VenvGate> {
        VenvGate::start(dir.to_path_buf(), on_fail, create)
    }

    #[test]
    fn creation_does_not_block_start_and_the_gate_waits_then_proceeds() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("v");
        let made = dir.clone();
        let t = std::time::Instant::now();
        let gate = gate_in(&dir, Box::new(|| {}), move |_| {
            std::thread::sleep(std::time::Duration::from_millis(400));
            std::fs::create_dir_all(&made).unwrap();
            true
        });
        assert!(t.elapsed() < std::time::Duration::from_millis(100), "start must return at once");
        assert!(gate.pending());
        assert!(gate.wait(std::time::Duration::from_secs(5)));
        assert!(t.elapsed() >= std::time::Duration::from_millis(400) && dir.is_dir());
        // Finished: free.
        let again = std::time::Instant::now();
        assert!(gate.wait(std::time::Duration::from_secs(5)) && again.elapsed() < std::time::Duration::from_millis(50));
    }

    #[test]
    fn a_failing_creator_releases_the_gate_promptly_and_cleans_up() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("v");
        let seen = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = seen.clone();
        let made = dir.clone();
        let gate = gate_in(&dir, Box::new(move || flag.store(true, std::sync::atomic::Ordering::SeqCst)), move |_| {
            std::fs::create_dir_all(&made).unwrap(); // a half-built directory
            false
        });
        let t = std::time::Instant::now();
        assert!(!gate.wait(std::time::Duration::from_secs(5)));
        assert!(t.elapsed() < std::time::Duration::from_secs(2));
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert!(!dir.exists() && seen.load(std::sync::atomic::Ordering::SeqCst), "half-built dir removed, variables withdrawn");
        assert!(!gate.wait(std::time::Duration::from_secs(5)), "stays failed, never hangs");
    }

    #[test]
    fn a_creator_past_the_wait_bound_is_abandoned_and_its_late_directory_removed() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("v");
        let made = dir.clone();
        let gate = gate_in(&dir, Box::new(|| {}), move |_| {
            std::thread::sleep(std::time::Duration::from_millis(600));
            std::fs::create_dir_all(&made).unwrap();
            true // finishes late and ignores the cancellation
        });
        let t = std::time::Instant::now();
        assert!(!gate.wait(std::time::Duration::from_millis(100)));
        assert!(t.elapsed() < std::time::Duration::from_millis(500), "released at the bound");
        std::thread::sleep(std::time::Duration::from_millis(900));
        assert!(!dir.exists(), "the late directory is cleaned, not left to corrupt the session");
        assert!(!gate.wait(std::time::Duration::from_secs(1)), "still without the venv");
    }

    #[cfg(unix)]
    #[test]
    fn shutdown_during_creation_kills_the_child_and_removes_the_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("v");
        let (pid_tx, pid_rx) = std::sync::mpsc::channel();
        let made = dir.clone();
        let gate = gate_in(&dir, Box::new(|| {}), move |gate| {
            std::fs::create_dir_all(made.join("bin")).unwrap();
            let mut cmd = std::process::Command::new("sh");
            cmd.args(["-c", "sleep 30 & wait"]);
            std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
            let mut child = cmd.spawn().unwrap();
            gate.child_group.store(child.id() as i32, std::sync::atomic::Ordering::SeqCst);
            pid_tx.send(child.id()).unwrap();
            let _ = child.wait();
            false
        });
        let pid = pid_rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        assert!(pid_alive(pid));
        let t = std::time::Instant::now();
        gate.shutdown();
        assert!(t.elapsed() < std::time::Duration::from_secs(1), "shutdown is not held up");
        assert!(!dir.exists());
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert!(!pid_alive(pid), "the creator's child (and its group) is gone and reaped");
        // The group's own grandchild is gone too.
        let left = std::process::Command::new("pgrep").args(["-g", &pid.to_string()]).output().map(|o| o.stdout.len()).unwrap_or(0);
        assert_eq!(left, 0, "nothing left in the creator's process group");
    }

    /// Serialises the tests that swap the process-wide gate.
    static GATE_TESTS: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn a_failed_or_timed_out_build_withdraws_the_venv_per_command_without_touching_the_environment() {
        let _serial = GATE_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("v");
        let path = std::env::join_paths([dir.join("bin"), PathBuf::from("/usr/bin")]).unwrap();
        let watched = ["FACTR_SESSION_VENV", "VIRTUAL_ENV"];
        let env_before: Vec<_> = watched.iter().map(std::env::var_os).collect();

        // Failing creator.
        let made = dir.clone();
        set_gate_for_test(Some(gate_in(&dir, Box::new(|| {}), move |_| {
            std::fs::create_dir_all(made.join("bin")).unwrap();
            false
        })));
        assert!(!ensure_session_venv(std::time::Duration::from_secs(5)));
        let stripped = withdrawn_path_from(&path).expect("failed build is withdrawn");
        assert_eq!(std::env::split_paths(&stripped).collect::<Vec<_>>(), vec![PathBuf::from("/usr/bin")]);
        assert!(!session_venv_active());

        // Creator past the wait bound.
        let made = dir.clone();
        set_gate_for_test(Some(gate_in(&dir, Box::new(|| {}), move |_| {
            std::thread::sleep(std::time::Duration::from_millis(500));
            std::fs::create_dir_all(&made).unwrap();
            true
        })));
        assert_eq!(withdrawn_path_from(&path), None, "still building: not withdrawn");
        assert!(!ensure_session_venv(std::time::Duration::from_millis(50)));
        assert!(withdrawn_path_from(&path).is_some());

        // A usable build is never withdrawn.
        let made = dir.clone();
        set_gate_for_test(Some(gate_in(&dir, Box::new(|| {}), move |_| {
            std::fs::create_dir_all(made.join("bin")).unwrap();
            std::fs::write(made.join("bin/python3"), "").unwrap();
            std::fs::set_permissions(made.join("bin/python3"), std::fs::Permissions::from_mode(0o755)).unwrap();
            true
        })));
        assert!(ensure_session_venv(std::time::Duration::from_secs(5)));
        assert_eq!(withdrawn_path_from(&path), None);
        set_gate_for_test(None);
        std::thread::sleep(std::time::Duration::from_millis(600));
        assert_eq!(watched.iter().map(std::env::var_os).collect::<Vec<_>>(), env_before, "no variable was set or removed");
    }

    #[test]
    fn canonical_lenient_resolves_the_existing_ancestor_and_keeps_the_rest() {
        let d = tempfile::tempdir().unwrap();
        let real = std::fs::canonicalize(d.path()).unwrap();
        assert_eq!(canonical_lenient(&d.path().join("a/b/c")), Some(real.join("a/b/c")));
        assert_eq!(canonical_lenient(d.path()), Some(real));
    }

    #[test]
    fn venv_command_lines_skip_pip_seeding_unless_the_base_has_none() {
        let (b, d) = (Path::new("/b/python3"), Path::new("/t/v"));
        let v = |uv, seed| venv_args(uv, b, d, seed).iter().map(|a| a.to_string_lossy().into_owned()).collect::<Vec<_>>().join(" ");
        assert_eq!(v(true, false), "venv --system-site-packages --python /b/python3 /t/v");
        assert_eq!(v(true, true), "venv --seed --system-site-packages --python /b/python3 /t/v");
        assert_eq!(v(false, false), "-m venv --system-site-packages --without-pip /t/v");
        assert_eq!(v(false, true), "-m venv --system-site-packages /t/v");
    }

    #[test]
    fn pip_detection_reads_the_base_interpreters_answer() {
        let tmp = tempfile::tempdir().unwrap();
        let say = |name: &str, out: &str| {
            let f = tmp.path().join(name);
            std::fs::write(&f, format!("#!/bin/sh\necho {out}\n")).unwrap();
            std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o755)).unwrap();
            f
        };
        assert!(base_has_pip(&say("yes", "True")));
        assert!(!base_has_pip(&say("no", "False")));
        assert!(!base_has_pip(&tmp.path().join("missing")));
    }

    #[test]
    fn pip_shims_route_to_the_venv_python_and_never_overwrite() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("pip3"), "real").unwrap();
        write_pip_shims(tmp.path());
        assert!(std::fs::read_to_string(tmp.path().join("pip")).unwrap().contains("-m pip"));
        assert_eq!(std::fs::read_to_string(tmp.path().join("pip3")).unwrap(), "real");
        assert!(std::fs::metadata(tmp.path().join("pip")).unwrap().permissions().mode() & 0o111 != 0);
    }

    fn exe(dir: &Path, name: &str) -> PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let file = dir.join(name);
        std::fs::write(&file, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
        file
    }

    #[test]
    fn explicit_wins_then_first_executable_python3_on_path() {
        let tmp = tempfile::tempdir().unwrap();
        let (a, b) = (tmp.path().join("a"), tmp.path().join("b"));
        std::fs::create_dir_all(&a).unwrap();
        std::fs::write(a.join("python3"), "").unwrap(); // not executable: skipped
        let real = exe(&b, "python3");
        let path = std::env::join_paths([&a, &b]).unwrap();
        assert_eq!(interpreter_from(None, Some(path.clone())), Some(real));
        assert_eq!(interpreter_from(Some("/x/py".into()), Some(path)), Some(PathBuf::from("/x/py")));
        assert_eq!(interpreter_from(None, Some(tmp.path().join("none").into())), None);
    }

    #[test]
    fn python_shim_only_when_python_is_missing_and_never_shadows_python3() {
        let tmp = tempfile::tempdir().unwrap();
        let bin = tmp.path().join("bin");
        let py3 = exe(&bin, "python3");
        let path: OsString = bin.clone().into();
        let home = tmp.path().join("home");
        let dir = python_shim_dir(&home, &py3, &path).expect("shim made");
        assert_eq!(dir, home.join("bin"));
        let launcher = std::fs::read_to_string(dir.join("python")).unwrap();
        assert!(launcher.contains(&format!("exec '{}'", py3.display())), "{launcher}");
        assert!(is_executable_file(&dir.join("python")));
        assert!(!dir.join("python3").exists());
        // idempotent
        assert_eq!(python_shim_dir(&home, &py3, &path), Some(dir));
        // a python already on PATH: nothing to add
        exe(&bin, "python");
        assert_eq!(python_shim_dir(&tmp.path().join("home2"), &py3, &path), None);
    }
}
