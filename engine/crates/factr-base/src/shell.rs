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

/// The actionable tail for a "module not found" error: the installers that exist, never run for the
/// model, then the standard-library fallback.
pub fn missing_module_hint(routes: &[&str], interpreter: Option<&str>) -> String {
    let py = interpreter.unwrap_or("python3");
    let mut ways = Vec::new();
    if routes.contains(&"uv") {
        ways.push(format!("`uv pip install --python {py} <pkg>`"));
    }
    if routes.contains(&"pip") || routes.contains(&"pip3") {
        ways.push(format!("`{py} -m pip install <pkg>`"));
    }
    let via = if ways.is_empty() { "No installer is on PATH; ".to_string() } else { format!("Install it with {}, or ", ways.join(" or ")) };
    format!("\nHint: {via}implement it with the standard library before declaring it impossible.")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_module_hint_names_only_existing_routes() {
        let h = missing_module_hint(&["uv", "pip"], Some("/x/python3"));
        assert!(h.contains("uv pip install --python /x/python3") && h.contains("-m pip install") && h.contains("standard library"));
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
