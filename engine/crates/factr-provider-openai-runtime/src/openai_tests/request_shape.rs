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
    let (input, api_tools) = match shape {
        None => (
            build_responses_input(&golden_messages()),
            build_tools(&tools),
        ),
        Some(shape) => (
            build_responses_input_with_shape(&golden_messages(), shape),
            build_tools_with_shape(&tools, shape),
        ),
    };
    let chatgpt = golden_full_request(true, &input, &api_tools, shape);
    let api_key = golden_full_request(false, &input, &api_tools, shape);
    let continuation = openai_stream_runtime::build_continuation_request(
        &chatgpt,
        "resp_golden",
        &input[input.len() - 2..],
    );
    let omit_cache_key = shape.is_some_and(|shape| shape.prewarm_omits_cache_key(true));
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

/// Leaf paths where two JSON values differ (array items are indexed, objects keyed).
fn json_diff(a: &Value, b: &Value, path: String, out: &mut Vec<String>) {
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            let keys: std::collections::BTreeSet<_> = x.keys().chain(y.keys()).collect();
            for k in keys {
                let (va, vb) = (x.get(k), y.get(k));
                match (va, vb) {
                    (Some(va), Some(vb)) => json_diff(va, vb, format!("{path}.{k}"), out),
                    _ => out.push(format!("{path}.{k}")),
                }
            }
        }
        (Value::Array(x), Value::Array(y)) if x.len() == y.len() => {
            for (i, (va, vb)) in x.iter().zip(y).enumerate() {
                json_diff(va, vb, format!("{path}[{i}]"), out);
            }
        }
        _ if a != b => out.push(path),
        _ => {}
    }
}

fn shape_diff(a: &RequestShape, b: &RequestShape) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    for ((name, ra), (_, rb)) in golden_requests(Some(a))
        .into_iter()
        .zip(golden_requests(Some(b)))
    {
        let mut paths = Vec::new();
        json_diff(&ra, &rb, String::new(), &mut paths);
        out.extend(paths.into_iter().map(|p| (name, p)));
    }
    out
}

#[test]
fn variant_defaults_through_the_production_builders() {
    let requests: HashMap<_, _> = golden_requests(Some(&RequestShape::VARIANT))
        .into_iter()
        .collect();
    let chatgpt = &requests["full_chatgpt"];
    for tool in chatgpt["tools"].as_array().unwrap() {
        assert_eq!(tool["strict"], false);
    }
    assert_eq!(chatgpt["prompt_cache_key"], "factr-session-golden");
    assert!(chatgpt.get("prompt_cache_retention").is_none());
    assert_eq!(chatgpt["parallel_tool_calls"], false);
    assert_eq!(chatgpt["input"][1]["type"], "reasoning");
    assert_eq!(chatgpt["input"][1]["id"], "rs_golden");
    assert_eq!(
        requests["continuation_chatgpt"]["parallel_tool_calls"],
        false
    );
    // The prewarm payload leaves the session key out (see the adoption test).
    assert!(
        requests["prewarm_chatgpt"]
            .get("prompt_cache_key")
            .is_none()
    );
}

#[test]
fn each_switch_alone_changes_only_its_own_fields() {
    let v = RequestShape::VARIANT;
    let is_tool_field = |p: &str| {
        p.starts_with(".tools[") && (p.contains("].strict") || p.contains("].parameters"))
    };

    let strict = shape_diff(
        &v,
        &RequestShape {
            strict_tools: true,
            ..v
        },
    );
    assert!(!strict.is_empty());
    for (name, path) in &strict {
        assert!(is_tool_field(path), "strict changed {name}{path}");
    }

    let key = shape_diff(
        &v,
        &RequestShape {
            chatgpt_cache_key: false,
            ..v
        },
    );
    let expected: Vec<(&str, String)> = vec![
        ("full_chatgpt", ".prompt_cache_key".into()),
        ("continuation_chatgpt", ".prompt_cache_key".into()),
    ];
    assert_eq!(key, expected);

    let parallel = shape_diff(
        &v,
        &RequestShape {
            parallel_tool_calls: true,
            ..v
        },
    );
    let expected: Vec<(&str, String)> = vec![
        ("full_chatgpt", ".parallel_tool_calls".into()),
        ("full_api_key", ".parallel_tool_calls".into()),
        ("continuation_chatgpt", ".parallel_tool_calls".into()),
        ("prewarm_chatgpt", ".parallel_tool_calls".into()),
    ];
    assert_eq!(parallel, expected);

    let id = shape_diff(
        &v,
        &RequestShape {
            replay_reasoning_id: false,
            ..v
        },
    );
    let expected: Vec<(&str, String)> = vec![
        ("full_chatgpt", ".input[1].id".into()),
        ("full_api_key", ".input[1].id".into()),
        ("native_compaction", ".input[1].id".into()),
    ];
    assert_eq!(id, expected);
}

/// Opt-in: `FACTR_OPENAI_CACHE_KEY_MODE=content`.
fn content_shape() -> RequestShape {
    RequestShape {
        content_cache_key: true,
        ..RequestShape::VARIANT
    }
}

fn content_request(system: &str, tools: &[ToolDefinition], input: &[Value], session_key: &str, chatgpt: bool) -> Value {
    let shape = content_shape();
    OpenAIProvider::build_response_request_with_shape(
        "gpt-5.4",
        system.to_string(),
        input,
        &build_tools_with_shape(tools, &shape),
        chatgpt,
        false,
        None,
        Some("high"),
        None,
        Some(session_key),
        None,
        Some(160_000),
        &shape,
    )
}

#[test]
fn content_key_depends_on_instructions_and_tools_not_on_the_session() {
    let tools = golden_tools();
    let input = build_responses_input(&golden_messages());
    let a = content_request("golden system", &tools, &input, "factr-session-a", true);
    let b = content_request("golden system", &tools, &[], "factr-session-b", true);
    let key = a["prompt_cache_key"].as_str().unwrap();
    assert!(key.starts_with("pck_") && key.len() == 28, "{key}");
    assert_eq!(a["prompt_cache_key"], b["prompt_cache_key"], "two sessions, same prompt and tools");
    let mut reordered = tools.clone();
    reordered.reverse();
    assert_eq!(content_request("golden system", &reordered, &[], "x", true)["prompt_cache_key"], a["prompt_cache_key"]);
    assert_ne!(content_request("golden system", &tools[..2], &[], "x", true)["prompt_cache_key"], a["prompt_cache_key"]);
    assert_ne!(content_request("other system", &tools, &[], "x", true)["prompt_cache_key"], a["prompt_cache_key"]);
    // API-key mode keeps its session key.
    assert_eq!(content_request("golden system", &tools, &[], "factr-session-a", false)["prompt_cache_key"], "factr-session-a");
}

#[test]
fn content_key_alone_changes_only_the_cache_key_and_the_prewarm_keeps_it() {
    let diff = shape_diff(&RequestShape::VARIANT, &content_shape());
    let expected: Vec<(&str, String)> = vec![
        ("full_chatgpt", ".prompt_cache_key".into()),
        ("continuation_chatgpt", ".prompt_cache_key".into()),
        ("prewarm_chatgpt", ".prompt_cache_key".into()),
    ];
    assert_eq!(diff, expected);
}

/// In content mode the prewarm sends the key and adoption compares it: the same prompt and
/// tools adopt the warm socket, a different tool list does not.
#[tokio::test]
async fn content_key_prewarm_is_adopted_only_with_the_same_key() {
    let shape = content_shape();
    let omit = shape.prewarm_omits_cache_key(true);
    assert!(!omit);
    assert!(RequestShape::VARIANT.prewarm_omits_cache_key(true), "session mode still omits");
    let credentials = CodexCredentials {
        access_token: "test-access".into(),
        refresh_token: "test-refresh".into(),
        id_token: None,
        account_id: Some("test-account".into()),
        expires_at: None,
    };
    let tools = golden_tools();
    let idle = content_request("golden system", &tools, &[], "factr-session-instance", true);
    let warm = openai_websocket_prewarm::prewarm_request(&idle, omit);
    assert_eq!(warm["prompt_cache_key"], idle["prompt_cache_key"], "the warmup carries the key");
    let input = build_responses_input(&golden_messages());
    for (foreground, adopted) in [
        (content_request("golden system", &tools, &input, "factr-session-real", true), true),
        (content_request("golden system", &tools[..2], &input, "factr-session-real", true), false),
    ] {
        let (mut state, _server) = test_persistent_ws_state().await;
        state.identity = openai_websocket_prewarm::prewarm_identity(&credentials);
        let slot = openai_websocket_prewarm::PrewarmSlot::default();
        slot.insert_ready_for_test(warm.clone(), state);
        assert_eq!(slot.take_ready(&foreground, &credentials, omit).is_some(), adopted);
    }
    // A different key alone (same tools, other instructions) is also rejected.
    let other = content_request("other system", &tools, &input, "s", true);
    let mut only_key = idle.clone();
    only_key["prompt_cache_key"] = other["prompt_cache_key"].clone();
    let (mut state, _server) = test_persistent_ws_state().await;
    state.identity = openai_websocket_prewarm::prewarm_identity(&credentials);
    let slot = openai_websocket_prewarm::PrewarmSlot::default();
    slot.insert_ready_for_test(openai_websocket_prewarm::prewarm_request(&only_key, omit), state);
    let foreground = content_request("golden system", &tools, &input, "s", true);
    assert!(slot.take_ready(&foreground, &credentials, omit).is_none(), "mismatched content key");
}

/// A request with no tools of its own (a REPL sub-call) gets no hosted image tool when the switch is on;
/// a request with tools keeps it, and the baseline attaches it to both.
#[test]
fn the_hosted_image_tool_needs_tools_of_its_own() {
    let has_image = |shape: &RequestShape, api_tools: &[Value]| {
        OpenAIProvider::build_response_request_with_shape(
            "gpt-5.4", "s".to_string(), &[], api_tools, true, true, None, Some("medium"), None, None, None, None, shape,
        )["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool["type"] == "image_generation")
    };
    let tools = build_tools_with_shape(&golden_tools(), &RequestShape::VARIANT);
    assert!(!has_image(&RequestShape::VARIANT, &[]));
    assert!(has_image(&RequestShape::VARIANT, &tools));
    assert!(has_image(&RequestShape::BASELINE, &[]));
    assert!(has_image(&RequestShape::BASELINE, &tools));
}

/// gpt-6-luna gets no hosted image tool with the switch on; other models keep it, and the baseline
/// attaches it to gpt-6-luna as in v0.0.15.
#[test]
fn the_hosted_image_tool_is_not_sent_to_luna() {
    let tools = build_tools_with_shape(&golden_tools(), &RequestShape::VARIANT);
    let has_image = |shape: &RequestShape, model: &str| {
        OpenAIProvider::build_response_request_with_shape(
            model, "s".to_string(), &[], &tools, true, true, None, Some("low"), None, None, None, None, shape,
        )["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool["type"] == "image_generation")
    };
    assert!(!has_image(&RequestShape::VARIANT, "gpt-6-luna"));
    assert!(has_image(&RequestShape::VARIANT, "gpt-5.4"));
    assert!(has_image(&RequestShape::BASELINE, "gpt-6-luna"));
}
