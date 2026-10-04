//! `/undo [N|chat|files]`, `/redo`, `session.undo_turn` and `session.redo_turn`: the engine's
//! transcript rewind plus the file checkpoint restore, with the bookkeeping in `crate::undo`.

use super::*;
use crate::undo::{self, FileReport, State};
use factr_app_core::checkpoint::{self, Store, undo_dir};
use factr_base::obs_sink;
use std::path::Path;

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Mode {
    Both,
    Chat,
    Files,
}

pub(crate) enum Failure {
    /// The request cannot apply now (running turn, nothing to undo): shown as is.
    Refused(String),
    Broken(anyhow::Error),
}

impl From<anyhow::Error> for Failure {
    fn from(e: anyhow::Error) -> Self {
        Failure::Broken(e)
    }
}

impl Failure {
    pub(crate) fn text(&self) -> String {
        match self {
            Failure::Refused(m) => m.clone(),
            Failure::Broken(e) => e.to_string(),
        }
    }

    pub(super) fn rpc(self) -> RpcError {
        match self {
            Failure::Refused(m) => RpcError::params(&m),
            Failure::Broken(e) => RpcError::internal(e),
        }
    }
}

pub(crate) struct UndoReport {
    pub text: String,
    pub users: usize,
    pub messages_removed: usize,
    pub files: FileReport,
    pub irreversible: Vec<String>,
    pub redo_available: usize,
}

pub(crate) struct RedoReport {
    pub messages_restored: usize,
    pub files: FileReport,
    pub redo_available: usize,
}

/// Ids of `session`'s running commands that started at or after `since` (a [`checkpoint::now_stamp`]).
pub(crate) fn started_since(tasks: &[factr_base::background::TaskStatusFile], session: &str, since: &str) -> Vec<String> {
    let Ok(since) = chrono::DateTime::parse_from_rfc3339(since) else { return Vec::new() };
    tasks
        .iter()
        .filter(|t| t.session_id == session && t.status == factr_base::bus::BackgroundTaskStatus::Running)
        .filter(|t| chrono::DateTime::parse_from_rfc3339(&t.started_at).is_ok_and(|at| at >= since))
        .map(|t| t.task_id.clone())
        .collect()
}

async fn cancel_started_since(mgr: &factr_base::background::BackgroundTaskManager, session: &str, since: &str) -> usize {
    let mut cancelled = 0;
    for id in started_since(&mgr.list().await, session, since) {
        cancelled += usize::from(mgr.cancel_with_grace(&id, std::time::Duration::from_millis(100)).await.unwrap_or(false));
    }
    cancelled
}

impl UndoReport {
    pub(crate) fn notice(&self, mode: Mode) -> String {
        let mut notice = match mode {
            Mode::Files => "Restored files only; the chat is unchanged".to_string(),
            _ => format!("Undid {} {} ({} message(s))", self.users, if self.users == 1 { "turn" } else { "turns" }, self.messages_removed),
        };
        if mode != Mode::Chat {
            notice.push_str(&format!("; {}", undo::files_line(&self.files)));
        }
        if !self.irreversible.is_empty() {
            notice.push_str(&format!("; cannot be undone: {}", self.irreversible.join(", ")));
        }
        if mode == Mode::Files {
            notice.push('.');
        } else {
            notice.push_str(". Edit and resubmit, or /redo.");
        }
        notice
    }
}

/// Load a session's undo state off the async workers (it reads a file).
async fn load_state(dir: &Path, id: &str) -> State {
    let (dir, id) = (dir.to_path_buf(), id.to_string());
    tokio::task::spawn_blocking(move || State::load(&dir, &id)).await.unwrap_or_default()
}

/// Save a session's undo state and sync its checkpoint pins off the async workers (git calls).
async fn save_state(state: State, dir: &Path, id: &str) -> State {
    let (dir, id) = (dir.to_path_buf(), id.to_string());
    tokio::task::spawn_blocking(move || {
        state.save_pinned(&dir, &id, Store::default_store().as_ref());
        state
    })
    .await
    .unwrap_or_default()
}

impl Conn {
    async fn cwd_of(&self, id: &str) -> String {
        self.session_cwd(id).await.unwrap_or_else(|| self.config.default_cwd.clone())
    }

    /// Row ids for `rows`, from the session's id store (positions when there is no home). The
    /// store is written only when the ids changed.
    pub(crate) async fn row_ids(&self, id: &str, rows: &[Value]) -> Vec<u64> {
        let Some(dir) = undo_dir() else { return rewind::position_ids(rows) };
        let (id, rows) = (id.to_string(), rows.to_vec());
        tokio::task::spawn_blocking(move || {
            undo::with_state(&dir, &id, None, |state| {
                let (next, before) = (state.next_row_id(), state.rows.clone());
                let ids = state.sync(&rows);
                (ids, state.next_row_id() != next || state.rows != before)
            })
        })
        .await
        .unwrap_or_else(|_| rewind::position_ids(&[]))
    }

    /// A user turn is about to be submitted: remember when it started and end the redo history
    /// (messages and files alike). No snapshot here: the first file change of the turn takes it
    /// (`checkpoint::before_change`), so a turn that edits nothing costs nothing and an undo of it
    /// restores nothing. Never fails the submit.
    pub(crate) async fn record_turn(self: &Arc<Self>, id: &str) {
        let (Some(dir), Ok(rows)) = (undo_dir(), self.history_rows(id).await) else { return };
        let (id, cwd) = (id.to_string(), self.cwd_of(id).await);
        let _ = tokio::task::spawn_blocking(move || {
            let store = Store::default_store();
            undo::with_state(&dir, &id, store.as_ref(), |state| {
                state.sync(&rows);
                state.mark_turn(&cwd, None);
                checkpoint::clear_engine_stack(&dir, &id);
                ((), true)
            })
        })
        .await;
    }

    /// A rewind that is not an undo (retry, edit/regenerate truncation, branch, rollback restore)
    /// ends the redo history: the gateway's records and the engine's removed-messages stack go
    /// together, so the two can never disagree about what a redo would restore.
    pub(crate) async fn drop_redo(&self, id: &str) {
        let Some(dir) = undo_dir() else { return };
        let id = id.to_string();
        let _ = tokio::task::spawn_blocking(move || {
            let store = Store::default_store();
            undo::with_state(&dir, &id, store.as_ref(), |state| {
                let had = !state.redo.is_empty();
                state.redo.clear();
                checkpoint::clear_engine_stack(&dir, &id);
                ((), had)
            })
        })
        .await;
    }

    /// The engine rewind of an undo: pushes the removed messages on the engine's stack, in step
    /// with the `Redo` record the caller pushes. (Other rewinds go through `apply_cut`.)
    pub(crate) async fn apply_undo_cut(&self, id: &str, cut: &rewind::Cut) -> Result<()> {
        self.call(json!({ "req": "rewind", "session_id": id, "message_index": cut.index })).await.map(|_| ())
    }

    /// Undo the last `n` user turns: conversation and/or files (see [`Mode`]). The files return to
    /// the checkpoint taken when the first undone turn began, in safe mode.
    pub(crate) async fn undo_turns(self: &Arc<Self>, id: &str, n: usize, mode: Mode) -> Result<(UndoReport, String), Failure> {
        let refuse = |m: &str| Failure::Refused(m.to_string());
        let started = std::time::Instant::now();
        // One undo/redo/prompt at a time per session over the state file.
        let lock = undo::session_lock(id);
        let _guard = lock.lock().await;
        if self.turn_running(id).await {
            return Err(refuse("session is running; /undo works on an idle session"));
        }
        let rows = self.history_rows(id).await?;
        let Some(cut) = rewind::cut_last_users(&rows, n) else { return Err(refuse("no user messages to undo")) };
        let dir = undo_dir().ok_or_else(|| Failure::Broken(anyhow!("no home directory")))?;
        let mut state = load_state(&dir, id).await;
        let ids = state.sync(&rows);
        let marker = state.turn(ids[cut.index]).cloned();
        let fallback_cwd = self.cwd_of(id).await;
        let users = rows[cut.index..].iter().filter(|m| m["role"] == "user").count();
        if mode != Mode::Files {
            self.apply_undo_cut(id, &cut).await?;
        }
        // Commands the undone turns started keep running against files about to change: stop those,
        // and only when the files really are restored. A chat-only undo leaves every command alone,
        // and a server the user started before the undone turns is never touched.
        let mut cancelled = 0;
        if mode != Mode::Chat {
            if let Some(since) = marker.as_ref().map(|t| t.at.as_str()) {
                cancelled = cancel_started_since(factr_base::background::global(), id, since).await;
            }
        }
        let work_dir = marker.as_ref().map_or(fallback_cwd, |t| t.dir.clone());
        let (files, before) = if mode == Mode::Chat {
            (FileReport::default(), None)
        } else {
            let has_marker = marker.is_some();
            let (hash, wd) = (marker.as_ref().and_then(|t| undo::turn_checkpoint(&dir, id, t)), work_dir.clone());
            let session = id.to_string();
            tokio::task::spawn_blocking(move || match (Store::default_store(), hash) {
                (Some(store), Some(hash)) if store.config().enabled => {
                    let before = undo::checkpoint_now(&store, Path::new(&wd), "before undo", Some(&session));
                    if let Some(b) = &before {
                        store.pin(Path::new(&wd), &session, b);
                    }
                    (undo::restore_files(&store, Path::new(&wd), &hash, Some(&session)), before)
                }
                (Some(store), None) if store.config().enabled => {
                    let why = if has_marker { "no file changes in those turns" } else { "no record of when that turn began" };
                    (FileReport { note: Some(why.into()), ..Default::default() }, None)
                }
                _ => (FileReport { note: Some("file checkpoints are off".into()), ..Default::default() }, None),
            })
            .await
            .map_err(|e| anyhow!(e))?
        };
        let irreversible = marker.as_ref().map(|t| checkpoint::effects_since(&dir, id, &t.at)).unwrap_or_default();
        state.push_undo(if mode == Mode::Files { rows.len() } else { cut.index }, &work_dir, before);
        let redo_available = state.redo.len();
        save_state(state, &dir, id).await;
        let report = UndoReport { text: cut.text, users, messages_removed: if mode == Mode::Files { 0 } else { cut.removed }, files, irreversible, redo_available };
        // Counts and a fixed reason label only: no paths, no message text.
        obs_sink::emit(
            obs_sink::Span::new("undo.turn")
                .session(id)
                .attr("mode", match mode { Mode::Both => "both", Mode::Chat => "chat", Mode::Files => "files" })
                .attr("turns", report.users)
                .attr("messages_removed", report.messages_removed)
                .attr("files_restored", report.files.restored.len())
                .attr("files_skipped", report.files.skipped.len())
                .attr("tasks_cancelled", cancelled)
                .attr("irreversible_effects", report.irreversible.len())
                .attr("redo_available", report.redo_available)
                .attr("reason", undo::note_class(&report.files.note))
                .took_ms(started.elapsed().as_millis() as u64),
        );
        let text = report.text.clone();
        Ok((report, text))
    }

    /// Re-apply the newest undo: messages first, then the files it restored.
    pub(crate) async fn redo_turns(self: &Arc<Self>, id: &str) -> Result<RedoReport, Failure> {
        let started = std::time::Instant::now();
        let session = id.to_string();
        let lock = undo::session_lock(id);
        let _guard = lock.lock().await;
        if self.turn_running(id).await {
            return Err(Failure::Refused("session is running; redo works on an idle session".into()));
        }
        let rows = self.history_rows(id).await?;
        let dir = undo_dir().ok_or_else(|| Failure::Broken(anyhow!("no home directory")))?;
        let mut state = load_state(&dir, id).await;
        state.sync(&rows);
        let Some(record) = state.take_redo() else {
            save_state(state, &dir, id).await;
            return Err(Failure::Refused("nothing to redo".into()));
        };
        let restored = record.keys.len();
        if restored > 0 {
            if let Err(e) = self.call(json!({ "req": "rewind_undo", "session_id": id })).await {
                // The engine no longer holds those messages: nothing older can be redone either.
                state.redo.clear();
                checkpoint::clear_engine_stack(&dir, id);
                save_state(state, &dir, id).await;
                return Err(Failure::Refused(format!("nothing to redo: {e}")));
            }
        }
        let (hash, wd) = (record.files_before.clone(), record.dir.clone());
        state.apply_redo(record);
        let state = save_state(state, &dir, id).await;
        let files = tokio::task::spawn_blocking(move || match (Store::default_store(), hash) {
            (Some(store), Some(hash)) => undo::restore_files(&store, Path::new(&wd), &hash, Some(&session)),
            _ => FileReport::default(),
        })
        .await
        .map_err(|e| anyhow!(e))?;
        obs_sink::emit(
            obs_sink::Span::new("undo.redo")
                .session(id)
                .attr("messages_restored", restored)
                .attr("files_restored", files.restored.len())
                .attr("files_skipped", files.skipped.len())
                .attr("redo_available", state.redo.len())
                .attr("reason", undo::note_class(&files.note))
                .took_ms(started.elapsed().as_millis() as u64),
        );
        Ok(RedoReport { messages_restored: restored, files, redo_available: state.redo.len() })
    }

    /// The engine-served slash commands (the `Route::Engine` rows of the `slash_forward` table) in the shape
    /// `command.dispatch` uses; `slash.exec` and `command.dispatch` both come here.
    pub(crate) async fn engine_slash(self: &Arc<Self>, name: &str, arg: &str, session_id: Option<&str>) -> Value {
        let say = |text: &str| json!({ "status": "ok", "type": "exec", "output": text, "message": text });
        if name == "focus" {
            return say(&self.focus_command(arg));
        }
        let Some(id) = session_id.filter(|s| !s.is_empty()) else { return say(&format!("/{name} needs an open session.")) };
        match name {
            "retry" => self.rewind_command(name, arg, session_id).await,
            "rollback" => self.rollback_command(arg, session_id).await,
            "undo" | "redo" => self.undo_command(name, arg, session_id).await,
            "compress" if self.turn_running(id).await => say("session is running; /compress works on an idle session"),
            "compress" => match self.slash_rpc("session.compress", json!({ "session_id": id })).await {
                Ok(_) => say("Compressed the conversation context."),
                Err(e) => say(&format!("/compress: {}", e.message)),
            },
            "branch" => match self.slash_rpc("session.branch", json!({ "session_id": id, "name": arg })).await {
                Ok(done) => say(&format!("Branched into session {} ({}). Open it from the session list.", done["session_id"].as_str().unwrap_or_default(), done["title"].as_str().unwrap_or("Branch"))),
                Err(e) => say(&format!("/branch: {}", e.message)),
            },
            "steer" if arg.trim().is_empty() => say("usage: /steer <prompt>"),
            "steer" => match self.slash_rpc("session.steer", json!({ "session_id": id, "text": arg })).await {
                Ok(_) => say("Steer queued: it arrives after the next tool call."),
                Err(e) => say(&format!("/steer: {}", e.message)),
            },
            _ => say(&format!("/{name} is not served by the engine.")),
        }
    }

    /// `/focus [on|off|status]`: Factr's display-only toggle, config `display.focus_view`, written
    /// through the settings path so the desktop reads it back. Needs no session.
    fn focus_command(&self, arg: &str) -> String {
        use super::settings;
        let home = std::path::Path::new(&self.config.home);
        let current = settings::get(home, "display.focus_view", &json!({}))
            .and_then(Result::ok)
            .is_some_and(|v| v["value"].as_bool() == Some(true));
        let state = |on: bool| if on { "on" } else { "off" };
        let next = match arg.trim().to_ascii_lowercase().as_str() {
            "" | "toggle" => !current,
            "on" | "enable" | "enabled" | "true" | "yes" | "1" => true,
            "off" | "disable" | "disabled" | "false" | "no" | "0" => false,
            "status" | "show" | "?" => return format!("focus view {}", state(current)),
            _ => return "usage: /focus [on|off|status]".into(),
        };
        match settings::set(home, "display.focus_view", &json!({ "value": next })) {
            Some(Ok(_)) => format!("focus view {}", state(next)),
            Some(Err(e)) => format!("/focus: {}", e.message),
            None => "/focus: settings are unavailable".into(),
        }
    }

    /// An RPC the engine answers itself. `dispatch` and `engine_slash` call each other, so the future is boxed.
    async fn slash_rpc(self: &Arc<Self>, method: &str, params: Value) -> Result<Value, RpcError> {
        Box::pin(self.dispatch(method, &params)).await
    }

    /// Tell the desktop the stored transcript changed under an idle session: its `status.update`
    /// handler re-reads the history on a `ready` edge (the same hook `/compress` ends with). `/undo`
    /// and `/redo` answer `prefill` / `exec` text only, so without this the truncated turns stay on
    /// screen until the chat is reopened.
    async fn transcript_changed(&self, id: &str) {
        self.emit("status.update", Some(id), json!({ "kind": "ready" })).await;
    }

    /// `/undo [N|chat|files]` and `/redo` in the shape `command.dispatch` uses.
    pub(crate) async fn undo_command(self: &Arc<Self>, name: &str, arg: &str, session_id: Option<&str>) -> Value {
        let say = |text: &str| json!({ "status": "ok", "type": "exec", "output": text, "message": text });
        let Some(id) = session_id.filter(|s| !s.is_empty()) else { return say(&format!("/{name} needs an open session.")) };
        if name == "redo" {
            return match self.redo_turns(id).await {
                Ok(r) => {
                    if r.messages_restored > 0 {
                        self.transcript_changed(id).await;
                    }
                    let files = if r.files.restored.is_empty() && r.files.skipped.is_empty() { String::new() } else { format!("; {}", undo::files_line(&r.files)) };
                    say(&format!("Redid {} message(s){files}. {} more to redo.", r.messages_restored, r.redo_available))
                }
                Err(f) => say(&f.text()),
            };
        }
        let word = arg.split_whitespace().next();
        let (mode, n) = match word {
            None => (Mode::Both, 1),
            Some("chat") => (Mode::Chat, 1),
            Some("files") => (Mode::Files, 1),
            Some(w) => match w.parse::<usize>() {
                Ok(n) if n > 0 => (Mode::Both, n),
                _ => return say(&format!("/undo: invalid argument {w:?}; use /undo, /undo N, /undo chat or /undo files")),
            },
        };
        let n = arg.split_whitespace().nth(1).and_then(|w| w.parse::<usize>().ok()).filter(|n| *n > 0).unwrap_or(n);
        match self.undo_turns(id, n, mode).await {
            Err(f) => say(&f.text()),
            Ok((report, text)) => {
                let notice = report.notice(mode);
                let mut out = json!({
                    "status": "ok", "type": if mode == Mode::Files { "exec" } else { "prefill" }, "message": text, "notice": notice,
                    "output": notice, "removed": report.messages_removed,
                    "files_restored": report.files.restored, "files_skipped": report.files.skipped,
                    "irreversible_effects": report.irreversible, "redo_available": report.redo_available,
                });
                if mode == Mode::Files {
                    out["message"] = json!(notice);
                } else if report.messages_removed > 0 {
                    self.transcript_changed(id).await;
                }
                out
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn undo_and_redo_are_refused_mid_turn_before_anything_changes() {
        let conn = super::super::tests::test_conn("undo-mid-turn");
        conn.observer.start_turn("s1", "hello", "invoke_agent", None);
        let undo = conn.undo_command("undo", "", Some("s1")).await;
        assert_eq!(undo["output"], "session is running; /undo works on an idle session");
        assert_eq!(undo["type"], "exec", "no prefill, nothing was undone");
        for arg in ["chat", "files", "2"] {
            assert_eq!(conn.undo_command("undo", arg, Some("s1")).await["output"], "session is running; /undo works on an idle session");
        }
        assert_eq!(conn.undo_command("redo", "", Some("s1")).await["output"], "session is running; redo works on an idle session");
        let rpc = conn.undo_turns("s1", 1, Mode::Both).await.err().unwrap().rpc();
        assert_eq!(rpc.message, "session is running; /undo works on an idle session");
        assert!(conn.undo_command("undo", "zzz", Some("s1")).await["output"].as_str().unwrap().contains("invalid argument"));
    }

    /// The desktop prefills its composer from a `prefill` reply and re-reads the transcript on a
    /// `status.update` `ready` edge; an undo or redo that removed messages must produce both.
    #[tokio::test]
    async fn slash_undo_prefills_and_tells_the_desktop_to_reread_the_transcript() {
        use tokio_tungstenite::tungstenite::Message;
        let (_env, home, before) = with_temp_home("slash-undo");
        let (conn, mut rx) = super::super::tests::test_conn_events("slash-undo", false);
        conn.known.lock().await.insert("s".into(), json!({ "working_dir": home }));
        let _engine = standin_engine(&conn, &[("user", "u1"), ("assistant", "a1"), ("user", "u2"), ("assistant", "a2")]).await;
        let ready = |rx: &mut tokio::sync::mpsc::Receiver<Message>| {
            let mut kinds = Vec::new();
            while let Ok(Message::Text(t)) = rx.try_recv() {
                let ev: Value = serde_json::from_str(&t).unwrap();
                if ev["params"]["type"] == "status.update" {
                    kinds.push(ev["params"]["payload"]["kind"].as_str().unwrap_or_default().to_string());
                }
            }
            kinds
        };
        let undo = conn.undo_command("undo", "chat", Some("s")).await;
        assert_eq!((undo["type"].as_str(), undo["message"].as_str()), (Some("prefill"), Some("u2")));
        assert!(undo["notice"].as_str().unwrap().starts_with("Undid 1 turn"));
        assert_eq!(ready(&mut rx), ["ready"]);
        let redo = conn.undo_command("redo", "", Some("s")).await;
        assert!(redo["output"].as_str().unwrap().starts_with("Redid 2 message(s)"));
        assert_eq!(ready(&mut rx), ["ready"]);
        // Nothing to redo any more: no edge.
        conn.undo_command("redo", "", Some("s")).await;
        assert!(ready(&mut rx).is_empty());
        restore_home(before);
        let _ = std::fs::remove_dir_all(home);
    }

    /// A stand-in engine behind `conn`'s link for session `s`: a mutable transcript that serves
    /// `get_history`, `rewind` and `rewind_undo`, and records every request.
    pub(crate) async fn standin_engine(conn: &Arc<Conn>, rows: &[(&str, &str)]) -> (Arc<std::sync::Mutex<Vec<Value>>>, Arc<std::sync::Mutex<Vec<Value>>>) {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(64);
        conn.links.lock().await.insert("s".into(), tx);
        let transcript = Arc::new(std::sync::Mutex::new(rows.iter().map(|(r, c)| json!({ "role": r, "content": c })).collect::<Vec<Value>>()));
        let seen = Arc::new(std::sync::Mutex::new(Vec::<Value>::new()));
        let (c, log, rows) = (conn.clone(), seen.clone(), transcript.clone());
        tokio::spawn(async move {
            let mut removed: Vec<Vec<Value>> = Vec::new();
            while let Some(line) = rx.recv().await {
                let frame: Value = serde_json::from_str(&line).unwrap();
                log.lock().unwrap().push(frame.clone());
                let reply = match frame["req"].as_str() {
                    Some("get_history") => json!({ "ev": "history", "messages": rows.lock().unwrap().clone() }),
                    Some("rewind") => {
                        let mut r = rows.lock().unwrap();
                        let at = frame["message_index"].as_u64().unwrap() as usize;
                        let at = at.min(r.len());
                        removed.push(r.split_off(at));
                        json!({ "ev": "ack" })
                    }
                    Some("rewind_undo") => match removed.pop() {
                        Some(back) => {
                            rows.lock().unwrap().extend(back);
                            json!({ "ev": "ack" })
                        }
                        None => json!({ "ev": "error", "message": "No rewind to undo." }),
                    },
                    _ => json!({ "ev": "ack" }),
                };
                if let Some(done) = c.pending.lock().await.remove(&frame["id"].as_u64().unwrap()) {
                    let _ = done.send(reply);
                }
            }
        });
        (transcript, seen)
    }

    fn with_temp_home(name: &str) -> (std::sync::MutexGuard<'static, ()>, std::path::PathBuf, Option<std::ffi::OsString>) {
        let guard = crate::factr_env::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let home = std::env::temp_dir().join(format!("undo-home-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        let before = std::env::var_os("FACTR_HOME");
        factr_base::env::set_var("FACTR_HOME", &home);
        (guard, home, before)
    }

    fn restore_home(before: Option<std::ffi::OsString>) {
        match before {
            Some(v) => factr_base::env::set_var("FACTR_HOME", v),
            None => factr_base::env::remove_var("FACTR_HOME"),
        }
    }

    /// Undo, then ten new messages: the redo records and the engine's stack are gone, so a later
    /// redo can never restore messages onto a transcript they no longer fit. A retry / truncation /
    /// rollback rewind ends the redo history the same way.
    #[tokio::test]
    async fn new_messages_and_non_undo_rewinds_clear_the_one_redo_history() {
        let (_env, home, before) = with_temp_home("redo-owner");
        let conn = super::super::tests::test_conn("redo-owner");
        conn.known.lock().await.insert("s".into(), json!({ "working_dir": home }));
        let (transcript, _seen) = standin_engine(&conn, &[("user", "u1"), ("assistant", "a1"), ("user", "u2"), ("assistant", "a2")]).await;
        let dir = undo_dir().unwrap();
        let engine_file = checkpoint::undo_file(&dir, "s", "engine.json");

        let (report, _) = conn.undo_turns("s", 1, Mode::Chat).await.ok().unwrap();
        assert_eq!((report.messages_removed, report.redo_available), (2, 1));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&engine_file, "[]").unwrap();
        for i in 0..10 {
            conn.record_turn("s").await;
            transcript.lock().unwrap().extend([json!({ "role": "user", "content": format!("m{i}") }), json!({ "role": "assistant", "content": "r" })]);
        }
        assert!(State::load(&dir, "s").redo.is_empty(), "ten new messages ended the redo history");
        assert!(!engine_file.exists(), "and the engine stack went with it");
        assert_eq!(conn.redo_turns("s").await.err().unwrap().text(), "nothing to redo");

        // a retry / truncation / rollback restore is a non-undo rewind
        conn.undo_turns("s", 1, Mode::Chat).await.ok().unwrap();
        assert_eq!(State::load(&dir, "s").redo.len(), 1);
        std::fs::write(&engine_file, "[]").unwrap();
        assert_eq!(conn.rewind_history_to_last_user_turn("s").await.unwrap(), 2);
        assert!(State::load(&dir, "s").redo.is_empty() && !engine_file.exists());
        assert_eq!(conn.redo_turns("s").await.err().unwrap().text(), "nothing to redo");

        // plain undo/redo still round-trips
        let n = transcript.lock().unwrap().len();
        conn.undo_turns("s", 1, Mode::Chat).await.ok().unwrap();
        let redone = conn.redo_turns("s").await.ok().unwrap();
        assert_eq!((redone.messages_restored, transcript.lock().unwrap().len()), (2, n));
        restore_home(before);
        let _ = std::fs::remove_dir_all(home);
    }

    /// Submitting costs no snapshot: only the turn's first file change takes it. A turn that edits
    /// nothing leaves the store empty and its undo rewinds the chat and restores no files; a turn
    /// that edits restores to the files as they were, even after the recent list has moved on.
    #[tokio::test]
    async fn the_turn_snapshot_is_lazy_and_undo_restores_from_it() {
        let (_env, home, before) = with_temp_home("lazy-snap");
        let work = home.join("work");
        std::fs::create_dir_all(&work).unwrap();
        let work = work.canonicalize().unwrap();
        std::fs::write(work.join("a.txt"), "v0").unwrap();
        let conn = super::super::tests::test_conn("lazy-snap");
        conn.known.lock().await.insert("s".into(), json!({ "working_dir": work }));
        let (transcript, _seen) = standin_engine(&conn, &[]).await;
        let store = Store::default_store().unwrap();
        let say = |t: &str| {
            transcript.lock().unwrap().extend([json!({ "role": "user", "content": t }), json!({ "role": "assistant", "content": "ok" })]);
        };

        // Turn 1 edits no files.
        conn.record_turn("s").await;
        say("just talking");
        assert!(store.list(&work).is_empty(), "submitting took no snapshot");
        let (report, _) = conn.undo_turns("s", 1, Mode::Both).await.ok().unwrap();
        assert_eq!((report.messages_removed, report.files.restored.len()), (2, 0));
        assert!(undo::files_line(&report.files).contains("no file changes"), "{}", undo::files_line(&report.files));
        assert!(store.list(&work).is_empty(), "and neither did the undo");
        assert_eq!(std::fs::read_to_string(work.join("a.txt")).unwrap(), "v0");
        assert!(conn.redo_turns("s").await.is_ok());

        // Turn 2 edits a.txt: the hook snapshots before the change.
        conn.record_turn("s").await;
        say("edit a");
        std::thread::sleep(std::time::Duration::from_millis(5));
        checkpoint::new_turn("s");
        assert!(checkpoint::ensure_checkpoint(&store, "s", &work, "before write: a.txt"));
        std::fs::write(work.join("a.txt"), "v1").unwrap();
        store.record_write(&work, &work.join("a.txt"));
        // The recent list moves on well past the turn's snapshot.
        for i in 0..25 {
            std::fs::write(work.join("other.txt"), format!("o{i}")).unwrap();
            store.snapshot(&work, "later").unwrap();
        }
        assert!(store.pin_count("s") >= 1);
        let (report, _) = conn.undo_turns("s", 1, Mode::Both).await.ok().unwrap();
        assert_eq!(report.files.restored, ["a.txt"], "{:?}", report.files);
        assert_eq!(std::fs::read_to_string(work.join("a.txt")).unwrap(), "v0");
        restore_home(before);
        let _ = std::fs::remove_dir_all(home);
    }

    /// Undo, redo, snapshots, restores and listings are observable, with counts only: no path and no
    /// message text reaches a span.
    #[tokio::test]
    async fn undo_redo_checkpoint_and_rollback_spans_carry_counts_only() {
        let (_env, home, before) = with_temp_home("obs-spans");
        let work = home.join("work");
        std::fs::create_dir_all(&work).unwrap();
        let work = work.canonicalize().unwrap();
        std::fs::write(work.join("secret-name.txt"), "v0").unwrap();
        let conn = super::super::tests::test_conn("obs-spans");
        conn.known.lock().await.insert("s-obs".into(), json!({ "working_dir": work }));
        let (transcript, _seen) = {
            // standin_engine serves session "s"; this test names its own session
            let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(64);
            conn.links.lock().await.insert("s-obs".into(), tx);
            let rows = Arc::new(std::sync::Mutex::new(Vec::<Value>::new()));
            let seen = Arc::new(std::sync::Mutex::new(Vec::<Value>::new()));
            let (c, r, removed) = (conn.clone(), rows.clone(), Arc::new(std::sync::Mutex::new(Vec::<Vec<Value>>::new())));
            tokio::spawn(async move {
                while let Some(line) = rx.recv().await {
                    let frame: Value = serde_json::from_str(&line).unwrap();
                    let reply = match frame["req"].as_str() {
                        Some("get_history") => json!({ "ev": "history", "messages": r.lock().unwrap().clone() }),
                        Some("rewind") => {
                            let mut all = r.lock().unwrap();
                            let at = (frame["message_index"].as_u64().unwrap() as usize).min(all.len());
                            removed.lock().unwrap().push(all.split_off(at));
                            json!({ "ev": "ack" })
                        }
                        Some("rewind_undo") => {
                            let back = removed.lock().unwrap().pop().unwrap_or_default();
                            r.lock().unwrap().extend(back);
                            json!({ "ev": "ack" })
                        }
                        _ => json!({ "ev": "ack" }),
                    };
                    if let Some(done) = c.pending.lock().await.remove(&frame["id"].as_u64().unwrap()) {
                        let _ = done.send(reply);
                    }
                }
            });
            (rows, seen)
        };
        let store = Store::default_store().unwrap();
        let spans = Arc::new(std::sync::Mutex::new(Vec::<obs_sink::Span>::new()));
        let mut attempt = 0;
        loop {
            attempt += 1;
            spans.lock().unwrap().clear();
            let sink = spans.clone();
            // Another test may install its own recorder at any moment: retry the whole flow.
            obs_sink::install(move |span| sink.lock().unwrap().push(span));
            std::fs::write(work.join("secret-name.txt"), "v0").unwrap();
            conn.record_turn("s-obs").await;
            transcript.lock().unwrap().extend([json!({ "role": "user", "content": "private prompt text" }), json!({ "role": "assistant", "content": "ok" })]);
            std::thread::sleep(std::time::Duration::from_millis(5));
            checkpoint::new_turn("s-obs");
            assert!(checkpoint::ensure_checkpoint(&store, "s-obs", &work, "before write: secret-name.txt") || attempt > 1);
            std::fs::write(work.join("secret-name.txt"), "v1").unwrap();
            store.record_write(&work, &work.join("secret-name.txt"));
            conn.undo_turns("s-obs", 1, Mode::Both).await.ok().unwrap();
            conn.redo_turns("s-obs").await.ok().unwrap();
            let cwd = work.to_str().unwrap();
            super::super::local_state::rollback_in(Some(&store), "rollback.list", cwd, "s-obs", &json!({})).unwrap();
            let kinds: Vec<&str> = spans.lock().unwrap().iter().map(|s| s.kind).collect();
            let all = ["undo.turn", "undo.redo", "checkpoint.snapshot", "checkpoint.restore", "rollback.list"];
            if all.iter().all(|k| kinds.contains(k)) || attempt >= 5 {
                break;
            }
        }
        let spans = spans.lock().unwrap();
        for kind in ["undo.turn", "undo.redo", "checkpoint.snapshot", "checkpoint.restore", "rollback.list"] {
            assert!(spans.iter().any(|s| s.kind == kind), "missing {kind}: {:?}", spans.iter().map(|s| s.kind).collect::<Vec<_>>());
        }
        let undo_span = spans.iter().find(|s| s.kind == "undo.turn").unwrap();
        assert_eq!((undo_span.attributes["mode"].as_str(), undo_span.attributes["messages_removed"].as_u64(), undo_span.attributes["files_restored"].as_u64()), (Some("both"), Some(2), Some(1)));
        let dump = spans.iter().map(|s| format!("{:?} {}", s.error, s.attributes)).collect::<Vec<_>>().join("\n");
        for secret in ["secret-name", "private prompt", work.to_str().unwrap(), "v1"] {
            assert!(!dump.contains(secret), "span leaked {secret}: {dump}");
        }
        restore_home(before);
        let _ = std::fs::remove_dir_all(home);
    }

    /// Every delete path ends in `forget_rows`: it must leave no `undo/<id>*` file behind.
    #[test]
    fn deleting_a_session_leaves_no_undo_files() {
        let (_env, home, before) = with_temp_home("delete-purge");
        let dir = undo_dir().unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        for id in ["gone", "kept"] {
            for suffix in ["json", "engine.json", "effects.json"] {
                std::fs::write(checkpoint::undo_file(&dir, id, suffix), "[]").unwrap();
            }
        }
        // The stores under the gateway home may not exist here; the undo purge still runs.
        let _ = crate::sessions_rest::forget_rows(home.to_str().unwrap(), "gone");
        let names: Vec<String> = std::fs::read_dir(&dir).unwrap().map(|f| f.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert!(names.iter().all(|n| !n.starts_with("gone")), "{names:?}");
        assert_eq!(names.iter().filter(|n| n.starts_with("kept")).count(), 3);
        restore_home(before);
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn only_commands_started_in_the_undone_turns_are_selected() {
        let task = |id: &str, session: &str, status: &str, at: &str| -> factr_base::background::TaskStatusFile {
            serde_json::from_value(json!({
                "task_id": id, "tool_name": "bash", "session_id": session, "status": status, "exit_code": null,
                "error": null, "started_at": at, "completed_at": null, "duration_secs": null,
            }))
            .unwrap()
        };
        let tasks = [
            task("dev-server", "s", "running", "2026-01-01T10:00:00Z"),
            task("build", "s", "running", "2026-01-01T10:05:00.250Z"),
            task("done", "s", "completed", "2026-01-01T10:06:00Z"),
            task("other", "t", "running", "2026-01-01T10:06:00Z"),
        ];
        assert_eq!(started_since(&tasks, "s", "2026-01-01T10:05:00.000Z"), ["build"], "the earlier dev server survives");
        assert_eq!(started_since(&tasks, "s", "garbage"), Vec::<String>::new());
    }

    #[test]
    fn notice_says_what_was_restored_skipped_and_cannot_be_undone() {
        let report = UndoReport {
            text: "hi".into(),
            users: 2,
            messages_removed: 5,
            files: FileReport { restored: vec!["a".into(), "b".into()], skipped: vec![json!({ "path": "c.txt", "reason": "x" })], note: None },
            irreversible: vec!["git push".into()],
            redo_available: 1,
        };
        let notice = report.notice(Mode::Both);
        assert!(notice.starts_with("Undid 2 turns (5 message(s)); restored 2 files, skipped 1 (c.txt"), "{notice}");
        assert!(notice.contains("cannot be undone: git push") && notice.ends_with("/redo."), "{notice}");
        assert!(!report.notice(Mode::Chat).contains("restored"), "chat-only says nothing about files");
        assert!(report.notice(Mode::Files).starts_with("Restored files only"));
    }
}
