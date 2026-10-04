//! End-of-turn gates through the real turn loop: a scripted provider replays text-only replies.

use super::*;

/// Replies (all `end_turn`) in order; returns the request count and the final text.
async fn run_headless(replies: &[&'static str]) -> (usize, String) {
    let _lock = crate::storage::lock_test_env();
    let home = tempfile::tempdir().unwrap();
    crate::env::set_var("FACTR_HOME", home.path());
    crate::config::Config::invalidate_cache();
    let provider = RefusalScriptProvider {
        script: Arc::new(std::sync::Mutex::new(replies.iter().map(|r| (*r, "end_turn")).collect())),
        calls: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
    };
    let mut agent = Agent::new(Arc::new(provider.clone()), Registry::empty());
    crate::headless::mark(&agent.session.id);
    let text = agent.run_once_capture("find the value").await.unwrap();
    (provider.calls.load(std::sync::atomic::Ordering::SeqCst), text)
}

#[tokio::test]
async fn a_decline_gets_one_more_attempt_then_the_turn_ends() {
    let (calls, text) = run_headless(&["I could not find it.", "I could not find it either.", "never asked"]).await;
    assert_eq!(calls, 2, "one nudge, then the second decline ends the turn");
    assert!(text.contains("either"), "{text}");
}

#[tokio::test]
async fn an_unreadable_input_decline_is_accepted_as_is() {
    let (calls, _) = run_headless(&["I cannot determine the total: the attached file is unreadable.", "never asked"]).await;
    assert_eq!(calls, 1);
}

#[tokio::test]
async fn a_plain_answer_is_one_request() {
    let (calls, _) = run_headless(&["The value is 7.", "never asked"]).await;
    assert_eq!(calls, 1);
}
