//! Cron runs as read-only transcripts. The engine's hidden session is the one record of a run (the
//! scheduler passes the run id with the turn and `link` remembers which session it became); runs
//! booked into `state.db` by older builds are still read, and a run that died before any session
//! exists is rebuilt from the ledger.
//!
//! The Cron screen lists a job's runs as sessions named `cron_{job}_{YYYYmmdd_HHMMSS}` (Factr books
//! one into `state.db` after the engine ran the turn: the job prompt, then the output) and opens one
//! like any chat, through `session.resume`. The engine's own session store has never heard of those
//! ids, so the resume failed ("Resume failed", endless spinner). This module answers it natively:
//! user = the job prompt, assistant = the run output, plus a final status message when the run did
//! not complete. The ledger `cron/executions.db` supplies the status and error; a run that failed
//! before it produced output has no session row at all, so it is rebuilt from the ledger and the
//! job's prompt in `jobs.json`.

use rusqlite::{Connection, OpenFlags, params};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// A ledger row is the same run when its claim is this close to the run id's timestamp (seconds).
const MATCH_WINDOW: i64 = 120;

pub(crate) struct CronRun {
    pub id: String,
    pub title: String,
    pub started_at: f64,
    pub ended_at: Option<f64>,
    /// `(role, text)`, oldest first.
    pub messages: Vec<(String, String)>,
    /// Ledger status (`completed`, `failed`, ...), when a ledger row matches.
    pub status: Option<String>,
    pub error: Option<String>,
    /// The job's working directory (`workdir` in jobs.json); the caller fills in the engine's default when empty.
    pub cwd: String,
}

/// `cron_{job}_{YYYYmmdd}_{HHMMSS}` -> (job, wall-clock stamp).
fn parse_id(id: &str) -> Option<(&str, chrono::NaiveDateTime)> {
    let rest = id.strip_prefix("cron_")?;
    let mut parts = rest.rsplitn(3, '_');
    let (time, date, job) = (parts.next()?, parts.next()?, parts.next()?);
    let stamp = chrono::NaiveDateTime::parse_from_str(&format!("{date}{time}"), "%Y%m%d%H%M%S").ok()?;
    (!job.is_empty() && job.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')).then_some((job, stamp))
}

/// The home and each profile's home.
fn stores(home: &Path) -> Vec<PathBuf> {
    let mut out = vec![home.to_path_buf()];
    if let Ok(entries) = std::fs::read_dir(home.join("profiles")) {
        let mut profiles: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
        profiles.sort();
        out.extend(profiles);
    }
    out
}

fn open(path: &Path) -> Option<Connection> {
    if !path.is_file() {
        return None;
    }
    let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX).ok()?;
    db.busy_timeout(std::time::Duration::from_secs(2)).ok()?;
    Some(db)
}

/// Factr writes local wall-clock ISO text with an offset; the run id carries the same wall clock.
fn wall_clock(iso: &str) -> Option<chrono::NaiveDateTime> {
    let head: String = iso.chars().take(19).collect();
    chrono::NaiveDateTime::parse_from_str(&head.replace(' ', "T"), "%Y-%m-%dT%H:%M:%S").ok()
}

/// The ledger row of `job` claimed nearest the run id's stamp: (status, error, finished_at).
fn ledger(store: &Path, job: &str, stamp: chrono::NaiveDateTime) -> Option<(String, Option<String>)> {
    let db = open(&store.join("cron").join("executions.db"))?;
    let mut stmt = db.prepare("SELECT claimed_at, status, error FROM executions WHERE job_id = ?1").ok()?;
    let rows = stmt
        .query_map(params![job], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<String>>(2)?)))
        .ok()?
        .flatten();
    rows.filter_map(|(claimed, status, error)| {
        let gap = (wall_clock(&claimed)? - stamp).num_seconds().abs();
        (gap <= MATCH_WINDOW).then_some((gap, status, error))
    })
    .min_by_key(|(gap, ..)| *gap)
    .map(|(_, status, error)| (status, error))
}

/// `(name, prompt, working dir)` of a job in `store`'s `jobs.json`.
fn job(store: &Path, job_id: &str) -> Option<(String, String, String)> {
    let value: Value = serde_json::from_str(&std::fs::read_to_string(store.join("cron").join("jobs.json")).ok()?).ok()?;
    let jobs = match &value {
        Value::Array(list) => list.as_slice(),
        Value::Object(map) => map.get("jobs")?.as_array()?.as_slice(),
        _ => return None,
    };
    let found = jobs.iter().find(|j| j["id"].as_str() == Some(job_id))?;
    let text = |k: &str| found[k].as_str().unwrap_or_default().to_string();
    let cwd = ["workdir", "working_dir", "cwd"].iter().map(|k| text(k)).find(|v| !v.is_empty()).unwrap_or_default();
    Some((text("name"), text("prompt"), cwd))
}

/// `<job name> · Oct 02 23:53`: what the window title and sidebar show for a run.
fn run_title(name: &str, job_id: &str, stamp: chrono::NaiveDateTime) -> String {
    let name = if name.is_empty() { format!("cron {job_id}") } else { name.to_string() };
    format!("{name} · {}", stamp.format("%b %d %H:%M"))
}

fn session(store: &Path, id: &str) -> Option<(String, f64, Option<f64>, Vec<(String, String)>)> {
    let db = open(&store.join("state.db"))?;
    let (title, started, ended) = db
        .query_row("SELECT COALESCE(title, ''), started_at, ended_at FROM sessions WHERE id = ?1 AND source = 'cron'", params![id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?, r.get::<_, Option<f64>>(2)?))
        })
        .ok()?;
    let mut stmt = db
        .prepare("SELECT role, COALESCE(content, '') FROM messages WHERE session_id = ?1 AND role IN ('user', 'assistant') ORDER BY id")
        .ok()?;
    let messages = stmt.query_map(params![id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))).ok()?.flatten().collect();
    Some((title, started, ended, messages))
}

/// The run `id` names, or None when it is not a cron run id or nothing records it.
#[cfg(test)]
pub(crate) fn load(home: &Path, id: &str) -> Option<CronRun> {
    load_with(home, id, None)
}

const LINK: &str = "cron_run:";

/// Remember that run `run_id` was the engine session `session_id`.
pub(crate) fn link(engine_home: &str, run_id: &str, session_id: &str) {
    if let Some(store) = crate::rpc::entries_or_log(engine_home) {
        let _ = store.set_setting(&format!("{LINK}{run_id}"), session_id);
    }
}

/// The engine session behind run `run_id`, when this build ran it.
pub(crate) fn linked_session(engine_home: &str, run_id: &str) -> Option<String> {
    if !run_id.starts_with("cron_") {
        return None;
    }
    crate::rpc::entries_or_log(engine_home)?.setting(&format!("{LINK}{run_id}"))
}

/// The engine session's conversation as `(role, text)` rows (tool calls and results summarised), start and end.
fn engine_session(session_id: &str) -> Option<(Vec<(String, String)>, f64, f64)> {
    use factr_base::message::{ContentBlock, Role};
    let session = factr_base::session::Session::load(session_id).ok()?;
    let clip = |t: &str| t.chars().take(400).collect::<String>();
    let mut rows = Vec::new();
    for m in session.messages.iter().filter(|m| !factr_base::session::is_internal_system_reminder_message(m)) {
        let role = if matches!(m.role, Role::User) { "user" } else { "assistant" };
        for b in &m.content {
            match b {
                ContentBlock::Text { text, .. } if !text.trim().is_empty() => rows.push((role.to_string(), text.clone())),
                ContentBlock::ToolUse { name, input, .. } => rows.push(("assistant".into(), format!("Used `{name}`: {}", clip(&input.to_string())))),
                ContentBlock::ToolResult { content, is_error, .. } => {
                    let tag = if *is_error == Some(true) { "Tool error" } else { "Tool result" };
                    rows.push(("assistant".into(), format!("{tag}: {}", clip(content))));
                }
                _ => {}
            }
        }
    }
    Some((rows, session.created_at.timestamp() as f64, session.updated_at.timestamp() as f64))
}

/// [`load`], preferring the engine session `engine` (the one this run became) over any `state.db` copy.
pub(crate) fn load_with(home: &Path, id: &str, engine: Option<&str>) -> Option<CronRun> {
    let (job_id, stamp) = parse_id(id)?;
    let engine = engine.and_then(engine_session);
    stores(home).into_iter().find_map(|store| {
        let (status, error) = ledger(&store, job_id, stamp).unzip();
        let error = error.flatten();
        let (name, prompt, cwd) = job(&store, job_id).unwrap_or_default();
        let title = run_title(&name, job_id, stamp);
        if let Some((messages, started_at, ended_at)) = &engine {
            return Some(CronRun { id: id.into(), title, started_at: *started_at, ended_at: Some(*ended_at), messages: messages.clone(), status, error, cwd });
        }
        if let Some((_, started_at, ended_at, messages)) = session(&store, id) {
            return Some(CronRun { id: id.into(), title, started_at, ended_at, messages, status, error, cwd });
        }
        // No session row: the run died before it had output. The ledger still knows it ran.
        let status = status?;
        let at = chrono::Local.from_local_datetime(&stamp).earliest().map_or(0.0, |d| d.timestamp() as f64);
        let messages = if prompt.is_empty() { Vec::new() } else { vec![("user".to_string(), prompt)] };
        Some(CronRun { id: id.into(), title, started_at: at, ended_at: Some(at), messages, status: Some(status), error, cwd })
    })
}

/// The canonical id of the job `key` names (an id or a name) in any store's `jobs.json`.
fn canonical_job(home: &Path, key: &str) -> String {
    for store in stores(home) {
        let Ok(text) = std::fs::read_to_string(store.join("cron").join("jobs.json")) else { continue };
        let Ok(value) = serde_json::from_str::<Value>(&text) else { continue };
        let jobs = match &value {
            Value::Array(list) => list.as_slice(),
            Value::Object(map) => map.get("jobs").and_then(Value::as_array).map_or(&[][..], Vec::as_slice),
            _ => continue,
        };
        let by = |field: &str| jobs.iter().find(|j| j[field].as_str() == Some(key));
        if let Some(id) = by("id").or_else(|| by("name")).and_then(|j| j["id"].as_str()) {
            return id.to_string();
        }
    }
    key.to_string()
}

/// A job's run history, newest first: runs the engine linked, runs older builds booked into `state.db`,
/// and ledger rows (`executions.db`) with no session (a run that died before it started) so every
/// attempt has an id and a status.
pub(crate) fn list(home: &Path, engine_home: &str, job_key: &str, limit: usize) -> Vec<Value> {
    let job_id = canonical_job(home, job_key);
    let mut ids: Vec<(String, Option<String>)> = Vec::new();
    if let Some(store) = crate::rpc::entries_or_log(engine_home) {
        for (run, session) in store.settings_with_prefix(LINK) {
            if parse_id(&run).is_some_and(|(job, _)| job == job_id) {
                ids.push((run, Some(session)));
            }
        }
    }
    let like = format!("cron\\_{}\\_%", job_id.replace('%', "").replace('_', "\\_"));
    for store in stores(home) {
        if let Some(db) = open(&store.join("state.db")) {
            if let Ok(mut stmt) = db.prepare("SELECT id FROM sessions WHERE source = 'cron' AND id LIKE ?1 ESCAPE '\\'") {
                let found: Vec<String> = stmt.query_map(params![like], |r| r.get::<_, String>(0)).map(|r| r.flatten().collect()).unwrap_or_default();
                ids.extend(found.into_iter().filter(|id| parse_id(id).is_some_and(|(job, _)| job == job_id)).map(|id| (id, None)));
            }
        }
    }
    let known: Vec<chrono::NaiveDateTime> = ids.iter().filter_map(|(id, _)| parse_id(id).map(|(_, t)| t)).collect();
    for store in stores(home) {
        let Some(db) = open(&store.join("cron").join("executions.db")) else { continue };
        let Ok(mut stmt) = db.prepare("SELECT claimed_at FROM executions WHERE job_id = ?1") else { continue };
        let claimed: Vec<String> = stmt.query_map(params![job_id], |r| r.get::<_, String>(0)).map(|r| r.flatten().collect()).unwrap_or_default();
        for at in claimed.iter().filter_map(|c| wall_clock(c)) {
            if !known.iter().any(|k| (*k - at).num_seconds().abs() <= MATCH_WINDOW) {
                ids.push((format!("cron_{job_id}_{}", at.format("%Y%m%d_%H%M%S")), None));
            }
        }
    }
    ids.sort();
    ids.dedup_by(|a, b| a.0 == b.0);
    let mut rows: Vec<CronRun> = ids.iter().filter_map(|(id, session)| load_with(home, id, session.as_deref())).collect();
    rows.sort_by(|a, b| b.started_at.total_cmp(&a.started_at));
    rows.into_iter().take(limit).map(|r| {
        let mut row = r.session_row();
        row["status"] = json!(r.status);
        row
    }).collect()
}

/// `GET /api/cron/jobs/{job}/runs`, served from the engine instead of Factr's `state.db`.
pub(super) async fn route(stream: &mut tokio::net::TcpStream, req: &super::Request, config: &super::Config) -> Option<anyhow::Result<()>> {
    let job = req.path.strip_prefix("/api/cron/jobs/")?.strip_suffix("/runs")?;
    if req.method != "GET" || job.is_empty() || job.contains('/') {
        return None;
    }
    let limit = super::query_u64(req, "limit").unwrap_or(20).clamp(1, 100) as usize;
    let home = factr_base::factr_config::home().unwrap_or_else(|| PathBuf::from(&config.home));
    let (engine_home, job) = (config.home.clone(), job.to_string());
    let runs = tokio::task::spawn_blocking(move || list(&home, &engine_home, &job, limit)).await.unwrap_or_default();
    Some(super::respond(stream, "200 OK", &json!({ "runs": runs, "limit": limit })).await)
}

impl CronRun {
    /// The transcript the desktop renders: the stored messages, then how the run ended unless it completed.
    pub fn transcript(&self) -> Vec<Value> {
        let mut rows: Vec<Value> = self.messages.iter().map(|(role, text)| json!({"role": role, "text": text, "content": text})).collect();
        let note = match (self.status.as_deref(), self.error.as_deref()) {
            (None | Some("completed"), _) => None,
            (Some(status), Some(error)) if !error.is_empty() => Some(format!("Run {status}: {error}")),
            (Some(status), _) => Some(format!("Run {status}.")),
        };
        if let Some(note) = note {
            rows.push(json!({"role": "assistant", "text": note, "content": note}));
        }
        rows
    }

    /// The `/api/sessions/{id}` row.
    pub fn session_row(&self) -> Value {
        let preview = self.messages.iter().rev().find(|(role, _)| role == "assistant").map(|(_, t)| t.clone()).unwrap_or_default();
        json!({
            "id": self.id, "title": self.title, "preview": preview, "source": "cron",
            "started_at": self.started_at, "last_active": self.ended_at.unwrap_or(self.started_at), "ended_at": self.ended_at,
            "is_active": false, "message_count": self.messages.len(), "archived": false, "cwd": self.cwd,
            "model": Value::Null, "parent_session_id": Value::Null,
        })
    }

    /// `session.resume` / `session.activate` result: a finished, never-running transcript.
    pub fn resume(&self, version: &str, model: &str, provider: &str, omit_messages: bool) -> Value {
        let messages = if omit_messages { Vec::new() } else { self.transcript() };
        json!({
            "session_id": self.id,
            "stored_session_id": self.id,
            "message_count": messages.len(),
            "messages": messages,
            "running": false,
            "read_only": true,
            "title": self.title,
            "source": "cron",
            "working_dir": self.cwd,
            "info": {
                "model": model, "provider": provider, "cwd": self.cwd, "running": false, "title": self.title, "source": "cron",
                "stored_session_id": self.id, "version": version, "desktop_contract": 8,
            },
        })
    }
}

use chrono::TimeZone as _;

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn home() -> PathBuf {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!("cron-runs-{}-{}", std::process::id(), N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
        std::fs::create_dir_all(dir.join("cron")).unwrap();
        // Same shapes Factr writes (factr_state_common.py / cron/executions.py), trimmed to what is read.
        let state = Connection::open(dir.join("state.db")).unwrap();
        state
            .execute_batch(
                "CREATE TABLE sessions (id TEXT PRIMARY KEY, source TEXT NOT NULL, started_at REAL NOT NULL, ended_at REAL, title TEXT);
                 CREATE TABLE messages (id INTEGER PRIMARY KEY AUTOINCREMENT, session_id TEXT NOT NULL, role TEXT NOT NULL, content TEXT, timestamp REAL NOT NULL);
                 INSERT INTO sessions VALUES ('cron_job1_20261002_235300', 'cron', 1790967184.4, 1790967188.0, 'gui-cron · Oct 02 23:53');
                 INSERT INTO messages (session_id, role, content, timestamp) VALUES
                   ('cron_job1_20261002_235300', 'user', 'Reply with exactly the word CRONOK.', 1), ('cron_job1_20261002_235300', 'assistant', 'CRONOK', 2);
                 INSERT INTO sessions VALUES ('chat_other', 'cli', 1.0, NULL, 'a chat');",
            )
            .unwrap();
        let ex = Connection::open(dir.join("cron/executions.db")).unwrap();
        ex.execute_batch(
            "CREATE TABLE executions (id TEXT PRIMARY KEY, job_id TEXT NOT NULL, source TEXT NOT NULL, process_id TEXT NOT NULL, pid INTEGER NOT NULL,
               process_started_at INTEGER, status TEXT NOT NULL, handoff_pending INTEGER NOT NULL DEFAULT 0, handoff_started_at REAL,
               claimed_at TEXT NOT NULL, started_at TEXT, finished_at TEXT, error TEXT);
             INSERT INTO executions (id, job_id, source, process_id, pid, status, claimed_at, error) VALUES
               ('a', 'job1', 'tick', 'p', 1, 'completed', '2026-10-02T23:53:00.152468+05:00', NULL),
               ('b', 'job1', 'tick', 'p', 1, 'failed', '2026-10-02T23:56:00.152892+05:00', 'RemoteDisconnected: Remote end closed connection without response');",
        )
        .unwrap();
        std::fs::write(dir.join("cron/jobs.json"), r#"{"jobs":[{"id":"job1","name":"gui-cron","workdir":"/work/proj","prompt":"Reply with exactly the word CRONOK."}]}"#).unwrap();
        dir
    }

    #[test]
    fn a_completed_run_is_prompt_then_output() {
        let dir = home();
        let run = load(&dir, "cron_job1_20261002_235300").unwrap();
        let rows = run.transcript();
        assert_eq!(rows.len(), 2, "completed: no status line");
        assert_eq!((rows[0]["role"].as_str(), rows[0]["text"].as_str()), (Some("user"), Some("Reply with exactly the word CRONOK.")));
        assert_eq!((rows[1]["role"].as_str(), rows[1]["text"].as_str()), (Some("assistant"), Some("CRONOK")));
        let resumed = run.resume("v", "m", "p", false);
        assert_eq!((resumed["running"].clone(), resumed["message_count"].clone(), resumed["stored_session_id"].clone()), (json!(false), json!(2), json!("cron_job1_20261002_235300")));
        assert_eq!(run.session_row()["preview"], "CRONOK");
        // Same shapes real sessions use: titled by job + run time, sourced `cron`, with a working dir.
        let (row, info) = (run.session_row(), &resumed["info"]);
        assert_eq!(row["title"], "gui-cron · Oct 02 23:53");
        assert_eq!((row["source"].clone(), row["cwd"].clone()), (json!("cron"), json!("/work/proj")));
        assert_eq!((info["title"].clone(), info["source"].clone(), info["cwd"].clone()), (json!("gui-cron · Oct 02 23:53"), json!("cron"), json!("/work/proj")));
        assert_eq!((resumed["title"].clone(), resumed["working_dir"].clone()), (json!("gui-cron · Oct 02 23:53"), json!("/work/proj")));
        // The prompt reaches the UI untouched.
        assert_eq!(rows[0]["text"], "Reply with exactly the word CRONOK.");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_failed_run_with_no_session_is_rebuilt_from_the_ledger() {
        let dir = home();
        let rows = load(&dir, "cron_job1_20261002_235601").unwrap().transcript();
        assert_eq!(rows[0]["role"], "user");
        assert_eq!(rows[0]["text"], "Reply with exactly the word CRONOK.");
        assert_eq!(rows.last().unwrap()["text"], "Run failed: RemoteDisconnected: Remote end closed connection without response");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn unknown_runs_and_non_run_ids_are_not_answered() {
        let dir = home();
        assert!(load(&dir, "cron_job1_20261002_120000").is_none(), "no session, no ledger row nearby");
        assert!(load(&dir, "cron_nojob_20261002_235300").is_none());
        assert!(load(&dir, "chat_other").is_none());
        assert!(load(&dir, "cron_../x_20261002_235300").is_none());
        assert!(load(&dir, "session_123").is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_run_list_has_ids_and_statuses_from_state_db_and_the_ledger() {
        let dir = home();
        let engine = home();
        let rows = list(&dir, engine.to_str().unwrap(), "gui-cron", 20);
        let ids: Vec<&str> = rows.iter().map(|r| r["id"].as_str().unwrap()).collect();
        assert_eq!(ids, ["cron_job1_20261002_235600", "cron_job1_20261002_235300"], "newest first, by name or id");
        assert_eq!(rows[0]["status"], "failed");
        assert_eq!(rows[1]["status"], "completed");
        assert_eq!(rows[1]["preview"], "CRONOK");
        assert_eq!(list(&dir, engine.to_str().unwrap(), "job1", 1).len(), 1);
        assert!(list(&dir, engine.to_str().unwrap(), "other", 20).is_empty());
        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_dir_all(engine);
    }

    #[test]
    fn a_run_links_to_its_engine_session() {
        let engine = home();
        let e = engine.to_str().unwrap();
        assert_eq!(linked_session(e, "cron_job1_20261003_000000"), None);
        link(e, "cron_job1_20261003_000000", "session_abc");
        assert_eq!(linked_session(e, "cron_job1_20261003_000000").as_deref(), Some("session_abc"));
        assert_eq!(linked_session(e, "chat_x"), None);
        let _ = std::fs::remove_dir_all(engine);
    }
}
