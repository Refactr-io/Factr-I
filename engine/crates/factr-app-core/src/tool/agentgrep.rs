//! Code search tool: content grep and file-name find over the workspace,
//! built on the `ignore` and `regex` crates (gitignore-aware walk).

use super::{Tool, ToolContext, ToolOutput};
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use ignore::WalkBuilder;
use ignore::overrides::OverrideBuilder;
use ignore::types::TypesBuilder;
use regex::RegexBuilder;
use serde::Deserialize;
use serde_json::{Value, json};
use std::fmt::Write as _;
use std::io::BufRead as _;
use std::path::Path;

/// Cap on rendered grep matches. The header always reports the true total.
const DEFAULT_GREP_MAX_REGIONS: usize = 200;
const DEFAULT_FIND_MAX_FILES: usize = 10;
const MAX_LINE_CHARS: usize = 300;
/// Per-file size cap (streamed, so memory stays bounded). Files above it are
/// skipped and counted in the result header, never silently.
const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Deserialize)]
struct AgentGrepInput {
    #[serde(default = "default_mode")]
    mode: String,
    // `pattern` accepted for legacy grep-tool calls aliased to agentgrep.
    #[serde(default, alias = "pattern")]
    query: Option<String>,
    // `file_path` accepted because agents frequently pass it instead of `file`.
    #[serde(default, alias = "file_path")]
    file: Option<String>,
    #[serde(default)]
    regex: Option<bool>,
    #[serde(default)]
    path: Option<String>,
    // `include` accepted for legacy grep-tool calls aliased to agentgrep.
    #[serde(default, alias = "include")]
    glob: Option<String>,
    #[serde(rename = "type", default)]
    file_type: Option<String>,
    #[serde(default)]
    hidden: Option<bool>,
    #[serde(default)]
    no_ignore: Option<bool>,
    #[serde(default)]
    max_files: Option<usize>,
    #[serde(default)]
    max_regions: Option<usize>,
    #[serde(default)]
    paths_only: Option<bool>,
}

fn default_mode() -> String {
    "grep".to_string()
}

pub struct AgentGrepTool;

impl AgentGrepTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for AgentGrepTool {
    fn name(&self) -> &str {
        "agentgrep"
    }

    fn description(&self) -> &str {
        "Search code and file names. Defaults to grep mode when mode is omitted."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "intent": super::intent_schema_property(),
                "mode": {
                    "type": "string",
                    "enum": ["grep", "find"],
                    "description": "Mode: grep (default, file contents) or find (file names)."
                },
                "query": {
                    "type": "string",
                    "description": "Grep: required, literal unless regex=true. Find: path words."
                },
                "regex": {
                    "type": "boolean",
                    "description": "In grep mode, treat query as a regex. Defaults to false (literal)."
                },
                "path": {
                    "type": "string",
                    "description": "Directory or file to search (default: workspace). Result paths are relative to it."
                },
                "glob": {
                    "type": "string",
                    "description": "Optional file glob filter such as **/*.rs. Omit to search everything."
                },
                "type": {
                    "type": "string",
                    "description": "Optional file type filter using ripgrep type names, such as rust (alias rs), py, js, ts, or md."
                },
                "max_files": {
                    "type": "integer",
                    "description": "Maximum number of files to return in find mode."
                },
                "max_regions": {
                    "type": "integer",
                    "description": "Maximum number of matching lines to return in grep mode."
                },
                "paths_only": {
                    "type": "boolean",
                    "description": "Grep mode: return only the matching file paths."
                }
            }
        })
    }

    async fn execute(&self, input: Value, ctx: ToolContext) -> Result<ToolOutput> {
        let params: AgentGrepInput = serde_json::from_value(input)?;
        // Blocking directory walk: keep it off the async workers.
        tokio::task::spawn_blocking(move || run_blocking(&params, &ctx))
            .await
            .map_err(|err| anyhow!("agentgrep task failed to join: {err}"))?
    }
}

fn run_blocking(params: &AgentGrepInput, ctx: &ToolContext) -> Result<ToolOutput> {
    let scope = params.path.as_deref().or(params.file.as_deref());
    let root = match scope {
        Some(p) => ctx.resolve_path(Path::new(p)),
        None => ctx.working_dir.clone().ok_or_else(|| {
            anyhow!("agentgrep requires a session working directory unless an absolute path is provided")
        })?,
    };
    if !root.exists() {
        return Err(anyhow!("path not found: {}", root.display()));
    }
    match params.mode.as_str() {
        "grep" => grep(params, &root),
        "find" => find(params, &root),
        other => Err(anyhow!(
            "Unsupported agentgrep mode: {other}. Use grep or find."
        )),
    }
}

fn walker(params: &AgentGrepInput, root: &Path) -> Result<ignore::Walk> {
    let respect_ignore = !params.no_ignore.unwrap_or(false);
    let mut builder = WalkBuilder::new(root);
    builder
        .hidden(!params.hidden.unwrap_or(false))
        .git_ignore(respect_ignore)
        .git_global(respect_ignore)
        .git_exclude(respect_ignore)
        .ignore(respect_ignore)
        .require_git(false);
    if let Some(glob) = params
        .glob
        .as_deref()
        .map(str::trim)
        .filter(|g| !g.is_empty() && !matches!(*g, "*" | "**" | "**/*" | "./*" | "./**" | "./**/*"))
    {
        let base = if root.is_file() {
            root.parent().unwrap_or(root)
        } else {
            root
        };
        let mut ov = OverrideBuilder::new(base);
        ov.add(glob)?;
        builder.overrides(ov.build()?);
    }
    if let Some(ty) = params.file_type.as_deref().filter(|t| !t.is_empty()) {
        let mut types = TypesBuilder::new();
        types.add_defaults();
        let ty = type_alias(ty);
        types.select(&ty);
        let built = types.build().map_err(|e| {
            anyhow!("{e}. Use ripgrep type names such as rust (alias rs), py, js, ts, md, go, json, toml, yaml, sh, html, java, c, cpp")
        })?;
        builder.types(built);
    }
    Ok(builder.build())
}

fn type_alias(ty: &str) -> String {
    let lower = ty.trim().to_lowercase();
    match lower.as_str() {
        "rs" => "rust".to_string(),
        "text" => "txt".to_string(),
        "python" | "javascript" | "typescript" | "markdown" | "golang" => lower,
        _ => lower,
    }
}

fn base_dir(root: &Path) -> &Path {
    if root.is_file() { root.parent().unwrap_or(root) } else { root }
}

/// One output line: trimmed, clipped with an explicit marker carrying the
/// original length so a cut line is never mistaken for the whole line.
fn clip_line(line: &str) -> String {
    let t = line.trim();
    let n = t.chars().count();
    if n <= MAX_LINE_CHARS {
        return t.to_string();
    }
    let cut: String = t.chars().take(MAX_LINE_CHARS).collect();
    format!("{cut}... [line truncated, {n} chars total]")
}

fn display(root: &Path, path: &Path) -> String {
    let base = base_dir(root);
    path.strip_prefix(base)
        .unwrap_or(path)
        .display()
        .to_string()
}

fn grep(params: &AgentGrepInput, root: &Path) -> Result<ToolOutput> {
    let query = params
        .query
        .as_deref()
        .filter(|q| !q.is_empty())
        .ok_or_else(|| anyhow!("agentgrep grep requires 'query'"))?;
    let pattern = if params.regex.unwrap_or(false) {
        query.to_string()
    } else {
        regex::escape(query)
    };
    let re = RegexBuilder::new(&pattern)
        .case_insensitive(!query.chars().any(char::is_uppercase))
        .build()
        .map_err(|e| anyhow!("invalid regex: {e}"))?;
    let max = params.max_regions.unwrap_or(DEFAULT_GREP_MAX_REGIONS);
    let paths_only = params.paths_only.unwrap_or(false);

    let (mut total, mut files, mut shown, mut body) = (0usize, 0usize, 0usize, String::new());
    let mut skipped = 0usize;
    for entry in walker(params, root)?.flatten() {
        let path = entry.path();
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        if entry.metadata().map(|m| m.len() > MAX_FILE_BYTES).unwrap_or(true) {
            skipped += 1;
            continue;
        }
        let Ok(file) = std::fs::File::open(path) else { continue };
        let name = display(root, path);
        // Stream line by line; matches are buffered per file so a NUL byte
        // anywhere (binary) discards the whole file like rg does.
        let mut reader = std::io::BufReader::new(file);
        let mut buf = Vec::new();
        let (mut file_total, mut pending, mut lineno) = (0usize, Vec::new(), 0usize);
        let mut binary = false;
        loop {
            buf.clear();
            match reader.read_until(b'\n', &mut buf) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
            lineno += 1;
            if buf.contains(&0) {
                binary = true;
                break;
            }
            if buf.last() == Some(&b'\n') {
                buf.pop();
                if buf.last() == Some(&b'\r') {
                    buf.pop();
                }
            }
            let line = String::from_utf8_lossy(&buf);
            if !re.is_match(&line) {
                continue;
            }
            file_total += 1;
            if !paths_only && shown + pending.len() < max {
                pending.push((lineno, clip_line(&line)));
            }
        }
        if binary || file_total == 0 {
            continue;
        }
        total += file_total;
        files += 1;
        if paths_only {
            if shown < max {
                shown += 1;
                let _ = writeln!(body, "{name}");
            }
        } else {
            for (n, text) in pending {
                shown += 1;
                let _ = writeln!(body, "{name}:{n}: {text}");
            }
        }
    }
    let mut out = format!("{total} matches in {files} files for {query:?}\n");
    let _ = writeln!(out, "paths are relative to {}", base_dir(root).display());
    if skipped > 0 {
        let _ = writeln!(out, "skipped {skipped} files over {} MiB", MAX_FILE_BYTES >> 20);
    }
    out.push_str(&body);
    if shown < if paths_only { files } else { total } {
        let _ = writeln!(out, "... output capped at {max}; raise max_regions or narrow the search");
    }
    Ok(ToolOutput::new(out).with_title("agentgrep grep"))
}

fn find(params: &AgentGrepInput, root: &Path) -> Result<ToolOutput> {
    let words: Vec<String> = params
        .query
        .as_deref()
        .unwrap_or_default()
        .split_whitespace()
        .map(str::to_lowercase)
        .collect();
    if words.is_empty()
        && params.path.is_none()
        && params.file.is_none()
        && params.glob.is_none()
        && params.file_type.is_none()
    {
        return Err(anyhow!(
            "agentgrep find requires 'query' unless path, glob, or type narrows the search"
        ));
    }
    let max = params.max_files.unwrap_or(DEFAULT_FIND_MAX_FILES);
    let mut hits: Vec<(usize, String)> = Vec::new();
    for entry in walker(params, root)?.flatten() {
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let name = display(root, entry.path());
        let lower = name.to_lowercase();
        if words.iter().all(|w| lower.contains(w)) {
            // File-name hits and shorter paths rank first.
            let base = Path::new(&lower)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let named = words.iter().filter(|w| base.contains(w.as_str())).count();
            hits.push((lower.len().saturating_sub(named * 1000), name));
        }
    }
    hits.sort();
    let total = hits.len();
    let mut out = format!("{total} files\n");
    let _ = writeln!(out, "paths are relative to {}", base_dir(root).display());
    for (_, name) in hits.iter().take(max) {
        let _ = writeln!(out, "{name}");
    }
    if total > max {
        let _ = writeln!(out, "... {} more; raise max_files or narrow the query", total - max);
    }
    Ok(ToolOutput::new(out).with_title("agentgrep find"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("agentgrep-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/a.rs"), "fn needle() {}\nlet x = 1;\n").unwrap();
        std::fs::write(dir.join("src/b.md"), "a Needle here\n").unwrap();
        std::fs::write(dir.join(".gitignore"), "ignored.txt\n").unwrap();
        std::fs::write(dir.join("ignored.txt"), "needle\n").unwrap();
        dir
    }

    fn input(v: Value) -> AgentGrepInput {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn grep_is_literal_case_smart_and_respects_gitignore() {
        let dir = tmp("grep");
        // Windows prints `src\a.rs`; separators are not what is under test.
        let out = grep(&input(json!({"query": "needle"})), &dir).unwrap().output.replace('\\', "/");
        assert!(out.starts_with("2 matches in 2 files"), "{out}");
        assert!(out.contains("src/a.rs:1: fn needle() {}"));
        assert!(!out.contains("ignored.txt"));
        let upper = grep(&input(json!({"query": "Needle"})), &dir).unwrap().output;
        assert!(upper.starts_with("1 matches in 1 files"), "{upper}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn grep_regex_type_glob_and_cap() {
        let dir = tmp("opts");
        let o = grep(&input(json!({"query": "n.edle", "regex": true, "type": "rust"})), &dir)
            .unwrap()
            .output;
        assert!(o.starts_with("1 matches in 1 files") && o.contains("a.rs"), "{o}");
        let o = grep(&input(json!({"query": "needle", "glob": "**/*.md"})), &dir).unwrap().output;
        assert!(o.contains("b.md") && !o.contains("a.rs"), "{o}");
        let o = grep(&input(json!({"query": "needle", "max_regions": 1})), &dir).unwrap().output;
        assert!(o.contains("output capped at 1"), "{o}");
        assert!(grep(&input(json!({})), &dir).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn grep_single_file_path_and_legacy_aliases() {
        let dir = tmp("file");
        let p = dir.join("src/a.rs");
        let o = grep(&input(json!({"pattern": "needle", "include": "*.rs"})), &p).unwrap().output;
        assert!(o.starts_with("1 matches"), "{o}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn find_matches_all_words_and_requires_a_narrowing() {
        let dir = tmp("find");
        let o = find(&input(json!({"mode": "find", "query": "src a"})), &dir)
            .unwrap()
            .output
            .replace('\\', "/");
        assert!(o.starts_with("1 files") && o.contains("src/a.rs"), "{o}");
        assert!(find(&input(json!({"mode": "find"})), &dir).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn grep_searches_big_files_and_reports_relative_root() {
        let dir = tmp("big");
        let big = "abc line\n".repeat(700_000); // ~5.6 MB, over the old 2 MiB cap
        std::fs::write(dir.join("big.txt"), &big).unwrap();
        let o = grep(&input(json!({"query": "abc", "path": "big.txt", "max_regions": 3})), &dir.join("big.txt"))
            .unwrap()
            .output;
        assert!(o.starts_with("700000 matches in 1 files"), "{}", &o[..80]);
        assert!(o.contains("paths are relative to"), "{o}");
        assert!(!o.contains("skipped"), "{o}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn grep_reports_files_over_the_cap() {
        let dir = tmp("cap");
        let f = std::fs::File::create(dir.join("huge.log")).unwrap();
        f.set_len(MAX_FILE_BYTES + 1).unwrap(); // sparse
        let o = grep(&input(json!({"query": "needle"})), &dir).unwrap().output;
        assert!(o.contains("skipped 1 files over 64 MiB"), "{o}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn grep_long_lines_get_a_marker_and_binary_files_are_skipped() {
        let dir = tmp("long");
        std::fs::write(dir.join("l.txt"), format!("{}\n", "a".repeat(1000))).unwrap();
        std::fs::write(dir.join("bin.dat"), b"a\0a\n").unwrap();
        std::fs::write(dir.join("crlf.txt"), b"a\r\na\r\n").unwrap();
        let o = grep(&input(json!({"query": "aaa", "regex": true})), &dir).unwrap().output;
        assert!(o.contains("[line truncated, 1000 chars total]"), "{o}");
        assert!(!o.contains("bin.dat"), "{o}");
        let o = grep(&input(json!({"query": "^a$", "regex": true})), &dir).unwrap().output;
        assert!(o.starts_with("2 matches in 1 files") && o.contains("crlf.txt:2: a"), "{o}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn missing_path_is_an_error_and_type_aliases_work() {
        let dir = tmp("path");
        let ctx = ToolContext {
            session_id: "t".into(),
            message_id: "t".into(),
            tool_call_id: "t".into(),
            working_dir: Some(dir.clone()),
            stdin_request_tx: None,
            graceful_shutdown_signal: None,
            execution_mode: crate::tool::ToolExecutionMode::Direct,
        };
        let err = run_blocking(&input(json!({"query": "needle", "path": "nope/x"})), &ctx).unwrap_err();
        assert!(err.to_string().contains("path not found"), "{err}");
        let o = grep(&input(json!({"query": "needle", "type": "rs"})), &dir).unwrap().output;
        assert!(o.starts_with("1 matches in 1 files") && o.contains("a.rs"), "{o}");
        let e = grep(&input(json!({"query": "needle", "type": "zzz"})), &dir).unwrap_err();
        assert!(e.to_string().contains("ripgrep type names"), "{e}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn schema_exposes_grep_and_find_only() {
        let s = AgentGrepTool::new().parameters_schema();
        assert_eq!(s["properties"]["mode"]["enum"], json!(["grep", "find"]));
    }
}
