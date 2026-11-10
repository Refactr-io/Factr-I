// Stream timing line (FACTR_STREAM_TIMING=1) on the WebSocket read loop; the SSE path is tested
// in factr-provider-openai. The flag is injected as the sink, so these tests never set process
// env; each test collects its lines in its own static.

const TIMING_COMPLETED: &str = r#"{"type":"response.completed","response":{"id":"resp_t","status":"completed","model":"gpt-5.4-served","service_tier":"priority","output":[],"usage":{"input_tokens":100,"input_tokens_details":{"cached_tokens":40},"output_tokens":50,"output_tokens_details":{"reasoning_tokens":20}}}}"#;

fn timing_frames() -> Vec<String> {
    vec![
        r#"{"type":"response.created","response":{"id":"resp_t"}}"#.to_string(),
        r#"{"type":"response.output_text.delta","delta":"hi"}"#.to_string(),
        TIMING_COMPLETED.to_string(),
    ]
}

fn parse_timing_line(lines: &std::sync::Mutex<Vec<String>>) -> Value {
    let lines = lines.lock().unwrap();
    assert_eq!(lines.len(), 1, "one line per finished call: {lines:?}");
    serde_json::from_str(&lines[0]).expect("timing line is JSON")
}

fn assert_timing_fields(line: &Value, transport: &str) {
    assert_eq!(line["factr_stream_timing"], 1);
    assert_eq!(line["transport"], transport);
    let (first, output, last) = (
        line["first_event_ms"].as_u64().unwrap(),
        line["first_output_ms"].as_u64().unwrap(),
        line["last_event_ms"].as_u64().unwrap(),
    );
    // Frames arrive 30 ms apart, so the three stamps are ordered and spread.
    assert!(first < output && output < last, "{line}");
    assert!(last - first >= 50, "{line}");
    assert_eq!(line["input_tokens"], 100);
    assert_eq!(line["cached_tokens"], 40);
    assert_eq!(line["output_tokens"], 50);
    assert_eq!(line["reasoning_tokens"], 20);
    assert_eq!(line["served_service_tier"], "priority");
    assert_eq!(line["response_model"], "gpt-5.4-served");
    for key in [
        "channel_wait_ms",
        "strict_tools",
        "parallel_tool_calls",
        "cache_key_sent",
        "native_compaction_sent",
        "reasoning_id_replayed",
    ] {
        assert!(line.get(key).is_some(), "missing {key}");
    }
}

#[test]
fn stream_timing_is_not_installed_when_the_flag_is_off() {
    assert!(!factr_provider_openai::stream_timing::resolve(&|_| None));
    let request = serde_json::json!({"model": "gpt-5.4"});
    assert!(StreamTiming::start(None, "https", &request, &RequestShape::VARIANT).is_none());
    let mut stream = OpenAIResponsesStream::new(futures::stream::empty());
    assert!(
        stream.timing_mut().is_none(),
        "existing callers get no observer"
    );
}

/// Runs one WebSocket continuation through the production read loop against a local scripted
/// server, with the given delay before the consumer takes each event.
async fn run_timed_ws_continuation(consumer_delay: Duration, sink: fn(&str)) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
        let _request = ws.next().await.unwrap().unwrap();
        for frame in timing_frames() {
            tokio::time::sleep(Duration::from_millis(30)).await;
            ws.send(WsMessage::Text(frame)).await.unwrap();
        }
        while let Some(Ok(message)) = ws.next().await {
            if matches!(message, WsMessage::Close(_)) {
                break;
            }
        }
    });
    let (client_ws, _) = connect_async(format!("ws://{}", addr)).await.unwrap();
    let mut guard = Some(PersistentWsState {
        ws_stream: client_ws,
        identity: openai_websocket_prewarm::prewarm_identity(&prewarm_test_credentials()),
        last_response_id: "resp_previous".to_string(),
        connected_at: Instant::now(),
        last_activity_at: Instant::now(),
        last_response_completed_at: Instant::now(),
        message_count: 1,
        last_input_item_count: 1,
        last_input_item_hashes: persistent_ws_input_item_hashes(&[
            serde_json::json!({"type":"message","role":"user","content":"first"}),
        ]),
    });
    let (tx, mut rx) = mpsc::channel(1);
    let consumer = tokio::spawn(async move {
        loop {
            tokio::time::sleep(consumer_delay).await;
            if rx.recv().await.is_none() {
                break;
            }
        }
    });
    let result = openai_stream_runtime::continue_persistent_ws_locked(
        &mut guard,
        &Arc::new(RwLock::new(prewarm_test_credentials())),
        &serde_json::json!({"model": "gpt-5.4", "parallel_tool_calls": true}),
        &[
            serde_json::json!({"type": "message", "role": "user", "content": "first"}),
            serde_json::json!({"type": "message", "role": "user", "content": "second"}),
        ],
        2,
        &tx,
        Some(sink),
    )
    .await;
    assert!(matches!(result, PersistentWsResult::Success));
    drop(guard);
    drop(tx);
    consumer.await.unwrap();
    server.await.unwrap();
}

static WS_TIMING_LINES: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

#[tokio::test]
async fn stream_timing_line_from_a_synthetic_websocket_continuation() {
    let _env_lock = factr_base::storage::lock_test_env();
    run_timed_ws_continuation(Duration::ZERO, |line| {
        WS_TIMING_LINES.lock().unwrap().push(line.to_string())
    })
    .await;
    let line = parse_timing_line(&WS_TIMING_LINES);
    assert_timing_fields(&line, "ws-continuation");
    assert_eq!(line["parallel_tool_calls"], true);
    assert_eq!(line["cache_key_sent"], false);
}

static SLOW_TIMING_LINES: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

#[tokio::test]
async fn stream_timing_counts_time_blocked_on_a_slow_consumer() {
    let _env_lock = factr_base::storage::lock_test_env();
    run_timed_ws_continuation(Duration::from_millis(80), |line| {
        SLOW_TIMING_LINES.lock().unwrap().push(line.to_string())
    })
    .await;
    let line = parse_timing_line(&SLOW_TIMING_LINES);
    assert!(
        line["channel_wait_ms"].as_u64().unwrap() > 0,
        "a slow consumer must show up as channel wait: {line}"
    );
}
