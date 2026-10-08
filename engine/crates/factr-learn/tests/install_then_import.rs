//! A package installed by a subprocess (what `bash` does) must be importable from the persistent
//! REPL worker without restarting it. The package is a tiny wheel built locally, installed offline.

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

const WORKER: &str = include_str!("../src/python_worker.py");
mod common;
use common::{PKG, build_wheel};

fn base_python() -> Option<PathBuf> {
    factr_base::python_env::interpreter_from(None, std::env::var_os("PATH"))
}

/// A persistent worker as the host starts it (`-E -u`, no `-S`), with `HOME` pointed at `home`.
struct Worker {
    child: Child,
    stdin: ChildStdin,
    out: BufReader<ChildStdout>,
}

impl Worker {
    fn start(python: &Path, home: &Path) -> Self {
        std::fs::create_dir_all(home).unwrap();
        let mut child = Command::new(python)
            .args(["-E", "-u", "-c", WORKER])
            .arg(home)
            .arg(home.join("skills"))
            .env_clear()
            .env("HOME", home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let mut out = BufReader::new(child.stdout.take().unwrap());
        let mut line = String::new();
        out.read_line(&mut line).unwrap();
        assert!(line.contains("ready"), "{line}");
        Self { child, stdin, out }
    }

    fn cell(&mut self, code: &str) -> Value {
        writeln!(self.stdin, "{}", json!({"op": "run", "code": code})).unwrap();
        let mut line = String::new();
        self.out.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    }

    fn imports(&mut self) -> bool {
        let done = self.cell(&format!("import {PKG}\n{PKG}.MARK"));
        done["error"].is_null() && done["value"] == "'probe-ok'"
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

fn make_venv(base: &Path, dir: &Path, system_site: bool) -> Option<PathBuf> {
    let mut cmd = Command::new(base);
    cmd.args(["-m", "venv"]);
    if system_site {
        cmd.arg("--system-site-packages");
    }
    let ok = cmd.arg(dir).stdout(Stdio::null()).stderr(Stdio::null()).status().ok()?.success();
    let py = dir.join("bin/python");
    (ok && py.is_file()).then_some(py)
}

fn pip_install(python: &Path, home: &Path, extra: &[&str], wheel: &Path) -> bool {
    Command::new(python)
        .args(["-m", "pip", "install", "--no-index", "--disable-pip-version-check", "-q"])
        .args(extra)
        .arg(wheel)
        .env("HOME", home)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

#[test]
fn pip_into_the_repl_venv_is_importable_without_restart() {
    let Some(base) = base_python() else { return eprintln!("skipped: no python3") };
    let root = tempfile::tempdir().unwrap();
    let wheel = build_wheel(&base, &root.path().join("wheel"));
    let Some(venv) = make_venv(&base, &root.path().join("venv"), true) else { return eprintln!("skipped: no venv support") };
    let home = root.path().join("home");
    let mut worker = Worker::start(&venv, &home);
    assert!(!worker.imports(), "not installed yet");
    if !pip_install(&venv, &home, &[], &wheel) {
        return eprintln!("skipped: no pip in the venv");
    }
    assert!(worker.imports(), "venv install must be importable without restarting the worker");
}

#[test]
fn uv_pip_into_the_repl_venv_is_importable_without_restart() {
    let Some(base) = base_python() else { return eprintln!("skipped: no python3") };
    let root = tempfile::tempdir().unwrap();
    let wheel = build_wheel(&base, &root.path().join("wheel"));
    let Some(venv) = make_venv(&base, &root.path().join("venv"), true) else { return eprintln!("skipped: no venv support") };
    let mut worker = Worker::start(&venv, &root.path().join("home"));
    assert!(!worker.imports());
    let ran = Command::new("uv").args(["pip", "install", "--offline", "--no-index", "--python"]).arg(&venv).arg(&wheel).output();
    if !ran.is_ok_and(|o| o.status.success()) {
        return eprintln!("skipped: no uv");
    }
    assert!(worker.imports());
}

#[test]
fn pip_user_into_a_directory_created_after_start_is_importable() {
    let Some(base) = base_python() else { return eprintln!("skipped: no python3") };
    let root = tempfile::tempdir().unwrap();
    let wheel = build_wheel(&base, &root.path().join("wheel"));
    let home = root.path().join("home");
    let mut worker = Worker::start(&base, &home);
    assert!(!worker.imports());
    if !pip_install(&base, &home, &["--user"], &wheel) {
        return eprintln!("skipped: no pip --user");
    }
    assert!(worker.imports(), "a user-site directory created after start must be picked up");
}

/// An install into a venv the worker is NOT running on stays invisible; the missing-module hint
/// names the REPL's own interpreter for that reason.
#[test]
fn install_into_a_foreign_venv_is_not_importable() {
    let Some(base) = base_python() else { return eprintln!("skipped: no python3") };
    let root = tempfile::tempdir().unwrap();
    let wheel = build_wheel(&base, &root.path().join("wheel"));
    let Some(other) = make_venv(&base, &root.path().join("other"), false) else { return eprintln!("skipped: no venv support") };
    let home = root.path().join("home");
    let mut worker = Worker::start(&base, &home);
    if !pip_install(&other, &home, &[], &wheel) {
        return eprintln!("skipped: no pip in the venv");
    }
    assert!(!worker.imports());
}
