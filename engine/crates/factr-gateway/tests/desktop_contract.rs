//! DESKTOP-VS-CONTRACT: reads the Factr desktop source (READ ONLY) and checks every gateway
//! method it calls against the contract and the sweep golden (contract_sweep.golden.json).
//!
//! Asserts: each method the desktop calls exists in the contract and is not not_found/timeout/crash.
//! Prints the desktop-used methods that are `forwarded_unavailable` (UI features needing the Python
//! backend) and asserts them only against tests/desktop_needs_python.txt, so a new UI dependency on
//! the Python backend is an explicit decision. Config keys passed to config.get/config.set are
//! pinned in tests/desktop_config_keys.txt the same way.
//!
//! Skips (passes) when the desktop checkout is absent. Override its location with DESKTOP_SRC.
//! REGENERATE the two lists after an intentional change:
//!   UPDATE_GOLDEN=1 nice -n 10 cargo test -p factr-gateway --test desktop_contract -j 4

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

fn tests_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests")
}

fn desktop_src() -> PathBuf {
    std::env::var_os("DESKTOP_SRC").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../backend/apps/desktop/src"))
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else if matches!(p.extension().and_then(|x| x.to_str()), Some("ts" | "tsx")) {
            let name = p.file_name().unwrap().to_string_lossy();
            if !name.contains(".test.") && !name.ends_with(".d.ts") {
                out.push(p);
            }
        }
    }
}

fn is_method(s: &str) -> bool {
    s.contains('.') && s.split('.').all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_lowercase() || c == '_'))
}

/// `(method, config_key)` for every `fooRequest...(` / `...rpc...(` / `...gateway...(` call with a literal method.
fn scan(src: &str, methods: &mut BTreeSet<String>, keys: &mut BTreeSet<String>) {
    let b = src.as_bytes();
    for (i, _) in src.match_indices('(') {
        let mut j = i;
        if j > 0 && b[j - 1] == b'>' {
            let mut depth = 0;
            while j > 0 {
                j -= 1;
                match b[j] {
                    b'>' => depth += 1,
                    b'<' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
            }
        }
        let end = j;
        while j > 0 && (b[j - 1].is_ascii_alphanumeric() || b[j - 1] == b'_') {
            j -= 1;
        }
        let ident = src[j..end].to_lowercase();
        if ident.is_empty() || ident.starts_with("gatewayevent") || !["request", "rpc", "gateway"].iter().any(|k| ident.contains(k)) {
            continue;
        }
        let rest = src[i + 1..].trim_start();
        let Some(q) = rest.chars().next().filter(|c| *c == '\'' || *c == '"') else { continue };
        let Some(close) = rest[1..].find(q) else { continue };
        let lit = &rest[1..1 + close];
        if !is_method(lit) {
            continue;
        }
        methods.insert(lit.to_string());
        if lit == "config.get" || lit == "config.set" {
            let window: String = rest.chars().take(400).collect();
            if let Some(k) = window.find("key:") {
                let after = window[k + 4..].trim_start();
                if let Some(q) = after.chars().next().filter(|c| *c == '\'' || *c == '"') {
                    if let Some(c) = after[1..].find(q) {
                        keys.insert(after[1..1 + c].to_string());
                    }
                }
            }
        }
    }
}

fn pinned(path: &Path, current: &BTreeSet<String>, what: &str) {
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::write(path, current.iter().map(|s| format!("{s}\n")).collect::<String>()).unwrap();
    }
    let text = std::fs::read_to_string(path).unwrap_or_else(|_| panic!("{} missing; run with UPDATE_GOLDEN=1", path.display()));
    let known: BTreeSet<String> = text.lines().filter(|l| !l.is_empty()).map(String::from).collect();
    let added: Vec<_> = current.difference(&known).collect();
    let gone: Vec<_> = known.difference(current).collect();
    assert!(added.is_empty() && gone.is_empty(), "{what} changed. new: {added:?} gone: {gone:?} (UPDATE_GOLDEN=1 if intended)");
}

#[test]
fn desktop_only_calls_methods_the_engine_answers() {
    let root = desktop_src();
    if !root.is_dir() {
        eprintln!("skipping: desktop source not found at {}", root.display());
        return;
    }
    let mut files = Vec::new();
    walk(&root, &mut files);
    let (mut methods, mut keys) = (BTreeSet::new(), BTreeSet::new());
    for f in &files {
        scan(&std::fs::read_to_string(f).unwrap_or_default(), &mut methods, &mut keys);
    }
    assert!(methods.len() > 30, "scanner found only {} methods in {} files; the source layout changed", methods.len(), files.len());

    let golden: BTreeMap<String, String> = serde_json::from_str(&std::fs::read_to_string(tests_dir().join("contract_sweep.golden.json")).expect("run contract_sweep first")).unwrap();
    let unknown: Vec<_> = methods.iter().filter(|m| !golden.contains_key(*m)).collect();
    assert!(unknown.is_empty(), "desktop calls methods that are not in the contract: {unknown:?}");
    let broken: Vec<_> = methods.iter().filter(|m| matches!(golden[*m].as_str(), "not_found" | "timeout" | "crash")).collect();
    assert!(broken.is_empty(), "desktop calls methods the engine does not answer: {broken:?}");

    let needs_python: BTreeSet<String> = methods.iter().filter(|m| golden[*m] == "forwarded_unavailable").cloned().collect();
    eprintln!("desktop methods needing the Python backend ({}): {needs_python:?}", needs_python.len());
    eprintln!("desktop config keys: {keys:?}");
    pinned(&tests_dir().join("desktop_needs_python.txt"), &needs_python, "desktop methods needing the Python backend");
    pinned(&tests_dir().join("desktop_config_keys.txt"), &keys, "desktop config keys");
}
