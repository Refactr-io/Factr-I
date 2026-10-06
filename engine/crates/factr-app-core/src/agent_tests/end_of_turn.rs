//! End-of-turn gates through the real turn loop: a scripted provider replays text-only replies.

use super::*;

/// Replies (all `end_turn`) in order; returns the request count and the final text.
async fn run_headless(replies: &[&'static str]) -> (usize, String) {
    run_headless_task("find the value", replies).await
}

async fn run_headless_task(task: &str, replies: &[&'static str]) -> (usize, String) {
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
    let text = agent.run_once_capture(task).await.unwrap();
    (provider.calls.load(std::sync::atomic::Ordering::SeqCst), text)
}

#[tokio::test]
async fn a_decline_gets_one_more_attempt_then_the_turn_ends() {
    let (calls, text) = run_headless(&["I could not find it.", "I could not find it either.", "never asked"]).await;
    assert_eq!(calls, 2, "one nudge, then the second decline ends the turn");
    assert_eq!(text, "I could not find it either.");
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

#[tokio::test]
async fn a_bare_confirmation_after_a_nudge_returns_the_earlier_whole_reply() {
    let (calls, text) = run_headless(&["I could not determine more, it is probably 7.", "Confirmed.", "never asked"]).await;
    assert_eq!(calls, 2, "shortfall nudge only");
    assert_eq!(text, "I could not determine more, it is probably 7.");
}

#[tokio::test]
async fn the_last_reply_alone_is_the_final_text_when_it_is_an_answer() {
    let (calls, text) = run_headless(&["I could not determine more, it is probably 7.", "The value is 7, taken from the second source I checked."]).await;
    assert_eq!(calls, 2);
    assert_eq!(text, "The value is 7, taken from the second source I checked.");
}

#[tokio::test]
async fn a_missing_required_line_gets_one_format_nudge_and_the_labelled_reply_is_returned() {
    let task = "Finish your answer with the following template: RESULT: [YOUR VALUE]";
    let (calls, text) = run_headless_task(task, &["It is 7.", "RESULT: 7", "never asked"]).await;
    assert_eq!(calls, 2);
    assert_eq!(text, "RESULT: 7");
    let (calls, _) = run_headless_task(task, &["RESULT: 7", "never asked"]).await;
    assert_eq!(calls, 1);
    // A labelled reply followed by a long unlabelled one: the labelled reply is the answer.
    let (_, text) = run_headless_task(task, &["RESULT: 7\nbecause the count of rows is seven, as shown above in detail.", "Here is a long unlabelled follow-up that comes after, more than forty characters."]).await;
    assert!(text.starts_with("RESULT: 7"), "{text}");
}
