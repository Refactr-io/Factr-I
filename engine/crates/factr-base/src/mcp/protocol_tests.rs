use super::*;

#[test]
fn factr_mcp_settings_are_loaded_for_engine_chats() {
    let _guard = crate::storage::lock_test_env();
    let previous_factr_home = std::env::var_os("FACTR_CONFIG_HOME");
    let factr_home = tempfile::tempdir().expect("Factr home");
    crate::env::set_var("FACTR_CONFIG_HOME", factr_home.path());
    std::fs::write(
        factr_home.path().join("config.yaml"),
        "mcp_servers:\n  factr-api-server:\n    command: /tmp/factr-mcp\n    args: [--from-settings]\n    enabled: true\n  remote:\n    url: https://example.test/mcp\n    type: http\n  no-transport:\n    type: sse\n",
    )
    .expect("write Factr config");

    let result = std::panic::catch_unwind(|| {
        let config = McpConfig::load();
        let server = config
            .servers
            .get("factr-api-server")
            .expect("server loaded");
        assert_eq!(server.command, "/tmp/factr-mcp");
        assert_eq!(server.args, ["--from-settings"]);
        assert!(server.is_enabled());
        assert!(config.servers["remote"].is_http());
        // Unsupported transports are dropped.
        assert!(!config.servers.contains_key("no-transport"));
    });

    if let Some(value) = previous_factr_home {
        crate::env::set_var("FACTR_CONFIG_HOME", value);
    } else {
        crate::env::remove_var("FACTR_CONFIG_HOME");
    }
    result.expect("Factr MCP settings load");
}

#[test]
fn test_json_rpc_request_serialization() {
    let request = JsonRpcRequest::new(1, "tools/list", None);
    let json = serde_json::to_string(&request).unwrap();
    assert!(json.contains("\"jsonrpc\":\"2.0\""));
    assert!(json.contains("\"id\":1"));
    assert!(json.contains("\"method\":\"tools/list\""));
}

#[test]
fn test_json_rpc_notification_serialization_omits_id() {
    let notification = JsonRpcNotification::new("notifications/initialized", None);
    let value = serde_json::to_value(notification).unwrap();
    assert_eq!(value["jsonrpc"], "2.0");
    assert_eq!(value["method"], "notifications/initialized");
    assert!(value.get("id").is_none());
    assert!(value.get("params").is_none());
}

#[test]
fn test_json_rpc_response_deserialization() {
    let json = r#"{"jsonrpc":"2.0","id":1,"result":{"tools":[]}}"#;
    let response: JsonRpcResponse = serde_json::from_str(json).unwrap();
    assert_eq!(response.id, Some(1));
    assert!(response.result.is_some());
    assert!(response.error.is_none());
}

#[test]
fn test_json_rpc_error_response() {
    let json = r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32600,"message":"Invalid Request"}}"#;
    let response: JsonRpcResponse = serde_json::from_str(json).unwrap();
    assert!(response.error.is_some());
    let err = response.error.unwrap();
    assert_eq!(err.code, -32600);
    assert_eq!(err.message, "Invalid Request");
}

#[test]
fn test_mcp_config_deserialization() {
    let json = r#"{
            "servers": {
                "test-server": {
                    "command": "/usr/bin/test-mcp",
                    "args": ["--port", "8080"],
                    "env": {"API_KEY": "secret"}
                }
            }
        }"#;
    let config: McpConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.servers.len(), 1);
    let server = config.servers.get("test-server").unwrap();
    assert_eq!(server.command, "/usr/bin/test-mcp");
    assert_eq!(server.args, vec!["--port", "8080"]);
    assert_eq!(server.env.get("API_KEY"), Some(&"secret".to_string()));
}

#[test]
fn test_mcp_config_timeout_secs_defaults_to_none_and_accepts_override() {
    // Issues #802 / #1174: per-server reply timeout. Absent keeps the 30s
    // default; an explicit value is honored.
    let json = r#"{
            "mcpServers": {
                "fast": {"command": "fast-mcp"},
                "slow": {"command": "slow-mcp", "timeout_secs": 120}
            }
        }"#;
    let config: McpConfig = serde_json::from_str(json).unwrap();
    let fast = config.servers.get("fast").unwrap();
    let slow = config.servers.get("slow").unwrap();
    assert_eq!(fast.timeout_secs, None);
    assert_eq!(slow.timeout_secs, Some(120));
    assert_eq!(
        crate::mcp::request_timeout_for(fast),
        crate::mcp::DEFAULT_MCP_REQUEST_TIMEOUT
    );
    assert_eq!(
        crate::mcp::request_timeout_for(slow),
        std::time::Duration::from_secs(120)
    );
    // Zero is treated as "unset" rather than an instant timeout.
    let zero = McpServerConfig {
        timeout_secs: Some(0),
        ..slow.clone()
    };
    assert_eq!(
        crate::mcp::request_timeout_for(&zero),
        crate::mcp::DEFAULT_MCP_REQUEST_TIMEOUT
    );
}

#[test]
fn test_mcp_config_empty() {
    let json = r#"{}"#;
    let config: McpConfig = serde_json::from_str(json).unwrap();
    assert!(config.servers.is_empty());
}

#[test]
fn test_mcp_config_accepts_claude_mcp_servers_key() {
    // Claude Code uses `mcpServers`, not `servers`.
    let json = r#"{
            "mcpServers": {
                "claude-server": {
                    "command": "npx",
                    "args": ["-y", "some-mcp"]
                }
            }
        }"#;
    let config: McpConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.servers.len(), 1);
    let server = config.servers.get("claude-server").unwrap();
    assert_eq!(server.command, "npx");
    assert!(server.is_stdio());
}

#[test]
fn test_mcp_http_server_is_not_stdio() {
    let json = r#"{
            "mcpServers": {
                "remote": {
                    "type": "http",
                    "url": "https://example.com/mcp"
                }
            }
        }"#;
    let config: McpConfig = serde_json::from_str(json).unwrap();
    let server = config.servers.get("remote").unwrap();
    assert!(!server.is_stdio());
    assert_eq!(server.url.as_deref(), Some("https://example.com/mcp"));
}

#[test]
fn environment_expansion_matches_claude_syntax_across_config_fields() {
    let json = r#"{
        "mcpServers": {
            "probe": {
                "command": "${BIN_DIR}/probe",
                "args": ["--token=${TOKEN:-fallback-token}", "${MISSING_DEFAULT:-fallback}", "${MISSING}"],
                "env": {
                    "PROBE_PATH": "${BIN_DIR}/marker",
                    "STILL_MISSING": "prefix-${MISSING}"
                },
                "type": "http",
                "url": "${BASE_URL:-https://example.test}/mcp",
                "headers": {"Authorization": "Bearer ${TOKEN}"}
            }
        }
    }"#;
    let mut config: McpConfig = serde_json::from_str(json).unwrap();

    let warnings = config.expand_environment_variables_with(|variable| match variable {
        "BIN_DIR" => Some("/opt/tools".to_string()),
        "TOKEN" => Some("secret-token".to_string()),
        _ => None,
    });

    let server = config.servers.get("probe").unwrap();
    assert_eq!(server.command, "/opt/tools/probe");
    assert_eq!(
        server.args,
        vec!["--token=secret-token", "fallback", "${MISSING}"]
    );
    assert_eq!(server.env["PROBE_PATH"], "/opt/tools/marker");
    assert_eq!(server.env["STILL_MISSING"], "prefix-${MISSING}");
    assert_eq!(server.url.as_deref(), Some("https://example.test/mcp"));
    assert_eq!(server.headers["Authorization"], "Bearer secret-token");
    assert_eq!(
        warnings,
        vec![UnresolvedEnvironmentVariable {
            server: "probe".to_string(),
            variable: "MISSING".to_string(),
        }],
        "an unresolved variable is preserved and warned once per server"
    );
}

#[test]
fn environment_expansion_preserves_malformed_and_unclosed_expressions() {
    let mut unresolved = std::collections::BTreeSet::new();
    let expanded = expand_environment_string(
        "${1INVALID} ${:-no} ${UNCLOSED",
        &|_| Some("unexpected".to_string()),
        &mut unresolved,
    );

    assert_eq!(expanded, "${1INVALID} ${:-no} ${UNCLOSED");
    assert!(unresolved.is_empty());
}

#[test]
fn load_expands_environment_in_factr_servers() {
    let _guard = crate::storage::lock_test_env();
    let previous_factr = std::env::var_os("FACTR_CONFIG_HOME");
    let previous_value = std::env::var_os("FACTR_MCP_EXPANSION_TEST_VALUE");
    let factr = tempfile::tempdir().expect("factr home");
    crate::env::set_var("FACTR_CONFIG_HOME", factr.path());
    crate::env::set_var("FACTR_MCP_EXPANSION_TEST_VALUE", "expanded-value");
    std::fs::write(
        factr.path().join("config.yaml"),
        "mcp_servers:\n  same-name:\n    command: project-bin\n    args: ['${FACTR_MCP_EXPANSION_TEST_VALUE}']\n",
    )
    .unwrap();

    let result = std::panic::catch_unwind(|| {
        let config = McpConfig::load();
        let server = &config.servers["same-name"];
        assert_eq!(server.command, "project-bin");
        assert_eq!(server.args, ["expanded-value"]);
    });

    match previous_factr {
        Some(value) => crate::env::set_var("FACTR_CONFIG_HOME", value),
        None => crate::env::remove_var("FACTR_CONFIG_HOME"),
    }
    match previous_value {
        Some(value) => crate::env::set_var("FACTR_MCP_EXPANSION_TEST_VALUE", value),
        None => crate::env::remove_var("FACTR_MCP_EXPANSION_TEST_VALUE"),
    }
    result.expect("config expansion assertions");
}

#[test]
fn expanded_values_invalidate_schema_cache_fingerprint() {
    let raw = r#"{"mcpServers":{"srv":{"command":"node","args":["${SCRIPT}"],"env":{"TOKEN":"${TOKEN}"}}}}"#;
    let mut first: McpConfig = serde_json::from_str(raw).unwrap();
    first.expand_environment_variables_with(|variable| match variable {
        "SCRIPT" => Some("first.js".to_string()),
        "TOKEN" => Some("token-one".to_string()),
        _ => None,
    });
    let mut second: McpConfig = serde_json::from_str(raw).unwrap();
    second.expand_environment_variables_with(|variable| match variable {
        "SCRIPT" => Some("second.js".to_string()),
        "TOKEN" => Some("token-two".to_string()),
        _ => None,
    });

    assert_ne!(
        crate::mcp::schema_cache::fingerprint_config(&first.servers["srv"]),
        crate::mcp::schema_cache::fingerprint_config(&second.servers["srv"]),
        "changing expanded environment values must invalidate cached schemas"
    );
}

#[test]
fn test_server_enabled_defaults_true() {
    // Existing configs without the flag keep current behavior (issue #436).
    let json = r#"{"servers":{"srv":{"command":"bin"}}}"#;
    let config: McpConfig = serde_json::from_str(json).unwrap();
    assert!(config.servers.get("srv").unwrap().is_enabled());
}

#[test]
fn test_server_enabled_false_opencode_style() {
    let json = r#"{"servers":{"srv":{"command":"bin","enabled":false}}}"#;
    let config: McpConfig = serde_json::from_str(json).unwrap();
    assert!(!config.servers.get("srv").unwrap().is_enabled());
}

#[test]
fn test_server_disabled_true_claude_style() {
    let json = r#"{"mcpServers":{"srv":{"command":"bin","disabled":true}}}"#;
    let config: McpConfig = serde_json::from_str(json).unwrap();
    assert!(!config.servers.get("srv").unwrap().is_enabled());
}

#[test]
fn test_server_disabled_wins_over_enabled() {
    // `disabled` (Claude Code style) wins when both spellings are present.
    let json = r#"{"servers":{"srv":{"command":"bin","enabled":true,"disabled":true}}}"#;
    let config: McpConfig = serde_json::from_str(json).unwrap();
    assert!(!config.servers.get("srv").unwrap().is_enabled());

    let json = r#"{"servers":{"srv":{"command":"bin","enabled":false,"disabled":false}}}"#;
    let config: McpConfig = serde_json::from_str(json).unwrap();
    assert!(config.servers.get("srv").unwrap().is_enabled());
}

#[test]
fn test_disabled_server_survives_serde_roundtrip() {
    // Disabled servers must stay in config (kept, not spawned).
    let json = r#"{"servers":{"off":{"command":"bin","enabled":false}}}"#;
    let config: McpConfig = serde_json::from_str(json).unwrap();
    let reloaded: McpConfig =
        serde_json::from_str(&serde_json::to_string(&config).unwrap()).unwrap();
    assert!(!reloaded.servers.get("off").unwrap().is_enabled());
}

#[test]
fn test_tool_def_deserialization() {
    let json = r#"{
            "name": "read_file",
            "description": "Read a file from disk",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": {"type": "string"}
                },
                "required": ["path"]
            }
        }"#;
    let tool: McpToolDef = serde_json::from_str(json).unwrap();
    assert_eq!(tool.name, "read_file");
    assert_eq!(tool.description, Some("Read a file from disk".to_string()));
}

#[test]
fn test_tool_call_result_text() {
    let json = r#"{
            "content": [{"type": "text", "text": "File contents here"}],
            "isError": false
        }"#;
    let result: ToolCallResult = serde_json::from_str(json).unwrap();
    assert!(!result.is_error);
    assert_eq!(result.content.len(), 1);
    match &result.content[0] {
        ContentBlock::Text { text, .. } => assert_eq!(text, "File contents here"),
        _ => panic!("Expected text block"),
    }
}

#[test]
fn test_tool_call_result_error() {
    let json = r#"{
            "content": [{"type": "text", "text": "File not found"}],
            "isError": true
        }"#;
    let result: ToolCallResult = serde_json::from_str(json).unwrap();
    assert!(result.is_error);
}

#[test]
fn test_initialize_result() {
    let json = r#"{
            "protocolVersion": "2024-11-05",
            "capabilities": {
                "tools": {"listChanged": true}
            },
            "serverInfo": {
                "name": "test-server",
                "version": "1.0.0"
            }
        }"#;
    let result: InitializeResult = serde_json::from_str(json).unwrap();
    assert_eq!(result.protocol_version, "2024-11-05");
    assert!(result.server_info.is_some());
}
