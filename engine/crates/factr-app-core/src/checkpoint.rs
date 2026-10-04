//! Engine hooks of the file checkpoints: tools call [`before_change`] / [`before_bash`] /
//! [`after_write`], the store itself is `factr_base::checkpoint_store` (re-exported here).
//!
//! Triggers: [`before_change`] (edit/write/apply_patch, before the first change) and
//! [`before_bash`] (destructive commands), each at most once per directory per turn;
//! [`new_turn`] resets that. [`after_write`] records the ledger entry.

pub use factr_base::checkpoint_store::{
    Checkpoint, Config, SnapEntry, Store, canonical, clear_engine_stack, eligible, first_snapshot_since, note_snapshot_in, now_stamp, purge_undo_files,
    retain_snapshots_since, safe_name, snapshots_of, undo_dir, undo_file,
};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

// ---- engine hooks ----

#[derive(Default)]
struct Turns(HashMap<String, HashSet<PathBuf>>);
static TURNS: Mutex<Option<Turns>> = Mutex::new(None);

/// Stores injected for tests, by session id (see [`use_store_for_test`]).
static TEST_STORES: Mutex<Option<HashMap<String, PathBuf>>> = Mutex::new(None);

/// Point the engine hooks of `session` at the store under `base` (tests only: without this, a
/// unit test of an edit/write tool never touches a checkpoint store, so no test writes into a real
/// `~/.factr/engine`). Production code never calls it.
#[doc(hidden)]
pub fn use_store_for_test(session: &str, base: PathBuf) {
    TEST_STORES.lock().unwrap_or_else(|p| p.into_inner()).get_or_insert_with(Default::default).insert(session.to_string(), base);
}

/// The store the tool hooks use for `session`: an injected one, else (outside unit tests of this
/// crate) `$FACTR_HOME/checkpoints`.
fn hook_store(session: &str) -> Option<Store> {
    if let Some(base) = TEST_STORES.lock().unwrap_or_else(|p| p.into_inner()).as_ref().and_then(|m| m.get(session)) {
        return Some(Store::at(base.clone()));
    }
    if cfg!(test) {
        return None;
    }
    Store::default_store()
}

/// A new user turn began for `session`: the next change may snapshot each directory again.
pub fn new_turn(session: &str) {
    if let Some(t) = TURNS.lock().unwrap_or_else(|p| p.into_inner()).as_mut() {
        t.0.remove(session);
    }
}


/// Snapshot `dir` at most once per directory per turn. Never fails the tool call.
pub fn ensure_checkpoint(store: &Store, session: &str, dir: &Path, reason: &str) -> bool {
    if !store.enabled_for(Some(session)) || !eligible(dir) {
        return false;
    }
    let first = TURNS.lock().unwrap_or_else(|p| p.into_inner()).get_or_insert_with(Turns::default).0.entry(session.to_string()).or_default().insert(canonical(dir));
    if !first {
        return false;
    }
    match store.snapshot_for(dir, reason, Some(session)) {
        Ok(made) => {
            // The files exactly as they are before this turn's first change: `/undo` finds the
            // turn's starting point here (an unchanged tree is the newest checkpoint already).
            if let Some(hash) = made.clone().or_else(|| store.head(dir)) {
                note_snapshot_in(&store.undo_base(), session, dir, &hash);
                store.pin(dir, session, &hash);
            }
            made.is_some()
        }
        Err(e) => {
            crate::logging::warn(&format!("[checkpoint] skipped in {}: {e}", dir.display()));
            false
        }
    }
}

fn dir_of(ctx: &crate::tool::ToolContext) -> PathBuf {
    ctx.working_dir.clone().or_else(|| std::env::current_dir().ok()).unwrap_or_else(|| PathBuf::from("."))
}

/// Before the first write/edit/apply_patch of a turn: `what` is e.g. `edit src/lib.rs`.
pub async fn before_change(ctx: &crate::tool::ToolContext, what: &str) {
    let (session, dir, reason) = (ctx.session_id.clone(), dir_of(ctx), what.to_string());
    let _ = tokio::task::spawn_blocking(move || hook_store(&session).is_some_and(|s| ensure_checkpoint(&s, &session, &dir, &reason))).await;
}

/// After a successful write: remember what the agent left on disk (safe-mode restore).
pub fn after_write(ctx: &crate::tool::ToolContext, file: &Path) {
    let (dir, abs) = (canonical(&dir_of(ctx)), canonical(file.parent().unwrap_or(Path::new("/"))).join(file.file_name().unwrap_or_default()));
    if !abs.starts_with(&dir) {
        note_effect(&ctx.session_id, &format!("wrote {} (outside the project folder)", abs.display()));
    }
    if let Some(s) = hook_store(&ctx.session_id).filter(|s| s.enabled_for(Some(&ctx.session_id))) {
        s.record_write(&dir_of(ctx), file);
    }
}

/// Whether a shell command may destroy or rewrite files (risk classifier + in-place editors).
pub fn is_destructive(command: &str, dir: &Path) -> bool {
    let ctx = factr_command_risk::RiskContext::from_env(Some(dir.to_path_buf()));
    factr_command_risk::assess(command, &ctx).level != factr_command_risk::RiskLevel::Safe
        || ["sed -i", "git reset", "git clean", "git checkout", "git restore", "git stash", "mv ", "truncate "].iter().any(|p| command.contains(p))
}

pub async fn before_bash(ctx: &crate::tool::ToolContext, command: &str) {
    for what in irreversible_effects(command) {
        note_effect(&ctx.session_id, &what);
    }
    let dir = dir_of(ctx);
    if !hook_store(&ctx.session_id).is_some_and(|s| s.enabled_for(Some(&ctx.session_id))) || !is_destructive(command, &dir) {
        return;
    }
    let short: String = command.chars().take(60).collect();
    before_change(ctx, &format!("before command: {short}")).await;
}

// ---- effects a file rollback cannot undo ----

/// Commands whose effect leaves the machine or the project folder (`/undo` reports them).
pub fn irreversible_effects(command: &str) -> Vec<String> {
    let mut out = Vec::new();
    for segment in command.split(['\n', ';', '|', '&']) {
        let words: Vec<&str> = segment.split_whitespace().collect();
        let has = |w: &str| words.iter().any(|x| *x == w);
        let pair = |a: &str, b: &str| words.windows(2).any(|p| p[0] == a && p[1] == b);
        let what = if pair("git", "push") {
            Some("git push")
        } else if pair("npm", "publish") || pair("cargo", "publish") {
            Some("package publish")
        } else if pair("docker", "push") {
            Some("docker push")
        } else if words.first().is_some_and(|w| ["curl", "wget"].contains(w)) {
            let write_method = words.windows(2).any(|p| ["-X", "--request", "--method"].contains(&p[0]) && !["GET", "HEAD"].contains(&p[1].to_ascii_uppercase().as_str()))
                || words.iter().any(|w| ["-d", "--data", "--data-raw", "--data-binary", "--post-data", "--post-file", "-F", "--form"].contains(w) || w.starts_with("--data="));
            write_method.then_some("network write (curl/wget POST)")
        } else if has("terraform") && (has("apply") || has("destroy")) {
            Some("terraform apply")
        } else {
            None
        };
        if let Some(w) = what {
            if !out.iter().any(|o: &String| o == w) {
                out.push(w.to_string());
            }
        }
    }
    out
}

fn effects_file(base: &Path, session: &str) -> PathBuf {
    undo_file(base, session, "effects.json")
}

/// Remember an irreversible effect of `session` (kept in `$FACTR_HOME/undo`, newest 200).
pub fn note_effect(session: &str, what: &str) {
    if let Some(base) = undo_dir() {
        note_effect_in(&base, session, what);
    }
}

pub fn note_effect_in(base: &Path, session: &str, what: &str) {
    let file = effects_file(base, session);
    let mut all: Vec<(String, String)> = std::fs::read_to_string(&file).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    all.push((now_stamp(), what.to_string()));
    let excess = all.len().saturating_sub(200);
    all.drain(..excess);
    let _ = std::fs::create_dir_all(base);
    let _ = std::fs::write(file, serde_json::to_string(&all).unwrap_or_default());
}

/// Distinct effects noted for `session` at or after `since` (a [`now_stamp`] value).
pub fn effects_since(base: &Path, session: &str, since: &str) -> Vec<String> {
    let all: Vec<(String, String)> = std::fs::read_to_string(effects_file(base, session)).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    let mut out: Vec<String> = Vec::new();
    for (_, what) in all.into_iter().filter(|(at, _)| at.as_str() >= since) {
        if !out.contains(&what) {
            out.push(what);
        }
    }
    out
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

    fn w(e: &Env, f: &str, c: &str) {
        let p = e.work.join(f);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, c).unwrap();
    }

    #[test]
    fn irreversible_commands_are_recognised() {
        assert_eq!(irreversible_effects("cd x && git push origin main"), ["git push"]);
        assert_eq!(irreversible_effects("curl -X POST https://a.b/c -d '{}'"), ["network write (curl/wget POST)"]);
        assert_eq!(irreversible_effects("curl --data x https://a.b"), ["network write (curl/wget POST)"]);
        assert!(irreversible_effects("curl https://a.b && git status && git pull").is_empty());
        assert!(irreversible_effects("curl -X GET https://a.b").is_empty());
        assert_eq!(irreversible_effects("npm publish"), ["package publish"]);
    }

    #[test]
    fn effects_are_kept_per_session_since_a_stamp() {
        let tmp = tempfile::tempdir().unwrap();
        note_effect_in(tmp.path(), "s1", "git push");
        std::thread::sleep(std::time::Duration::from_millis(5));
        let mark = now_stamp();
        std::thread::sleep(std::time::Duration::from_millis(5));
        note_effect_in(tmp.path(), "s1", "docker push");
        note_effect_in(tmp.path(), "s2", "other");
        assert_eq!(effects_since(tmp.path(), "s1", &mark), ["docker push"]);
        assert_eq!(effects_since(tmp.path(), "s1", "").len(), 2);
        assert!(effects_since(tmp.path(), "none", "").is_empty());
    }

    #[test]
    fn settings_default_on_and_follow_config_yaml_and_disabled_skips_snapshots() {
        let e = env();
        let set = |yaml: &str| {
            std::fs::create_dir_all(e._tmp.path().join("factr")).unwrap();
            std::fs::write(e._tmp.path().join("factr/config.yaml"), yaml).unwrap();
        };
        assert_eq!((e.store.config().enabled, e.store.config().max_snapshots), (true, 20));
        set("checkpoints:\n  enabled: false\n  max_snapshots: 7\n");
        assert_eq!((e.store.config().enabled, e.store.config().max_snapshots), (false, 7));
        w(&e, "a.txt", "one");
        assert!(!ensure_checkpoint(&e.store, "s", &e.work, "x"));
        assert!(e.store.list(&e.work).is_empty());
        set("checkpoints:\n  enabled: true\n");
        assert!(ensure_checkpoint(&e.store, "s", &e.work, "x"));
        // once per turn
        w(&e, "a.txt", "two");
        assert!(!ensure_checkpoint(&e.store, "s", &e.work, "x"));
        new_turn("s");
        assert!(ensure_checkpoint(&e.store, "s", &e.work, "x"));
    }

    #[test]
    fn the_turn_snapshot_is_recorded_where_the_store_says_undo_state_lives() {
        let e = env();
        w(&e, "a.txt", "one");
        assert!(ensure_checkpoint(&e.store, "s-undo", &e.work, "x"));
        let base = e.store.undo_base();
        assert!(base.starts_with(e._tmp.path()) && undo_file(&base, "s-undo", "snaps.json").exists());
        assert!(first_snapshot_since(&base, "s-undo", &e.work, "").is_some(), "a reader given the store's base finds it");
    }

    #[test]
    fn destructive_classification() {
        let d = std::env::temp_dir();
        assert!(is_destructive("rm -rf build", &d) && is_destructive("sed -i s/a/b/ f", &d) && is_destructive("git reset --hard", &d));
        assert!(!is_destructive("ls -la", &d) && !is_destructive("cargo test", &d));
    }
}
