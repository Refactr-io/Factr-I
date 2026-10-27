//! The check an unattended run names, remembered per session.
//!
//! Persistence makes the engine run the task's check after every turn. When the model has just run the same check on
//! byte-identical sources, running it again says nothing new, so its result is reused. The same record notices a check
//! the model runs that fails the same way twice. Sources are compared by content (the check's own outputs and caches do
//! not count).

use crate::goal_ratchet::report_failed;
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

const FINGERPRINT_FILES: usize = 400;
const FINGERPRINT_FILE_BYTES: u64 = 512 * 1024;
const SOURCE_EXTS: [&str; 40] = [
    "rs", "py", "js", "jsx", "ts", "tsx", "mjs", "cjs", "go", "java", "kt", "swift", "c", "h", "cc", "cpp", "hpp", "cs", "rb", "php", "scala", "sh",
    "lua", "dart", "zig", "ex", "cu", "cuh", "cl", "metal", "wgsl", "glsl", "hlsl", "jl", "r", "sql", "toml", "yaml", "yml", "cfg",
];
const SKIP_DIRS: [&str; 9] = [".git", "node_modules", "target", "venv", ".venv", "__pycache__", ".tox", ".pytest_cache", "build"];
/// How much of a check's output a remembered result keeps.
const OUTPUT_TAIL: usize = 3000;

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

/// A persisting run's gate commands (each with its timeout in seconds) and working directory.
pub fn register(session: &str, gates: &[(String, u64)], cwd: Option<&Path>) {
    REGISTRY.lock().unwrap_or_else(|e| e.into_inner()).insert(session.to_string(), Entry { gates: gates.to_vec(), cwd: cwd.map(Path::to_path_buf), memos: Vec::new() });
}

pub fn forget(session: &str) {
    REGISTRY.lock().unwrap_or_else(|e| e.into_inner()).remove(session);
    OBSERVED.lock().unwrap_or_else(|e| e.into_inner()).remove(session);
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
    let gates: Vec<String> = match REGISTRY.lock().unwrap_or_else(|e| e.into_inner()).get(session) {
        Some(entry) => entry.gates.iter().map(|(g, _)| g.clone()).filter(|g| line_runs(command, g)).collect(),
        None => return,
    };
    let exit = exit_code_of(result);
    for gate in gates {
        remember(session, &gate, exit == 0 && !report_failed(result), exit, result);
    }
}

/// What the model's last failing run of the check looked like, per session: (the failure, a hash of the sources).
static OBSERVED: LazyLock<Mutex<HashMap<String, (String, u64)>>> = LazyLock::new(Default::default);

/// When the model runs the task's check and it fails the same way as its previous run, or the sources are the same as
/// then, a one-line note for the end of that result: a different approach is needed, not a repeat.
pub fn stall_note(session: &str, command: &str, result: &str) -> Option<String> {
    let cwd = {
        let reg = REGISTRY.lock().unwrap_or_else(|e| e.into_inner());
        let entry = reg.get(session)?;
        if !entry.gates.iter().any(|(g, _)| line_runs(command, g)) {
            return None;
        }
        entry.cwd.clone()
    };
    let mut seen = OBSERVED.lock().unwrap_or_else(|e| e.into_inner());
    if exit_code_of(result) == 0 && !report_failed(result) {
        seen.remove(session);
        return None;
    }
    let failure: String = result.split_whitespace().rev().take(120).collect::<Vec<_>>().join(" ");
    let now = (failure, cwd.as_deref().map_or(0, source_fingerprint));
    let stalled = seen.insert(session.to_string(), now.clone()).is_some_and(|before| before.0 == now.0 || before.1 == now.1);
    stalled.then(|| "[The check failed the same way again, or the files are unchanged since its last run. Try a different approach instead of repeating the last attempt.]".to_string())
}

/// The remembered verdict `(passed, exit code, output)` of `gate`, when the sources have not changed since.
pub fn recall(session: &str, gate: &str) -> Option<(bool, i64, String)> {
    let reg = REGISTRY.lock().unwrap_or_else(|e| e.into_inner());
    let entry = reg.get(session)?;
    let memo = entry.memos.iter().find(|m| m.gate == gate)?;
    (entry.cwd.as_deref().map_or(0, source_fingerprint) == memo.fingerprint).then(|| (memo.passed, memo.exit_code, memo.output.clone()))
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

    #[test]
    fn a_remembered_verdict_is_reused_only_for_unchanged_sources() {
        let d = std::env::temp_dir().join(format!("memo-run-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("k.py"), "bad").unwrap();
        register("s-memo", &[("./check.sh".to_string(), 30)], Some(&d));
        assert!(recall("s-memo", "./check.sh").is_none(), "nothing run yet");
        // The model ran the check and it failed: the verdict is reused while the files stay as they are.
        note_observed("s-memo", "cd . && ./check.sh", "{\"exec_acc\": false}\n\nExit code: 0");
        assert_eq!(recall("s-memo", "./check.sh").map(|r| r.0), Some(false), "a failing report counts as a failure");
        // The model changes a file: the old verdict no longer applies.
        std::fs::write(d.join("k.py"), "good").unwrap();
        assert!(recall("s-memo", "./check.sh").is_none());
        note_observed("s-memo", "./check.sh --fast", "{\"exec_acc\": true}\n\nExit code: 0");
        assert_eq!(recall("s-memo", "./check.sh").map(|r| r.0), Some(true));
        // A command that is not the check is not remembered, and a forgotten session has nothing.
        note_observed("s-memo", "cat ./check.sh", "Exit code: 1");
        assert_eq!(recall("s-memo", "./check.sh").map(|r| r.0), Some(true));
        forget("s-memo");
        assert!(recall("s-memo", "./check.sh").is_none());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_check_that_fails_the_same_way_twice_gets_a_note() {
        let dir = std::env::temp_dir().join(format!("stall-note-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.py"), "1").unwrap();
        register("stall", &[("./check".to_string(), 60)], Some(&dir));
        let bad = |n: &str| format!("error in a.py: {n}\nExit code: 1");
        assert!(stall_note("stall", "./check", &bad("x")).is_none(), "first failure");
        std::fs::write(dir.join("a.py"), "2").unwrap();
        assert!(stall_note("stall", "./check", &bad("x")).is_some(), "same failure after a change");
        std::fs::write(dir.join("a.py"), "3").unwrap();
        assert!(stall_note("stall", "./check", &bad("y")).is_none(), "new failure, new file");
        assert!(stall_note("stall", "./check", &bad("z")).is_some(), "files unchanged");
        assert!(stall_note("stall", "ls", &bad("z")).is_none(), "not the check");
        assert!(stall_note("stall", "./check", "ok").is_none(), "it passed");
        forget("stall");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
