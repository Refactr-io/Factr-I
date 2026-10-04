//! The checkpoint store behind `/rollback`, `/undo` and goal checkpoints (port of Factr'
//! `checkpoint_manager.py`). Lives in factr-base so the engine's tool hooks (factr-app-core), the
//! gateway and the goal ratchet (factr-learn) all use the one store.
//!
//! One shared bare git store under `$FACTR_HOME/checkpoints/store.git`; each project (working
//! directory) owns the parentless commits behind `refs/factr/<hash16>/snap/<seq>` (pruning drops
//! refs, never rewrites a commit, so hashes stay valid; [`Store::pin`] keeps a commit alive under
//! `refs/factr/<hash16>/pins/<session>/<hash>`), a private index (`indexes/<hash16>`, kept so
//! an unchanged tree is a cheap stat-only pass) and an agent-write ledger (`ledgers/<hash16>.json`:
//! abs path -> sha256 after the agent's last write). The user's own repo, index and HEAD are never
//! touched, and it works outside git repos. Everything goes through the `git` CLI with
//! GIT_DIR / GIT_WORK_TREE / GIT_INDEX_FILE pointed at the store.
//!
//! Triggers live in the engine's tool hooks (`factr-app-core/src/checkpoint.rs`: `before_change`,
//! `before_bash`, `new_turn`, `after_write`); they call [`Store::snapshot_for`] and
//! [`Store::record_write`] here.
//!
//! History rewind: a full restore returns `success`; Factr then rewinds the live transcript by
//! the latest user turn and sets `history_removed`. That belongs to the gateway's session code, so
//! [`Store::restore`] leaves `history_removed` at 0 and the caller overwrites it.

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use crate::obs_sink::{self, Span};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;

const MAX_FILES: usize = 50_000;
const LEDGER_MAX: usize = 2000;
const MB: u64 = 1024 * 1024;
const DEFAULT_EXCLUDES: &[&str] = &[
    "node_modules/", "dist/", "build/", "target/", "out/", ".next/", ".nuxt/", "__pycache__/", "*.pyc", "*.pyo", ".cache/",
    ".pytest_cache/", ".mypy_cache/", ".ruff_cache/", "coverage/", ".coverage", ".venv/", "venv/", "env/", ".git/", ".hg/",
    ".svn/", ".worktrees/", "*.so", "*.dylib", "*.dll", "*.o", "*.a", "*.jar", "*.class", "*.exe", "*.obj", "*.mp4", "*.mov",
    "*.mkv", "*.webm", "*.zip", "*.tar", "*.tar.gz", "*.tgz", "*.7z", "*.rar", "*.iso", ".env", ".env.*", ".env.local",
    ".env.*.local", ".DS_Store", "Thumbs.db", "*.log",
];
/// Directory names the file-count walk never enters (mirrors the excludes above).
const SKIP_DIRS: &[&str] = &["node_modules", "dist", "build", "target", "out", ".next", ".nuxt", "__pycache__", ".cache", "coverage", ".venv", "venv", "env", ".git", ".hg", ".svn", ".worktrees"];

/// Serializes every store mutation in this process (prune rewrites refs and drops objects).
static STORE_LOCK: Mutex<()> = Mutex::new(());

/// `checkpoints.*` settings, read from Factr's `config.yaml` (the one owner; Settings writes it).
/// Absent key: ON for interactive sessions (the desktop's undo needs it), OFF for headless runs
/// (`FACTR_HEADLESS`, `/api/agent/run`, cron); an explicit `checkpoints.enabled` always wins.
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub enabled: bool,
    pub max_snapshots: usize,
    pub max_total_size_mb: u64,
    pub max_file_size_mb: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self { enabled: true, max_snapshots: 20, max_total_size_mb: 500, max_file_size_mb: 10 }
    }
}

pub struct Checkpoint {
    pub hash: String,
    pub timestamp: String,
    pub message: String,
}

pub struct Store {
    base: PathBuf,
    /// Where `config.yaml` is read from; `None`: the process's Factr dir (`factr_config::home()`).
    config_dir: Option<PathBuf>,
}

fn sha_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

fn hash_file(path: &Path) -> Option<String> {
    std::fs::read(path).ok().map(|b| sha_hex(&b))
}

/// A session id as a file name component (shared by every `$FACTR_HOME/undo/<session>.*` file).
pub fn safe_name(session: &str) -> String {
    session.chars().map(|c| if c.is_ascii_alphanumeric() || "-_.".contains(c) { c } else { '_' }).collect()
}

fn ref_safe(session: &str) -> String {
    safe_name(session).replace('.', "_")
}

/// The default store's undo dir (`$FACTR_HOME/undo`). Derived from [`Store::undo_base`], the one place
/// that builds the path, so a writer holding a store and a reader of the default can never disagree;
/// code holding any other store passes `store.undo_base()` explicitly.
pub fn undo_dir() -> Option<PathBuf> {
    Store::default_store().map(|s| s.undo_base())
}

/// `<base>/<session>.<suffix>`: `json` (gateway rows/turns/redo), `engine.json` (removed
/// messages), `effects.json` (irreversible effects).
pub fn undo_file(base: &Path, session: &str, suffix: &str) -> PathBuf {
    base.join(format!("{}.{suffix}", safe_name(session)))
}

/// A snapshot a turn took before its first file change: `(when, project dir, commit)`. The engine's
/// tool hooks write these (`undo/<session>.snaps.json`); `/undo` reads them to find the files as
/// they were when a turn began, so submitting a prompt never has to snapshot anything.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SnapEntry {
    pub at: String,
    pub dir: String,
    pub hash: String,
}

const SNAPS_MAX: usize = 400;

pub fn snapshots_of(base: &Path, session: &str) -> Vec<SnapEntry> {
    std::fs::read_to_string(undo_file(base, session, "snaps.json")).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn write_snapshots(base: &Path, session: &str, all: &[SnapEntry]) {
    let file = undo_file(base, session, "snaps.json");
    if all.is_empty() {
        let _ = std::fs::remove_file(&file);
        return;
    }
    let _ = std::fs::create_dir_all(base);
    let tmp = file.with_extension("tmp");
    if std::fs::write(&tmp, serde_json::to_string(all).unwrap_or_default()).is_ok() {
        let _ = std::fs::rename(&tmp, &file);
    }
}

/// Record that `session` snapshotted `dir` as `hash` just now (newest 400 kept).
pub fn note_snapshot_in(base: &Path, session: &str, dir: &Path, hash: &str) {
    let mut all = snapshots_of(base, session);
    all.push(SnapEntry { at: now_stamp(), dir: canonical(dir).to_string_lossy().into_owned(), hash: hash.to_string() });
    let excess = all.len().saturating_sub(SNAPS_MAX);
    all.drain(..excess);
    write_snapshots(base, session, &all);
}

/// The first snapshot of `dir` that `session` took at or after `since` (a [`now_stamp`] value): the
/// files as they were before anything the turns since then changed. `None`: nothing changed files.
pub fn first_snapshot_since(base: &Path, session: &str, dir: &Path, since: &str) -> Option<String> {
    let dir = canonical(dir).to_string_lossy().into_owned();
    snapshots_of(base, session).into_iter().filter(|e| e.dir == dir && e.at.as_str() >= since).min_by(|a, b| a.at.cmp(&b.at)).map(|e| e.hash)
}

/// Drop recorded snapshots older than `since` (`None`: all of them).
pub fn retain_snapshots_since(base: &Path, session: &str, since: Option<&str>) {
    let all = snapshots_of(base, session);
    let kept: Vec<SnapEntry> = all.iter().filter(|e| since.is_some_and(|s| e.at.as_str() >= s)).cloned().collect();
    if kept.len() != all.len() {
        write_snapshots(base, session, &kept);
    }
}

/// Forget the engine's removed-messages stack for `session` (a rewind that must not be redoable).
pub fn clear_engine_stack(base: &Path, session: &str) {
    let _ = std::fs::remove_file(undo_file(base, session, "engine.json"));
}

/// Delete every undo file of `session` (and their temp files).
pub fn purge_undo_files(base: &Path, session: &str) {
    for suffix in ["json", "engine.json", "effects.json", "snaps.json", "tmp", "engine.tmp", "effects.tmp", "snaps.tmp"] {
        let _ = std::fs::remove_file(undo_file(base, session, suffix));
    }
}

pub fn canonical(dir: &Path) -> PathBuf {
    dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf())
}

fn project_hash(dir: &Path) -> String {
    sha_hex(canonical(dir).to_string_lossy().as_bytes())[..16].to_string()
}

/// A snapshot reason as a fixed label for spans (the reason itself carries file paths).
fn reason_class(reason: &str) -> &'static str {
    const KNOWN: [(&str, &str); 8] = [
        ("before write", "write"),
        ("before edit", "edit"),
        ("before patch", "patch"),
        ("before command", "command"),
        ("pre-rollback", "pre_rollback"),
        ("goal checkpoint", "goal"),
        ("before undo", "undo"),
        ("turn start", "turn_start"),
    ];
    KNOWN.iter().find(|(prefix, _)| reason.starts_with(prefix)).map_or("other", |(_, label)| label)
}

impl Store {
    pub fn at(base: PathBuf) -> Self {
        Self { base, config_dir: None }
    }

    /// Read the settings from `dir/config.yaml` instead of the process's Factr dir (tests).
    pub fn with_config_dir(mut self, dir: PathBuf) -> Self {
        self.config_dir = Some(dir);
        self
    }

    /// `$FACTR_HOME/checkpoints`.
    pub fn default_store() -> Option<Self> {
        // Unit tests of other tools must never write into a real ~/.factr/engine.
        if cfg!(test) {
            return None;
        }
        crate::storage::factr_dir().ok().map(|d| Self::at(d.join("checkpoints")))
    }

    fn git_dir(&self) -> PathBuf {
        self.base.join("store.git")
    }

    /// The live `checkpoints:` settings over the defaults; a `config.set` applies on the next read.
    pub fn config(&self) -> Config {
        let factr = match &self.config_dir {
            Some(dir) => crate::factr_config::load_from(dir),
            None => crate::factr_config::current(),
        };
        let (set, default) = (&factr.checkpoints, Config::default());
        Config {
            enabled: set.enabled.unwrap_or(default.enabled && !crate::headless::process()),
            max_snapshots: set.max_snapshots.filter(|n| *n >= 1).map_or(default.max_snapshots, |n| n as usize),
            max_total_size_mb: set.max_total_size_mb.filter(|n| *n >= 0).map_or(default.max_total_size_mb, |n| n as u64),
            max_file_size_mb: set.max_file_size_mb.filter(|n| *n >= 0).map_or(default.max_file_size_mb, |n| n as u64),
        }
    }

    /// Whether `session` takes checkpoints: the explicit `checkpoints.enabled`, else on unless the
    /// session runs headless (the engine process, or a session the gateway marked unattended).
    pub fn enabled_for(&self, session: Option<&str>) -> bool {
        let factr = match &self.config_dir {
            Some(dir) => crate::factr_config::load_from(dir),
            None => crate::factr_config::current(),
        };
        factr.checkpoints.enabled.unwrap_or_else(|| {
            !session.map_or_else(crate::headless::process, crate::headless::is)
        })
    }

    // ---- git plumbing ----

    fn git(&self, workdir: &Path, index: Option<&Path>, envs: &[(&str, &str)], args: &[&str]) -> Result<String, String> {
        let mut c = Command::new("git");
        c.args(["-c", "core.fsmonitor=false", "-c", "core.hooksPath=/dev/null", "-c", "core.attributesFile=/dev/null", "-c", "core.quotePath=false"]);
        c.args(args).current_dir(workdir);
        for k in ["GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE", "GIT_NAMESPACE", "GIT_ALTERNATE_OBJECT_DIRECTORIES"] {
            c.env_remove(k);
        }
        c.env("GIT_DIR", self.git_dir()).env("GIT_WORK_TREE", workdir);
        if let Some(i) = index {
            c.env("GIT_INDEX_FILE", i);
        }
        c.env("GIT_CONFIG_GLOBAL", "/dev/null").env("GIT_CONFIG_SYSTEM", "/dev/null").env("GIT_CONFIG_NOSYSTEM", "1").env("GIT_ATTR_NOSYSTEM", "1");
        c.env("GIT_AUTHOR_NAME", "Factr-I checkpoint").env("GIT_AUTHOR_EMAIL", "checkpoint@localhost");
        c.env("GIT_COMMITTER_NAME", "Factr-I checkpoint").env("GIT_COMMITTER_EMAIL", "checkpoint@localhost");
        for (k, v) in envs {
            c.env(k, v);
        }
        let out = c.stdin(Stdio::null()).output().map_err(|e| e.to_string())?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        } else {
            Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
        }
    }

    fn init(&self) -> Result<(), String> {
        let dir = self.git_dir();
        if dir.join("HEAD").exists() {
            return Ok(());
        }
        for d in ["indexes", "ledgers"] {
            std::fs::create_dir_all(self.base.join(d)).map_err(|e| e.to_string())?;
        }
        let out = Command::new("git").args(["init", "--bare", "-q"]).arg(&dir).env_remove("GIT_DIR").output().map_err(|e| e.to_string())?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).into_owned());
        }
        std::fs::create_dir_all(dir.join("info")).map_err(|e| e.to_string())?;
        std::fs::write(dir.join("info/exclude"), DEFAULT_EXCLUDES.join("\n") + "\n").map_err(|e| e.to_string())?;
        for (k, v) in [("gc.auto", "0"), ("core.autocrlf", "false"), ("commit.gpgsign", "false"), ("core.bare", "false")] {
            self.git(&self.base, None, &[], &["config", k, v])?;
        }
        Ok(())
    }

    /// One ref per snapshot, `refs/factr/<project>/snap/<seq>`: the commits are parentless, so
    /// dropping a ref never rewrites another commit and every hash stays valid for as long as some
    /// ref (a snap ref or a pin) points at it.
    fn snap_prefix(dir: &Path) -> String {
        format!("refs/factr/{}/snap/", project_hash(dir))
    }

    fn pin_ref(dir: &Path, session: &str, hash: &str) -> String {
        format!("refs/factr/{}/pins/{}/{hash}", project_hash(dir), ref_safe(session))
    }

    fn index_path(&self, dir: &Path) -> PathBuf {
        self.base.join("indexes").join(project_hash(dir))
    }

    /// `(refname, hash)` of the project's snapshots, oldest first.
    fn snap_refs(&self, dir: &Path) -> Vec<(String, String)> {
        let out = self.git(dir, None, &[], &["for-each-ref", "--format=%(refname)%1f%(objectname)", &Self::snap_prefix(dir)]).unwrap_or_default();
        out.lines().filter_map(|l| l.split_once('\x1f').map(|(r, h)| (r.to_string(), h.to_string()))).collect()
    }

    fn tip(&self, dir: &Path) -> Option<String> {
        self.snap_refs(dir).pop().map(|(_, h)| h)
    }

    /// Point the next free `snap/<seq>` ref at `hash` (retries when another process took the number).
    fn add_snap_ref(&self, dir: &Path, hash: &str) -> Result<(), String> {
        let mut last = String::new();
        for _ in 0..8 {
            let seq = self.snap_refs(dir).last().and_then(|(r, _)| r.rsplit('/').next()?.parse::<u64>().ok()).map_or(1, |n| n + 1);
            let name = format!("{}{seq:012}", Self::snap_prefix(dir));
            match self.git(dir, None, &[], &["update-ref", &name, hash, ""]) {
                Ok(_) => return Ok(()),
                Err(e) => last = e,
            }
        }
        Err(last)
    }

    /// The project's newest checkpoint (its tip), if any.
    pub fn head(&self, dir: &Path) -> Option<String> {
        let dir = canonical(dir);
        self.tip(&dir)
    }

    /// Keep `hash` alive for `session` under its own ref: pruning, size caps and `gc` never
    /// delete a pinned commit. `false` when the commit no longer exists.
    pub fn pin(&self, dir: &Path, session: &str, hash: &str) -> bool {
        let dir = canonical(dir);
        let _g = STORE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        self.pin_locked(&dir, session, hash)
    }

    fn pin_locked(&self, dir: &Path, session: &str, hash: &str) -> bool {
        self.git(dir, None, &[], &["update-ref", &Self::pin_ref(dir, session, hash), hash]).is_ok()
    }

    /// Release one pin (no-op when it does not exist).
    pub fn unpin(&self, dir: &Path, session: &str, hash: &str) {
        let dir = canonical(dir);
        let _g = STORE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        if self.git_dir().join("HEAD").exists() {
            let _ = self.git(&dir, None, &[], &["update-ref", "-d", &Self::pin_ref(&dir, session, hash)]);
        }
    }

    /// The commits `session` pins in `dir`'s project, newest first.
    pub fn pinned(&self, dir: &Path, session: &str) -> Vec<Checkpoint> {
        let dir = canonical(dir);
        let prefix = format!("refs/factr/{}/pins/{}/", project_hash(&dir), ref_safe(session));
        let out = self.git(&dir, None, &[], &["for-each-ref", "--sort=-committerdate", "--format=%(objectname)%1f%(committerdate:iso-strict)%1f%(subject)", &prefix]).unwrap_or_default();
        out.lines().filter_map(|l| {
            let mut f = l.splitn(3, '\x1f');
            Some(Checkpoint { hash: f.next()?.into(), timestamp: f.next()?.into(), message: f.next()?.into() })
        }).collect()
    }

    /// The undo dir of this store: `undo` beside its `checkpoints` dir (`$FACTR_HOME/undo` for the default
    /// store). The only place that builds this path; everything undo keeps for a session lives here.
    pub fn undo_base(&self) -> PathBuf {
        self.base.parent().unwrap_or(&self.base).join("undo")
    }

    /// Where the bare store lives (`git --git-dir=<this>` reads its checkpoints).
    pub fn git_dir_path(&self) -> PathBuf {
        self.git_dir()
    }

    /// Make the session's pins exactly `wanted` (`(project dir, commit)`): add the missing ones,
    /// drop the rest. Returns how many pins the session holds afterwards.
    pub fn sync_pins(&self, session: &str, wanted: &[(PathBuf, String)]) -> usize {
        if !self.git_dir().join("HEAD").exists() {
            return 0;
        }
        let _g = STORE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let safe = ref_safe(session);
        let mut want: BTreeMap<String, (PathBuf, String)> = BTreeMap::new();
        for (dir, hash) in wanted {
            let dir = canonical(dir);
            want.insert(Self::pin_ref(&dir, session, hash), (dir, hash.clone()));
        }
        let have = self.git(&self.base, None, &[], &["for-each-ref", "--format=%(refname)", &format!("refs/factr/*/pins/{safe}/*")]).unwrap_or_default();
        let have: Vec<&str> = have.lines().filter(|r| r.split('/').nth(4) == Some(safe.as_str())).collect();
        for r in &have {
            if !want.contains_key(*r) {
                let _ = self.git(&self.base, None, &[], &["update-ref", "-d", r]);
            }
        }
        let mut held = 0;
        for (r, (dir, hash)) in &want {
            if have.contains(&r.as_str()) || self.pin_locked(dir, session, hash) {
                held += 1;
            }
        }
        held
    }

    /// How many commits `session` currently pins.
    pub fn pin_count(&self, session: &str) -> usize {
        let safe = ref_safe(session);
        let have = self.git(&self.base, None, &[], &["for-each-ref", "--format=%(refname)", &format!("refs/factr/*/pins/{safe}/*")]).unwrap_or_default();
        have.lines().filter(|r| r.split('/').nth(4) == Some(safe.as_str())).count()
    }

    /// Delete every unreferenced object now (what the size cap does; tests use it to prove pins hold).
    pub fn gc_now(&self) {
        let _g = STORE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let _ = self.git(&self.base, None, &[], &["gc", "--prune=now", "-q"]);
    }

    /// Whether the store holds the commit `hash` for `dir`'s project (a snapshot or a pin).
    pub fn has_commit(&self, dir: &Path, hash: &str) -> bool {
        hash.len() == 40 && hash.chars().all(|c| c.is_ascii_hexdigit()) && self.git_dir().join("HEAD").exists() && self.known_commit(&canonical(dir), hash)
    }

    /// Whether `hash` is one of the project's snapshots or something pinned in it.
    fn known_commit(&self, dir: &Path, hash: &str) -> bool {
        let prefix = format!("refs/factr/{}/", project_hash(dir));
        self.git(dir, None, &[], &["for-each-ref", "--count=1", "--format=%(refname)", "--points-at", hash, &prefix]).is_ok_and(|o| !o.trim().is_empty())
    }

    /// Whether the agent ever recorded a write in `dir` (safe-mode restores need one).
    pub fn has_agent_writes(&self, dir: &Path) -> bool {
        !self.load_ledger(&canonical(dir)).is_empty()
    }

    fn too_many_files(dir: &Path) -> bool {
        let (mut n, mut stack) = (0usize, vec![dir.to_path_buf()]);
        while let Some(d) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&d) else { continue };
            for e in rd.flatten() {
                let Ok(t) = e.file_type() else { continue };
                if t.is_dir() {
                    if !SKIP_DIRS.contains(&e.file_name().to_string_lossy().as_ref()) {
                        stack.push(e.path());
                    }
                } else {
                    n += 1;
                    if n > MAX_FILES {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Stage the whole working tree into the project's private index, minus oversize files.
    fn stage(&self, dir: &Path, cfg: &Config) -> Result<PathBuf, String> {
        self.init()?;
        if Self::too_many_files(dir) {
            return Err(format!("more than {MAX_FILES} files"));
        }
        let index = self.index_path(dir);
        if !index.exists() {
            if let Some(tip) = self.tip(dir) {
                let _ = self.git(dir, Some(&index), &[], &["read-tree", &tip]);
            }
        }
        // add's exit status is non-zero when one file is unreadable; the rest is staged regardless.
        let added = self.git(dir, Some(&index), &[], &["add", "-A", "--ignore-errors"]);
        if cfg.max_file_size_mb > 0 {
            let listing = self.git(dir, Some(&index), &[], &["ls-files", "--cached", "-z"]).unwrap_or_default();
            let cap = cfg.max_file_size_mb * MB;
            let big: Vec<&str> = listing.split('\0').filter(|r| !r.is_empty() && std::fs::metadata(dir.join(r)).is_ok_and(|m| m.len() > cap)).collect();
            for chunk in big.chunks(200) {
                let mut args = vec!["rm", "--cached", "--quiet", "--"];
                args.extend(chunk);
                let _ = self.git(dir, Some(&index), &[], &args);
            }
        }
        if let Err(e) = added {
            // Only fatal when nothing could be staged at all.
            self.git(dir, Some(&index), &[], &["write-tree"]).map_err(|_| e)?;
        }
        Ok(index)
    }

    /// Snapshot `dir`. `Ok(None)` when the tree matches the newest checkpoint (no new commit).
    pub fn snapshot(&self, dir: &Path, reason: &str) -> Result<Option<String>, String> {
        self.snapshot_for(dir, reason, None)
    }

    /// [`Self::snapshot`] attributed to a session in its `checkpoint.snapshot` span.
    pub fn snapshot_for(&self, dir: &Path, reason: &str, session: Option<&str>) -> Result<Option<String>, String> {
        let started = std::time::Instant::now();
        let out = {
            let _g = STORE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
            self.snapshot_locked(dir, reason)
        };
        let mut span = Span::new("checkpoint.snapshot").attr("reason", reason_class(reason)).took_ms(started.elapsed().as_millis() as u64);
        span = match &out {
            Ok(made) => span.attr("made", made.is_some()),
            Err(_) => span.error("snapshot failed"),
        };
        if let Some(s) = session {
            span = span.session(s);
        }
        obs_sink::emit(span);
        out
    }

    fn snapshot_locked(&self, dir: &Path, reason: &str) -> Result<Option<String>, String> {
        let dir = &canonical(dir);
        let cfg = self.config();
        self.init()?;
        let index = self.stage(dir, &cfg)?;
        let tree = self.git(dir, Some(&index), &[], &["write-tree"])?.trim().to_string();
        let tip = self.tip(dir);
        if let Some(tip) = &tip {
            if self.git(dir, None, &[], &["rev-parse", &format!("{tip}^{{tree}}")]).is_ok_and(|t| t.trim() == tree) {
                return Ok(None);
            }
        }
        let mut message = reason.lines().next().unwrap_or("auto").to_string();
        if let Some(tip) = &tip {
            let changed = self.git(dir, None, &[], &["diff-tree", "-r", "--name-only", "--no-renames", "-z", tip, &tree]).unwrap_or_default();
            let names: Vec<&str> = changed.split('\0').filter(|n| !n.is_empty()).collect();
            if !names.is_empty() {
                let more = if names.len() > 3 { format!(", +{} more", names.len() - 3) } else { String::new() };
                message = format!("{message} (changed since last: {}{more})", names[..names.len().min(3)].join(", "));
            }
        }
        let sha = self.git(dir, None, &[], &["commit-tree", tree.as_str(), "-m", message.as_str()])?.trim().to_string();
        self.add_snap_ref(dir, &sha)?;
        self.prune(dir, cfg.max_snapshots);
        self.enforce_size_cap(dir, &cfg);
        Ok(Some(sha))
    }

    /// Drop the oldest snapshot refs beyond `keep`. No commit is rewritten, so surviving hashes
    /// never change; a commit someone pinned stays reachable through its pin.
    fn prune(&self, dir: &Path, keep: usize) {
        let refs = self.snap_refs(dir);
        let keep = keep.max(1);
        if refs.len() <= keep {
            return;
        }
        for (r, _) in &refs[..refs.len() - keep] {
            let _ = self.git(dir, None, &[], &["update-ref", "-d", r]);
        }
        let _ = self.git(dir, None, &[], &["prune", "--expire=10.minutes.ago", "-q"]);
    }

    fn store_bytes(&self, dir: &Path) -> u64 {
        let out = self.git(dir, None, &[], &["count-objects", "-v"]).unwrap_or_default();
        let kb = |key: &str| out.lines().find_map(|l| l.strip_prefix(key)?.trim().parse::<u64>().ok()).unwrap_or(0);
        (kb("size:") + kb("size-pack:")) * 1024
    }

    /// Over `max_total_size_mb`: gc, then drop the oldest checkpoint of the fattest project until under.
    fn enforce_size_cap(&self, dir: &Path, cfg: &Config) {
        let cap = cfg.max_total_size_mb * MB;
        if cap == 0 || self.store_bytes(dir) <= cap {
            return;
        }
        let _ = self.git(dir, None, &[], &["gc", "--prune=now", "-q"]);
        for _ in 0..200 {
            if self.store_bytes(dir) <= cap {
                return;
            }
            let all = self.git(dir, None, &[], &["for-each-ref", "--format=%(refname)", "refs/factr/*/snap/*"]).unwrap_or_default();
            let mut per_project: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
            for r in all.lines() {
                if let Some(project) = r.split('/').nth(2) {
                    per_project.entry(project).or_default().push(r);
                }
            }
            // Oldest ref (lowest seq) of the project holding the most snapshots, if it has more than one.
            let Some(oldest) = per_project.values().filter(|v| v.len() > 1).max_by_key(|v| v.len()).and_then(|v| v.first().copied()) else { return };
            let _ = self.git(dir, None, &[], &["update-ref", "-d", oldest]);
            let _ = self.git(dir, None, &[], &["gc", "--prune=now", "-q"]);
        }
    }

    // ---- ledger ----

    fn ledger_path(&self, dir: &Path) -> PathBuf {
        self.base.join("ledgers").join(format!("{}.json", project_hash(dir)))
    }

    fn load_ledger(&self, dir: &Path) -> BTreeMap<String, String> {
        std::fs::read_to_string(self.ledger_path(dir)).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    /// Record the content hash of a file the agent just wrote (`""` = deleted by the agent).
    pub fn record_write(&self, dir: &Path, file: &Path) {
        let (dir, file) = (canonical(dir), file.to_path_buf());
        let _g = STORE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let mut ledger = self.load_ledger(&dir);
        let key = canonical(file.parent().unwrap_or(Path::new("/"))).join(file.file_name().unwrap_or_default());
        ledger.insert(key.to_string_lossy().into_owned(), hash_file(&key).unwrap_or_default());
        while ledger.len() > LEDGER_MAX {
            let first = ledger.keys().next().cloned().unwrap_or_default();
            ledger.remove(&first);
        }
        let _ = std::fs::create_dir_all(self.base.join("ledgers"));
        let _ = std::fs::write(self.ledger_path(&dir), serde_json::to_string(&ledger).unwrap_or_default());
    }

    // ---- list / diff / restore ----

    /// Newest first, capped at `max_snapshots`.
    pub fn list(&self, dir: &Path) -> Vec<Checkpoint> {
        let dir = canonical(dir);
        let n = format!("--count={}", self.config().max_snapshots);
        let out = self.git(&dir, None, &[], &["for-each-ref", "--sort=-refname", &n, "--format=%(objectname)%1f%(committerdate:iso-strict)%1f%(subject)", &Self::snap_prefix(&dir)]).unwrap_or_default();
        out.lines().filter_map(|l| {
            let mut f = l.splitn(3, '\x1f');
            Some(Checkpoint { hash: f.next()?.into(), timestamp: f.next()?.into(), message: f.next()?.into() })
        }).collect()
    }

    /// A full hash (also one only pinned, no longer in the list), a hash prefix (>= 4 hex chars)
    /// or a 1-based index into [`Self::list`].
    pub fn resolve(&self, dir: &Path, target: &str) -> Option<String> {
        let list = self.list(dir);
        if let Some(c) = target.parse::<usize>().ok().and_then(|n| list.get(n.checked_sub(1)?)) {
            return Some(c.hash.clone());
        }
        let hex = target.len() >= 4 && target.chars().all(|c| c.is_ascii_hexdigit());
        let target = target.to_ascii_lowercase();
        if let Some(c) = hex.then(|| list.iter().find(|c| c.hash.starts_with(&target))).flatten() {
            return Some(c.hash.clone());
        }
        (hex && target.len() == 40 && self.known_commit(&canonical(dir), &target)).then_some(target)
    }

    /// `{stat, diff, rendered}` between the checkpoint and the working tree (diff capped at 4000 chars).
    pub fn diff(&self, dir: &Path, hash: &str) -> Result<Value, String> {
        let dir = canonical(dir);
        let _g = STORE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let index = self.stage(&dir, &self.config())?;
        let run = |extra: &[&str]| {
            let mut a = vec!["diff", "--cached", "--no-ext-diff", "--no-textconv", "--no-color"];
            a.extend(extra);
            a.push(hash);
            self.git(&dir, Some(&index), &[], &a).unwrap_or_default()
        };
        let (stat, diff) = (run(&["--stat"]), run(&[]).chars().take(4000).collect::<String>());
        let rendered: String = diff.lines().map(|l| {
            let color = match l.chars().next() {
                Some('+') if !l.starts_with("+++") => "32",
                Some('-') if !l.starts_with("---") => "31",
                Some('@') => "36",
                _ if l.starts_with("diff ") || l.starts_with("+++") || l.starts_with("---") => "1",
                _ => return format!("{l}\n"),
            };
            format!("\x1b[{color}m{l}\x1b[0m\n")
        }).collect();
        Ok(json!({ "stat": stat.trim_end(), "diff": diff, "rendered": rendered }))
    }

    /// Restore the tree (or one file) to checkpoint `hash`. Takes a pre-rollback snapshot first.
    /// `safe` (full restore only): paths the agent did not last write are left alone.
    pub fn restore(&self, dir: &Path, hash: &str, file: Option<&str>, safe: bool) -> Value {
        self.restore_for(dir, hash, file, safe, None)
    }

    /// [`Self::restore`] attributed to a session in its `checkpoint.restore` span (counts only).
    pub fn restore_for(&self, dir: &Path, hash: &str, file: Option<&str>, safe: bool, session: Option<&str>) -> Value {
        let started = std::time::Instant::now();
        let out = self.restore_inner(dir, hash, file, safe);
        let count = |k: &str| out[k].as_array().map_or(0, Vec::len);
        let mut span = Span::new("checkpoint.restore")
            .attr("scope", if file.is_some() { "file" } else { "tree" })
            .attr("safe", safe)
            .attr("restored", count("restored_files"))
            .attr("skipped_user_edits", count("skipped_user_edits"))
            .attr("skipped_oversize", count("skipped_oversize"))
            .attr("failed", count("failed_deletes"))
            .took_ms(started.elapsed().as_millis() as u64);
        if out["success"] != true {
            span = span.error("restore failed");
        }
        if let Some(s) = session {
            span = span.session(s);
        }
        obs_sink::emit(span);
        out
    }

    fn restore_inner(&self, dir: &Path, hash: &str, file: Option<&str>, safe: bool) -> Value {
        let dir = canonical(dir);
        let fail = |e: String| json!({ "success": false, "error": e });
        let _g = STORE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let cfg = self.config();
        let rel_file = match file {
            Some(f) => {
                let p = Path::new(f);
                let rel = p.strip_prefix(&dir).unwrap_or(p);
                if rel.is_absolute() || rel.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
                    return fail("file_path must be inside the working directory".into());
                }
                Some(rel.to_string_lossy().into_owned())
            }
            None => None,
        };
        // The snapshot also leaves the private index describing the current tree.
        let short = &hash[..hash.len().min(8)];
        if let Err(e) = self.snapshot_locked(&dir, &format!("pre-rollback snapshot (restoring to {short})")) {
            return fail(format!("could not snapshot before restoring: {e}"));
        }
        let index = self.index_path(&dir);
        let names = match self.git(&dir, Some(&index), &[], &["diff", "--cached", "--name-status", "--no-renames", "-z", hash]) {
            Ok(n) => n,
            Err(e) => return fail(format!("could not compute changed files: {e}")),
        };
        let mut parts = names.split('\0').filter(|s| !s.is_empty());
        let mut changes: Vec<(char, String)> = Vec::new();
        while let (Some(st), Some(path)) = (parts.next(), parts.next()) {
            changes.push((st.chars().next().unwrap_or('M'), path.to_string()));
        }
        if let Some(f) = &rel_file {
            changes.retain(|(_, p)| p == f || p.starts_with(&format!("{}/", f.trim_end_matches('/'))));
        }
        let ledger = self.load_ledger(&dir);
        let planned = safe && file.is_none() && !ledger.is_empty();
        let (mut restore, mut skipped_user, mut skipped_big) = (Vec::<(char, String)>::new(), Vec::new(), Vec::new());
        for (st, rel) in changes {
            let abs = dir.join(&rel);
            // Agent-authored: recorded, and still what the agent wrote (or gone). A file that is
            // missing now but was in the checkpoint is always safe to bring back.
            let authored = st == 'D' || ledger.get(&abs.to_string_lossy().into_owned()).is_some_and(|rec| hash_file(&abs).is_none_or(|h| &h == rec));
            if planned && !authored {
                skipped_user.push(rel);
            } else if st == 'A' && cfg.max_file_size_mb > 0 && std::fs::metadata(&abs).is_ok_and(|m| m.len() > cfg.max_file_size_mb * MB) {
                // Kept out of every checkpoint: no prior copy exists, deleting would lose it.
                skipped_big.push(rel);
            } else {
                restore.push((st, rel));
            }
        }
        let (mut checkout, mut restored, mut failed) = (Vec::<String>::new(), Vec::<String>::new(), Vec::<String>::new());
        for (st, rel) in &restore {
            if *st == 'A' {
                let abs = dir.join(rel);
                let _ = std::fs::remove_file(&abs);
                if abs.exists() {
                    failed.push(rel.clone());
                    continue;
                }
                restored.push(rel.clone());
                let mut p = abs.parent();
                while let Some(d) = p.filter(|d| *d != dir && d.starts_with(&dir)) {
                    if std::fs::remove_dir(d).is_err() {
                        break;
                    }
                    p = d.parent();
                }
            } else {
                checkout.push(rel.clone());
            }
        }
        for chunk in checkout.chunks(200) {
            let mut args = vec!["checkout", hash, "--"];
            args.extend(chunk.iter().map(String::as_str));
            if let Err(e) = self.git(&dir, Some(&index), &[], &args) {
                return fail(format!("restore failed: {e}"));
            }
            restored.extend(chunk.iter().cloned());
        }
        if !restored.is_empty() {
            let mut ledger = ledger;
            for rel in &restored {
                ledger.remove(&dir.join(rel).to_string_lossy().into_owned());
            }
            let _ = std::fs::write(self.ledger_path(&dir), serde_json::to_string(&ledger).unwrap_or_default());
        }
        let reason = self.git(&dir, None, &[], &["log", "-1", "--format=%s", hash]).unwrap_or_default().trim().to_string();
        let mut out = json!({
            "success": true, "restored_to": hash, "reason": reason, "directory": dir.to_string_lossy(),
            "restored_files": restored, "skipped_user_edits": skipped_user, "skipped_oversize": skipped_big,
            "history_removed": 0,
        });
        if let Some(f) = file {
            out["file"] = json!(f);
        }
        if !failed.is_empty() {
            out["failed_deletes"] = json!(failed);
        }
        out
    }
}


pub fn eligible(dir: &Path) -> bool {
    let d = canonical(dir);
    d.is_dir() && d.parent().is_some() && dirs::home_dir().is_none_or(|h| canonical(&h) != d)
}

/// `RFC 3339` UTC timestamp with milliseconds: the format effects and turn markers share.
pub fn now_stamp() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}


#[cfg(test)]
mod tests {
    use super::*;

    struct Env {
        _tmp: tempfile::TempDir,
        store: Store,
        work: PathBuf,
    }

    fn env() -> Env {
        let tmp = tempfile::tempdir().unwrap();
        let work = tmp.path().join("work");
        std::fs::create_dir_all(&work).unwrap();
        let work = work.canonicalize().unwrap();
        let store = Store::at(tmp.path().join("store")).with_config_dir(tmp.path().join("factr"));
        Env { _tmp: tmp, store, work }
    }

    /// Write the `checkpoints:` block of this store's `config.yaml` (what Settings does).
    fn settings(e: &Env, lines: &str) {
        let dir = e._tmp.path().join("factr");
        std::fs::create_dir_all(&dir).unwrap();
        let body: String = lines.lines().map(|l| format!("  {l}\n")).collect();
        std::fs::write(dir.join("config.yaml"), format!("checkpoints:\n{body}")).unwrap();
    }

    #[test]
    fn settings_come_from_config_yaml_with_defaults_and_floors() {
        let _guard = crate::storage::lock_test_env();
        let e = env();
        assert_eq!(e.store.config(), Config::default());
        settings(&e, "enabled: false\nmax_snapshots: 0\nmax_total_size_mb: 9\nmax_file_size_mb: -4");
        assert_eq!(e.store.config(), Config { enabled: false, max_total_size_mb: 9, ..Config::default() }, "a floor of 1 snapshot; negatives ignored");
        assert!(e.store.snapshot(&e.work, "x").is_ok());
        assert!(!e.store.git_dir().join("config.json").exists() && !e.store.base.join("config.json").exists(), "no private config file");
    }

    #[test]
    fn the_default_follows_the_run_mode_and_an_explicit_setting_wins() {
        let _guard = crate::storage::lock_test_env();
        let e = env();
        let prev = std::env::var_os("FACTR_HEADLESS");
        // Interactive: on.
        crate::env::remove_var("FACTR_HEADLESS");
        assert!(e.store.config().enabled && e.store.enabled_for(None) && e.store.enabled_for(Some("desk")));
        // A session the gateway marked unattended (`/api/agent/run`, cron): off, others stay on.
        crate::headless::mark("cron-1");
        assert!(!e.store.enabled_for(Some("cron-1")) && e.store.enabled_for(Some("desk")));
        crate::headless::unmark("cron-1");
        // A headless engine process (`FACTR_HEADLESS=1`): off for every session.
        crate::env::set_var("FACTR_HEADLESS", "1");
        assert!(!e.store.config().enabled && !e.store.enabled_for(None) && !e.store.enabled_for(Some("desk")));
        // An explicit `checkpoints.enabled: true` turns it on even when headless; `false` turns it off.
        settings(&e, "enabled: true");
        assert!(e.store.config().enabled && e.store.enabled_for(Some("desk")));
        settings(&e, "enabled: false");
        crate::env::remove_var("FACTR_HEADLESS");
        assert!(!e.store.config().enabled && !e.store.enabled_for(Some("desk")));
        if let Some(prev) = prev {
            crate::env::set_var("FACTR_HEADLESS", prev);
        }
    }

    fn w(e: &Env, f: &str, c: &str) {
        let p = e.work.join(f);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, c).unwrap();
    }

    fn r(e: &Env, f: &str) -> String {
        std::fs::read_to_string(e.work.join(f)).unwrap_or_default()
    }

    #[test]
    fn snapshot_then_restore_brings_the_file_back_and_deletes_agent_created_files() {
        let e = env(); // not a git repo
        w(&e, "a.txt", "one");
        e.store.snapshot(&e.work, "before edit a.txt").unwrap().unwrap();
        w(&e, "a.txt", "two");
        w(&e, "sub/new.txt", "fresh");
        e.store.record_write(&e.work, &e.work.join("a.txt"));
        e.store.record_write(&e.work, &e.work.join("sub/new.txt"));
        let first = e.store.list(&e.work).last().unwrap().hash.clone();
        let out = e.store.restore(&e.work, &first, None, true);
        assert_eq!(out["success"], true, "{out}");
        assert_eq!(r(&e, "a.txt"), "one");
        assert!(!e.work.join("sub/new.txt").exists() && !e.work.join("sub").exists());
        assert_eq!(out["history_removed"], 0);
    }

    #[test]
    fn safe_mode_skips_files_the_user_edited_after_the_agent_write() {
        let e = env();
        w(&e, "a.txt", "one");
        w(&e, "b.txt", "bee");
        e.store.snapshot(&e.work, "start").unwrap();
        let first = e.store.list(&e.work)[0].hash.clone();
        w(&e, "a.txt", "agent");
        w(&e, "b.txt", "agent-b");
        e.store.record_write(&e.work, &e.work.join("a.txt"));
        e.store.record_write(&e.work, &e.work.join("b.txt"));
        w(&e, "b.txt", "user hand edit");
        let out = e.store.restore(&e.work, &first, None, true);
        assert_eq!(r(&e, "a.txt"), "one");
        assert_eq!(r(&e, "b.txt"), "user hand edit");
        assert_eq!(out["skipped_user_edits"], json!(["b.txt"]));
        assert_eq!(out["restored_files"], json!(["a.txt"]));
        // unsafe mode overwrites everything
        e.store.restore(&e.work, &first, None, false);
        assert_eq!(r(&e, "b.txt"), "bee");
    }

    #[test]
    fn one_file_restore_touches_only_that_file() {
        let e = env();
        w(&e, "a.txt", "one");
        w(&e, "b.txt", "bee");
        e.store.snapshot(&e.work, "start").unwrap();
        let first = e.store.list(&e.work)[0].hash.clone();
        w(&e, "a.txt", "x");
        w(&e, "b.txt", "y");
        let out = e.store.restore(&e.work, &first, Some("a.txt"), true);
        assert_eq!((r(&e, "a.txt"), r(&e, "b.txt")), ("one".into(), "y".into()));
        assert_eq!(out["file"], "a.txt");
        assert_eq!(e.store.restore(&e.work, &first, Some("../x"), true)["success"], false);
    }

    #[test]
    fn unchanged_tree_makes_no_new_commit_and_list_is_newest_first() {
        let e = env();
        w(&e, "a.txt", "one");
        assert!(e.store.snapshot(&e.work, "first").unwrap().is_some());
        assert!(e.store.snapshot(&e.work, "again").unwrap().is_none());
        w(&e, "a.txt", "two");
        assert!(e.store.snapshot(&e.work, "second").unwrap().is_some());
        let list = e.store.list(&e.work);
        assert_eq!(list.len(), 2);
        assert!(list[0].message.starts_with("second") && list[0].message.contains("a.txt"), "{}", list[0].message);
        assert_eq!(list[1].message, "first");
        assert_eq!(e.store.resolve(&e.work, "2").unwrap(), list[1].hash);
        assert_eq!(e.store.resolve(&e.work, &list[0].hash[..8]).unwrap(), list[0].hash);
    }

    #[test]
    fn pruning_keeps_the_newest_n() {
        let e = env();
        settings(&e, "max_snapshots: 3");
        for i in 0..6 {
            w(&e, "a.txt", &format!("v{i}"));
            e.store.snapshot(&e.work, &format!("s{i}")).unwrap();
        }
        let list = e.store.list(&e.work);
        assert_eq!(list.len(), 3);
        assert!(list[0].message.starts_with("s5") && list[2].message.starts_with("s3"));
        assert_eq!(e.store.snap_refs(&e.work).len(), 3, "older snapshot refs are gone, not just hidden");
        // the kept oldest still restores
        let out = e.store.restore(&e.work, &list[2].hash, None, false);
        assert_eq!((out["success"].as_bool(), r(&e, "a.txt")), (Some(true), "v3".into()));
    }

    #[test]
    fn pruning_never_changes_hashes_and_pinned_commits_survive_gc() {
        let e = env();
        let mut hashes = Vec::new();
        let mut pinned = None;
        for i in 0..25 {
            w(&e, "a.txt", &format!("v{i}"));
            let h = e.store.snapshot(&e.work, &format!("s{i}")).unwrap().unwrap();
            if i == 0 {
                assert!(e.store.pin(&e.work, "sess", &h));
                pinned = Some(h.clone());
            }
            hashes.push(h);
            let listed: Vec<String> = e.store.list(&e.work).into_iter().map(|c| c.hash).collect();
            let expect: Vec<String> = hashes.iter().rev().take(20).cloned().collect();
            assert_eq!(listed, expect, "listed hashes are exactly the newest 20, unchanged by every prune (i={i})");
        }
        let oldest = pinned.unwrap();
        assert!(!e.store.list(&e.work).iter().any(|c| c.hash == oldest), "the pinned commit left the list");
        e.store.git(&e.work, None, &[], &["gc", "--prune=now", "-q"]).unwrap();
        assert!(e.store.resolve(&e.work, &oldest).is_some(), "a pinned full hash still resolves");
        let out = e.store.restore(&e.work, &oldest, None, false);
        assert_eq!((out["success"].as_bool(), r(&e, "a.txt")), (Some(true), "v0".into()), "{out}");
        // unpinned and pruned commits are gone for good after gc
        let gone = &hashes[1];
        assert!(e.store.git(&e.work, None, &[], &["cat-file", "-e", gone]).is_err());
        // dropping the pin lets the commit go
        assert_eq!(e.store.sync_pins("sess", &[]), 0);
        e.store.git(&e.work, None, &[], &["gc", "--prune=now", "-q"]).unwrap();
        assert!(e.store.git(&e.work, None, &[], &["cat-file", "-e", &oldest]).is_err());
    }

    #[test]
    fn sync_pins_adds_missing_and_drops_stale_per_session() {
        let e = env();
        w(&e, "a.txt", "1");
        let a = e.store.snapshot(&e.work, "a").unwrap().unwrap();
        w(&e, "a.txt", "2");
        let b = e.store.snapshot(&e.work, "b").unwrap().unwrap();
        assert_eq!(e.store.sync_pins("s1", &[(e.work.clone(), a.clone()), (e.work.clone(), b.clone())]), 2);
        assert_eq!(e.store.sync_pins("s2", &[(e.work.clone(), a.clone())]), 1);
        assert_eq!(e.store.sync_pins("s1", &[(e.work.clone(), b.clone())]), 1);
        let pins = e.store.git(&e.work, None, &[], &["for-each-ref", "--format=%(refname)", "refs/factr/*/pins/*/*"]).unwrap();
        assert_eq!(pins.lines().count(), 2, "{pins}");
        assert!(pins.contains("/s2/") && pins.contains(&b));
    }

    #[test]
    fn a_store_outside_the_factr_dir_keeps_its_undo_state_beside_itself() {
        let e = env();
        // `env()` roots the store in a temp dir, nowhere near the factr dir.
        let base = e.store.undo_base();
        assert_eq!(base, e._tmp.path().join("undo"), "beside the store, not in $FACTR_HOME");
        note_snapshot_in(&base, "s1", &e.work, "abc123");
        assert!(undo_file(&base, "s1", "snaps.json").exists());
        assert_eq!(snapshots_of(&base, "s1").len(), 1);
    }

    #[test]
    fn excludes_and_oversize_files_stay_out() {
        let e = env();
        w(&e, "keep.txt", "k");
        w(&e, "node_modules/x/index.js", "n");
        w(&e, ".env", "SECRET=1");
        w(&e, "debug.log", "l");
        w(&e, "big.dat", &"x".repeat(2 * 1024 * 1024));
        settings(&e, "max_file_size_mb: 1");
        e.store.snapshot(&e.work, "s").unwrap();
        let h = e.store.list(&e.work)[0].hash.clone();
        let files = e.store.git(&e.work, None, &[], &["ls-tree", "-r", "--name-only", &h]).unwrap();
        assert_eq!(files.trim(), "keep.txt");
        // restore never deletes the oversize file it has no copy of
        w(&e, "keep.txt", "changed");
        e.store.record_write(&e.work, &e.work.join("big.dat"));
        e.store.record_write(&e.work, &e.work.join("keep.txt"));
        let out = e.store.restore(&e.work, &h, None, true);
        assert!(e.work.join("big.dat").exists() && e.work.join(".env").exists());
        assert_eq!(out["skipped_oversize"], json!([]), "big.dat is not even a change: it was never staged");
        assert_eq!(r(&e, "keep.txt"), "k");
    }

    #[test]
    fn pre_rollback_snapshot_lets_you_undo_the_restore() {
        let e = env();
        w(&e, "a.txt", "one");
        e.store.snapshot(&e.work, "start").unwrap();
        let first = e.store.list(&e.work)[0].hash.clone();
        w(&e, "a.txt", "two");
        e.store.restore(&e.work, &first, None, false);
        assert_eq!(r(&e, "a.txt"), "one");
        let list = e.store.list(&e.work);
        assert!(list[0].message.starts_with("pre-rollback snapshot"), "{}", list[0].message);
        e.store.restore(&e.work, &list[0].hash, None, false);
        assert_eq!(r(&e, "a.txt"), "two");
    }

    #[test]
    fn diff_has_stat_diff_and_rendered_and_never_touches_the_users_repo() {
        let e = env();
        let git = |a: &[&str]| assert!(Command::new("git").args(a).current_dir(&e.work).output().unwrap().status.success());
        git(&["init", "-q"]);
        w(&e, "a.txt", "one\n");
        e.store.snapshot(&e.work, "start").unwrap();
        let first = e.store.list(&e.work)[0].hash.clone();
        w(&e, "a.txt", "two\n");
        let d = e.store.diff(&e.work, &first).unwrap();
        assert!(d["stat"].as_str().unwrap().contains("a.txt"));
        assert!(d["diff"].as_str().unwrap().contains("-one") && d["rendered"].as_str().unwrap().contains("\x1b[32m+two"));
        let status = Command::new("git").args(["status", "--porcelain"]).current_dir(&e.work).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&status.stdout).trim(), "?? a.txt", "the user's index and HEAD are untouched");
    }
}
