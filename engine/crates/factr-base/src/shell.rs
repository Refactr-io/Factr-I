//! The ONE POSIX shell of a session: `bash` when it is on `PATH`, else `/bin/sh`.
//!
//! The `bash` tool, background tasks and the auto-verify gate all start commands through this, so a
//! host without bash (a minimal image) still runs them instead of failing to spawn.

use std::ffi::OsStr;
use std::sync::OnceLock;

/// `bash` if an executable `bash` is in `path`, else `/bin/sh`.
pub fn pick(path: &OsStr) -> &'static str {
    let found = std::env::split_paths(path).any(|d| {
        let p = d.join("bash");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            p.metadata().is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        }
        #[cfg(not(unix))]
        {
            p.is_file()
        }
    });
    if found { "bash" } else { "/bin/sh" }
}

/// The shell for this process, resolved once from the process `PATH`.
pub fn posix_shell() -> &'static str {
    static SHELL: OnceLock<&'static str> = OnceLock::new();
    SHELL.get_or_init(|| pick(&std::env::var_os("PATH").unwrap_or_default()))
}

/// Whether the resolved shell is bash (login-shell flags and bashisms are safe).
pub fn is_bash() -> bool {
    posix_shell() == "bash"
}

/// Package installers found on `PATH` (probed once): the routes for a missing library.
pub fn package_routes() -> &'static [&'static str] {
    static ROUTES: OnceLock<Vec<&'static str>> = OnceLock::new();
    ROUTES.get_or_init(|| {
        let path = std::env::var_os("PATH").unwrap_or_default();
        ["uv", "pip", "pip3", "npm", "pnpm", "yarn", "apt-get", "brew"]
            .into_iter()
            .filter(|n| std::env::split_paths(&path).any(|d| d.join(n).is_file()))
            .collect()
    })
}

/// The one install command that works first try for `interpreter` (the session environment's python
/// when there is one): `uv pip install --python <py> <pkg>` when uv exists, else `<py> -m pip install <pkg>`.
pub fn install_command(routes: &[&str], interpreter: Option<&str>) -> Option<String> {
    let py = interpreter.unwrap_or("python3");
    if routes.contains(&"uv") {
        Some(format!("uv pip install --python {py} <pkg>"))
    } else if routes.contains(&"pip") || routes.contains(&"pip3") || interpreter.is_some() {
        Some(format!("{py} -m pip install <pkg>"))
    } else {
        None
    }
}

/// The actionable tail for a "module not found" error: exactly one installer command, never run for
/// the model (a package installed that way is importable in the REPL on its next cell, no restart),
/// then the standard-library fallback.
pub fn missing_module_hint(routes: &[&str], interpreter: Option<&str>) -> String {
    match install_command(routes, interpreter).filter(|_| !routes.is_empty()) {
        Some(cmd) => format!("\nHint: install it with `{cmd}` (importable in the REPL on its next cell, no restart), or implement it with the standard library before declaring it impossible."),
        None => "\nHint: No installer is on PATH; implement it with the standard library before declaring it impossible.".to_string(),
    }
}

/// A one-line tail for a failed `pip`/`uv` install whose cause is structural (unwritable prefix,
/// externally-managed interpreter, read-only filesystem): use the session environment's command, not
/// `--break-system-packages`, `--target` or a different prefix. Empty when the output shows no such cause.
pub fn install_failure_hint(command: &str, output: &str, routes: &[&str], interpreter: Option<&str>) -> String {
    let installs = command.contains("pip install") || command.contains("pip3 install") || command.contains("uv pip") || command.contains("uv run --with");
    let structural = ["externally-managed-environment", "Permission denied", "Read-only file system", "not writable", "EACCES"]
        .iter()
        .any(|m| output.contains(m));
    if !installs || !structural {
        return String::new();
    }
    match (crate::python_env::session_venv_active(), install_command(routes, interpreter)) {
        (true, Some(cmd)) => format!("\nHint: that prefix is not writable; install into the session environment with `{cmd}` instead of --break-system-packages or --target."),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_module_hint_names_only_existing_routes() {
        let h = missing_module_hint(&["uv", "pip"], Some("/x/python3"));
        assert!(h.contains("uv pip install --python /x/python3") && !h.contains("-m pip install") && h.contains("standard library"));
        let p = missing_module_hint(&["pip"], Some("/x/python3"));
        assert!(p.contains("`/x/python3 -m pip install <pkg>`") && !p.contains("uv pip"));
        let none = missing_module_hint(&[], None);
        assert!(!none.contains("pip install") && none.contains("No installer") && none.contains("standard library"));
    }

    #[test]
    fn falls_back_to_sh_when_path_has_no_bash() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(pick(d.path().as_os_str()), "/bin/sh");
        assert_eq!(pick(OsStr::new("")), "/bin/sh");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let f = d.path().join("bash");
            std::fs::write(&f, "#!/bin/sh\n").unwrap();
            assert_eq!(pick(d.path().as_os_str()), "/bin/sh", "not executable");
            std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o755)).unwrap();
            assert_eq!(pick(d.path().as_os_str()), "bash");
        }
    }
}
