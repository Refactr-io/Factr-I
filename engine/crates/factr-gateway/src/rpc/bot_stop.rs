//! Stop a session family through existing cancellation/worker-stop APIs.
use super::*;
use std::sync::{LazyLock, Mutex as StdMutex};
use std::time::Instant;
static STOPPED: LazyLock<StdMutex<HashMap<String, Instant>>> = LazyLock::new(Default::default);
pub(super) fn cancelled(id: &str) -> bool {
    STOPPED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .contains_key(id)
}

fn descendants(rows: &[Value], root: &str) -> Vec<String> {
    let mut family = vec![root.to_string()];
    let mut at = 0;
    while at < family.len() {
        let parent = family[at].clone();
        for row in rows {
            if row["parent_session_id"] == parent {
                if let Some(id) = row["session_id"]
                    .as_str()
                    .filter(|id| !family.iter().any(|s| s == id))
                {
                    family.push(id.to_string());
                }
            }
        }
        at += 1;
    }
    family.into_iter().skip(1).rev().collect()
}

impl Conn {
    pub(super) async fn stop_bot_children(self: &Arc<Self>, parent: &str) -> Result<usize> {
        let list = self
            .call(json!({"req":"list_sessions","include_archived":true}))
            .await?;
        let rows = list["sessions"]
            .as_array()
            .context("engine returned no session list")?;
        let children = descendants(rows, parent);
        {
            let mut stopped = STOPPED.lock().unwrap_or_else(|e| e.into_inner());
            stopped.retain(|_, at| at.elapsed() < Duration::from_secs(6 * 3600));
            for child in &children {
                stopped.insert(child.clone(), Instant::now());
            }
        }
        let mut failures = Vec::new();
        for child in &children {
            // Cancelling first preserves the partial transcript. Worker stop also clears pending waits.
            let cancelled = match self.ensure_attached(child).await {
                Ok(_) => self.call(json!({"req":"cancel","session_id":child})).await,
                Err(error) => Err(error),
            };
            let stopped = self.call(json!({"req":"comm_stop","session_id":parent,"target_session":child,"force":false})).await;
            if cancelled.is_err() && stopped.is_err() {
                STOPPED
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(child);
                failures.push(child.clone());
            } else {
                self.finish_child(child, "interrupted", "Stopped with the parent task")
                    .await;
            }
        }
        anyhow::ensure!(
            failures.is_empty(),
            "could not stop child sessions: {}",
            failures.join(", ")
        );
        Ok(children.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stops_only_the_subtree_deepest_first_and_tolerates_cycles() {
        let rows = vec![
            json!({"session_id":"c","parent_session_id":"p"}),
            json!({"session_id":"g","parent_session_id":"c"}),
            json!({"session_id":"p","parent_session_id":"g"}),
            json!({"session_id":"other","parent_session_id":"elsewhere"}),
        ];
        assert_eq!(descendants(&rows, "p"), ["g", "c"]);
    }
}
