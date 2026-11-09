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

fn golden_full_request(is_chatgpt_mode: bool, input: &[Value], api_tools: &[Value]) -> Value {
    let mut request = OpenAIProvider::build_response_request(
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
    // `text` comes from the user's config file, not from the request shape.
    request.as_object_mut().unwrap().remove("text");
    request
}

/// Every golden request, built through the production builders.
fn golden_requests() -> Vec<(&'static str, Value)> {
    let input = build_responses_input(&golden_messages());
    let api_tools = build_tools(&golden_tools());
    let chatgpt = golden_full_request(true, &input, &api_tools);
    let api_key = golden_full_request(false, &input, &api_tools);
    let continuation = openai_stream_runtime::build_continuation_request(
        &chatgpt,
        "resp_golden",
        &input[input.len() - 2..],
    );
    let prewarm = openai_websocket_prewarm::prewarm_request(&chatgpt);
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
    for (name, request) in golden_requests() {
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
