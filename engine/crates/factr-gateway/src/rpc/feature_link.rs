//! A feature backend can idle-exit while the Desktop's own WebSocket stays connected.
use super::*;

pub(super) struct Replies(Mutex<Option<HashMap<String, oneshot::Sender<Value>>>>);

impl Default for Replies {
    fn default() -> Self { Self(Mutex::new(Some(HashMap::new()))) }
}

impl Replies {
    pub(super) async fn is_open(&self) -> bool { self.0.lock().await.is_some() }

    pub(super) async fn register(&self, id: String) -> Result<oneshot::Receiver<Value>> {
        let mut slot = self.0.lock().await;
        let pending = slot.as_mut().context("the feature backend restarted; try again")?;
        let (tx, rx) = oneshot::channel();
        pending.insert(id, tx);
        Ok(rx)
    }

    pub(super) async fn resolve(&self, id: &str, reply: Value) {
        if let Some(tx) = self.0.lock().await.as_mut().and_then(|p| p.remove(id)) { let _ = tx.send(reply); }
    }

    pub(super) async fn remove(&self, id: &str) {
        if let Some(pending) = self.0.lock().await.as_mut() { pending.remove(id); }
    }

    pub(super) async fn close(&self) {
        // Closing and registration share a lock: a late caller cannot wait on a dead reader.
        self.0.lock().await.take();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn disconnect_releases_pending_and_refuses_late_registration() {
        let replies = Replies::default();
        let first = replies.register("first".into()).await.unwrap();
        replies.close().await;
        assert!(first.await.is_err());
        assert!(!replies.is_open().await);
        assert!(replies.register("late".into()).await.is_err());
        replies.close().await;
    }

    #[tokio::test]
    async fn live_responses_and_timeout_removal_keep_their_identity() {
        let replies = Replies::default();
        let first = replies.register("first".into()).await.unwrap();
        let second = replies.register("second".into()).await.unwrap();
        replies.resolve("first", json!({ "result": 7 })).await;
        assert_eq!(first.await.unwrap()["result"], 7);
        replies.remove("second").await;
        assert!(second.await.is_err());
        replies.resolve("second", json!({ "result": 9 })).await;
        assert!(replies.is_open().await);
    }
}
