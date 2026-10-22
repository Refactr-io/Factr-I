use super::*;

/// Inline tool-schema budget (estimated tokens, chars/4, same meter as the
/// bench counting proxy). Measured 7,618 across 26 tools before lazy loading, 2,027 across 9 after.
const INLINE_SCHEMA_BUDGET: usize = 2_200;

/// An agent with the factr-engine tool set. The env gates are only read
/// while the registry is built, so restore them for neighbouring tests.
async fn factr_agent() -> Agent {
    let keys = ["FACTR_REPL_WORKER", "FACTR_REPL_PYTHON"];
    let saved = keys.map(|k| (k, std::env::var_os(k)));
    crate::env::set_var("FACTR_REPL_WORKER", "/bin/factr");
    crate::env::set_var("FACTR_REPL_PYTHON", "/bin/sh");
    let provider: Arc<dyn Provider> = Arc::new(NativeAutoCompactionProvider);
    let registry = Registry::new(provider.clone()).await;
    for (key, value) in saved {
        match value {
            Some(value) => crate::env::set_var(key, value),
            None => crate::env::remove_var(key),
        }
    }
    Agent::new(provider, registry)
}

fn names(defs: &[ToolDefinition]) -> Vec<&str> {
    defs.iter().map(|d| d.name.as_str()).collect()
}

fn model_calls(agent: &mut Agent, tool: &str, input: serde_json::Value) {
    agent.add_message(
        Role::Assistant,
        vec![ContentBlock::ToolUse {
            id: format!("call-{tool}"),
            name: tool.to_string(),
            input,
            thought_signature: None,
        }],
    );
}

#[tokio::test]
async fn inline_tool_schemas_stay_within_budget() {
    let _guard = crate::storage::lock_test_env();
    let mut agent = factr_agent().await;
    let defs = agent.tool_definitions().await;
    let total = ToolDefinition::aggregate_prompt_token_estimate(&defs);
    eprintln!("inline schema tokens: {total} across {} tools", defs.len());
    assert!(total <= INLINE_SCHEMA_BUDGET, "{total} tokens > {INLINE_SCHEMA_BUDGET}: {:?}", names(&defs));
    for core in ["read", "write", "edit", "apply_patch", "bash", "agentgrep", "load_tools"] {
        assert!(names(&defs).contains(&core), "{core} must stay inline");
    }
    assert!(!names(&defs).contains(&"todo"));
}

#[tokio::test]
async fn load_tools_output_names_loaded_and_rejects_unknown() {
    let _guard = crate::storage::lock_test_env();
    let agent = factr_agent().await;
    let out = agent
        .execute_tool("load_tools", serde_json::json!({"names": ["todo", "bash", "nope"]}))
        .await
        .unwrap();
    assert!(out.output.contains("Loaded: todo.") && out.output.contains("Not loadable: bash, nope."), "{}", out.output);
}

#[tokio::test]
async fn loaded_tools_become_callable_and_persist_across_turns() {
    let _guard = crate::storage::lock_test_env();
    let mut agent = factr_agent().await;
    let first = agent.tool_definitions().await;
    assert!(!names(&first).contains(&"todo") && !names(&first).contains(&"bg"));
    // No new load: the locked list is reused untouched.
    assert_eq!(names(&first), names(&agent.tool_definitions().await));

    model_calls(&mut agent, "load_tools", serde_json::json!({"names": ["todo"]}));
    let second = agent.tool_definitions().await;
    assert!(names(&second).contains(&"todo") && !names(&second).contains(&"bg"));
    assert!(names(&second).contains(&"load_tools"), "bg still deferred");

    // Later turns (more history, nothing new loaded) keep it, and it is stable.
    agent.add_message(Role::User, vec![ContentBlock::Text { text: "next".into(), cache_control: None }]);
    assert_eq!(names(&second), names(&agent.tool_definitions().await));

    // Calling a deferred tool directly also loads it from then on.
    model_calls(&mut agent, "bg", serde_json::json!({"action": "list"}));
    let third = agent.tool_definitions().await;
    assert!(names(&third).contains(&"todo") && names(&third).contains(&"bg"));
}

#[tokio::test]
async fn loaded_set_is_rebuilt_from_the_transcript() {
    let _guard = crate::storage::lock_test_env();
    let mut agent = factr_agent().await;
    model_calls(&mut agent, "load_tools", serde_json::json!({"names": ["browser", "webfetch"]}));
    let defs = agent.tool_definitions().await;
    assert!(names(&defs).contains(&"browser") && names(&defs).contains(&"webfetch"));
    agent.locked_tools = None;
    agent.locked_deferred.clear();
    assert!(names(&agent.tool_definitions().await).contains(&"browser"));
}

#[tokio::test]
async fn explicit_allowlist_is_never_deferred() {
    let _guard = crate::storage::lock_test_env();
    let mut agent = factr_agent().await;
    agent.allowed_tools = Some(["todo".to_string(), "read".to_string()].into_iter().collect());
    let defs = agent.tool_definitions().await;
    assert_eq!(names(&defs), ["read", "todo"]);
}

#[tokio::test]
async fn repl_is_deferred_until_loaded() {
    // The REPL is only registered where the macOS sandbox exists.
    if !factr_learn::host::sandbox_available() {
        return;
    }
    let _guard = crate::storage::lock_test_env();
    let mut agent = factr_agent().await;
    assert!(!names(&agent.tool_definitions().await).contains(&"repl"));
    let out = agent
        .execute_tool("load_tools", serde_json::json!({"names": ["repl"]}))
        .await
        .unwrap();
    assert!(out.output.contains("Loaded: repl."), "{}", out.output);
    model_calls(&mut agent, "load_tools", serde_json::json!({"names": ["repl"]}));
    assert!(names(&agent.tool_definitions().await).contains(&"repl"));
}

/// Points `FACTR_CONFIG_HOME` at a config with (or without) `tools.routing` until dropped.
struct RoutingConfig {
    dir: std::path::PathBuf,
}

impl RoutingConfig {
    fn new(on: bool) -> Self {
        let dir = std::env::temp_dir().join(format!("routing-{}-{}", std::process::id(), rand::random::<u64>()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("config.yaml"), if on { "tools: {routing: true}\n" } else { "" }).unwrap();
        crate::env::set_var("FACTR_CONFIG_HOME", &dir);
        Self { dir }
    }
}

impl Drop for RoutingConfig {
    fn drop(&mut self) {
        crate::env::remove_var("FACTR_CONFIG_HOME");
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn user_says(agent: &mut Agent, text: &str) {
    agent.add_message(Role::User, vec![ContentBlock::Text { text: text.into(), cache_control: None }]);
}

const ASKING: &str = "please read https://example.com and search the web for rust news";

#[tokio::test]
async fn routing_off_leaves_the_tool_list_byte_identical() {
    let _guard = crate::storage::lock_test_env();
    let _config = RoutingConfig::new(false);
    let mut plain = factr_agent().await;
    let mut asking = factr_agent().await;
    user_says(&mut asking, ASKING);
    let left = serde_json::to_vec(&plain.tool_definitions().await).unwrap();
    let defs = asking.tool_definitions().await;
    assert_eq!(left, serde_json::to_vec(&defs).unwrap());
    assert!(!names(&defs).contains(&"webfetch") && !names(&defs).contains(&"websearch"));
}

#[tokio::test]
async fn routing_on_adds_the_asked_for_tools_and_only_adds() {
    let _guard = crate::storage::lock_test_env();
    let _config = RoutingConfig::new(true);
    let mut plain = factr_agent().await;
    let mut asking = factr_agent().await;
    let base = plain.tool_definitions().await;
    user_says(&mut asking, ASKING);
    let routed = asking.tool_definitions().await;
    assert!(names(&routed).contains(&"webfetch") && names(&routed).contains(&"websearch"));
    // Everything the plain list had is still there, in registry (sorted) order, byte for byte.
    for def in &base {
        let same = routed.iter().find(|d| d.name == def.name).expect("a tool was removed by routing");
        assert_eq!(serde_json::to_vec(def).unwrap(), serde_json::to_vec(same).unwrap(), "{}", def.name);
    }
    let sorted: Vec<&str> = names(&routed);
    let mut expected = sorted.clone();
    expected.sort_unstable();
    assert_eq!(sorted, expected);
    // A later neutral turn keeps the same locked list, and a rebuild from the transcript reproduces it.
    user_says(&mut asking, "thanks");
    assert_eq!(names(&routed), names(&asking.tool_definitions().await));
    asking.locked_tools = None;
    asking.locked_deferred.clear();
    assert_eq!(names(&routed), names(&asking.tool_definitions().await));
}

#[tokio::test]
async fn routing_on_with_neutral_text_changes_nothing_and_stays_in_budget() {
    let _guard = crate::storage::lock_test_env();
    let _config = RoutingConfig::new(true);
    let mut plain = factr_agent().await;
    let mut neutral = factr_agent().await;
    user_says(&mut neutral, "fix the failing test in src/lib.rs");
    let left = serde_json::to_vec(&plain.tool_definitions().await).unwrap();
    let defs = neutral.tool_definitions().await;
    assert_eq!(left, serde_json::to_vec(&defs).unwrap());
    assert!(ToolDefinition::aggregate_prompt_token_estimate(&defs) <= INLINE_SCHEMA_BUDGET);
}
