use super::*;
use std::sync::Mutex;

/// First request: two calls to tools this run does not offer. Later requests: a final answer.
struct OffListProvider(Mutex<u32>);

#[async_trait]
impl Provider for OffListProvider {
    async fn complete(&self, _: &[Message], _: &[ToolDefinition], _: &str, _: Option<&str>) -> Result<EventStream> {
        let first = {
            let mut n = self.0.lock().unwrap();
            *n += 1;
            *n == 1
        };
        let events: Vec<Result<StreamEvent>> = if first {
            vec![
                Ok(StreamEvent::ToolUseStart { id: "a".into(), name: "made_up_tool".into() }),
                Ok(StreamEvent::ToolInputDelta("{}".into())),
                Ok(StreamEvent::ToolUseEnd),
                Ok(StreamEvent::ToolUseStart { id: "b".into(), name: "capture".into() }),
                Ok(StreamEvent::ToolInputDelta("{}".into())),
                Ok(StreamEvent::ToolUseEnd),
                Ok(StreamEvent::MessageEnd { stop_reason: Some("tool_use".into()) }),
            ]
        } else {
            vec![
                Ok(StreamEvent::TextDelta("answer".into())),
                Ok(StreamEvent::MessageEnd { stop_reason: Some("end_turn".into()) }),
            ]
        };
        Ok(Box::pin(futures::stream::iter(events)))
    }
    fn name(&self) -> &str {
        "off-list-test"
    }
    fn supports_compaction(&self) -> bool {
        false
    }
    fn fork(&self) -> Arc<dyn Provider> {
        Arc::new(OffListProvider(Mutex::new(*self.0.lock().unwrap())))
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

#[tokio::test]
async fn a_call_to_a_tool_off_the_list_is_a_tool_error_and_the_run_continues() {
    let _sandbox = crate::auth::test_sandbox::AuthTestSandbox::new().unwrap();
    let provider: Arc<dyn Provider> = Arc::new(OffListProvider(Mutex::new(0)));
    let registry = Registry::empty();
    registry.register("capture".into(), Arc::new(Capture)).await;
    let mut agent = Agent::new(provider, registry);
    agent.disabled_tools.insert("capture".to_string());
    agent.add_message(Role::User, vec![ContentBlock::Text { text: "go".into(), cache_control: None }]);
    let (tx, _rx) = tokio_mpsc::unbounded_channel();
    agent.run_turn_streaming_mpsc(tx).await.expect("an off-list call must not end the run");
    let errors: Vec<String> = agent
        .messages()
        .iter()
        .flat_map(|m| m.content.iter())
        .filter_map(|b| match b {
            ContentBlock::ToolResult { content, is_error: Some(true), .. } => Some(content.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(errors.len(), 2, "{errors:?}");
    assert!(errors.iter().any(|e| e.contains("'capture' is not available in this run") && e.contains("Available tools:")), "{errors:?}");
    let last = agent.messages().last().unwrap();
    assert!(last.content.iter().any(|b| matches!(b, ContentBlock::Text { text, .. } if text == "answer")));
}
