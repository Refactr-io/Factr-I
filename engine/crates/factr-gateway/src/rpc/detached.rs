//! A window that goes away mid-turn (renderer reload, network drop, window closed) does not take its
//! turn with it. Dropping a chat's bridge link makes the engine abort the turn as crashed, so the
//! closed window's connection keeps the link of each turn it was running, here, until that turn ends:
//! the reply is saved, the run closes, learning runs, and the link is let go (`release_after_turn`).
//! A window that opens the chat meanwhile takes the link over and sees the rest of the turn live; the
//! replay buffer covers what it missed.

use super::{Conn, SessionState};
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::{Arc, LazyLock};

/// Running turns of closed windows, by session: the only strong hold on their connection.
static DETACHED: LazyLock<std::sync::Mutex<HashMap<String, Arc<Conn>>>> = LazyLock::new(Default::default);

fn registry() -> std::sync::MutexGuard<'static, HashMap<String, Arc<Conn>>> {
    DETACHED.lock().unwrap_or_else(|e| e.into_inner())
}

/// `sid`'s turn no longer needs its closed window's connection kept (it ended, or a window took it).
pub(super) fn forget(sid: &str, conn: &Conn) {
    let mut held = registry();
    if held.get(sid).is_some_and(|c| std::ptr::eq(Arc::as_ptr(c), conn)) {
        held.remove(sid);
    }
}

/// Whether a closed window's connection still holds `sid`'s running turn.
#[cfg(test)]
pub(super) fn holds(sid: &str) -> bool {
    registry().contains_key(sid)
}

/// The window behind `conn` is gone: keep each turn it was running (not a child it only followed,
/// which runs under its owner) until the turn ends. Returns the sessions kept.
pub(super) async fn keep_running_turns(conn: &Arc<Conn>) -> Vec<String> {
    conn.detached.store(true, Ordering::Release);
    // No window renders these chats now: the driver and the approval hub must not count this one.
    conn.client.sessions.lock().await.clear();
    let open: Vec<String> = conn.links.lock().await.keys().cloned().collect();
    let mut kept = Vec::new();
    for sid in open {
        let running = conn.sessions.lock().await.get(&sid).is_some_and(SessionState::turn_active);
        if !running || conn.children.lock().await.contains_key(&sid) {
            continue;
        }
        registry().insert(sid.clone(), conn.clone());
        // Registered before it is marked, then checked once: a turn that ended in between is let go now.
        conn.release_at_turn_end(&sid).await;
        if conn.links.lock().await.contains_key(&sid) {
            kept.push(sid);
        } else {
            conn.schedule_learning(sid, crate::learn::Trigger::Dispose);
        }
    }
    kept
}

impl Conn {
    pub(super) fn is_detached(&self) -> bool {
        self.detached.load(Ordering::Acquire)
    }

    /// A window opening `sid` takes over the turn a closed window left running on it: the link, the
    /// turn's state and the reader's frames move here, so the rest of the turn reaches this window.
    /// The reader is switched between two frames, so every event is either already in the replay
    /// buffer or sent here live. False when no closed window holds `sid`.
    pub(super) async fn adopt(self: &Arc<Self>, sid: &str) -> bool {
        if !self.window.load(Ordering::Acquire) || self.is_detached() {
            return false;
        }
        let Some(old) = registry().get(sid).cloned() else { return false };
        let Some(link) = old.links.lock().await.get(sid).cloned() else { return false };
        let owner = {
            let tasks = old.link_tasks.lock().await;
            tasks.iter().find(|t| t.link.upgrade().is_some_and(|l| l.same_channel(&link))).map(|t| t.owner.clone())
        };
        let Some(owner) = owner else { return false };
        // Wait out a frame the old connection is handling; none starts until the switch is done.
        let mut reader = owner.lock().await;
        // The turn may have ended (and its link been let go) while that frame was handled.
        if old.links.lock().await.remove(sid).is_none() {
            return false;
        }
        let task = {
            let mut tasks = old.link_tasks.lock().await;
            tasks.iter().position(|t| t.link.upgrade().is_some_and(|l| l.same_channel(&link))).map(|i| tasks.remove(i))
        };
        old.release_after_turn.lock().unwrap_or_else(|e| e.into_inner()).remove(sid);
        if let Some(state) = old.sessions.lock().await.remove(sid) {
            self.sessions.lock().await.insert(sid.to_string(), state);
        }
        if let Some(info) = old.known.lock().await.remove(sid) {
            self.known.lock().await.insert(sid.to_string(), info);
        }
        if old.fresh.lock().await.remove(sid) {
            self.fresh.lock().await.insert(sid.to_string());
        }
        self.links.lock().await.insert(sid.to_string(), link);
        if let Some(task) = task {
            self.link_tasks.lock().await.push(task);
        }
        self.client.sessions.lock().await.insert(sid.to_string());
        *reader = Arc::downgrade(self);
        forget(sid, &old);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{tasks_of, test_conn, test_conn_events};
    use super::super::{LinkTasks, window_closed};
    use super::*;
    use serde_json::json;
    use std::time::Duration;

    async fn ended(tasks: &LinkTasks) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while !tasks.is_finished() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the link's bridge, writer and reader ended");
    }

    /// A window that goes away mid-turn keeps that turn's link (dropping it would make the engine
    /// abort the turn) and lets go of the rest; the turn's end lets go of its link too.
    #[tokio::test]
    async fn a_closed_windows_turn_keeps_its_link_until_it_ends() {
        let conn = test_conn("detach-keep");
        let (busy, idle) = (conn.open_link().await.unwrap(), conn.open_link().await.unwrap());
        conn.links.lock().await.extend([("busy".to_string(), busy.clone()), ("idle".to_string(), idle.clone())]);
        conn.sessions.lock().await.entry("busy".into()).or_default().mark_running();
        conn.sessions.lock().await.entry("idle".into()).or_default();
        conn.observer.start_turn("busy", "work", "invoke_agent", None);
        let (busy_tasks, idle_tasks) = (tasks_of(&conn, &busy).await, tasks_of(&conn, &idle).await);
        drop((busy, idle));
        window_closed(&conn).await;
        ended(&idle_tasks).await;
        assert!(!busy_tasks.is_finished() && holds("busy"), "the running turn keeps its link");
        assert!(conn.observer.has_active_run("busy"), "its run stays open until the turn really ends");
        assert!(conn.attach_once("another").await.is_err(), "a closed window's connection opens no new link");
        conn.on_harness_frame(json!({ "ev": "turn_done", "session_id": "busy" })).await;
        ended(&busy_tasks).await;
        assert!(!holds("busy") && conn.links.lock().await.is_empty(), "nothing is held once the turn ended");
        assert!(!conn.observer.has_active_run("busy"));
    }

    /// A window that opens the chat takes the running turn over: link, turn state and the reader's
    /// frames move to it, so the rest of the turn is rendered there.
    #[tokio::test]
    async fn a_reopening_window_takes_the_running_turn_over() {
        let old = test_conn("detach-old");
        let (window, mut frames) = test_conn_events("detach-new", false);
        window.window.store(true, Ordering::Release);
        let link = old.open_link().await.unwrap();
        old.links.lock().await.insert("turn".into(), link.clone());
        old.sessions.lock().await.entry("turn".into()).or_default().mark_running();
        old.known.lock().await.insert("turn".into(), json!({ "working_dir": "/work" }));
        let tasks = tasks_of(&old, &link).await;
        drop(link);
        window_closed(&old).await;
        let attached = window.attach_once("turn").await.unwrap();
        assert_eq!(attached["session"]["working_dir"], "/work", "the chat's info moves with it");
        assert!(!holds("turn") && old.links.lock().await.is_empty() && !tasks.is_finished());
        assert!(window.sessions.lock().await.get("turn").is_some_and(SessionState::turn_active));
        assert!(window.client.sessions.lock().await.contains("turn"), "the window has the chat open");
        let reader = tasks.owner.lock().await.upgrade().unwrap();
        assert!(Arc::ptr_eq(&reader, &window), "the link's frames reach the window now");
        reader.on_harness_frame(json!({ "ev": "turn_done", "session_id": "turn" })).await;
        drop(reader);
        let complete = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let tokio_tungstenite::tungstenite::Message::Text(text) = frames.recv().await.unwrap() else { continue };
                let frame: serde_json::Value = serde_json::from_str(&text).unwrap();
                if frame["params"]["type"] == "message.complete" {
                    return frame;
                }
            }
        })
        .await
        .expect("the end of the turn reaches the window");
        assert_eq!(complete["params"]["session_id"], "turn");
        // That window closing with the chat idle lets the link go.
        window_closed(&window).await;
        ended(&tasks).await;
    }
}
