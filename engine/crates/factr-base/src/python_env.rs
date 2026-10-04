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
    let Some(python) = interpreter_from(std::env::var_os("FACTR_REPL_PYTHON"), Some(path.clone())) else { return };
    if let Some(dir) = python_shim_dir(home, &python, &path) {
        let mut dirs = vec![dir];
        dirs.extend(std::env::split_paths(&path));
        if let Ok(joined) = std::env::join_paths(dirs) {
            crate::env::set_var("PATH", joined);
        }
    }
}

/// Directories the interpreter imports from (its `sys.path` after `site` ran: site-packages, the
/// user site, `.pth` additions such as editable installs), resolved to real paths, plus the virtualenv
/// root when `python` lives in one. A sandboxed worker needs read access to exactly these.
pub fn read_roots(python: &Path) -> Vec<PathBuf> {
    let code = "import sys, site, json\nprint(json.dumps(sys.path + [site.getusersitepackages(), sys.prefix, sys.base_prefix]))";
    let output = std::process::Command::new(python)
        .args(["-E", "-c", code])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output();
    let listed: Vec<String> = output
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
    if let Some(venv) = python.parent().and_then(Path::parent).filter(|v| v.join("pyvenv.cfg").is_file()) {
        roots.extend(std::fs::canonicalize(venv).ok());
    }
    roots.sort();
    roots.dedup();
    roots
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

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
