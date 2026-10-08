//! The per-session environment: made once, reused, bypassed by an explicit interpreter or the guard
//! switch, and what the model installs into it is importable in the (sandboxed) REPL without a restart.

use factr_learn::host::{ExtraHostFns, Refine};
use factr_learn::{LlmQuery, ReplHost};
use std::process::Command;
use std::sync::Arc;

mod common;
use common::{PKG, build_wheel};

fn llm() -> LlmQuery {
    Arc::new(|prompt: String| Box::pin(async move { Ok(prompt) }))
}

fn no_refine() -> Refine {
    Arc::new(|_op: String| Box::pin(async move { Ok(r#"{"scheduled":false}"#.to_string()) }))
}

#[tokio::test(flavor = "multi_thread")]
async fn session_venv_is_made_once_bypassable_and_installs_reach_the_repl() {
    use factr_base::python_env as pe;
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let path = std::env::var_os("PATH").unwrap();
    let Some(base) = pe::interpreter_from(None, Some(path.clone())) else { return eprintln!("skipped: no python3") };
    for key in ["FACTR_REPL_PYTHON", "FACTR_GUARD_SESSION_VENV", "FACTR_SESSION_VENV", "VIRTUAL_ENV"] {
        factr_base::env::remove_var(key);
    }
    factr_base::env::set_var("FACTR_HOME", &home);
    // The test must start from a non-virtualenv base, or nothing is made by design.
    if base.parent().and_then(|p| p.parent()).is_some_and(|v| v.join("pyvenv.cfg").is_file()) {
        return eprintln!("skipped: python3 on PATH is already a virtualenv");
    }

    // Guard switch off, and an explicit interpreter: nothing is created.
    factr_base::env::set_var("FACTR_GUARD_SESSION_VENV", "0");
    pe::install_python_shim(&home);
    assert!(std::env::var_os("FACTR_SESSION_VENV").is_none() && pe::session_venv_python().is_none());
    factr_base::env::remove_var("FACTR_GUARD_SESSION_VENV");
    factr_base::env::set_var("FACTR_REPL_PYTHON", &base);
    pe::install_python_shim(&home);
    assert!(std::env::var_os("FACTR_SESSION_VENV").is_none(), "an explicit FACTR_REPL_PYTHON is never replaced");
    assert_eq!(pe::interpreter(), Some(base.clone()));
    factr_base::env::remove_var("FACTR_REPL_PYTHON");
    factr_base::env::set_var("PATH", &path);

    // Default: one venv, which becomes the interpreter of bash, the REPL and the environment line.
    pe::install_python_shim(&home);
    assert!(pe::ensure_session_venv(pe::VENV_WAIT), "the background build finishes");
    let venv_python = pe::session_venv_python().expect("session venv made");
    let venv = std::path::PathBuf::from(std::env::var_os("FACTR_SESSION_VENV").unwrap());
    assert_eq!(pe::interpreter(), Some(venv.join("bin/python3")));
    let stamp = std::fs::metadata(venv.join("pyvenv.cfg")).unwrap().modified().unwrap();
    // Reused, not rebuilt, when the engine asks again (e.g. a child process inheriting the variable).
    factr_base::env::set_var("PATH", &path);
    pe::install_python_shim(&home);
    assert_eq!(std::env::var_os("FACTR_SESSION_VENV").map(std::path::PathBuf::from), Some(venv.clone()));
    assert_eq!(std::fs::metadata(venv.join("pyvenv.cfg")).unwrap().modified().unwrap(), stamp);

    // The hint names exactly this interpreter.
    let hint = factr_base::shell::missing_module_hint(&["uv", "pip"], venv_python.to_str());
    assert!(hint.contains(&format!("--python {}", venv_python.display())), "{hint}");

    // REPL (sandboxed on macOS): not installed, install through the venv's pip, importable in the same worker.
    let host = ReplHost::new(venv_python.clone());
    let work = root.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    let cell = format!("import {PKG}\n{PKG}.MARK");
    let before = host.run("s", &cell, Some(&work), llm(), no_refine(), ExtraHostFns::default(), 2 << 30).await.unwrap();
    assert!(before.error.as_deref().is_some_and(|e| e.contains("No module named")), "{:?}", before.error);
    let wheel = build_wheel(&base, &root.path().join("wheel"));
    let installed = Command::new(&venv_python)
        .args(["-m", "pip", "install", "--no-index", "--disable-pip-version-check", "-q"])
        .arg(&wheel)
        .output()
        .unwrap();
    assert!(installed.status.success(), "{}", String::from_utf8_lossy(&installed.stderr));
    let after = host.run("s", &cell, Some(&work), llm(), no_refine(), ExtraHostFns::default(), 2 << 30).await.unwrap();
    assert_eq!(after.value.as_deref(), Some("'probe-ok'"), "{:?}", after.error);
    assert_eq!(host.live_workers().await, 1, "same worker, no restart");
    let _ = std::fs::remove_dir_all(&venv);
}
