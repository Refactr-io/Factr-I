//! One-time environment line for a session's first user message: cwd, which
//! common tools are on PATH (no subprocess per tool), python package probe
//! (one `python3 -c`, 1.5 s cap), and the first cwd entries with sizes. Hard cap 1200 chars.

use std::path::Path;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

const TOOLS: [&str; 16] = [
    "python3", "python", "pip", "node", "npm", "cargo", "go", "java", "gcc", "g++", "cmake", "git", "uv", "jq", "curl",
    "systemctl",
];
const MAX_CHARS: usize = 1200;
/// Same threshold as the system prompt's large-input rule (~20K characters).
const LARGE_INPUT_BYTES: u64 = 20_000;
/// Line counts are only computed for files up to this size.
const LINE_COUNT_MAX_BYTES: u64 = 8 * 1024 * 1024;

fn on_path(name: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(name).is_file()))
}

/// "pytest 8.1, numpy 1.26" / "no pytest, no numpy", cached for the process, probed on the same
/// interpreter the REPL and `bash` use (`factr_base::python_env`).
fn python_probe() -> &'static str {
    static PROBE: OnceLock<String> = OnceLock::new();
    PROBE.get_or_init(|| {
        let code = "import importlib\nfor m in ('pytest','numpy'):\n try: print(m, importlib.import_module(m).__version__)\n except Exception: print('no', m)";
        let Some(python) = factr_base::python_env::interpreter() else {
            return String::new();
        };
        let Ok(mut child) = std::process::Command::new(python)
            .args(["-c", code])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
        else {
            return String::new();
        };
        let start = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if start.elapsed() < Duration::from_millis(1500) => std::thread::sleep(Duration::from_millis(20)),
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return String::new();
                }
            }
        }
        let mut out = String::new();
        if let Some(mut so) = child.stdout.take() {
            let _ = std::io::Read::read_to_string(&mut so, &mut out);
        }
        out.lines().collect::<Vec<_>>().join(", ")
    })
}

pub(super) fn enabled() -> bool {
    std::env::var("FACTR_ENV_SNAPSHOT").map_or(true, |v| v != "0")
        && crate::config::config().agents.environment_snapshot
}

fn human_size(bytes: u64) -> String {
    match bytes {
        0..1024 => format!("{bytes} B"),
        1024..1_048_576 => format!("{} KB", bytes.div_ceil(1024)),
        _ => format!("{:.1} MB", bytes as f64 / 1_048_576.0),
    }
}

/// `name`, or `name (size, N lines[, large input])` for a regular file; a directory gets a `/`.
fn describe_entry(path: &Path, name: &str) -> String {
    let Ok(meta) = std::fs::metadata(path) else { return name.to_string() };
    if meta.is_dir() {
        return format!("{name}/");
    }
    if !meta.is_file() {
        return name.to_string();
    }
    let size = meta.len();
    let mut parts = vec![human_size(size)];
    if size > 0 && size <= LINE_COUNT_MAX_BYTES {
        if let Ok(bytes) = std::fs::read(path) {
            if !bytes[..bytes.len().min(8192)].contains(&0) {
                let newlines = bytes.iter().filter(|&&b| b == b'\n').count();
                let lines = newlines + usize::from(bytes.last() != Some(&b'\n'));
                parts.push(format!("{lines} lines"));
            }
        }
    }
    if size > LARGE_INPUT_BYTES {
        parts.push("large input".to_string());
    }
    format!("{name} ({})", parts.join(", "))
}

fn installers_field(on: bool, routes: &[&str]) -> String {
    if on {
        format!("installers: {}; ", if routes.is_empty() { "none".to_string() } else { routes.join(",") })
    } else {
        String::new()
    }
}

pub(super) fn snapshot(cwd: &Path) -> String {
    let (have, missing): (Vec<&str>, Vec<&str>) = TOOLS.iter().partition(|t| on_path(t));
    let mut have: Vec<String> = have.iter().map(|t| t.to_string()).collect();
    if let Some(py) = have.iter_mut().find(|t| *t == "python3") {
        let probe = python_probe();
        if !probe.is_empty() {
            *py = format!("python3 ({probe})");
        }
    }
    let mut names: Vec<String> = std::fs::read_dir(cwd)
        .map(|rd| rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect())
        .unwrap_or_default();
    names.sort();
    names.truncate(20);
    let names: Vec<String> = names.iter().map(|n| describe_entry(&cwd.join(n), n)).collect();
    let routes = factr_base::shell::package_routes();
    // `FACTR_GUARD_ENV_INSTALLERS=0` drops the installers field (on by default) (read once per process).
    static INSTALLERS: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let installers = installers_field(*INSTALLERS.get_or_init(|| factr_base::prompt::guard_switch("ENV_INSTALLERS")), routes);
    let text = format!(
        "<environment>cwd {}; shell: {}; repl: {}; {}have: {}; missing: {}; files: {}</environment>",
        cwd.display(),
        if factr_base::shell::is_bash() { "bash" } else { "sh" },
        if crate::tool::repl_available() { "on" } else { "off" },
        installers,
        have.join(", "),
        missing.join(", "),
        names.join(" ")
    );
    if text.chars().count() <= MAX_CHARS {
        return text;
    }
    let mut cut: String = text.chars().take(MAX_CHARS - 17).collect();
    cut.push_str("...</environment>");
    cut
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_is_capped_and_lists_files() {
        let d = std::env::temp_dir().join(format!("envsnap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        for i in 0..40 {
            std::fs::write(d.join(format!("file_with_a_long_name_{i:02}.txt")), "").unwrap();
        }
        let s = snapshot(&d);
        assert!(s.chars().count() <= MAX_CHARS, "{}", s.chars().count());
        assert!(s.starts_with("<environment>cwd ") && s.ends_with("</environment>"));
        assert!(s.contains("; shell: ") && s.contains("; repl: ") && s.contains("; installers: "), "{s}");
        let d2 = d.join("sub");
        std::fs::create_dir_all(&d2).unwrap();
        std::fs::write(d2.join("a.txt"), "").unwrap();
        assert!(snapshot(&d2).contains("files: a.txt"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn installers_field_follows_its_switch() {
        assert_eq!(installers_field(false, &["pip"]), "");
        assert_eq!(installers_field(true, &[]), "installers: none; ");
        assert_eq!(installers_field(true, &["pip", "npm"]), "installers: pip,npm; ");
    }

    #[test]
    fn snapshot_shows_sizes_lines_and_large_marker() {
        let d = std::env::temp_dir().join(format!("envsnap-sizes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("sub")).unwrap();
        let big: String = (0..1237).map(|i| format!("line {i:05} category-{} with filler text to pad it out a bit\n", i % 7)).collect();
        assert!(big.len() as u64 > LARGE_INPUT_BYTES);
        std::fs::write(d.join("big.txt"), &big).unwrap();
        std::fs::write(d.join("small.txt"), "a\nb").unwrap();
        std::fs::write(d.join("blob.bin"), [0u8, 1, 2, 3]).unwrap();
        let s = snapshot(&d);
        assert!(s.contains(&format!("big.txt ({} KB, 1237 lines, large input)", big.len().div_ceil(1024))), "{s}");
        assert!(s.contains("small.txt (3 B, 2 lines)"), "{s}");
        assert!(s.contains("blob.bin (4 B)"), "{s}");
        assert!(s.contains("sub/"), "{s}");
        assert!(s.chars().count() <= MAX_CHARS);
        let _ = std::fs::remove_dir_all(&d);
    }
}
