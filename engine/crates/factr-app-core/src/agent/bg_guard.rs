//! Background-task guards: a note when an edit leaves an older background process running stale
//! code, and the readiness recheck at a headless stop. Both read the background manager only.

use crate::background::BackgroundTaskManager;
use crate::bus::BackgroundTaskStatus;
use chrono::{DateTime, Utc};
use std::collections::HashSet;

const READY_WORDS: [&str; 7] = ["listening", "ready", "serving on", "running on", "started server", "server started", "accepting connections"];

/// A background task this run started that may be a server: `flagged` when the model waited on it
/// with `until=` or its output printed a ready line.
#[derive(Debug, Clone)]
pub(super) struct Watch {
    pub id: String,
    pub name: String,
    pub flagged: bool,
}

pub(super) fn looks_ready(output: &str) -> bool {
    let o = output.to_ascii_lowercase();
    READY_WORDS.iter().any(|w| o.contains(w))
}

pub(super) fn stale_note_text(id: &str) -> String {
    format!("bg task {id} started before this edit still runs the old code; cancel and restart it, then bg wait until=<ready line>.")
}

pub(super) fn service_nudge(id: &str, name: &str, code: &str) -> String {
    format!("<system-reminder>bg task {id} ({name}) exited with status {code} since you started it; restart it and check it responds before finishing.</system-reminder>")
}

/// Note for a successful edit: running bash tasks of this session that started before the edit.
/// Once per task id (`warned`).
pub(super) async fn stale_note(mgr: &BackgroundTaskManager, session_id: &str, edit_started: DateTime<Utc>, warned: &mut HashSet<String>) -> Option<String> {
    let mut notes = Vec::new();
    for t in mgr.list().await {
        if t.session_id != session_id || t.tool_name != "bash" || t.status != BackgroundTaskStatus::Running || warned.contains(&t.task_id) {
            continue;
        }
        let before = DateTime::parse_from_rfc3339(&t.started_at).map_or(true, |s| s.with_timezone(&Utc) < edit_started);
        if before {
            warned.insert(t.task_id.clone());
            notes.push(stale_note_text(&t.task_id));
        }
    }
    (!notes.is_empty()).then(|| format!("\n\n{}", notes.join("\n")))
}

/// Append the stale-process note to a successful edit tool's result (`FACTR_GUARD_STALE_BG=0` turns it off).
pub(super) async fn annotate_edit(tool: &str, session_id: &str, started: DateTime<Utc>, warned: &mut HashSet<String>, output: &mut crate::tool::ToolOutput) {
    if !super::stop_nudge::EDIT_TOOLS.contains(&tool) || !super::stop_nudge::guard_on("STALE_BG") || super::auto_verify::exit_code_of(&output.output, false) != 0 {
        return;
    }
    if let Some(note) = stale_note(crate::background::global(), session_id, started, warned).await {
        output.output.push_str(&note);
    }
}

/// Watched tasks that are no longer running: (id, name, status code).
pub(super) async fn exited_services(mgr: &BackgroundTaskManager, watch: &[Watch]) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    for w in watch {
        let Some(t) = mgr.status(&w.id).await else { continue };
        if t.status == BackgroundTaskStatus::Running {
            continue;
        }
        let clean = t.status == BackgroundTaskStatus::Completed && t.exit_code.unwrap_or(0) == 0;
        if w.flagged || (t.duration_secs.unwrap_or(0.0) > 5.0 && !clean) {
            let code = t.exit_code.map_or_else(|| format!("{:?}", t.status).to_ascii_lowercase(), |c| c.to_string());
            out.push((w.id.clone(), w.name.clone(), code));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    async fn spawn_sleeper(m: &BackgroundTaskManager, session: &str, exit_after: Option<Duration>) -> String {
        m.spawn("bash", session, move |path| async move {
            tokio::fs::write(&path, "listening on :8080\n").await?;
            match exit_after {
                Some(d) => {
                    tokio::time::sleep(d).await;
                    Ok(crate::background::TaskResult::failed(Some(3), "exit 3".to_string()))
                }
                None => {
                    tokio::time::sleep(Duration::from_secs(60)).await;
                    Ok(crate::background::TaskResult::completed(Some(0)))
                }
            }
        })
        .await
        .task_id
    }

    #[tokio::test]
    async fn stale_note_names_running_tasks_of_this_session_once() {
        let dir = tempfile::TempDir::new().unwrap();
        let m = BackgroundTaskManager::with_output_dir(dir.path().into());
        let mine = spawn_sleeper(&m, "s1", None).await;
        let other = spawn_sleeper(&m, "s2", None).await;
        let mut warned = HashSet::new();
        let edit = Utc::now() + chrono::Duration::seconds(1);
        let note = stale_note(&m, "s1", edit, &mut warned).await.unwrap();
        assert!(note.contains(&stale_note_text(&mine)) && !note.contains(&other));
        assert!(stale_note(&m, "s1", edit, &mut warned).await.is_none(), "once per task id");
        let mut fresh = HashSet::new();
        assert!(stale_note(&m, "s1", Utc::now() - chrono::Duration::seconds(60), &mut fresh).await.is_none(), "started after the edit");
        m.cancel(&mine).await.unwrap();
        assert!(stale_note(&m, "s1", edit, &mut HashSet::new()).await.is_none(), "no longer running");
        m.cancel(&other).await.unwrap();
    }

    #[tokio::test]
    async fn an_edit_result_gets_the_note_through_the_global_manager() {
        let session = format!("stale-test-{}", std::process::id());
        let id = spawn_sleeper(crate::background::global(), &session, None).await;
        let mut out = crate::tool::ToolOutput::new("Edited a.py".to_string());
        let mut warned = HashSet::new();
        let at = Utc::now() + chrono::Duration::seconds(1);
        annotate_edit("edit", &session, at, &mut warned, &mut out).await;
        assert!(out.output.ends_with(&stale_note_text(&id)), "{}", out.output);
        let mut again = crate::tool::ToolOutput::new("Edited a.py".to_string());
        annotate_edit("edit", &session, at, &mut warned, &mut again).await;
        assert_eq!(again.output, "Edited a.py");
        let mut other = crate::tool::ToolOutput::new("ok".to_string());
        annotate_edit("read", &session, at, &mut HashSet::new(), &mut other).await;
        assert_eq!(other.output, "ok", "not an edit tool");
        crate::background::global().cancel(&id).await.unwrap();
    }

    #[tokio::test]
    async fn exited_service_reported_running_one_not() {
        let dir = tempfile::TempDir::new().unwrap();
        let m = BackgroundTaskManager::with_output_dir(dir.path().into());
        let live = spawn_sleeper(&m, "s", None).await;
        let dead = spawn_sleeper(&m, "s", Some(Duration::from_millis(50))).await;
        tokio::time::sleep(Duration::from_millis(400)).await;
        let watch = |id: &str, flagged| Watch { id: id.to_string(), name: "srv".into(), flagged };
        assert!(exited_services(&m, &[watch(&live, true)]).await.is_empty(), "still running");
        let got = exited_services(&m, &[watch(&live, true), watch(&dead, true)]).await;
        assert_eq!(got, vec![(dead.clone(), "srv".to_string(), "3".to_string())]);
        assert!(exited_services(&m, &[watch(&dead, false)]).await.is_empty(), "unflagged and ran under 5s");
        m.cancel(&live).await.unwrap();
    }
}
