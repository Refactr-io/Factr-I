//! `process.*`, `agents.list` and `rollback.*`, answered from the engine's own state
//! (factr's background-task registry, child sessions, goal-ratchet checkpoint refs)
//! so the desktop's status stack never wakes the Python backend.

use super::*;
use factr_base::background::BackgroundTaskManager;
use factr_base::bus::BackgroundTaskStatus;

pub(super) fn handles(method: &str) -> bool {
    matches!(
        method,
        "process.list" | "process.kill" | "process.stop" | "agents.list" | "rollback.list" | "rollback.restore" | "rollback.diff"
    )
}

fn command_of(t: &factr_base::background::TaskStatusFile) -> String {
    t.display_name.clone().unwrap_or_else(|| t.tool_name.clone())
}

fn uptime(started_at: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(started_at).map_or(0, |s| (chrono::Utc::now() - s.to_utc()).num_seconds().max(0))
}

async fn running(mgr: &BackgroundTaskManager, session: Option<&str>) -> Vec<factr_base::background::TaskStatusFile> {
    let mut tasks = mgr.list().await;
    tasks.retain(|t| t.status == BackgroundTaskStatus::Running && session.is_none_or(|s| t.session_id == s));
    tasks
}

pub(super) async fn process_list(mgr: &BackgroundTaskManager, session: &str) -> Value {
    let rows: Vec<Value> = running(mgr, Some(session)).await.iter().map(|t| json!({
        "session_id": t.task_id, "command": command_of(t), "pid": t.pid, "started_at": t.started_at,
        "uptime_seconds": uptime(&t.started_at), "status": "running", "output_preview": "",
        "session_scoped": true, "detached": t.detached, "notify_on_complete": t.notify,
    })).collect();
    json!({ "processes": rows })
}

pub(super) async fn process_kill(mgr: &BackgroundTaskManager, session: &str, id: &str) -> Result<Value, RpcError> {
    if id.is_empty() {
        return Err(RpcError { code: 4012, message: "process_id required".into(), data: None });
    }
    let Some(task) = mgr.status(id).await.filter(|t| t.session_id == session) else {
        return Err(RpcError { code: 4044, message: format!("no such process: {id}"), data: None });
    };
    let status = if task.status != BackgroundTaskStatus::Running {
        "already_exited"
    } else if mgr.cancel(id).await.map_err(RpcError::internal)? {
        "killed"
    } else {
        "error"
    };
    Ok(json!({ "status": status, "session_id": id, "command": command_of(&task), "exit_code": task.exit_code }))
}

pub(super) async fn process_stop(mgr: &BackgroundTaskManager) -> Value {
    let mut killed = 0;
    for t in running(mgr, None).await {
        killed += usize::from(mgr.cancel(&t.task_id).await.unwrap_or(false));
    }
    json!({ "killed": killed })
}

#[cfg(test)]
fn git(cwd: &str, args: &[&str]) -> Option<String> {
    git_capped(cwd, args, usize::MAX)
}

/// Run git with the repo's own config unable to execute anything, reading at most `max` bytes of
/// output (a huge diff is cut off, not read whole).
#[cfg(test)]
fn git_capped(cwd: &str, args: &[&str], max: usize) -> Option<String> {
    use std::io::Read;
    let mut command = std::process::Command::new("git");
    command.current_dir(cwd).args(factr_learn::goal_ratchet::SAFE_GIT).args(args);
    factr_learn::goal_ratchet::safe_git_env(&mut command);
    let mut child = command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let mut buf = Vec::new();
    child.stdout.take()?.take(max.min(u64::MAX as usize) as u64).read_to_end(&mut buf).ok()?;
    let cut = buf.len() >= max;
    if cut {
        let _ = child.kill();
    }
    let status = child.wait().ok()?;
    (cut || status.success()).then(|| String::from_utf8_lossy(&buf).trim_end().to_string())
}

/// A full hash, a hash prefix, or a 1-based index into the list.
fn resolve(list: &[(String, String, String)], target: &str) -> Option<String> {
    let by_index = target.parse::<usize>().ok().and_then(|n| list.get(n.checked_sub(1)?));
    by_index.or_else(|| list.iter().find(|(h, _, _)| h.starts_with(target))).map(|c| c.0.clone())
}

/// Routed from the `config.get` / `config.set` arms in `rpc.rs`.
///
/// `config.get` / `config.set` for `checkpoints.*` (enabled, max_snapshots, max_total_size_mb,
/// max_file_size_mb): the same `config.yaml` the desktop's own Settings save writes and the checkpoint
/// store reads, through the settings layer. The engine keeps no private copy.
pub(super) fn checkpoint_config(key: &str, set: Option<&Value>) -> Result<Value, RpcError> {
    let engine_home = factr_base::storage::factr_dir().map_err(RpcError::internal)?;
    let done = match set {
        Some(v) => super::settings::set(&engine_home, key, &json!({ "value": v })),
        None => super::settings::get(&engine_home, key, &json!({})),
    };
    done.unwrap_or_else(|| Err(RpcError { code: 4002, message: format!("unknown checkpoint setting: {key}"), data: None }))
}

pub(super) fn is_checkpoint_key(key: &str) -> bool {
    key.starts_with("checkpoints.")
}

pub(super) fn rollback(method: &str, cwd: &str, session: &str, p: &Value) -> Result<Value, RpcError> {
    rollback_in(factr_app_core::checkpoint::Store::default_store().as_ref(), method, cwd, session, p)
}

/// One checkpoint list: the store's recent snapshots and this session's pinned goal checkpoints (they
/// outlive the recent list); newest first. Nothing is read from the user's own repo.
/// Every restore and diff goes through the store (pre-rollback snapshot, safe mode). A full restore leaves
/// `history_removed` at 0: the session-history rewind is the caller's (see `checkpoint.rs`).
pub(super) fn rollback_in(store: Option<&factr_app_core::checkpoint::Store>, method: &str, cwd: &str, session: &str, p: &Value) -> Result<Value, RpcError> {
    let err = |code, message: &str| RpcError { code, message: message.into(), data: None };
    let Some(store) = store.filter(|s| s.config().enabled) else {
        return match method {
            "rollback.list" => Ok(json!({ "enabled": false, "checkpoints": [] })),
            "rollback.restore" => Ok(json!({ "success": false, "error": "checkpoints are disabled (checkpoints.enabled)" })),
            _ => Err(err(5022, "checkpoints are disabled (checkpoints.enabled)")),
        };
    };
    let dir = std::path::Path::new(cwd);
    let owner = factr_learn::goal_ratchet::pin_owner(session);
    let listing_started = std::time::Instant::now();
    let mut list: Vec<(String, String, String)> = store.list(dir).into_iter().map(|c| (c.hash, c.timestamp, c.message)).collect();
    let mut pinned_goals = 0usize;
    for c in store.pinned(dir, &owner) {
        if !list.iter().any(|l| l.0 == c.hash) {
            pinned_goals += 1;
            list.push((c.hash, c.timestamp, c.message));
        }
    }
    let at = |t: &str| chrono::DateTime::parse_from_rfc3339(t).map_or(0, |d| d.timestamp());
    list.sort_by_key(|c| std::cmp::Reverse(at(&c.1)));
    let listed_ms = listing_started.elapsed().as_millis() as u64;
    if method == "rollback.list" {
        factr_base::obs_sink::emit(
            factr_base::obs_sink::Span::new("rollback.list")
                .session(session)
                .attr("checkpoints", list.len())
                .attr("goal_pinned", pinned_goals)
                .took_ms(listed_ms),
        );
        let rows: Vec<Value> = list.iter().map(|(hash, timestamp, message)| json!({ "hash": hash, "timestamp": timestamp, "message": message })).collect();
        return Ok(json!({ "enabled": true, "checkpoints": rows }));
    }
    let target = p["hash"].as_str().unwrap_or_default();
    if target.is_empty() {
        return Err(err(4014, "hash required"));
    }
    let Some(hash) = resolve(&list, target) else {
        return if method == "rollback.diff" { Err(err(5022, "unknown checkpoint")) } else { Ok(json!({ "success": false, "error": "unknown checkpoint" })) };
    };
    if method == "rollback.diff" {
        return store.diff(dir, &hash).map_err(|e| err(5022, &e));
    }
    let file = p["file_path"].as_str().filter(|f| !f.is_empty());
    Ok(store.restore_for(dir, &hash, file, p["safe"].as_bool().unwrap_or(true), Some(session)))
}

/// `/rollback` (list), `/rollback diff N`, `/rollback N [file]`, as plain text.
fn rollback_text(rows: &Value) -> String {
    let list = rows["checkpoints"].as_array().cloned().unwrap_or_default();
    if rows["enabled"] == false {
        return "Checkpoints are off (checkpoints.enabled).".into();
    }
    if list.is_empty() {
        return "No checkpoints yet. One is taken before the agent first changes files in a turn.".into();
    }
    let mut out = String::from("Checkpoints, newest first:\n");
    for (i, c) in list.iter().enumerate() {
        let hash = c["hash"].as_str().unwrap_or_default();
        let at = c["timestamp"].as_str().unwrap_or_default().get(..16).unwrap_or_default().replace('T', " ");
        out.push_str(&format!("{:>3}. {}  {}  {}\n", i + 1, &hash[..hash.len().min(8)], at, c["message"].as_str().unwrap_or_default()));
    }
    out.push_str("\n/rollback <N> restores every file, /rollback <N> <file> one file, /rollback diff <N> previews.");
    out
}

impl Conn {
    /// The engine's `/rollback`, backed by the native `rollback.*` methods and the one checkpoint
    /// store (never Factr's Python store).
    pub(crate) async fn rollback_command(self: &Arc<Self>, arg: &str, session_id: Option<&str>) -> Value {
        let say = |text: String| json!({ "status": "ok", "type": "exec", "output": text, "message": text });
        let session = session_id.unwrap_or_default();
        let words: Vec<&str> = arg.split_whitespace().collect();
        let (method, params) = match words.as_slice() {
            [] | ["list"] => ("rollback.list", json!({ "session_id": session })),
            ["diff", n] => ("rollback.diff", json!({ "session_id": session, "hash": n })),
            ["diff", ..] => return say("usage: /rollback diff <N>".into()),
            [n] => ("rollback.restore", json!({ "session_id": session, "hash": n })),
            [n, file] => ("rollback.restore", json!({ "session_id": session, "hash": n, "file_path": file })),
            _ => return say("usage: /rollback [list] | /rollback diff <N> | /rollback <N> [file]".into()),
        };
        let out = match self.local_state(method, &params).await {
            Ok(v) => v,
            Err(e) => return say(e.message),
        };
        say(match method {
            "rollback.list" => rollback_text(&out),
            "rollback.diff" => {
                let (stat, diff) = (out["stat"].as_str().unwrap_or_default(), out["diff"].as_str().unwrap_or_default());
                if diff.is_empty() { "No differences from that checkpoint.".to_string() } else { format!("{stat}\n\n{diff}") }
            }
            _ if out["success"] == true => {
                let count = |k: &str| out[k].as_array().map_or(0, Vec::len);
                let mut line = format!("Restored {} file(s) from checkpoint {}", count("restored_files"), out["restored_to"].as_str().unwrap_or_default().get(..8).unwrap_or_default());
                if count("skipped_user_edits") > 0 {
                    line.push_str(&format!("; kept {} file(s) you edited since", count("skipped_user_edits")));
                }
                if out["history_removed"].as_u64().unwrap_or(0) > 0 {
                    line.push_str(&format!("; rewound {} message(s)", out["history_removed"]));
                }
                line.push_str(". A pre-rollback checkpoint was taken, so this is undoable.");
                line
            }
            _ => format!("Could not restore: {}", out["error"].as_str().unwrap_or("unknown checkpoint")),
        })
    }

    pub(super) async fn local_state(self: &Arc<Self>, method: &str, p: &Value) -> Result<Value, RpcError> {
        let session = p["session_id"].as_str().unwrap_or_default();
        let mgr = factr_base::background::global();
        match method {
            "process.list" => Ok(process_list(mgr, session).await),
            "process.kill" => process_kill(mgr, session, p["process_id"].as_str().unwrap_or_default()).await,
            "process.stop" => Ok(process_stop(mgr).await),
            "agents.list" => {
                let mut rows: Vec<Value> = running(mgr, None).await.iter().map(|t| json!({
                    "session_id": t.task_id, "command": command_of(t).chars().take(80).collect::<String>(),
                    "status": "running", "uptime": uptime(&t.started_at),
                })).collect();
                if !session.is_empty() {
                    let reply = self.call(json!({ "req": "list_sessions" })).await.map_err(RpcError::internal)?;
                    let live = self.sessions.lock().await;
                    for child in reply["sessions"].as_array().into_iter().flatten().filter(|s| s["parent_session_id"].as_str() == Some(session)) {
                        let status = Self::map_subagent_status(child, &live, self.followed(child["session_id"].as_str().unwrap_or_default()).await.is_some_and(|f| f.running));
                        let name = child["agent_label"].as_str().or(child["title"].as_str()).unwrap_or("subagent");
                        rows.push(json!({ "session_id": child["session_id"], "command": name.chars().take(80).collect::<String>(), "status": status, "uptime": 0 }));
                    }
                }
                Ok(json!({ "processes": rows }))
            }
            _ => {
                let cwd = match self.session_cwd(session).await {
                    Some(cwd) => cwd,
                    None => self.config.default_cwd.clone(),
                };
                // A full restore also rewinds the conversation (Factr `history_removed`), so it is
                // refused mid-turn; a one-file restore only touches disk.
                let full = method == "rollback.restore" && p["file_path"].as_str().is_none_or(str::is_empty) && !session.is_empty();
                if full && !p["hash"].as_str().unwrap_or_default().is_empty() && self.turn_running(session).await {
                    return Err(RpcError { code: 4009, message: "session is running; rollback restore works on an idle session".into(), data: None });
                }
                let (m, s, q) = (method.to_string(), session.to_string(), p.clone());
                let mut out = tokio::task::spawn_blocking(move || rollback(&m, &cwd, &s, &q))
                    .await
                    .map_err(|e| RpcError::internal(anyhow!(e)))??;
                if full && out["success"] == true {
                    out["history_removed"] = json!(self.rewind_history_to_last_user_turn(session).await.map_err(|e| RpcError::internal(anyhow!("checkpoint restored, but session history rewind failed: {e}")))?);
                }
                Ok(out)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use factr_base::background::TaskStatusFile;

    fn task(id: &str, session: &str, status: &str) -> TaskStatusFile {
        serde_json::from_value(json!({
            "task_id": id, "tool_name": "bash", "display_name": "npm run dev", "session_id": session, "status": status,
            "exit_code": null, "error": null, "started_at": chrono::Utc::now().to_rfc3339(), "completed_at": null, "duration_secs": null,
        })).unwrap()
    }

    #[tokio::test]
    async fn process_calls_answer_from_the_engine_registry_and_are_scoped_to_the_session() {
        let dir = std::env::temp_dir().join(format!("local-state-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mgr = BackgroundTaskManager::with_output_dir(dir.clone());
        for t in [task("a1", "s1", "running"), task("b2", "s2", "running"), task("c3", "s1", "completed")] {
            std::fs::write(dir.join(format!("{}.status.json", t.task_id)), serde_json::to_string(&t).unwrap()).unwrap();
        }
        let listed = process_list(&mgr, "s1").await;
        assert_eq!(listed["processes"].as_array().unwrap().len(), 1);
        assert_eq!((listed["processes"][0]["session_id"].as_str(), listed["processes"][0]["command"].as_str()), (Some("a1"), Some("npm run dev")));
        assert_eq!(process_list(&mgr, "nobody").await, json!({ "processes": [] }));
        assert_eq!(process_kill(&mgr, "s1", "b2").await.unwrap_err().code, 4044, "another session's process");
        assert_eq!(process_kill(&mgr, "s1", "c3").await.unwrap()["status"], "already_exited");
        assert_eq!(process_stop(&BackgroundTaskManager::with_output_dir(dir.join("empty"))).await, json!({ "killed": 0 }));
        let _ = std::fs::remove_dir_all(dir);
    }

    struct Tmp(std::path::PathBuf);
    impl Tmp {
        fn new() -> Self {
            static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            let p = std::env::temp_dir().join(format!("ls-ckpt-{}-{}", std::process::id(), N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).unwrap();
            Self(p)
        }
        fn path(&self) -> &std::path::Path {
            &self.0
        }
    }
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn temp_store() -> (factr_app_core::checkpoint::Store, Tmp) {
        let tmp = Tmp::new();
        (factr_app_core::checkpoint::Store::at(tmp.path().join("ckpt")).with_config_dir(tmp.path().join("factr")), tmp)
    }

    fn rb(st: &factr_app_core::checkpoint::Store, method: &str, cwd: &str, session: &str, p: &Value) -> Result<Value, RpcError> {
        rollback_in(Some(st), method, cwd, session, p)
    }

    #[test]
    fn native_checkpoints_list_diff_restore_outside_a_repo_merged_with_goal_refs_and_honor_disabled() {
        let (st, _keep) = temp_store();
        let dir = Tmp::new();
        let root = dir.path().canonicalize().unwrap();
        let cwd = root.to_str().unwrap();
        std::fs::write(root.join("f.txt"), "one").unwrap();
        st.snapshot(&root, "before edit: f.txt").unwrap();
        std::fs::write(root.join("f.txt"), "two").unwrap();
        st.record_write(&root, &root.join("f.txt"));
        std::fs::write(root.join("made.txt"), "agent").unwrap();
        st.record_write(&root, &root.join("made.txt"));
        let list = rb(&st, "rollback.list", cwd, "s1", &json!({})).unwrap();
        assert_eq!(list["enabled"], true);
        assert_eq!(list["checkpoints"].as_array().unwrap().len(), 1);
        assert!(list["checkpoints"][0]["message"].as_str().unwrap().starts_with("before edit"));
        let diff = rb(&st, "rollback.diff", cwd, "s1", &json!({ "hash": "1" })).unwrap();
        assert!(diff["diff"].as_str().unwrap().contains("-one") && diff["rendered"].is_string() && diff["stat"].as_str().unwrap().contains("f.txt"));
        let out = rb(&st, "rollback.restore", cwd, "s1", &json!({ "hash": "1" })).unwrap();
        assert_eq!((out["success"].clone(), out["history_removed"].clone()), (json!(true), json!(0)), "{out}");
        assert_eq!(std::fs::read_to_string(root.join("f.txt")).unwrap(), "one");
        assert!(!root.join("made.txt").exists());
        // newest first: the pre-rollback snapshot leads, and it undoes the restore
        let list = rb(&st, "rollback.list", cwd, "s1", &json!({})).unwrap();
        assert!(list["checkpoints"][0]["message"].as_str().unwrap().starts_with("pre-rollback"), "{list}");
        rb(&st, "rollback.restore", cwd, "s1", &json!({ "hash": "1" })).unwrap();
        assert_eq!(std::fs::read_to_string(root.join("f.txt")).unwrap(), "two");
        // single file
        std::fs::write(root.join("f.txt"), "three").unwrap();
        let one = rb(&st, "rollback.restore", cwd, "s1", &json!({ "hash": "2", "file_path": "f.txt" })).unwrap();
        assert_eq!(one["file"], "f.txt");
        // disabled
        std::fs::create_dir_all(_keep.path().join("factr")).unwrap();
        std::fs::write(_keep.path().join("factr/config.yaml"), "checkpoints:\n  enabled: false\n").unwrap();
        assert_eq!(rb(&st, "rollback.list", cwd, "s1", &json!({})).unwrap(), json!({ "enabled": false, "checkpoints": [] }));
        assert_eq!(rb(&st, "rollback.restore", cwd, "s1", &json!({ "hash": "1" })).unwrap()["success"], false);
        assert_eq!(rb(&st, "rollback.diff", cwd, "s1", &json!({ "hash": "1" })).unwrap_err().code, 5022);
    }

    #[test]
    fn goal_and_native_checkpoints_share_one_newest_first_list() {
        let (st, _keep) = temp_store();
        let dir = Tmp::new();
        let root = dir.path().canonicalize().unwrap();
        let cwd = root.to_str().unwrap();
        git(cwd, &["init", "-q"]).unwrap();
        std::fs::write(root.join("f.txt"), "one").unwrap();
        st.snapshot(&root, "before write: f.txt").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1100));
        std::fs::write(root.join("f.txt"), "two").unwrap();
        factr_learn::goal_ratchet::snapshot_in(&st, &root, "s1", 1).unwrap();
        let list = rb(&st, "rollback.list", cwd, "s1", &json!({})).unwrap();
        let msgs: Vec<&str> = list["checkpoints"].as_array().unwrap().iter().map(|c| c["message"].as_str().unwrap()).collect();
        assert!(msgs[0].starts_with("goal checkpoint 1") && msgs[1] == "before write: f.txt" && msgs.len() == 2, "{msgs:?}");
    }

    /// `/rollback` is the engine's: the three forms answer from the native store, and neither
    /// `slash.exec` nor `command.dispatch` can hand it to the Factr forwarder.
    #[tokio::test]
    async fn rollback_slash_command_is_served_natively_and_never_forwarded() {
        let _env = crate::factr_env::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let jhome = std::env::temp_dir().join(format!("rb-slash-{}", std::process::id()));
        let work = jhome.join("work");
        let _ = std::fs::remove_dir_all(&jhome);
        std::fs::create_dir_all(&work).unwrap();
        let work = work.canonicalize().unwrap();
        let before = std::env::var_os("FACTR_HOME");
        factr_base::env::set_var("FACTR_HOME", &jhome);
        let store = factr_app_core::checkpoint::Store::default_store().unwrap();
        std::fs::write(work.join("a.txt"), "one").unwrap();
        store.snapshot(&work, "before write: a.txt").unwrap().unwrap();
        std::fs::write(work.join("a.txt"), "two").unwrap();
        store.record_write(&work, &work.join("a.txt"));

        let conn = crate::rpc::tests::test_conn("rb-slash");
        conn.known.lock().await.insert("s".into(), json!({ "working_dir": work }));
        let said = |v: Value| v["output"].as_str().unwrap().to_string();
        let list = said(conn.dispatch("slash.exec", &json!({ "command": "/rollback", "session_id": "s" })).await.unwrap());
        assert!(list.contains("before write: a.txt") && list.contains("  1."), "{list}");
        let diff = said(conn.dispatch("command.dispatch", &json!({ "name": "rollback", "arg": "diff 1", "session_id": "s" })).await.unwrap());
        assert!(diff.contains("-one") && diff.contains("+two"), "{diff}");
        assert!(said(conn.dispatch("slash.exec", &json!({ "command": "/rollback diff", "session_id": "s" })).await.unwrap()).starts_with("usage"));
        // one file back: no chat rewind needed, no running turn
        let one = said(conn.dispatch("slash.exec", &json!({ "command": "/rollback 1 a.txt", "session_id": "s" })).await.unwrap());
        assert!(one.starts_with("Restored 1 file(s)") && one.contains("undoable"), "{one}");
        assert_eq!(std::fs::read_to_string(work.join("a.txt")).unwrap(), "one");
        assert!(said(conn.dispatch("slash.exec", &json!({ "command": "/rollback 99", "session_id": "s" })).await.unwrap()).starts_with("Could not restore"));
        // the forwarder itself refuses it, whatever reaches it
        let refused = conn.forward("command.dispatch", &json!({ "name": "rollback", "arg": "" })).await.unwrap_err();
        assert!(refused.message.contains("served by the engine"), "{}", refused.message);
        match before {
            Some(v) => factr_base::env::set_var("FACTR_HOME", v),
            None => factr_base::env::remove_var("FACTR_HOME"),
        }
        let _ = std::fs::remove_dir_all(jhome);
    }

    #[test]
    fn checkpoint_settings_live_in_config_yaml_and_the_store_follows() {
        /// Restores both homes even when an assertion fails.
        struct Restore(Option<std::ffi::OsString>, Option<std::ffi::OsString>);
        impl Drop for Restore {
            fn drop(&mut self) {
                match self.0.take() { Some(v) => unsafe { std::env::set_var("FACTR_CONFIG_HOME", v) }, None => unsafe { std::env::remove_var("FACTR_CONFIG_HOME") } }
                match self.1.take() { Some(v) => factr_app_core::env::set_var("FACTR_HOME", v), None => factr_app_core::env::remove_var("FACTR_HOME") }
            }
        }
        let _env = crate::factr_env::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = Tmp::new();
        let factr = tmp.path().join("factr");
        let _restore = Restore(std::env::var_os("FACTR_CONFIG_HOME"), std::env::var_os("FACTR_HOME"));
        // SAFETY: env is only touched under ENV_LOCK.
        unsafe { std::env::set_var("FACTR_CONFIG_HOME", &factr) };
        factr_app_core::env::set_var("FACTR_HOME", tmp.path().join("factr"));
        let store = factr_app_core::checkpoint::Store::at(tmp.path().join("ckpt"));
        assert_eq!(checkpoint_config("checkpoints.enabled", None).unwrap()["value"], true, "default on");
        checkpoint_config("checkpoints.max_snapshots", Some(&json!("5"))).unwrap();
        assert_eq!(checkpoint_config("checkpoints.max_snapshots", None).unwrap()["value"], 5);
        let yaml = std::fs::read_to_string(factr.join("config.yaml")).unwrap();
        assert!(yaml.contains("max_snapshots: 5"), "{yaml}");
        assert_eq!(store.config().max_snapshots, 5, "the store reads the same file");
        assert!(!tmp.path().join("ckpt/config.json").exists());
        assert_eq!(checkpoint_config("checkpoints.max_snapshots", Some(&json!("many"))).unwrap_err().code, 4002);
        assert_eq!(checkpoint_config("checkpoints.nope", None).unwrap_err().code, 4002);
        assert!(is_checkpoint_key("checkpoints.enabled") && !is_checkpoint_key("model"));
    }

    #[test]
    fn rollback_answers_empty_outside_a_repo_and_never_reads_refs_in_the_users_repo() {
        let (st, _keep) = temp_store();
        let plain = std::env::temp_dir().join(format!("rollback-plain-{}", std::process::id()));
        std::fs::create_dir_all(&plain).unwrap();
        let none = rb(&st, "rollback.list", plain.to_str().unwrap(), "s1", &json!({})).unwrap();
        // /tmp may itself sit inside a repo on odd setups; either way the shape holds.
        assert!(none["checkpoints"].as_array().unwrap().is_empty());

        let repo = std::env::temp_dir().join(format!("rollback-repo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&repo);
        std::fs::create_dir_all(&repo).unwrap();
        let repo = repo.canonicalize().unwrap();
        let cwd = repo.to_str().unwrap();
        git(cwd, &["init", "-q"]).unwrap();
        std::fs::write(repo.join("f.txt"), "one").unwrap();
        git(cwd, &["add", "f.txt"]).unwrap();
        git(cwd, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "init"]).unwrap();
        // A goal-shaped ref some other tool left in the repo is not ours: not listed, not restorable.
        git(cwd, &["update-ref", "refs/other-tool/goals/s1/1", "HEAD"]).unwrap();
        std::fs::write(repo.join("f.txt"), "two").unwrap();
        let list = rb(&st, "rollback.list", cwd, "s1", &json!({})).unwrap();
        assert_eq!((list["enabled"].clone(), list["checkpoints"].clone()), (json!(true), json!([])), "{list}");
        let out = rb(&st, "rollback.restore", cwd, "s1", &json!({ "hash": "1" })).unwrap();
        assert_eq!(out["success"], false, "{out}");
        assert_eq!(std::fs::read_to_string(repo.join("f.txt")).unwrap(), "two", "nothing was restored");
        let _ = std::fs::remove_dir_all(repo);
        let _ = std::fs::remove_dir_all(plain);
    }

    #[test]
    fn a_goal_checkpoint_restore_is_undoable_and_never_overwrites_hand_edits() {
        let (st, _keep) = temp_store();
        let dir = Tmp::new();
        let root = dir.path().canonicalize().unwrap();
        let cwd = root.to_str().unwrap();
        let (a, b) = (root.join("a.txt"), root.join("b.txt"));
        std::fs::write(&a, "a0").unwrap();
        std::fs::write(&b, "b0").unwrap();
        let goal = factr_learn::goal_ratchet::snapshot_in(&st, &root, "s1", 1).unwrap();
        for _ in 0..25 {
            // enough later snapshots that the goal commit leaves the recent list
            std::fs::write(&a, format!("a{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos())).unwrap();
            st.snapshot(&root, "churn").unwrap();
        }
        std::fs::write(&a, "a-agent").unwrap();
        std::fs::write(&b, "b-agent").unwrap();
        st.record_write(&root, &a);
        st.record_write(&root, &b);
        std::fs::write(&b, "b-by-hand").unwrap();
        let list = rb(&st, "rollback.list", cwd, "s1", &json!({})).unwrap();
        assert!(list["checkpoints"].as_array().unwrap().iter().any(|c| c["hash"] == goal.as_str()), "a pinned goal checkpoint outlives the recent list");
        let out = rb(&st, "rollback.restore", cwd, "s1", &json!({ "hash": goal })).unwrap();
        assert_eq!(out["success"], true, "{out}");
        assert_eq!((std::fs::read_to_string(&a).unwrap(), std::fs::read_to_string(&b).unwrap()), ("a0".into(), "b-by-hand".into()), "safe mode kept the hand edit");
        assert_eq!(out["skipped_user_edits"], json!(["b.txt"]));
        let list = rb(&st, "rollback.list", cwd, "s1", &json!({})).unwrap();
        assert!(list["checkpoints"][0]["message"].as_str().unwrap().starts_with("pre-rollback snapshot"), "{list}");
    }

    #[cfg(unix)]
    #[test]
    fn repo_config_cannot_run_code_through_snapshot_diff_or_restore() {
        use std::os::unix::fs::PermissionsExt;
        let (st, _keep) = temp_store();
        let root = std::env::temp_dir().join(format!("rollback-fsmon-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let repo = root.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let (marker, script) = (root.join("ran"), root.join("fsmon.sh"));
        std::fs::write(&script, format!("#!/bin/sh\ntouch '{}'\n", marker.display())).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let raw = |args: &[&str]| std::process::Command::new("git").current_dir(&repo).args(args).output().unwrap();
        raw(&["init", "-q"]);
        raw(&["config", "core.fsmonitor", script.to_str().unwrap()]);
        std::fs::write(repo.join("f.txt"), "one").unwrap();
        raw(&["status"]);
        assert!(marker.exists(), "sanity: unprotected git runs the configured script");
        std::fs::remove_file(&marker).unwrap();

        let cwd = repo.to_str().unwrap();
        factr_learn::goal_ratchet::snapshot_in(&st, &repo, "s1", 1).unwrap();
        std::fs::write(repo.join("f.txt"), "two").unwrap();
        rb(&st, "rollback.list", cwd, "s1", &json!({})).unwrap();
        assert!(rb(&st, "rollback.diff", cwd, "s1", &json!({ "hash": "1" })).unwrap()["diff"].as_str().unwrap().contains("-one"));
        rb(&st, "rollback.restore", cwd, "s1", &json!({ "hash": "1" })).unwrap();
        assert!(!marker.exists(), "repo-controlled fsmonitor must not run");
        let _ = std::fs::remove_dir_all(root);
    }

    /// A full restore rewinds the conversation like Factr (`history_removed`), is refused mid-turn
    /// (4009), and the one-file variant leaves history alone.
    #[tokio::test]
    async fn full_restore_rewinds_history_and_is_refused_mid_turn() {
        let _env = crate::factr_env::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let jhome = std::env::temp_dir().join(format!("rb-jh-{}", std::process::id()));
        let work = std::env::temp_dir().join(format!("rb-work-{}", std::process::id()));
        for d in [&jhome, &work] {
            let _ = std::fs::remove_dir_all(d);
            std::fs::create_dir_all(d).unwrap();
        }
        let work = work.canonicalize().unwrap();
        let before = std::env::var_os("FACTR_HOME");
        factr_base::env::set_var("FACTR_HOME", &jhome);
        let store = factr_app_core::checkpoint::Store::default_store().unwrap();
        std::fs::write(work.join("a.txt"), "one").unwrap();
        store.snapshot(&work, "before edit a.txt").unwrap().unwrap();
        std::fs::write(work.join("a.txt"), "two").unwrap();

        let conn = crate::rpc::tests::test_conn("rb-full");
        conn.known.lock().await.insert("s".into(), json!({ "working_dir": work }));
        // A stand-in engine link: serves get_history (two user turns) and records the other requests.
        let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(8);
        conn.links.lock().await.insert("s".into(), tx);
        let seen = Arc::new(std::sync::Mutex::new(Vec::<Value>::new()));
        let (c, log) = (conn.clone(), seen.clone());
        tokio::spawn(async move {
            while let Some(line) = rx.recv().await {
                let frame: Value = serde_json::from_str(&line).unwrap();
                let rows: Vec<Value> = ["u1", "a1", "u2", "a2"].iter().map(|t| json!({ "role": if t.starts_with('u') { "user" } else { "assistant" }, "content": t })).collect();
                let reply = if frame["req"] == "get_history" { json!({ "ev": "history", "messages": rows }) } else { json!({ "ev": "ack" }) };
                log.lock().unwrap().push(frame.clone());
                if let Some(done) = c.pending.lock().await.remove(&frame["id"].as_u64().unwrap()) {
                    let _ = done.send(reply);
                }
            }
        });
        let hash = store.list(&work)[0].hash.clone();
        let file_only = conn.dispatch("rollback.restore", &json!({ "session_id": "s", "hash": hash, "file_path": "a.txt" })).await.unwrap();
        assert_eq!(file_only["success"], true, "{file_only}");
        assert!(seen.lock().unwrap().is_empty(), "a one-file restore never touches the conversation");
        std::fs::write(work.join("a.txt"), "two").unwrap();

        let run = conn.observer.start_turn("s", "work", "invoke_agent", None);
        let busy = conn.dispatch("rollback.restore", &json!({ "session_id": "s", "hash": hash })).await.unwrap_err();
        assert_eq!(busy.code, 4009);
        assert_eq!(std::fs::read_to_string(work.join("a.txt")).unwrap(), "two", "refused before touching disk");
        conn.observer.failed_submit("s", &run, "turn ended");
        assert!(!conn.observer.has_active_run("s"));
        let out = conn.dispatch("rollback.restore", &json!({ "session_id": "s", "hash": hash })).await.unwrap();
        assert_eq!(out["success"], true, "{out}");
        assert_eq!(out["history_removed"], 2, "the last user turn and its reply");
        assert_eq!(std::fs::read_to_string(work.join("a.txt")).unwrap(), "one");
        assert!(seen.lock().unwrap().iter().any(|f| f["req"] == "rewind" && f["message_index"] == 2));
        match before {
            Some(v) => factr_base::env::set_var("FACTR_HOME", v),
            None => factr_base::env::remove_var("FACTR_HOME"),
        }
    }
}
