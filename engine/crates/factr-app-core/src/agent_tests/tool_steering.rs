//! L5 on the real tool list: off is the registry's descriptions, on adds the sentences after the
//! allowlist and SDK overrides, naming only tools in the final list.

use super::*;
use crate::protocol::{SessionToolConfig, SessionToolDefinition};

async fn descriptions(steer: bool, allowed: Option<&[&str]>, sdk_write: bool) -> Vec<(String, String)> {
    let _lock = crate::storage::lock_test_env();
    let provider: Arc<dyn Provider> = Arc::new(RefusalScriptProvider { script: Default::default(), calls: Default::default() });
    let registry = Registry::new(provider.clone()).await;
    let mut agent = Agent::new(provider, registry);
    agent.loop_shape.tool_steering = steer;
    agent.allowed_tools = allowed.map(|a| a.iter().map(|s| s.to_string()).collect());
    let session = agent.session.id.clone();
    if sdk_write {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let custom = SessionToolDefinition { name: "write".into(), description: "Client write.".into(), parameters: serde_json::json!({"type": "object"}) };
        crate::tool::sdk::configure(&session, "owner", SessionToolConfig { enabled: None, disabled: vec![], custom: vec![custom] }, tx).unwrap();
    }
    let out = agent.tool_definitions_for_debug().await.into_iter().map(|t| (t.name, t.description)).collect();
    crate::tool::sdk::remove_session(&session);
    out
}

fn of<'a>(defs: &'a [(String, String)], name: &str) -> Option<&'a str> {
    defs.iter().find(|(n, _)| n == name).map(|(_, d)| d.as_str())
}

#[tokio::test]
async fn steering_sentences_follow_the_final_tool_list() {
    let off = descriptions(false, None, false).await;
    let on = descriptions(true, None, false).await;
    assert_eq!(off.len(), on.len());
    for ((name, plain), (_, steered)) in off.iter().zip(&on) {
        assert!(steered.starts_with(plain.trim_end()), "{name}: steering only appends");
        if !["bash", "read", "agentgrep", "write"].contains(&name.as_str()) {
            assert_eq!(plain, steered, "{name}");
        }
    }
    let bash = of(&on, "bash").unwrap();
    assert!(bash.contains("instead of cat/head/tail"), "{bash}");
    assert!(of(&on, "write").unwrap().contains("just to confirm the save"));

    let restricted = descriptions(true, Some(&["bash", "read"]), false).await;
    assert_eq!(
        of(&restricted, "bash").unwrap().split_once(". Use ").map(|(_, s)| s),
        Some("read for reading and searching files, instead of cat/head/tail."),
    );

    let sdk = descriptions(true, None, true).await;
    assert_eq!(of(&sdk, "write"), Some("Client write."), "a client-supplied definition is left as given");
}
