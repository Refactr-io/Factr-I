//! L1 through the real turn loops: a nudge at call N stays in place, so call N+1 (after a tool
//! call) extends call N's input. Off, the nudge drops out and call N+1 no longer extends it.

use super::*;
use std::sync::Mutex;

/// Call 1: a decline (nudged headless). Call 2: a tool call. Call 3: the answer.
#[derive(Clone, Default)]
struct NudgeThenToolProvider {
    requests: Arc<Mutex<Vec<Vec<String>>>>,
}

#[async_trait]
impl Provider for NudgeThenToolProvider {
    async fn complete(&self, messages: &[Message], _: &[ToolDefinition], _: &str, _: Option<&str>) -> Result<EventStream> {
        let n = {
            let mut requests = self.requests.lock().unwrap();
            requests.push(messages.iter().map(|m| serde_json::to_string(&m.content).unwrap()).collect());
            requests.len()
        };
        let events = match n {
            1 => vec![StreamEvent::TextDelta("I could not find it.".into()), StreamEvent::MessageEnd { stop_reason: Some("end_turn".into()) }],
            2 => vec![
                StreamEvent::ToolUseStart { id: "look".into(), name: "capture".into() },
                StreamEvent::ToolInputDelta("{}".into()),
                StreamEvent::ToolUseEnd,
                StreamEvent::MessageEnd { stop_reason: Some("tool_use".into()) },
            ],
            _ => vec![StreamEvent::TextDelta("The value is 7.".into()), StreamEvent::MessageEnd { stop_reason: Some("end_turn".into()) }],
        };
        Ok(Box::pin(futures::stream::iter(events.into_iter().map(Ok))))
    }
    fn name(&self) -> &str {
        "sticky-injections-test"
    }
    fn supports_compaction(&self) -> bool {
        false
    }
    fn fork(&self) -> Arc<dyn Provider> {
        Arc::new(self.clone())
    }
}

struct Capture;
#[async_trait]
impl crate::tool::Tool for Capture {
    fn name(&self) -> &str {
        "capture"
    }
    fn description(&self) -> &str {
        "x"
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type":"object"})
    }
    async fn execute(&self, _: serde_json::Value, _: crate::tool::ToolContext) -> Result<ToolOutput> {
        Ok(ToolOutput::new("ran"))
    }
}

/// Returns the three requests' message lists.
async fn run(sticky: bool, streaming: bool) -> Vec<Vec<String>> {
    let _lock = crate::storage::lock_test_env();
    let home = tempfile::tempdir().unwrap();
    crate::env::set_var("FACTR_HOME", home.path());
    crate::config::Config::invalidate_cache();
    let provider = NudgeThenToolProvider::default();
    let registry = Registry::empty();
    registry.register("capture".into(), Arc::new(Capture)).await;
    let mut agent = Agent::new(Arc::new(provider.clone()), registry);
    agent.loop_shape.sticky_nudges = sticky;
    crate::headless::mark(&agent.session.id);
    if streaming {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let reminder = Some("turn reminder".to_string());
        agent.run_once_streaming_mpsc("find the value", Vec::new(), reminder, tx).await.unwrap();
    } else {
        agent.run_once_capture("find the value").await.unwrap();
    }
    let requests = provider.requests.lock().unwrap().clone();
    assert_eq!(requests.len(), 3, "decline, nudged call with a tool call, answer");
    requests
}

fn extends(next: &[String], prev: &[String]) -> bool {
    next.len() > prev.len() && next[..prev.len()] == *prev
}

#[tokio::test]
async fn a_kept_nudge_lets_the_call_after_the_tool_call_extend_the_nudged_call() {
    for streaming in [false, true] {
        let on = run(true, streaming).await;
        assert!(on[1].iter().any(|m| m.contains("List what you tried")), "call 2 carries the decline nudge");
        assert!(!on[0].iter().any(|m| m.contains("List what you tried")));
        assert!(extends(&on[1], &on[0]), "streaming={streaming}");
        assert!(extends(&on[2], &on[1]), "streaming={streaming}: {:#?}", on);
        let off = run(false, streaming).await;
        assert!(!extends(&off[2], &off[1]), "streaming={streaming}: the baseline drops the nudge");
    }
}
