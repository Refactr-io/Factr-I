//! A provider's stream task lives only as long as someone reads its events.

use std::future::Future;
use tokio::sync::mpsc::Sender;

/// Run `work` (the task that streams a model response into `tx`) until it finishes or the receiving
/// end is dropped, whichever comes first. Dropping the receiver is how a turn is stopped; without this
/// the task keeps its HTTP request open, waiting for a model that was already abandoned. `None`: the
/// receiver went first and `work` was dropped, which closes its connection.
pub async fn while_receiver_open<T, F: Future>(tx: &Sender<T>, work: F) -> Option<F::Output> {
    tokio::select! {
        out = work => Some(out),
        () = tx.closed() => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, atomic::{AtomicBool, Ordering}};

    struct Flag(Arc<AtomicBool>);
    impl Drop for Flag {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    #[tokio::test]
    async fn dropping_the_receiver_drops_the_work() {
        let (tx, rx) = tokio::sync::mpsc::channel::<u8>(1);
        let dropped = Arc::new(AtomicBool::new(false));
        let guard = Flag(dropped.clone());
        let task = tokio::spawn(async move {
            while_receiver_open(&tx, async move {
                let _held = guard;
                std::future::pending::<()>().await
            })
            .await
        });
        drop(rx);
        assert_eq!(task.await.unwrap(), None);
        assert!(dropped.load(Ordering::SeqCst), "the connection-holding future was dropped");
    }

    #[tokio::test]
    async fn finished_work_returns_its_output() {
        let (tx, _rx) = tokio::sync::mpsc::channel::<u8>(1);
        assert_eq!(while_receiver_open(&tx, async { 7 }).await, Some(7));
    }
}
