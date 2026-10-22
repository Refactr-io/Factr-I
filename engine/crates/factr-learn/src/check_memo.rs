//! The check an unattended run names, remembered per session (flags default off).
//!
//! Persistence makes the engine run the task's check after every turn. When the model has just run the same
//! check on byte-identical sources, running it again says nothing new, so the result is reused
//! (`FACTR_PERSIST_REUSE=1`). With `FACTR_PERSIST_AUTOCHECK=1` the engine also runs the check right after every
//! file change and appends the result to that change's output: one action writes and checks, the model needs no
//! separate check call and no text-only round to hear the verdict. Sources are compared by content (the check's
//! own outputs and caches do not count).

use crate::agent_loop::run_command;
use crate::goal_ratchet::report_failed;
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

const FINGERPRINT_FILES: usize = 400;
const FINGERPRINT_FILE_BYTES: u64 = 512 * 1024;
const SOURCE_EXTS: [&str; 40] = [
    "rs", "py", "js", "jsx", "ts", "tsx", "mjs", "cjs", "go", "java", "kt", "swift", "c", "h", "cc", "cpp", "hpp", "cs", "rb", "php", "scala", "sh",
    "lua", "dart", "zig", "ex", "cu", "cuh", "cl", "metal", "wgsl", "glsl", "hlsl", "jl", "r", "sql", "toml", "yaml", "yml", "cfg",
];
const SKIP_DIRS: [&str; 9] = [".git", "node_modules", "target", "venv", ".venv", "__pycache__", ".tox", ".pytest_cache", "build"];
/// How much of a check's output a remembered result or an appended note keeps.
const OUTPUT_TAIL: usize = 3000;
const NOTE_TAIL: usize = 800;

struct Memo {
    gate: String,
    fingerprint: u64,
    passed: bool,
    exit_code: i64,
    output: String,
}

struct Entry {
    gates: Vec<(String, u64)>,
    cwd: Option<PathBuf>,
    memos: Vec<Memo>,
}

static REGISTRY: LazyLock<Mutex<HashMap<String, Entry>>> = LazyLock::new(Default::default);

/// `FACTR_PERSIST_REUSE=1` (or AUTOCHECK, which implies it): the gate reuses a result for unchanged sources.
pub fn reuse_enabled() -> bool {
    factr_base::factr_config::env_bool("FACTR_PERSIST_REUSE").unwrap_or(false) || autocheck_enabled()
}

#[cfg(test)]
thread_local! {
    /// Unit tests switch the flag per thread: the environment is process-wide and the tests run in parallel.
    static TEST_AUTOCHECK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// `FACTR_PERSIST_AUTOCHECK=1`: the engine runs the check after every file change.
pub fn autocheck_enabled() -> bool {
    #[cfg(test)]
    if TEST_AUTOCHECK.with(std::cell::Cell::get) {
        return true;
    }
    factr_base::factr_config::env_bool("FACTR_PERSIST_AUTOCHECK").unwrap_or(false)
}

/// A persisting run's gate commands (each with its timeout in seconds) and working directory.
pub fn register(session: &str, gates: &[(String, u64)], cwd: Option<&Path>) {
    REGISTRY.lock().unwrap_or_else(|e| e.into_inner()).insert(session.to_string(), Entry { gates: gates.to_vec(), cwd: cwd.map(Path::to_path_buf), memos: Vec::new() });
}

pub fn forget(session: &str) {
    REGISTRY.lock().unwrap_or_else(|e| e.into_inner()).remove(session);
}

/// Hash of the contents of the source files under `dir` (depth <= 3, bounded): what the model could have changed.
pub fn source_fingerprint(dir: &Path) -> u64 {
    let mut files = Vec::new();
    let mut stack = vec![(dir.to_path_buf(), 0u8)];
    while let Some((d, depth)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let (path, Ok(meta)) = (e.path(), e.metadata()) else { continue };
            let name = e.file_name().to_string_lossy().into_owned();
            if meta.is_dir() {
                if depth < 3 && !SKIP_DIRS.contains(&name.as_str()) {
                    stack.push((path, depth + 1));
                }
            } else if meta.len() <= FINGERPRINT_FILE_BYTES && path.extension().and_then(|x| x.to_str()).is_some_and(|x| SOURCE_EXTS.contains(&x.to_ascii_lowercase().as_str())) {
                files.push(path);
            }
        }
    }
    files.sort();
    files.truncate(FINGERPRINT_FILES);
    let mut h = DefaultHasher::new();
    for f in files {
        f.hash(&mut h);
        std::fs::read(&f).unwrap_or_default().hash(&mut h);
    }
    h.finish()
}

/// Whether the shell line `line` runs `command`: some `&&`, `;` or `|` segment starts with its words
/// (`cd w && ./eval` runs `./eval`, `./evaluate` does not).
pub fn line_runs(line: &str, command: &str) -> bool {
    let named: Vec<&str> = command.split_whitespace().collect();
    !named.is_empty()
        && line.split("&&").flat_map(|s| s.split(['|', ';'])).any(|segment| {
            let words: Vec<&str> = segment.split_whitespace().collect();
            words.len() >= named.len() && words[..named.len()] == named[..]
        })
}

/// The exit status a bash tool result reports (`Exit code: N` on the last line), 0 when it reports none.
fn exit_code_of(result: &str) -> i64 {
    result.lines().rev().find(|l| !l.trim().is_empty()).and_then(|l| l.trim().trim_end_matches(" ---").rsplit_once("xit code: ")).and_then(|(_, c)| c.trim().parse().ok()).unwrap_or(0)
}

fn tail(text: &str, chars: usize) -> String {
    text.chars().rev().take(chars).collect::<Vec<_>>().into_iter().rev().collect()
}

fn remember(session: &str, gate: &str, passed: bool, exit_code: i64, output: &str) {
    let mut reg = REGISTRY.lock().unwrap_or_else(|e| e.into_inner());
    let Some(entry) = reg.get_mut(session) else { return };
    let fingerprint = entry.cwd.as_deref().map_or(0, source_fingerprint);
    entry.memos.retain(|m| m.gate != gate);
    entry.memos.push(Memo { gate: gate.to_string(), fingerprint, passed, exit_code, output: tail(output, OUTPUT_TAIL) });
}

/// The model ran a shell command: when it ran a registered check, remember the verdict for these sources.
pub fn note_observed(session: &str, command: &str, result: &str) {
    if !reuse_enabled() {
        return;
    }
    let gates: Vec<String> = match REGISTRY.lock().unwrap_or_else(|e| e.into_inner()).get(session) {
        Some(entry) => entry.gates.iter().map(|(g, _)| g.clone()).filter(|g| line_runs(command, g)).collect(),
        None => return,
    };
    let exit = exit_code_of(result);
    for gate in gates {
        remember(session, &gate, exit == 0 && !report_failed(result), exit, result);
    }
}

/// The remembered verdict `(passed, exit code, output)` of `gate`, when the sources have not changed since.
pub fn recall(session: &str, gate: &str) -> Option<(bool, i64, String)> {
    if !reuse_enabled() {
        return None;
    }
    let reg = REGISTRY.lock().unwrap_or_else(|e| e.into_inner());
    let entry = reg.get(session)?;
    let memo = entry.memos.iter().find(|m| m.gate == gate)?;
    (entry.cwd.as_deref().map_or(0, source_fingerprint) == memo.fingerprint).then(|| (memo.passed, memo.exit_code, memo.output.clone()))
}

/// After a file change: run the session's checks (unless already known for these sources) and return the note to
/// append to the change's output. `None` when the session has no checks or the flag is off.
pub fn autocheck_note(session: &str) -> Option<String> {
    if !autocheck_enabled() {
        return None;
    }
    let (gates, cwd) = {
        let reg = REGISTRY.lock().unwrap_or_else(|e| e.into_inner());
        let entry = reg.get(session)?;
        (entry.gates.clone(), entry.cwd.clone())
    };
    let mut notes = Vec::new();
    for (gate, timeout_secs) in gates {
        let (passed, exit_code, output) = match recall(session, &gate) {
            Some(known) => known,
            None => {
                let r = run_command(&gate, cwd.as_deref(), Duration::from_secs(timeout_secs.max(1)));
                let failing_report = report_failed(&r.output);
                let passed = r.passed && !failing_report;
                remember(session, &gate, passed, r.exit_code, &r.output);
                (passed, r.exit_code, r.output)
            }
        };
        notes.push(if passed {
            format!("[Host check `{gate}` after this change: PASSED]")
        } else {
            format!("[Host check `{gate}` after this change: FAILED (exit {exit_code})]\n{}", tail(&output, NOTE_TAIL))
        });
        if !passed {
            break;
        }
    }
    Some(notes.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_command_line_runs_a_check_word_for_word() {
        assert!(line_runs("./eval", "./eval"));
        assert!(line_runs("cd w && ./eval --fast", "./eval"));
        assert!(line_runs("python3 x.py | tee out", "python3 x.py"));
        assert!(!line_runs("./evaluate", "./eval"));
        assert!(!line_runs("cat ./eval", "./eval"));
    }

    #[test]
    fn the_exit_code_comes_from_the_last_line() {
        assert_eq!(exit_code_of("out\n\nExit code: 3"), 3);
        assert_eq!(exit_code_of("out\nExit code: 0"), 0);
        assert_eq!(exit_code_of("no code"), 0);
    }

    #[test]
    fn the_fingerprint_follows_source_contents_not_outputs() {
        let d = std::env::temp_dir().join(format!("memo-fp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("k.py"), "a").unwrap();
        let before = source_fingerprint(&d);
        std::fs::write(d.join("results.json"), "x").unwrap();
        assert_eq!(source_fingerprint(&d), before);
        std::fs::write(d.join("k.py"), "b").unwrap();
        assert_ne!(source_fingerprint(&d), before);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_remembered_verdict_is_reused_only_for_unchanged_sources_and_autocheck_runs_the_check() {
        TEST_AUTOCHECK.with(|c| c.set(true));
        let d = std::env::temp_dir().join(format!("memo-run-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("k.py"), "bad").unwrap();
        std::fs::write(d.join("check.sh"), "#!/bin/sh\necho ran >> runs.log\nif grep -q good k.py; then echo '{\"exec_acc\": true}'; else echo '{\"exec_acc\": false, \"detail\": \"k.py is bad\"}'; fi\n").unwrap();
        std::fs::set_permissions(d.join("check.sh"), std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        register("s-memo", &[("./check.sh".to_string(), 30)], Some(&d));
        let runs = || std::fs::read_to_string(d.join("runs.log")).map_or(0, |t| t.lines().count());
        // The engine runs the check after a change and reports the failure.
        let note = autocheck_note("s-memo").unwrap();
        assert!(note.contains("FAILED") && note.contains("k.py is bad"), "{note}");
        assert_eq!(runs(), 1);
        // Same sources: reused, not run again (also for the end-of-turn gate).
        assert!(autocheck_note("s-memo").unwrap().contains("FAILED"));
        assert_eq!(recall("s-memo", "./check.sh").map(|r| r.0), Some(false));
        assert_eq!(runs(), 1);
        // The model fixes the file: the old verdict no longer applies, the check runs again and passes.
        std::fs::write(d.join("k.py"), "good").unwrap();
        assert!(recall("s-memo", "./check.sh").is_none());
        assert!(autocheck_note("s-memo").unwrap().contains("PASSED"));
        assert_eq!(runs(), 2);
        // A check the model ran itself is remembered too.
        std::fs::write(d.join("k.py"), "good # edited").unwrap();
        note_observed("s-memo", "cd . && ./check.sh", "{\"exec_acc\": true}\n\nExit code: 0");
        assert_eq!(recall("s-memo", "./check.sh").map(|r| r.0), Some(true));
        forget("s-memo");
        assert!(autocheck_note("s-memo").is_none(), "an unregistered session has no checks");
        TEST_AUTOCHECK.with(|c| c.set(false));
        std::fs::remove_dir_all(&d).unwrap();
    }
}
