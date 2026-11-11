//! L3 through the real streaming loop and the real write tool: the next request's copy of the
//! result has no diff block and carries the save note; the session and `ToolDone` keep it all.

use super::*;
use std::sync::Mutex;

#[derive(Clone, Default)]
struct WriteThenDoneProvider {
    requests: Arc<Mutex<Vec<Vec<Message>>>>,
}

#[async_trait]
impl Provider for WriteThenDoneProvider {
    async fn complete(&self, messages: &[Message], _: &[ToolDefinition], _: &str, _: Option<&str>) -> Result<EventStream> {
        let first = {
            let mut requests = self.requests.lock().unwrap();
            requests.push(messages.to_vec());
            requests.len() == 1
        };
        let events = if first {
            vec![
                StreamEvent::ToolUseStart { id: "w1".into(), name: "write".into() },
                StreamEvent::ToolInputDelta(r#"{"file_path":"a.py","content":"print(1)\n"}"#.into()),
                StreamEvent::ToolUseEnd,
                StreamEvent::MessageEnd { stop_reason: Some("tool_use".into()) },
            ]
        } else {
            vec![StreamEvent::TextDelta("DONE".into()), StreamEvent::MessageEnd { stop_reason: Some("end_turn".into()) }]
        };
        Ok(Box::pin(futures::stream::iter(events.into_iter().map(Ok))))
    }
    fn name(&self) -> &str {
        "compact-write-test"
    }
    fn supports_compaction(&self) -> bool {
        false
    }
    fn fork(&self) -> Arc<dyn Provider> {
        Arc::new(self.clone())
    }
}

fn write_result(messages: &[Message]) -> String {
    messages
        .iter()
        .flat_map(|m| &m.content)
        .find_map(|b| match b {
            ContentBlock::ToolResult { tool_use_id, content, .. } if tool_use_id == "w1" => Some(content.clone()),
            _ => None,
        })
        .expect("the write result")
}

async fn run(compact: bool) -> (String, String, String) {
    let _lock = crate::storage::lock_test_env();
    let home = tempfile::tempdir().unwrap();
    crate::env::set_var("FACTR_HOME", home.path());
    crate::config::Config::invalidate_cache();
    let work = tempfile::tempdir().unwrap();
    let provider = WriteThenDoneProvider::default();
    let registry = Registry::new(Arc::new(provider.clone())).await;
    let mut agent = Agent::new(Arc::new(provider.clone()), registry);
    agent.loop_shape.compact_write_result = compact;
    agent.session.working_dir = Some(work.path().to_string_lossy().into_owned());
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    agent.run_once_streaming_mpsc("write a.py", Vec::new(), None, tx).await.unwrap();
    assert_eq!(std::fs::read_to_string(work.path().join("a.py")).unwrap(), "print(1)\n");
    let mut tool_done = None;
    while let Ok(event) = rx.try_recv() {
        if let ServerEvent::ToolDone { id, output, .. } = event
            && id == "w1"
        {
            tool_done = Some(output);
        }
    }
    let sent = write_result(&provider.requests.lock().unwrap()[1]);
    let stored = write_result(&agent.messages().iter().map(|m| m.to_message()).collect::<Vec<_>>());
    (sent, stored, tool_done.expect("ToolDone for the write"))
}

#[tokio::test]
async fn the_model_gets_the_write_result_without_the_diff_and_everyone_else_keeps_it() {
    let (sent, stored, tool_done) = run(true).await;
    assert!(stored.starts_with("Created a.py (1 lines)") && stored.contains("\n\nFile diff:\n```diff\n"), "{stored}");
    assert_eq!(tool_done, stored, "ToolDone keeps the full text");
    let summary = &stored[..stored.find("\n\nFile diff:").unwrap()];
    // The request copy may carry a `[tool timing: ...]` prefix (added after the projection).
    assert!(sent.ends_with(&format!("{summary}{}", super::super::compact_tool_results::SAVED_NOTE)), "{sent}");
    assert!(!sent.contains("File diff"), "{sent}");
    // Off: the request carries the stored text unchanged.
    let (sent, stored, _) = run(false).await;
    assert!(sent.ends_with(&stored), "{sent}");
}
