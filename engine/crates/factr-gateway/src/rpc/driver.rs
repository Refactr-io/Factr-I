//! The one engine-level owner of goal and loop (heartbeat) continuations.
//!
//! It is not a desktop window: a single task started with the gateway holds
//! its own engine link (`Conn { driver: true }`), so goals keep going with no
//! window attached, resume after an engine restart, and heartbeats fire from a
//! timer. The link watches only sessions that have an active goal, loop or
//! heartbeat, and yields (renders and observes nothing) for any session a
//! window has open, since that window's own connection does all of it.
//! Continuations are sent through the normal `prompt.submit` path, so tracing
//! and learning see them like any other turn.
//!
//! No idle cost: with no active goal, loop or heartbeat the task just waits
//! for a poke (a finished turn, `/goal`, `/heartbeat`, `session.control`).

use super::*;
use factr_learn::agent_loop::{Continuation, ControlStore, ErrorAction, after_error_turn, after_turn_in, apply_supervisor, due_heartbeat, resume_prompt, retry_due};
use std::sync::OnceLock;
use tokio::sync::Notify;

/// Heartbeat resolution while something is active (heartbeats are >= 60 s).
const TICK: Duration = Duration::from_secs(15);

struct Driver {
    conn: Arc<Conn>,
    wake: Notify,
    /// Sessions found active when this process started, until their resume prompt is accepted.
    resume: std::sync::Mutex<Option<std::collections::HashSet<String>>>,
    /// When each parked goal was first seen waiting on sub-agents.
    waiting_since: std::sync::Mutex<HashMap<String, std::time::Instant>>,
    /// Desktop window connections: a loop on a session a window has open fires through that window.
    windows: std::sync::Mutex<Vec<std::sync::Weak<Conn>>>,
    /// Sessions a window just asked for: the driver keeps its hands off until the window's attach lands.
    yielded: std::sync::Mutex<HashMap<String, std::time::Instant>>,
}

/// How long the driver stays away from a session a window asked for (its attach is retried within this).
const YIELD_FOR: Duration = Duration::from_secs(30);

/// A window connection opened: the driver fires due loops for the sessions it has open.
pub(crate) fn register_window(conn: &Arc<Conn>) {
    if let Some(driver) = DRIVER.get() {
        let mut windows = driver.windows.lock().unwrap_or_else(|e| e.into_inner());
        windows.retain(|w| w.strong_count() > 0);
        windows.push(Arc::downgrade(conn));
    }
}

static DRIVER: OnceLock<Arc<Driver>> = OnceLock::new();

/// Something may have become active: re-scan now.
pub(crate) fn poke() {
    if let Some(driver) = DRIVER.get() {
        driver.wake.notify_one();
    }
}

/// A window wants `session_id`: the driver lets go of its engine link to it (the window renders,
/// observes and prompts it now). The next scan skips it while the window has it open.
pub(crate) async fn release(session_id: &str) {
    let Some(driver) = DRIVER.get() else { return };
    driver.yield_to_window(session_id).await;
}

/// Whether the driver let go of `session_id` for a window that asked for it.
fn yielded_to_window(session_id: &str) -> bool {
    DRIVER.get().is_some_and(|driver| driver.yielded.lock().unwrap_or_else(|e| e.into_inner()).contains_key(session_id))
}

/// A parked goal's sub-agents finished: send its continuation on the next scan.
pub(super) fn resume_soon(session_id: &str) {
    if let Some(driver) = DRIVER.get() {
        if let Some(resume) = driver.resume.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            resume.insert(session_id.to_string());
        }
    }
    poke();
}

/// The parent of `session_id` when it was the last of that parent's children still running.
async fn parent_left_waiting(conn: &Arc<Conn>, waiting: &[String], session_id: &str) -> Option<String> {
    let reply = conn.call(json!({ "req": "list_sessions" })).await.ok()?;
    let list = reply["sessions"].as_array()?;
    let parent = list.iter().find(|s| s["session_id"].as_str() == Some(session_id))?["parent_session_id"].as_str()?;
    if !waiting.iter().any(|w| w == parent) {
        return None;
    }
    let running = children_running(list, parent, Some(session_id), &*conn.sessions.lock().await);
    (!running).then(|| parent.to_string())
}

/// Whether process `pid` exists (it may belong to another user: that still counts).
fn pid_alive(pid: i64) -> bool {
    u32::try_from(pid).is_ok_and(factr_base::platform::is_process_running)
}

/// A parked goal gives up on its sub-agents after this long and carries on.
const WAIT_CAP: Duration = Duration::from_secs(30 * 60);

/// Waiting sessions whose sub-agents are done (per the engine's `list`) or have overrun `cap`:
/// `(session, timed_out)`. `since` tracks when each was first seen waiting.
fn released_waits(waiting: &[String], since: &mut HashMap<String, std::time::Instant>, list: &[Value], live: &HashMap<String, SessionState>, cap: Duration) -> Vec<(String, bool)> {
    since.retain(|sid, _| waiting.contains(sid));
    waiting
        .iter()
        .filter_map(|sid| {
            let timed_out = since.entry(sid.clone()).or_insert_with(std::time::Instant::now).elapsed() >= cap;
            (timed_out || !children_running(list, sid, None, live)).then(|| (sid.clone(), timed_out))
        })
        .collect()
}

/// Start the driver task (once, with the gateway).
pub(crate) fn start(config: Arc<Config>, hub: Arc<Hub>, observer: Arc<Observer>) {
    tokio::spawn(async move {
        let (to_ws, mut ws_out) = mpsc::channel::<Message>(1024);
        tokio::spawn(async move { while ws_out.recv().await.is_some() {} });
        let client = Arc::new(Client {
            id: hub.next_client_id(),
            to_ws: to_ws.clone(),
            sessions: Mutex::new(Default::default()),
        });
        let conn = Conn::new(config, to_ws, hub, client, observer, true, "invoke_agent", None, None);
        let driver = Arc::new(Driver { conn, wake: Notify::new(), resume: Default::default(), waiting_since: Default::default(), windows: Default::default(), yielded: Default::default() });
        let _ = DRIVER.set(driver.clone());
        let (mut failures, mut last_error) = (0u32, String::new());
        loop {
            // The engine may be down at start or restart later: keep trying, forever, with backoff.
            if let Err(err) = driver.conn.ensure_control().await {
                let err = format!("{err:#}");
                if err != last_error {
                    eprintln!("factr: goal driver could not reach the engine (retrying): {err}");
                    last_error = err;
                }
                let delay = retry_delay(failures);
                failures = failures.saturating_add(1);
                tokio::select! {
                    _ = tokio::time::sleep(delay) => {}
                    _ = driver.wake.notified() => {}
                }
                continue;
            }
            (failures, last_error) = (0, String::new());
            report_backup_error(&driver.conn.hub, factr_base::migrate::backup_error()).await;
            let active = supervised(&driver.conn.hub, { let d = driver.clone(); async move { d.tick().await } }).await;
            if active {
                tokio::select! {
                    _ = tokio::time::sleep(TICK) => {}
                    _ = driver.wake.notified() => {}
                }
            } else {
                driver.wake.notified().await;
            }
        }
    });
}

/// Run one tick in its own task: a panic is logged and shown, and the loop carries on (the
/// session stays active, so the next tick retries) instead of the driver dying silently.
async fn supervised(hub: &Hub, tick: impl std::future::Future<Output = bool> + Send + 'static) -> bool {
    match tokio::spawn(tick).await {
        Ok(active) => active,
        Err(err) => {
            error_once(hub, "goal driver", format!("The goal driver hit an internal error and restarted; goals and heartbeats continue: {err}")).await;
            true
        }
    }
}

/// Seconds to wait after `failures` failed engine connects: 1, 2, 4 ... capped at 30.
fn retry_delay(failures: u32) -> Duration {
    Duration::from_secs((1u64 << failures.min(5)).min(30))
}

/// The user approved a command this goal session was denied while unattended: tell it so it can
/// go on. Only sessions with an active goal, loop or heartbeat are resumed (a cron or bot run has
/// ended; its late approval just lets the next run through).
pub(crate) fn resume(session_id: &str, command: &str) {
    let Some(driver) = DRIVER.get().cloned() else { return };
    let (sid, text) = (
        session_id.to_string(),
        format!("The user has now approved this command: {command}\nYou may run it; carry on with the task."),
    );
    tokio::spawn(async move {
        let active = ControlStore::open_cached(Path::new(&driver.conn.config.home)).and_then(|s| s.active_sessions());
        if active.is_ok_and(|a| a.contains(&sid)) {
            if let Err(err) = driver.conn.dispatch("prompt.submit", &json!({ "session_id": sid, "text": text })).await {
                eprintln!("factr: resume {sid} after approval: {}", err.message);
            }
        }
    });
}

fn resume_goals_on_start() -> bool {
    std::env::var("FACTR_RESUME_GOALS").is_ok_and(|v| matches!(v.trim(), "1" | "true" | "yes" | "on"))
}

/// Pause every active goal and every user loop (`/loop`): nothing the user set going may run again
/// just because the app was relaunched. Returns the sessions that have nothing active left.
/// (Agent-made RLM heartbeats keep running.)
fn pause_for_restart(store: &ControlStore) -> Vec<String> {
    use factr_learn::agent_loop::{GoalStatus, HeartbeatStatus};
    let reason = "the app was restarted; /goal resume to continue".to_string();
    let mut paused = Vec::new();
    for sid in store.active_sessions().unwrap_or_default() {
        let mut hit = false;
        if let Ok(Some(mut g)) = store.get_goal(&sid) {
            if g.status == GoalStatus::Active {
                g.status = GoalStatus::Paused;
                g.paused_reason = Some(reason.clone());
                g.updated_at_ms = factr_learn::agent_loop::now_ms();
                hit = store.set_goal(&sid, Some(&g)).is_ok();
            }
        }
        for mut hb in store.list_heartbeats(&sid).unwrap_or_default() {
            if hb.source == "user" && hb.status == HeartbeatStatus::Active {
                hb.status = HeartbeatStatus::Paused;
                hit |= store.upsert_heartbeat(&hb).is_ok();
            }
        }
        if hit {
            paused.push(sid);
        }
    }
    // A session with something else still active (an RLM heartbeat) stays on the work list.
    paused.retain(|sid| !store.active_sessions().unwrap_or_default().contains(sid));
    paused
}

fn prompt_of(continuation: Continuation) -> String {
    match continuation {
        Continuation::Goal(p) => p,
        Continuation::Heartbeat { prompt, .. } => prompt,
    }
}

/// Prompts to send now: for each idle active session a due heartbeat, and for the sessions in
/// `resume` (active at process start, so nothing is running) the goal or loop continuation, so
/// work resumes where it stopped. A session stays in `resume` until the caller has sent its prompt.
fn due_work(store: &ControlStore, active: &[String], resume: &mut std::collections::HashSet<String>, busy: impl Fn(&str) -> bool) -> Vec<(String, String)> {
    resume.retain(|sid| active.contains(sid) && !busy(sid));
    active
        .iter()
        .filter(|sid| !busy(sid))
        .filter_map(|sid| {
            let resumed = if resume.contains(sid) {
                let cont = resume_prompt(store, sid).ok().flatten();
                if cont.is_none() {
                    resume.remove(sid);
                }
                cont
            } else {
                None
            };
            let cont = resumed.or_else(|| due_heartbeat(store, sid).ok().flatten())?;
            Some((sid.clone(), prompt_of(cont)))
        })
        .collect()
}

impl Driver {
    /// Let go of `session_id` for a window that wants it (at the end of a turn the driver is running
    /// on it), and stay away until its attach lands.
    async fn yield_to_window(&self, session_id: &str) {
        self.yielded.lock().unwrap_or_else(|e| e.into_inner()).insert(session_id.to_string(), std::time::Instant::now());
        self.conn.release_at_turn_end(session_id).await;
    }

    /// Drop `sid` from the resume queue: its continuation was sent, or it has none.
    fn resumed(&self, sid: &str) {
        if let Some(resume) = self.resume.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            resume.remove(sid);
        }
    }

    /// For each session a window has open that is idle with a due loop: the window's connection and
    /// `(session, prompt)`. Marks the tick fired (as `due_heartbeat` does), so call it once per scan.
    async fn windowed_due(&self, store: &ControlStore, windowed: &[String]) -> Vec<(Arc<Conn>, String, String)> {
        let windows: Vec<Arc<Conn>> = self.windows.lock().unwrap_or_else(|e| e.into_inner()).iter().filter_map(std::sync::Weak::upgrade).collect();
        let mut out = Vec::new();
        for sid in windowed {
            let mut owner = None;
            for w in &windows {
                if w.client.sessions.lock().await.contains(sid) && w.links.lock().await.contains_key(sid) {
                    owner = Some(w.clone());
                    break;
                }
            }
            let Some(window) = owner else { continue };
            let busy = window.sessions.lock().await.get(sid).is_some_and(SessionState::turn_active) || window.observer.has_active_run(sid);
            if busy {
                continue;
            }
            // A goal waiting to go on (its sub-agents finished, or the driver handed it over at the
            // end of a turn) goes on through the window too; the caller drops it once sent.
            let resumed = self.resume.lock().unwrap_or_else(|e| e.into_inner()).as_ref().is_some_and(|r| r.contains(sid));
            let cont = resumed.then(|| resume_prompt(store, sid).ok().flatten()).flatten();
            if resumed && cont.is_none() {
                self.resumed(sid);
            }
            if let Some(cont) = cont.or_else(|| due_heartbeat(store, sid).ok().flatten()) {
                out.push((window, sid.clone(), prompt_of(cont)));
            }
        }
        out
    }

    /// One scan: watch the active sessions, send due work. `false` when
    /// nothing is active (the task then sleeps until poked).
    async fn tick(&self) -> bool {
        let conn = &self.conn;
        let Some(store) = conn.control_store().await else { return false };
        let mut active = store.active_sessions().unwrap_or_default();
        let first = {
            let mut resume = self.resume.lock().unwrap_or_else(|e| e.into_inner());
            let first = resume.is_none();
            resume.get_or_insert_with(|| active.iter().cloned().collect());
            first
        };
        if first {
            // Nothing is running after a restart: whatever a goal was waiting on is over.
            for sid in store.waiting_sessions().unwrap_or_default() {
                let _ = store.clear_wait(&sid);
            }
            // A goal or loop must not run again just because the app was relaunched (it ran for
            // hours unseen): pause it, `/goal resume` continues. Opt in to unattended resume with
            // FACTR_RESUME_GOALS=1.
            if !resume_goals_on_start() {
                let paused = pause_for_restart(&store);
                for sid in &paused {
                    active.retain(|a| a != sid);
                }
                self.resume.lock().unwrap_or_else(|e| e.into_inner()).as_mut().unwrap().retain(|sid| !paused.contains(sid));
            }
        }
        // A parked goal's sub-agents may finish on a connection nobody watches: ask the engine.
        let mut notes = HashMap::new();
        let waiting = store.waiting_sessions().unwrap_or_default();
        if !waiting.is_empty() {
            match conn.call(json!({ "req": "list_sessions" })).await {
                Ok(reply) => {
                    let list = reply["sessions"].as_array().map_or(&[][..], Vec::as_slice);
                    let live = conn.sessions.lock().await;
                    let released = released_waits(&waiting, &mut self.waiting_since.lock().unwrap_or_else(|e| e.into_inner()), list, &live, WAIT_CAP);
                    drop(live);
                    for (sid, timed_out) in released {
                        if store.clear_wait(&sid).unwrap_or(false) {
                            if timed_out {
                                notes.insert(sid.clone(), "Note: your sub-agents did not finish within 30 minutes and timed out; continue without their results.\n\n".to_string());
                            }
                            resume_soon(&sid);
                        }
                    }
                }
                Err(err) => eprintln!("factr: goal driver could not list sub-agents: {err:#}"),
            }
        }
        // A goal parked on a process (`/goal wait <pid>`) goes on once that process is gone.
        for (sid, pid) in store.pid_waits().unwrap_or_default() {
            if !pid_alive(pid) && store.clear_wait(&sid).unwrap_or(false) {
                notes.insert(sid.clone(), format!("Note: the process you were waiting on (pid {pid}) has exited.\n\n"));
                resume_soon(&sid);
            }
        }
        let mut gone = Vec::new();
        // A session a window has open is the window's: holding it here would make the window's own
        // attach fail as "already live" (the relaunch "Couldn't load this session").
        let mut windowed = Vec::new();
        self.yielded.lock().unwrap_or_else(|e| e.into_inner()).retain(|_, at| at.elapsed() < YIELD_FOR);
        for sid in &active {
            let yielded = self.yielded.lock().unwrap_or_else(|e| e.into_inner()).contains_key(sid);
            if yielded || conn.hub.has_window(sid).await {
                windowed.push(sid.clone());
            }
        }
        // Nothing ticks a loop inside a window's own connection (it only reacts to finished turns),
        // so a due loop on a session a window has open is fired here, through that window.
        for (window, sid, text) in self.windowed_due(&store, &windowed).await {
            let text = format!("{}{text}", notes.remove(&sid).unwrap_or_default());
            match window.dispatch("prompt.submit", &json!({ "session_id": sid, "text": text })).await {
                Ok(_) => {
                    self.resumed(&sid);
                }
                Err(err) => eprintln!("factr: loop submit for {sid}: {}", err.message),
            }
        }
        for sid in &active {
            if windowed.contains(sid) {
                continue;
            }
            if let Err(err) = conn.ensure_attached(sid).await {
                // A chat deleted behind our back (its file is gone) has no work left.
                if !factr_base::session::session_exists(sid) {
                    if let Err(err) = crate::sessions_rest::forget_rows(&conn.config.home, sid) {
                        eprintln!("factr: {err:#}");
                    }
                    gone.push(sid.clone());
                } else if first {
                    eprintln!("factr: goal driver cannot attach {sid}: {err:#}");
                }
            }
        }
        active.retain(|sid| !gone.contains(sid));
        let watching: Vec<String> = active.iter().filter(|sid| !windowed.contains(sid)).cloned().collect();
        let stale: Vec<String> = conn.links.lock().await.keys().filter(|k| !watching.contains(k)).cloned().collect();
        for sid in stale {
            conn.release_at_turn_end(&sid).await;
        }
        let busy: std::collections::HashSet<String> = {
            let sessions = conn.sessions.lock().await;
            watching
                .iter()
                .filter(|sid| sessions.get(*sid).is_some_and(SessionState::turn_active) || conn.observer.has_active_run(sid))
                .cloned()
                .collect()
        };
        let work = due_work(&store, &watching, self.resume.lock().unwrap_or_else(|e| e.into_inner()).as_mut().unwrap(), |sid| busy.contains(sid));
        for (sid, text) in work {
            let text = format!("{}{text}", notes.remove(&sid).unwrap_or_default());
            match conn.dispatch("prompt.submit", &json!({ "session_id": sid, "text": text })).await {
                Ok(_) => {
                    self.resume.lock().unwrap_or_else(|e| e.into_inner()).as_mut().unwrap().remove(&sid);
                }
                Err(err) => eprintln!("factr: goal driver submit for {sid}: {}", err.message),
            }
        }
        !active.is_empty()
    }
}

/// AVO self-supervisor: at most one cheap aux call per plateau episode (and
/// never within 5 turns of the last), asking for 2-3 alternative strategies
/// injected once into this continuation. Any failure keeps the text steer.
async fn supervise(conn: &Arc<Conn>, store: &ControlStore, sid: &str, fallback: String) -> String {
    let Ok(Some(goal)) = store.get_goal(sid) else { return fallback };
    if !goal.supervisor_due() {
        return fallback;
    }
    let (system, user) = goal.supervisor_request();
    let started = crate::observability::now();
    let reply = match conn.config.complete.clone() {
        Some(complete) => complete(system, user).await,
        None => Err(anyhow::anyhow!("no model available")),
    };
    conn.observer.record_aux(
        sid, "other", Some("Goal supervisor"), None, None, started,
        reply.as_ref().ok().and_then(|d| d.usage), reply.as_ref().err().map(|e| e.to_string()).as_deref(),
    );
    apply_supervisor(store, sid, reply.ok().as_ref().map(|d| d.text.as_str())).unwrap_or(fallback)
}

/// After a completed turn on `conn`: maybe inject the goal / loop / heartbeat
/// continuation through that connection, then poke the driver so it watches
/// (or stops watching) the session.
pub(super) fn turn_done(conn: Arc<Conn>, session_id: String, payload: Value) {
    tokio::spawn(async move {
        let Some(store) = conn.control_store().await else { return };
        // A sub-agent finishing may be the last thing its parent's goal was waiting on.
        if let Some(waiting) = store.waiting_sessions().ok().filter(|w| !w.is_empty()) {
            if let Some(parent) = parent_left_waiting(&conn, &waiting, &session_id).await {
                if store.clear_wait(&parent).unwrap_or(false) {
                    resume_soon(&parent);
                }
            }
        }
        let usage = &payload["usage"];
        let tokens = usage["total"]
            .as_u64()
            .unwrap_or_else(|| usage["input"].as_u64().unwrap_or(0) + usage["output"].as_u64().unwrap_or(0)) as i64;
        let interrupted = payload["status"].as_str() == Some("interrupted");
        let busy = || async { conn.sessions.lock().await.get(&session_id).is_some_and(SessionState::turn_active) };
        if payload["status"].as_str() == Some("error") {
            let error = payload["error"].as_str().unwrap_or("model error");
            match after_error_turn(&store, &session_id, error) {
                Ok(Some(ErrorAction::Paused(text))) => conn.emit("status.update", Some(&session_id), json!({ "kind": "status", "text": format!("Goal paused: {text}") })).await,
                Ok(Some(ErrorAction::Retry { after, stamp, prompt, note })) => {
                    conn.emit("status.update", Some(&session_id), json!({ "kind": "status", "text": note })).await;
                    tokio::time::sleep(after).await;
                    if retry_due(&store, &session_id, stamp) && !busy().await {
                        let text = prompt_of(prompt);
                        if let Err(err) = conn.dispatch("prompt.submit", &json!({ "session_id": session_id, "text": text })).await {
                            eprintln!("factr: error retry for {session_id}: {}", err.message);
                        }
                    }
                }
                Ok(None) => {}
                Err(err) => eprintln!("factr: after_error_turn for {session_id}: {err:#}"),
            }
            return poke();
        }
        let subagents_running = conn.child_sessions_running(&session_id).await;
        let cwd = conn.session_cwd(&session_id).await;
        // Ratchet checkpoints shell out to git: keep that off the async workers.
        let turn = {
            let (store, sid, cwd) = (store.clone(), session_id.clone(), cwd.clone());
            tokio::task::spawn_blocking(move || after_turn_in(&store, &sid, tokens, subagents_running, interrupted, cwd.as_deref().map(Path::new)))
                .await
                .unwrap_or_else(|e| Err(anyhow::anyhow!(e)))
        };
        let continuation = match turn {
            Ok(None) if !interrupted && !busy().await => due_heartbeat(&store, &session_id).ok().flatten(),
            Ok(next) => next,
            Err(err) => return eprintln!("factr: after_turn for {session_id}: {err:#}"),
        };
        let continuation = match continuation {
            Some(Continuation::Goal(p)) => Some(Continuation::Goal(supervise(&conn, &store, &session_id, p).await)),
            other => other,
        };
        // The driver let go of this chat at the end of this turn: the window that asked for it sends
        // the continuation once its attach lands (see `windowed_due`).
        match continuation {
            // A closed window's turn: no window sends it, so the driver does (its next scan finds the chat free).
            Some(_) if (conn.driver && yielded_to_window(&session_id)) || conn.is_detached() => resume_soon(&session_id),
            Some(cont) => {
                tokio::time::sleep(Duration::from_millis(400)).await;
                if !busy().await {
                    let text = prompt_of(cont);
                    if let Err(err) = conn.dispatch("prompt.submit", &json!({ "session_id": session_id, "text": text })).await {
                        eprintln!("factr: agent loop submit for {session_id}: {}", err.message);
                    }
                }
            }
            None => {}
        }
        // The goal may have completed or the loop run out: the window's chip follows the store.
        conn.emit_control(&session_id).await;
        poke();
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use factr_learn::agent_loop::{Heartbeat, SessionGoal};

    #[tokio::test]
    async fn a_goal_turn_lost_with_its_bridge_is_resumed() {
        let conn = crate::rpc::tests::test_conn_as("link-lost", true);
        let store = ControlStore::open_cached(Path::new(&conn.config.home)).unwrap();
        store.set_goal("s", Some(&SessionGoal::new("ship the parser"))).unwrap();
        let driver = Arc::new(Driver { conn: conn.clone(), wake: Notify::new(), resume: std::sync::Mutex::new(Some(Default::default())), waiting_since: Default::default(), windows: Default::default(), yielded: Default::default() });
        let _ = DRIVER.set(driver.clone());
        // Mid-turn on a session link: the turn is running and the observer has a run open.
        conn.ensure_control().await.unwrap();
        let link = conn.control.lock().await.clone().unwrap();
        conn.links.lock().await.insert("s".into(), link.clone());
        conn.sessions.lock().await.entry("s".into()).or_default().mark_running();
        conn.observer.start_turn("s", "work", "invoke_agent", None);
        assert!(conn.sessions.lock().await.get("s").is_some_and(SessionState::turn_active) && conn.observer.has_active_run("s"));
        crate::rpc::tests::tasks_of(&conn, &link).await.bridge.abort(); // the bridge drops; message.complete is lost
        tokio::time::timeout(Duration::from_secs(5), async {
            while conn.control.lock().await.is_some() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the dead link is forgotten");
        tokio::time::sleep(Duration::from_millis(50)).await;
        let busy = conn.sessions.lock().await.get("s").is_some_and(SessionState::turn_active) || conn.observer.has_active_run("s");
        assert!(!busy, "the lost turn no longer wedges the session");
        let mut resume = driver.resume.lock().unwrap().take().unwrap();
        assert!(resume.contains("s"), "queued for resume");
        let work = due_work(&store, &["s".to_string()], &mut resume, |_| busy);
        assert_eq!(work.len(), 1, "the driver resumes the goal");
        assert!(work[0].1.contains("ship the parser"));
    }

    /// A loop on a chat a window has open fires through that window (nothing else ticks it), once per
    /// due tick, until `--times` is spent; a busy window is left alone.
    #[tokio::test]
    async fn a_loop_on_a_windowed_session_fires_n_times_then_stops() {
        let conn = crate::rpc::tests::test_conn_as("loop-window", true);
        let window = crate::rpc::tests::test_conn_as("loop-window-w", false);
        let store = ControlStore::open_cached(Path::new(&conn.config.home)).unwrap();
        let driver = Driver { conn: conn.clone(), wake: Notify::new(), resume: Default::default(), waiting_since: Default::default(), windows: Default::default(), yielded: Default::default() };
        driver.windows.lock().unwrap().push(Arc::downgrade(&window));
        window.ensure_control().await.unwrap();
        let link = window.control.lock().await.clone().unwrap();
        window.links.lock().await.insert("lw".into(), link);
        window.client.sessions.lock().await.insert("lw".into());
        let msg = factr_learn::agent_loop::handle_heartbeat_command(&store, "lw", "every 1m say hello --times 2").unwrap();
        assert!(msg.contains("Stops after 2 ticks"), "{msg}");
        let sids = ["lw".to_string()];
        let make_due = || {
            let mut hb = store.user_heartbeat("lw").unwrap().unwrap();
            hb.last_fired_at_ms = factr_learn::agent_loop::now_ms() - 61_000;
            store.upsert_heartbeat(&hb).unwrap();
        };
        assert!(driver.windowed_due(&store, &sids).await.is_empty(), "not due yet");
        make_due();
        window.sessions.lock().await.entry("lw".into()).or_default().mark_running();
        assert!(driver.windowed_due(&store, &sids).await.is_empty(), "a busy window is left alone");
        window.sessions.lock().await.entry("lw".into()).or_default().end_turn();
        for n in 1..=2 {
            make_due();
            let due = driver.windowed_due(&store, &sids).await;
            assert_eq!(due.len(), 1, "tick {n}");
            assert!(due[0].2.contains("say hello") && due[0].2.contains(&format!("#{n}/2")), "{}", due[0].2);
        }
        make_due();
        assert!(driver.windowed_due(&store, &sids).await.is_empty(), "the loop is done after 2 ticks");
        let status = factr_learn::agent_loop::handle_heartbeat_command(&store, "lw", "status").unwrap();
        assert!(status.contains("fired 2") && status.contains("2/2 ticks"), "{status}");
    }

    /// The driver does not hold a session a window just asked for: it lets go of its link and the next
    /// scan treats the session as the window's until the attach lands.
    #[tokio::test]
    async fn a_released_session_stays_the_windows_until_its_attach_lands() {
        let conn = crate::rpc::tests::test_conn_as("yield", true);
        let driver = Driver { conn: conn.clone(), wake: Notify::new(), resume: Default::default(), waiting_since: Default::default(), windows: Default::default(), yielded: Default::default() };
        conn.ensure_control().await.unwrap();
        let link = conn.control.lock().await.clone().unwrap();
        conn.links.lock().await.insert("ys".into(), link);
        driver.yield_to_window("ys").await;
        assert!(driver.yielded.lock().unwrap().contains_key("ys"));
        assert!(!conn.links.lock().await.contains_key("ys"), "the driver's link to the session is dropped");
    }

    /// A goal the driver handed to a window at the end of a turn (or whose sub-agents finished while a
    /// window had it open) goes on through that window: nothing else would send its continuation.
    #[tokio::test]
    async fn a_goal_handed_to_a_window_goes_on_through_it() {
        let conn = crate::rpc::tests::test_conn_as("handoff", true);
        let window = crate::rpc::tests::test_conn_as("handoff-w", false);
        let store = ControlStore::open_cached(Path::new(&conn.config.home)).unwrap();
        store.set_goal("hg", Some(&SessionGoal::new("ship the parser"))).unwrap();
        let driver = Driver { conn: conn.clone(), wake: Notify::new(), resume: std::sync::Mutex::new(Some(["hg".to_string()].into())), waiting_since: Default::default(), windows: Default::default(), yielded: Default::default() };
        driver.windows.lock().unwrap().push(Arc::downgrade(&window));
        window.ensure_control().await.unwrap();
        let link = window.control.lock().await.clone().unwrap();
        window.links.lock().await.insert("hg".into(), link);
        window.client.sessions.lock().await.insert("hg".into());
        let due = driver.windowed_due(&store, &["hg".to_string()]).await;
        assert_eq!(due.len(), 1);
        assert!(due[0].2.contains("ship the parser"), "{}", due[0].2);
        driver.resumed("hg");
        assert!(driver.windowed_due(&store, &["hg".to_string()]).await.is_empty(), "sent once");
    }

    /// `/goal clear` tells the window the goal is gone, so the composer footer drops "Goal active".
    #[tokio::test]
    async fn goal_clear_pushes_the_cleared_control_state_to_the_window() {
        let (conn, mut rx) = crate::rpc::tests::test_conn_events("goal-clear", false);
        let store = ControlStore::open_cached(Path::new(&conn.config.home)).unwrap();
        store.set_goal("gc", Some(&SessionGoal::new("ship it"))).unwrap();
        let said = conn.harness_command(&["goal", "clear"], Some("gc")).await.unwrap();
        assert!(said.contains("Goal cleared"), "{said}");
        let mut update = None;
        while let Ok(Message::Text(frame)) = rx.try_recv() {
            let v: Value = serde_json::from_str(&frame).unwrap();
            if v["params"]["type"] == "session.control.update" {
                update = Some(v);
            }
        }
        let update = update.expect("a session.control.update frame");
        assert_eq!(update["params"]["session_id"], "gc");
        assert!(update["params"]["payload"]["control"]["goal"].is_null(), "{update}");
    }

    #[tokio::test]
    async fn a_panicking_tick_does_not_stop_the_next_one() {
        let hub = Hub::default();
        assert!(supervised(&hub, async { panic!("tick blew up") }).await, "the panic keeps the loop going");
        assert!(!supervised(&hub, async { false }).await, "the next tick runs normally");
    }

    #[test]
    fn engine_reconnects_back_off_from_one_to_thirty_seconds() {
        let secs: Vec<u64> = (0..8).map(|n| retry_delay(n).as_secs()).collect();
        assert_eq!(secs, [1, 2, 4, 8, 16, 30, 30, 30]);
        assert_eq!(retry_delay(u32::MAX).as_secs(), 30);
    }

    #[test]
    fn a_parked_goal_is_released_by_the_engines_view_of_its_children_or_the_cap() {
        let list = |status: &str| vec![json!({ "session_id": "child", "parent_session_id": "parent", "status": status })];
        let waiting = ["parent".to_string()];
        let (mut since, live) = (HashMap::new(), HashMap::new());
        let day = Duration::from_secs(86400);
        // A child the engine reports running (on any connection) keeps the parent parked.
        assert!(released_waits(&waiting, &mut since, &list("running"), &live, day).is_empty());
        // Once it is no longer running, the parent is released without a timeout.
        assert_eq!(released_waits(&waiting, &mut since, &list("idle"), &live, day), [("parent".to_string(), false)]);
        // A child that never finishes is given up on at the cap.
        assert_eq!(released_waits(&waiting, &mut since, &list("running"), &live, Duration::ZERO), [("parent".to_string(), true)]);
        // Sessions no longer waiting are forgotten.
        released_waits(&[], &mut since, &[], &live, day);
        assert!(since.is_empty());
    }

    #[test]
    fn driver_resumes_goals_once_and_fires_due_heartbeats_only_when_idle() {
        let home = std::env::temp_dir().join(format!("driver-{}", std::process::id()));
        let store = ControlStore::open(&home).unwrap();
        store.set_goal("goal", Some(&SessionGoal::new("ship the parser"))).unwrap();
        let mut beat = Heartbeat::new("beat", "check the build", 60);
        beat.last_fired_at_ms = 0;
        store.upsert_heartbeat(&beat).unwrap();
        let active = store.active_sessions().unwrap();
        assert_eq!(active, ["beat", "goal"]);

        let mut resume: std::collections::HashSet<String> = active.iter().cloned().collect();

        // A busy session is left alone (and its heartbeat stays due).
        assert!(due_work(&store, &active, &mut resume, |_| true).is_empty());
        let mut resume: std::collections::HashSet<String> = active.iter().cloned().collect();
        // After a restart: the goal resumes and the due heartbeat fires.
        let work = due_work(&store, &active, &mut resume, |_| false);
        assert_eq!(work.len(), 2);
        assert!(work.iter().any(|(sid, p)| sid == "goal" && p.contains("ship the parser")));
        assert!(work.iter().any(|(sid, p)| sid == "beat" && p.contains("check the build")));
        // A failed submit is retried: the goal stays queued until the caller drops it, and resuming
        // recorded no turn and no attempt.
        let again = due_work(&store, &active, &mut resume, |_| false);
        assert_eq!(again.len(), 1);
        assert_eq!(again[0].0, "goal");
        let goal = store.get_goal("goal").unwrap().unwrap();
        assert_eq!((goal.turns_used, goal.attempt_log.len()), (0, 0));
        // Once submitted (caller drops it) goals are not re-sent by later ticks.
        resume.remove("goal");
        assert!(due_work(&store, &active, &mut resume, |_| false).is_empty());
        std::fs::remove_dir_all(home).ok();
    }

    #[test]
    fn goals_and_loops_are_paused_on_app_restart() {
        let home = std::env::temp_dir().join(format!("driver-restart-{}", std::process::id()));
        let store = ControlStore::open(&home).unwrap();
        store.set_goal("goal", Some(&SessionGoal::new("ship the parser"))).unwrap();
        store.set_goal("both", Some(&SessionGoal::new("x"))).unwrap();
        store.upsert_heartbeat(&Heartbeat::new("both", "ping", 60)).unwrap();
        let mut looped = Heartbeat::new("loop", "ping", 60);
        looped.times = 2;
        store.upsert_heartbeat(&looped).unwrap();
        let mut rlm = Heartbeat::new("rlm", "watch", 60);
        rlm.source = "rlm".into();
        store.upsert_heartbeat(&rlm).unwrap();
        let mut paused = pause_for_restart(&store);
        paused.sort();
        assert_eq!(paused, ["both", "goal", "loop"], "goals and user loops are paused; sessions with nothing left active drop off the work list");
        for sid in ["goal", "both"] {
            let g = store.get_goal(sid).unwrap().unwrap();
            assert_eq!(g.status, factr_learn::agent_loop::GoalStatus::Paused);
            assert!(g.paused_reason.unwrap().contains("/goal resume"));
        }
        assert_eq!(store.user_heartbeat("loop").unwrap().unwrap().status, factr_learn::agent_loop::HeartbeatStatus::Paused);
        assert_eq!(store.active_sessions().unwrap(), ["rlm"], "only the agent's own heartbeat keeps running");
        std::fs::remove_dir_all(home).ok();
    }

    #[test]
    fn a_goal_parked_for_sub_agents_is_sent_on_once_they_finish() {
        let home = std::env::temp_dir().join(format!("driver-wait-{}", std::process::id()));
        let store = ControlStore::open(&home).unwrap();
        let mut goal = SessionGoal::new("ship the parser");
        goal.waiting_on_subagents = true;
        store.set_goal("parent", Some(&goal)).unwrap();
        assert_eq!(store.waiting_sessions().unwrap(), ["parent"]);
        assert!(store.clear_wait("parent").unwrap());
        let mut resume: std::collections::HashSet<String> = ["parent".to_string()].into();
        let work = due_work(&store, &["parent".to_string()], &mut resume, |_| false);
        assert_eq!(work.len(), 1);
        assert!(!store.get_goal("parent").unwrap().unwrap().waiting_on_subagents);
        std::fs::remove_dir_all(home).ok();
    }
}
