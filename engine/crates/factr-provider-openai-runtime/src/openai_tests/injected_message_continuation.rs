// A message injected into one call's request (a stop nudge, a reminder) decides whether the next
// call of the turn can continue the socket chain: it can only when the injected message is still
// at the same position in the next call's input.

/// Sends each request in turn through the public stream. `reuse[i]` says whether request `i`
/// (from the second on) is expected as a delta on the same socket or as a full request on a
/// fresh one.
async fn injected_message_case(requests: Vec<Vec<ChatMessage>>, reuse: Vec<bool>) {
    let _lock = factr_base::storage::lock_test_env();
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let _base = EnvVarGuard::set("FACTR_OPENAI_API_BASE", &format!("http://{addr}/v1"));
    let _transport = EnvVarGuard::set("FACTR_OPENAI_TRANSPORT", "websocket");
    let _prewarm = EnvVarGuard::set("FACTR_OPENAI_PREWARM", "0");
    let inputs: Vec<_> = requests.iter().map(|m| build_responses_input(m)).collect();
    // Pin the cause to the `input_prefix_changed` branch: every input grows (so not
    // `input_not_growing`), the fixture stays healthy, and the old hashes are a prefix of the
    // new ones exactly when the socket is expected to be reused.
    for (i, pair) in inputs.windows(2).enumerate() {
        assert!(pair[1].len() > pair[0].len(), "request {} must grow", i + 1);
        let (old, new) = (
            persistent_ws_input_item_hashes(&pair[0]),
            persistent_ws_input_item_hashes(&pair[1]),
        );
        assert_eq!(new.starts_with(&old), reuse[i], "request {}", i + 1);
    }
    let mut server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(tcp).await.unwrap();
        let mut cursor = 0;
        for (i, input) in inputs.iter().enumerate() {
            if i > 0 && !reuse[i - 1] {
                // The client must drop the old chain; any frame on it is a reuse.
                while let Some(frame) = socket.next().await {
                    match frame {
                        Ok(WsMessage::Ping(payload)) => {
                            socket.send(WsMessage::Pong(payload)).await.unwrap()
                        }
                        Ok(WsMessage::Close(_)) | Err(_) => break,
                        other => panic!("request {i} reused the old socket: {other:?}"),
                    }
                }
                let (tcp, _) = listener.accept().await.unwrap();
                socket = tokio_tungstenite::accept_async(tcp).await.unwrap();
                cursor = 0;
            }
            let request = prefix_test_request(&mut socket).await;
            if cursor == 0 {
                assert!(request.get("previous_response_id").is_none(), "{request}");
            } else {
                assert_eq!(request["previous_response_id"], format!("resp_injected_{}", i - 1));
            }
            assert_eq!(request["input"], serde_json::json!(&input[cursor..]), "request {i}");
            cursor = input.len();
            prefix_test_response(&mut socket, &format!("resp_injected_{i}")).await;
        }
    });
    let provider = prefix_test_provider().await;
    let outcome = tokio::time::timeout(Duration::from_secs(5), async {
        for (i, request) in requests.iter().enumerate() {
            prefix_test_complete(&provider, request, &format!("resp_injected_{i}")).await;
        }
        (&mut server).await.expect("loopback server assertions");
    })
    .await;
    if outcome.is_err() {
        server.abort();
        let result = server.await;
        assert!(
            !matches!(&result, Err(error) if error.is_panic()),
            "loopback server panicked: {result:?}"
        );
    }
    outcome.expect("completions and server must finish");
}

/// The smoke-run turn: write, reply DONE, nudged call, read back.
fn injected_message_turn(nudge_stays: bool) -> Vec<Vec<ChatMessage>> {
    let mut history = vec![
        ChatMessage::user("write one file with the file tool, do not run it, reply DONE"),
        assistant_tool_use("call_write", "write", serde_json::json!({"file_path":"a.py"})),
        ChatMessage::tool_result("call_write", "Created a.py", false),
        ChatMessage::assistant_text("DONE"),
    ];
    let before_nudge = history.clone();
    let nudge = ChatMessage::user("Run or read back what you produced, then finish.");
    let mut nudged = history.clone();
    nudged.push(nudge.clone());
    if nudge_stays {
        history.push(nudge);
    }
    history.push(assistant_tool_use("call_read", "read", serde_json::json!({"file_path":"a.py"})));
    history.push(ChatMessage::tool_result("call_read", "1 print()", false));
    vec![before_nudge, nudged, history]
}

#[tokio::test]
async fn an_ephemeral_injected_message_forces_a_full_request_on_the_next_call() {
    // The nudged call itself extends the previous input; the call after it does not.
    injected_message_case(injected_message_turn(false), vec![true, false]).await;
}

#[tokio::test]
async fn an_injected_message_kept_in_place_lets_the_next_call_continue() {
    injected_message_case(injected_message_turn(true), vec![true, true]).await;
}
