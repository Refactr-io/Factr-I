//! One Python environment: the REPL and `bash` import the same packages from the same interpreter.

use factr_learn::host::{ExtraHostFns, Refine};
use factr_learn::{LlmQuery, ReplHost};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

const MEMORY_LIMIT: u64 = 2 << 30;

fn llm() -> LlmQuery {
    Arc::new(|prompt: String| Box::pin(async move { Ok(prompt) }))
}

fn no_refine() -> Refine {
    Arc::new(|_op: String| Box::pin(async move { Ok(r#"{"scheduled":false}"#.to_string()) }))
}

/// A venv under `root` holding a stub package and no `python` (a user with just `python3` on PATH). `None` (skip) when there is no python3 to build it.
fn stub_environment(root: &Path) -> Option<(PathBuf, OsString)> {
    let base = factr_base::python_env::interpreter_from(None, std::env::var_os("PATH"))?;
    let venv = root.join("venv");
    let made = Command::new(&base).args(["-m", "venv", "--without-pip"]).arg(&venv).status().ok()?;
    if !made.success() {
        return None;
    }
    let python = venv.join("bin/python3");
    let site = Command::new(&python)
        .args(["-c", "import site; print(site.getsitepackages()[0])"])
        .output()
        .ok()?;
    let site = PathBuf::from(String::from_utf8(site.stdout).ok()?.trim());
    std::fs::create_dir_all(site.join("stubpkg")).unwrap();
    std::fs::write(site.join("stubpkg/__init__.py"), "MARK = 'stub-ok'\n").unwrap();
    // An activated venv that has only `python3` (no `python`), as on a distro that ships no `python`.
    std::fs::remove_file(venv.join("bin/python")).ok()?;
    let only = venv.join("bin");
    Some((python, only.into()))
}

#[tokio::test(flavor = "multi_thread")]
async fn repl_and_bash_share_one_environment_and_python_resolves() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("factr-home");
    std::fs::create_dir_all(&home).unwrap();
    // Skills are staged under $FACTR_HOME: never the developer's real ~/.factr/engine.
    factr_base::env::set_var("FACTR_HOME", &home);
    let Some((python3, only)) = stub_environment(root.path()) else {
        eprintln!("skipped: no python3 with venv support");
        return;
    };

    // The resolver finds that python3; a `python` shim appears because only python3 exists.
    let interpreter = factr_base::python_env::interpreter_from(None, Some(only.clone())).unwrap();
    assert_eq!(interpreter, python3);
    let shim = factr_base::python_env::python_shim_dir(&home, &interpreter, &only).expect("shim");
    let path = std::env::join_paths([shim, PathBuf::from(&only)]).unwrap();

    // bash: `python` and `python3` both import the stub package.
    for name in ["python", "python3"] {
        let out = Command::new("/bin/sh")
            .args(["-c", &format!("{name} -c 'import stubpkg; print(stubpkg.MARK)'")])
            .env("PATH", &path)
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "stub-ok", "{name}: {}", String::from_utf8_lossy(&out.stderr));
    }

    // The REPL, on the same resolved interpreter, imports it too (sandboxed on macOS).
    let host = ReplHost::new(interpreter);
    let work = root.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    let out = host
        .run("s", "import stubpkg\nstubpkg.MARK", Some(&work), llm(), no_refine(), ExtraHostFns::default(), MEMORY_LIMIT)
        .await
        .unwrap();
    assert_eq!(out.value.as_deref(), Some("'stub-ok'"), "{:?}", out.error);
    // The working directory does not shadow installed packages or the standard library.
    std::fs::write(work.join("stubpkg.py"), "MARK = 'shadowed'\n").unwrap();
    let out = host
        .run("t", "import stubpkg\nstubpkg.MARK", Some(&work), llm(), no_refine(), ExtraHostFns::default(), MEMORY_LIMIT)
        .await
        .unwrap();
    assert_eq!(out.value.as_deref(), Some("'stub-ok'"), "{:?}", out.error);
}
