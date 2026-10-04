//! Factr `fallback_providers` end to end: a fake primary that fails, fake fallbacks behind it.

use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Fails every request while its model starts with `primary`; any other model answers "rescued".
/// A model containing `nokey` is refused by `set_model` (no credentials for that provider).
struct FlakyProvider {
    model: std::sync::Mutex<String>,
    error: &'static str,
    primary_calls: Arc<AtomicUsize>,
    fallback_calls: Arc<AtomicUsize>,
}

#[async_trait]
impl Provider for FlakyProvider {
    async fn complete(&self, _m: &[Message], _t: &[ToolDefinition], _s: &str, _r: Option<&str>) -> Result<EventStream> {
        if self.model.lock().unwrap().starts_with("primary") {
            self.primary_calls.fetch_add(1, Ordering::SeqCst);
            anyhow::bail!("{}", self.error);
        }
        self.fallback_calls.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = tokio_mpsc::channel::<Result<StreamEvent>>(8);
        tokio::spawn(async move {
            let _ = tx.send(Ok(StreamEvent::TextDelta("rescued".to_string()))).await;
            let _ = tx.send(Ok(StreamEvent::MessageEnd { stop_reason: Some("end_turn".to_string()) })).await;
        });
        Ok(Box::pin(ReceiverStream::new(rx)))
    }
    fn name(&self) -> &str {
        "flaky"
    }
    fn model(&self) -> String {
        self.model.lock().unwrap().clone()
    }
    fn set_model(&self, model: &str) -> Result<()> {
        if model.contains("nokey") {
            anyhow::bail!("no credentials for {model}");
        }
        *self.model.lock().unwrap() = model.to_string();
        Ok(())
    }
    fn fork(&self) -> Arc<dyn Provider> {
        Arc::new(Self {
            model: std::sync::Mutex::new(self.model()),
            error: self.error,
            primary_calls: self.primary_calls.clone(),
            fallback_calls: self.fallback_calls.clone(),
        })
    }
}

struct Rig {
    agent: Agent,
    primary_calls: Arc<AtomicUsize>,
    fallback_calls: Arc<AtomicUsize>,
    spans: Arc<std::sync::Mutex<Vec<factr_base::obs_sink::Span>>>,
}

async fn rig(config_yaml: &str, error: &'static str) -> Rig {
    let dir = std::env::temp_dir().join(format!("fallback-factr-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("config.yaml"), config_yaml).unwrap();
    crate::env::set_var("FACTR_CONFIG_HOME", &dir);
    let (primary_calls, fallback_calls) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
    let provider: Arc<dyn Provider> = Arc::new(FlakyProvider {
        model: std::sync::Mutex::new("primary-model".into()),
        error,
        primary_calls: primary_calls.clone(),
        fallback_calls: fallback_calls.clone(),
    });
    let registry = Registry::new(provider.clone()).await;
    let spans = Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = spans.clone();
    factr_base::obs_sink::install(move |s| sink.lock().unwrap().push(s));
    Rig { agent: Agent::new(provider, registry), primary_calls, fallback_calls, spans }
}

async fn turn(agent: &mut Agent) -> (Result<()>, String) {
    agent.add_message(Role::User, vec![ContentBlock::Text { text: "go".to_string(), cache_control: None }]);
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let result = agent.run_turn_streaming_mpsc(tx).await;
    let mut text = String::new();
    while let Ok(event) = rx.try_recv() {
        if let ServerEvent::TextDelta { text: t } = event {
            text.push_str(&t);
        }
    }
    (result, text)
}

fn fallback_spans(rig: &Rig) -> Vec<(String, String)> {
    rig.spans
        .lock()
        .unwrap()
        .iter()
        .filter(|s| s.kind == "loop.guard" && s.attributes.get("reason").and_then(|v| v.as_str()) == Some("provider_fallback"))
        .map(|s| (s.attributes["from"].as_str().unwrap().to_string(), s.attributes["to"].as_str().unwrap().to_string()))
        .collect()
}

const TWO_ENTRIES: &str = "fallback_providers:\n  - {provider: openrouter, model: nokey-model}\n  - {provider: openrouter, model: rescue}\n";

#[tokio::test]
async fn a_failed_primary_continues_the_turn_on_the_first_usable_fallback() {
    let _guard = crate::storage::lock_test_env();
    let mut rig = rig(TWO_ENTRIES, "500 internal server error").await;
    let (result, text) = turn(&mut rig.agent).await;
    result.expect("the fallback answered");
    assert_eq!(text, "rescued");
    // The entry without credentials was skipped; the next one is serving.
    assert_eq!(rig.agent.provider.model(), "openrouter:rescue");
    assert_eq!(rig.primary_calls.load(Ordering::SeqCst), 1);
    // One span, provider names only.
    assert_eq!(fallback_spans(&rig), vec![("flaky".to_string(), "flaky".to_string())]);
    // A non-rate-limit failure returns to the primary at the next turn (which fails again and falls back again).
    let (result, _) = turn(&mut rig.agent).await;
    result.unwrap();
    assert_eq!(rig.primary_calls.load(Ordering::SeqCst), 2, "the primary was tried again");
}

#[tokio::test]
async fn a_rate_limited_primary_stays_cooled_down_on_the_fallback() {
    let _guard = crate::storage::lock_test_env();
    let mut rig = rig(TWO_ENTRIES, "429 Too Many Requests").await;
    turn(&mut rig.agent).await.0.unwrap();
    turn(&mut rig.agent).await.0.unwrap();
    assert_eq!(rig.primary_calls.load(Ordering::SeqCst), 1, "no second try on the primary inside the 60 s cooldown");
    assert_eq!(rig.fallback_calls.load(Ordering::SeqCst), 2);
    // Once the cooldown has run out the next turn goes back to the primary.
    rig.agent.fallback.until = Some(Instant::now() - Duration::from_secs(1));
    turn(&mut rig.agent).await.0.unwrap();
    assert_eq!(rig.primary_calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn nothing_configured_or_nothing_usable_fails_as_before_and_never_loops() {
    let _guard = crate::storage::lock_test_env();
    let mut none = rig("{}", "500 boom").await;
    let (result, text) = turn(&mut none.agent).await;
    assert!(result.unwrap_err().to_string().contains("boom") && text.is_empty());
    assert!(fallback_spans(&none).is_empty());

    let mut all_refused = rig("fallback_providers:\n  - {provider: openrouter, model: nokey-a}\n  - {provider: gemini, model: nokey-b}\n", "500 boom").await;
    assert!(turn(&mut all_refused.agent).await.0.is_err());
    assert_eq!(all_refused.primary_calls.load(Ordering::SeqCst), 1, "one request, no retry loop");
    assert!(fallback_spans(&all_refused).is_empty());
}

#[tokio::test]
async fn a_context_limit_error_is_not_a_provider_failure() {
    let _guard = crate::storage::lock_test_env();
    let mut rig = rig(TWO_ENTRIES, "prompt is too long: maximum context length exceeded").await;
    assert!(turn(&mut rig.agent).await.0.is_err());
    assert_eq!(rig.fallback_calls.load(Ordering::SeqCst), 0);
    assert!(fallback_spans(&rig).is_empty());
}

#[tokio::test]
async fn a_subagent_stops_at_its_delegation_iteration_cap() {
    let _guard = crate::storage::lock_test_env();
    let mut rig = rig("{}", "unused").await;
    rig.agent.provider.set_model("healthy").unwrap();
    // A cap of 1 allows the one request a plain answer needs.
    factr_base::factr_config::set_iteration_cap(&rig.agent.session.id, 1);
    let (result, text) = turn(&mut rig.agent).await;
    result.unwrap();
    assert_eq!(text, "rescued");
    // A cap of 0 stops before any request is made, with a limit-reached stop.
    factr_base::factr_config::set_iteration_cap(&rig.agent.session.id, 0);
    rig.agent.add_message(Role::User, vec![ContentBlock::Text { text: "again".to_string(), cache_control: None }]);
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    rig.agent.run_turn_streaming_mpsc(tx).await.unwrap();
    let mut stopped = false;
    while let Ok(event) = rx.try_recv() {
        stopped |= matches!(event, ServerEvent::TurnStopped { reason: crate::protocol::TurnStopReason::LimitReached, .. });
    }
    assert!(stopped, "a LimitReached stop");
    assert_eq!(rig.fallback_calls.load(Ordering::SeqCst), 1, "no second request");
}
