//! Delegated children as the desktop's subagent rows. A `delegate` spawn is a child session; this
//! connection follows it (attaches to it) and turns its own events into the parent's `subagent.start`,
//! `subagent.progress` and `subagent.complete`, one row per child, keyed by the child's session id.

use super::*;
use crate::map::Out;
use std::time::Instant;

/// One delegated child this connection follows.
pub(super) struct Child {
    pub(super) parent: String,
    goal: String,
    started: Instant,
    running: bool,
    /// The child has begun a turn of its own; the idle `turn_done` an attach replays is not its answer.
    turn: bool,
    tools: u64,
    /// The child's cumulative usage (`session.usage` payload), for the finished row.
    usage: Value,
    /// What the child said this turn: its summary when it finishes.
    text: String,
}

/// `subagent.*` payload for a child: the fields the desktop's subagent store reads.
fn payload(id: &str, c: &Child, status: &str) -> Value {
    json!({
        "subagent_id": id, "parent_id": c.parent, "child_session_id": id, "goal": c.goal, "status": status,
        "task_index": 0, "task_count": 1,
    })
}

/// The child's task as its row title: an explicit label, else the first line of the prompt.
fn goal_of(args: &Value) -> String {
    let text = args["label"].as_str().filter(|l| !l.is_empty()).or(args["prompt"].as_str()).unwrap_or("Subagent");
    text.lines().next().unwrap_or(text).chars().take(120).collect()
}

/// The child id of a successful `delegate` spawn (`Spawned new agent: <id>`).
pub(super) fn spawned_child<'a>(tool_name: &str, result_text: &'a str) -> Option<&'a str> {
    (tool_name == "delegate").then_some(result_text)?.strip_prefix("Spawned new agent: ").map(str::trim).filter(|id| !id.is_empty())
}

impl Conn {
    /// A `delegate` spawn finished: start following its child. Boxed: following attaches, and attaching
    /// opens a link whose frames end up here again, a cycle the compiler cannot size.
    pub(super) fn follow_child(self: Arc<Self>, parent: String, child: String, args: Value) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
        Box::pin(async move {
            let row = Child { parent: parent.clone(), goal: goal_of(&args), started: Instant::now(), running: true, turn: false, tools: 0, usage: json!({}), text: String::new() };
            let start = payload(&child, &row, "running");
            self.children.lock().await.insert(child.clone(), row);
            self.emit("subagent.start", Some(&parent), start).await;
            // Events from here on are the child's own. A child that already finished before the attach
            // has none: its stored answer is the outcome.
            if let Err(err) = self.ensure_attached(&child).await {
                self.finish_child(&child, "failed", &format!("{err:#}")).await;
                return;
            }
            let working = self.sessions.lock().await.get(&child).is_some_and(SessionState::turn_active);
            if !working && let Ok(history) = self.call(json!({ "req": "get_history", "session_id": child })).await {
                // No answer yet: the child has not started its turn, and its own events will tell.
                let messages = crate::map::transcript(&history["messages"]);
                let answer = messages.iter().rev().find(|m| m["role"] == "assistant").and_then(|m| m["text"].as_str()).filter(|t| !t.is_empty());
                if let Some(answer) = answer {
                    self.finish_child(&child, "completed", answer).await;
                }
            }
        })
    }

    /// Take the follow-worthy events of a followed child out of `outs` (they become `subagent.*`),
    /// and hand back the rest (an approval the child waits on is still the user's to answer).
    pub(super) async fn child_events(&self, child: &str, outs: Vec<Out>) -> Vec<Out> {
        let mut rest = Vec::new();
        for out in outs {
            let Out::Event { ty, payload: event, .. } = &out else {
                rest.push(out);
                continue;
            };
            let mut children = self.children.lock().await;
            let Some(c) = children.get_mut(child) else { continue };
            // Any output is the child's turn under way (the first one, or a new one after a steer).
            if matches!(*ty, "message.delta" | "tool.start" | "session.usage") {
                c.turn = true;
                if !c.running {
                    (c.running, c.started, c.text) = (true, Instant::now(), String::new());
                    let start = payload(child, c, "running");
                    let parent = c.parent.clone();
                    drop(children);
                    self.emit("subagent.start", Some(&parent), start).await;
                    children = self.children.lock().await;
                }
            }
            let Some(c) = children.get_mut(child) else { continue };
            match *ty {
                "message.delta" => c.text.push_str(event["text"].as_str().unwrap_or_default()),
                "session.usage" => c.usage = event["usage"].clone(),
                "tool.complete" => c.tools += 1,
                "tool.start" => {
                    let mut progress = payload(child, c, "running");
                    progress["tool_name"] = event["name"].clone();
                    progress["tool_preview"] = json!(event["args"].to_string().chars().take(96).collect::<String>());
                    let parent = c.parent.clone();
                    drop(children);
                    self.emit("subagent.progress", Some(&parent), progress).await;
                }
                "message.complete" if c.turn => {
                    let status = match event["status"].as_str() {
                        Some("interrupted") => "interrupted",
                        Some("error") => "failed",
                        _ => "completed",
                    };
                    let summary = match status {
                        "failed" => event["error"].as_str().unwrap_or_default().to_string(),
                        _ => c.text.clone(),
                    };
                    drop(children);
                    self.finish_child(child, status, &summary).await;
                }
                _ => {}
            }
        }
        rest
    }

    async fn finish_child(&self, child: &str, status: &str, summary: &str) {
        let mut children = self.children.lock().await;
        let Some(c) = children.get_mut(child) else { return };
        if !c.running {
            return;
        }
        c.running = false;
        let mut done = payload(child, c, status);
        done["summary"] = json!(summary.chars().take(2000).collect::<String>());
        done["duration_seconds"] = json!(c.started.elapsed().as_secs_f64());
        done["tool_count"] = json!(c.tools);
        done["input_tokens"] = c.usage["input"].clone();
        done["output_tokens"] = c.usage["output"].clone();
        let parent = c.parent.clone();
        drop(children);
        self.emit("subagent.complete", Some(&parent), done).await;
        // The row is final: holding the child's link on would pin a bridge and its pipes per delegate.
        self.unfollow(child).await;
    }

    /// Let go of a followed child's link. The engine leaves a child whose turn is still running to
    /// its own owner (the parent's delegate), so this never stops the child.
    pub(super) async fn unfollow(&self, child: &str) {
        self.links.lock().await.remove(child);
        self.sessions.lock().await.remove(child);
        self.client.sessions.lock().await.remove(child);
    }

    /// The followed child's row title, and whether it is still working; `None` for a child this
    /// connection does not follow (one from an earlier run).
    pub(super) async fn followed(&self, child: &str) -> Option<Followed> {
        self.children.lock().await.get(child).map(|c| Followed { goal: c.goal.clone(), running: c.running })
    }
}

/// What a snapshot row takes from a followed child.
pub(super) struct Followed {
    pub(super) goal: String,
    pub(super) running: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_delegate_spawn_names_its_child_and_nothing_else_does() {
        assert_eq!(spawned_child("delegate", "Spawned new agent: session_x_1\n"), Some("session_x_1"));
        assert_eq!(spawned_child("delegate", "Failed to spawn"), None);
        assert_eq!(spawned_child("bash", "Spawned new agent: session_x_1"), None);
    }

    #[test]
    fn a_child_is_running_failed_or_done_by_its_real_state() {
        let none = HashMap::new();
        let status = |swarm: &str, followed| Conn::map_subagent_status(&json!({ "session_id": "c", "swarm_status": swarm }), &none, followed);
        assert_eq!(status("ready", true), "running", "a followed child that has not finished");
        assert_eq!(status("ready", false), "completed", "an idle child is history");
        assert_eq!(status("failed", false), "failed");
        assert_eq!(status("stopped", false), "interrupted");
        let rows: Vec<Value> = ["c1", "c2"].iter().map(|id| Conn::subagent_snapshot(&json!({ "session_id": id, "agent_label": "delegate" }), "p", &none, None)).collect();
        assert_ne!(rows[0]["subagent_id"], rows[1]["subagent_id"], "two spawns with one label are two rows");
        assert_eq!(rows[0]["child_session_id"], "c1");
    }

    fn row(parent: &str) -> Child {
        Child { parent: parent.into(), goal: "g".into(), started: Instant::now(), running: true, turn: true, tools: 0, usage: json!({}), text: String::new() }
    }

    /// The driver renders no rows: a delegate spawn on its link follows nothing (a link it held to
    /// the child would be dropped by its next scan, and the engine would stop the child).
    #[tokio::test]
    async fn only_a_window_follows_a_delegates_child() {
        let spawn = json!({ "ev": "tool_done", "session_id": "p", "call_id": "c1", "name": "delegate", "output": "Spawned new agent: kid" });
        let driver = crate::rpc::tests::test_conn_as("follow-driver", true);
        let window = crate::rpc::tests::test_conn("follow-window");
        driver.on_harness_frame(spawn.clone()).await;
        window.on_harness_frame(spawn).await;
        tokio::time::timeout(Duration::from_secs(5), async {
            while !window.children.lock().await.contains_key("kid") {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the window follows its child");
        assert!(driver.children.lock().await.is_empty(), "the driver follows nothing");
    }

    /// A finished child's link is let go at once, and a closed parent takes its followed children's
    /// links with it: none outlives the row it fed.
    #[tokio::test]
    async fn a_finished_child_or_a_closed_parent_releases_the_child_link() {
        let conn = crate::rpc::tests::test_conn("unfollow");
        for kid in ["done-kid", "busy-kid"] {
            let link = conn.open_link().await.unwrap();
            conn.links.lock().await.insert(kid.into(), link);
            conn.children.lock().await.insert(kid.into(), row("p"));
        }
        conn.finish_child("done-kid", "completed", "ok").await;
        assert!(!conn.links.lock().await.contains_key("done-kid"), "a finished child is let go");
        assert!(conn.followed("done-kid").await.is_some_and(|f| !f.running), "its row stays, finished");
        assert!(conn.links.lock().await.contains_key("busy-kid"));
        conn.forget_link("p").await;
        assert!(conn.links.lock().await.is_empty(), "closing the parent lets its children go");
        assert!(conn.children.lock().await.is_empty());
    }

    #[test]
    fn a_row_is_titled_by_its_label_else_the_first_prompt_line() {
        assert_eq!(goal_of(&json!({ "label": "explore", "prompt": "long" })), "explore");
        assert_eq!(goal_of(&json!({ "prompt": "first line\nsecond" })), "first line");
        assert_eq!(goal_of(&json!({})), "Subagent");
    }
}
