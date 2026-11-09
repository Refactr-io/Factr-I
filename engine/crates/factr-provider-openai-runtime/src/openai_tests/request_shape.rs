// Golden request fixtures. They were generated on the base commit, before the request-shape
// switches existed, so the baseline shape must keep producing these bytes exactly.
// Regenerate (only on purpose) with FACTR_WRITE_REQUEST_GOLDEN=1.

const GOLDEN_DIR: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/fixtures/openai/request_shape_golden"
);

fn golden_tools() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "bash".to_string(),
            description: "run shell".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["command"],
                "properties": {
                    "command": { "type": "string" },
                    "timeout": { "type": "integer" }
                }
            }),
        },
        ToolDefinition {
            name: "read".to_string(),
            description: "read a file".to_string(),
            input_schema: serde_json::json!({
                "allOf": [
                    { "type": "object", "properties": { "file_path": { "type": "string" } }, "required": ["file_path"] },
                    { "type": "object", "properties": { "start_line": { "type": "integer" } } }
                ]
            }),
        },
        // A free-form object node: this one can never be strict.
        ToolDefinition {
            name: "batch".to_string(),
            description: "batch calls".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["tool_calls"],
                "properties": {
                    "tool_calls": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "required": ["tool", "parameters"],
                            "properties": {
                                "tool": { "type": "string" },
                                "parameters": { "type": "object" }
                            }
                        }
                    }
                }
            }),
        },
    ]
}

fn golden_messages() -> Vec<ChatMessage> {
    let msg = |role, content| ChatMessage {
        role,
        content,
        timestamp: None,
        tool_duration_ms: None,
    };
    vec![
        ChatMessage::user("List the files."),
        msg(
            Role::Assistant,
            vec![
                ContentBlock::OpenAIReasoning {
                    id: "rs_golden".to_string(),
                    summary: vec!["Run ls.".to_string()],
                    encrypted_content: Some("enc_golden".to_string()),
                    status: Some("completed".to_string()),
                },
                ContentBlock::ToolUse {
                    id: "call_golden".to_string(),
                    name: "bash".to_string(),
                    input: serde_json::json!({ "command": "ls" }),
                    thought_signature: None,
                },
            ],
        ),
        msg(
            Role::User,
            vec![ContentBlock::ToolResult {
                tool_use_id: "call_golden".to_string(),
                content: "a.txt\nb.txt".to_string(),
                is_error: None,
            }],
        ),
        ChatMessage::user("Now read a.txt."),
    ]
}

fn golden_full_request(
    is_chatgpt_mode: bool,
    input: &[Value],
    api_tools: &[Value],
    shape: Option<&RequestShape>,
) -> Value {
    let args = (
        "gpt-5.4",
        "golden system".to_string(),
        input,
        api_tools,
        is_chatgpt_mode,
        false,
        Some(16384),
        Some("high"),
        None,
        Some("factr-session-golden"),
        Some("24h"),
        Some(160_000),
    );
    let mut request = match shape {
        // The pre-switch entry point, pinned to the baseline shape.
        None => OpenAIProvider::build_response_request(
            args.0, args.1, args.2, args.3, args.4, args.5, args.6, args.7, args.8, args.9,
            args.10, args.11,
        ),
        Some(shape) => OpenAIProvider::build_response_request_with_shape(
            args.0, args.1, args.2, args.3, args.4, args.5, args.6, args.7, args.8, args.9,
            args.10, args.11, shape,
        ),
    };
    // `text` comes from the user's config file, not from the request shape.
    request.as_object_mut().unwrap().remove("text");
    request
}

/// Every golden request, built through the production builders: with `None` through the public
/// wrappers (the ones the Copilot runtime also calls), with a shape through the shape-aware ones.
fn golden_requests(shape: Option<&RequestShape>) -> Vec<(&'static str, Value)> {
    let tools = golden_tools();
    let input = build_responses_input(&golden_messages());
    let api_tools = match shape {
        None => build_tools(&tools),
        Some(shape) => build_tools_with_shape(&tools, shape),
    };
    let chatgpt = golden_full_request(true, &input, &api_tools, shape);
    let api_key = golden_full_request(false, &input, &api_tools, shape);
    let continuation = openai_stream_runtime::build_continuation_request(
        &chatgpt,
        "resp_golden",
        &input[input.len() - 2..],
    );
    let omit_cache_key = shape.is_some_and(|shape| shape.chatgpt_cache_key);
    let prewarm = openai_websocket_prewarm::prewarm_request(&chatgpt, omit_cache_key);
    let compaction = openai_provider_impl::native_compaction_request("gpt-5.4", &input);
    vec![
        ("full_chatgpt", chatgpt),
        ("full_api_key", api_key),
        ("continuation_chatgpt", continuation),
        ("prewarm_chatgpt", prewarm),
        ("native_compaction", compaction),
    ]
}

#[test]
fn baseline_requests_match_golden_fixtures() {
    let write = std::env::var("FACTR_WRITE_REQUEST_GOLDEN").is_ok_and(|v| v == "1");
    let wrappers = golden_requests(None);
    let baseline = golden_requests(Some(&RequestShape::BASELINE));
    for ((name, request), (_, shaped)) in wrappers.into_iter().zip(baseline) {
        assert_eq!(
            serde_json::to_string(&shaped).unwrap(),
            serde_json::to_string(&request).unwrap(),
            "{name}: the baseline shape differs from the wrappers"
        );
        let path = format!("{GOLDEN_DIR}/{name}.json");
        // The compact form is the exact bytes put on the wire.
        let bytes = serde_json::to_string(&request).unwrap();
        if write {
            std::fs::create_dir_all(GOLDEN_DIR).unwrap();
            std::fs::write(&path, &bytes).unwrap();
            continue;
        }
        let golden = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
        assert_eq!(bytes, golden, "{name} drifted from its golden fixture");
    }
}

#[test]
fn variant_tools_are_not_strict_and_keep_the_compatible_schema() {
    let tools = golden_tools();
    let built = build_tools_with_shape(&tools, &RequestShape::VARIANT);
    for (api, def) in built.iter().zip(&tools) {
        assert_eq!(api["strict"], false, "{}", def.name);
        assert_eq!(
            api["parameters"],
            factr_provider_core::openai_schema::openai_compatible_schema(&def.input_schema),
            "{}",
            def.name
        );
    }
    // The baseline sibling and the public wrapper (also used by the Copilot runtime) agree.
    assert_eq!(
        serde_json::to_string(&build_tools_with_shape(&tools, &RequestShape::BASELINE)).unwrap(),
        serde_json::to_string(&build_tools(&tools)).unwrap()
    );
}

/// An idle prewarm runs before any session context exists, so its cache key is the instance key;
/// the foreground request names the session. With the ChatGPT key on, they must still match.
#[tokio::test]
async fn variant_idle_prewarm_is_adopted_by_a_foreground_request_with_a_session_key() {
    for shape in [
        RequestShape::VARIANT,
        RequestShape {
            parallel_tool_calls: true,
            ..RequestShape::VARIANT
        },
    ] {
        let api_tools = build_tools_with_shape(&golden_tools(), &shape);
        let build = |input: &[Value], key: &str| {
            OpenAIProvider::build_response_request_with_shape(
                "gpt-5.4",
                "golden system".to_string(),
                input,
                &api_tools,
                true,
                false,
                None,
                Some("high"),
                None,
                Some(key),
                None,
                Some(160_000),
                &shape,
            )
        };
        let idle = build(&[], "factr-session-instance");
        let foreground = build(
            &build_responses_input(&golden_messages()),
            "factr-session-real",
        );
        assert_eq!(foreground["prompt_cache_key"], "factr-session-real");
        let omit = shape.chatgpt_cache_key;
        assert_ne!(
            openai_websocket_prewarm::prewarm_request(&idle, false),
            openai_websocket_prewarm::prewarm_request(&foreground, false),
            "without the omission the session key would discard every warm socket"
        );

        let (mut state, _server) = test_persistent_ws_state().await;
        let credentials = CodexCredentials {
            access_token: "test-access".into(),
            refresh_token: "test-refresh".into(),
            id_token: None,
            account_id: Some("test-account".into()),
            expires_at: None,
        };
        state.identity = openai_websocket_prewarm::prewarm_identity(&credentials);
        let slot = openai_websocket_prewarm::PrewarmSlot::default();
        slot.insert_ready_for_test(
            openai_websocket_prewarm::prewarm_request(&idle, omit),
            state,
        );
        assert!(
            slot.take_ready(&foreground, &credentials, omit).is_some(),
            "the idle warm socket must be adopted"
        );

        let continuation =
            openai_stream_runtime::build_continuation_request(&foreground, "resp_golden", &[]);
        assert_eq!(
            continuation["prompt_cache_key"],
            foreground["prompt_cache_key"]
        );
        assert_eq!(
            continuation["parallel_tool_calls"],
            foreground["parallel_tool_calls"]
        );
    }
}
