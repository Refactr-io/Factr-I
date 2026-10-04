//! Compact subagent delegation (spawn / message / read / list / stop / status) via swarm internals.

use super::communicate::CommunicateTool;
use super::{Tool, ToolContext, ToolOutput};
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::{Value, json};

pub struct DelegateTool {
    inner: CommunicateTool,
}

impl DelegateTool {
    pub fn new() -> Self {
        Self {
            inner: CommunicateTool::new(),
        }
    }
}

#[async_trait]
impl Tool for DelegateTool {
    fn name(&self) -> &str {
        "delegate"
    }

    fn description(&self) -> &str {
        "Spawn or manage child agents: spawn (optional spec: name prefix), message, read (a child's recent transcript), list, stop, status."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["action"],
            "properties": {
                "intent": super::intent_schema_property(),
                "action": { "type": "string", "enum": ["spawn", "message", "read", "list", "stop", "status"] },
                "label": { "type": "string", "description": "Short chip label for spawn" },
                "prompt": { "type": "string", "description": "Task for spawn or body for message" },
                "target_session": { "type": "string", "description": "Child session or label for message/read/stop" },
                "to_session": { "type": "string", "description": "Alias of target_session" },
                "model": { "type": "string" },
                "working_dir": { "type": "string" }
            }
        })
    }

    async fn execute(&self, input: Value, ctx: ToolContext) -> Result<ToolOutput> {
        let action = input["action"].as_str().context("action is required")?;
        let mut mapped = input.clone();
        mapped["action"] = json!(match action {
            "message" => "message",
            "read" => "read_context",
            "list" => "list",
            "stop" => "stop",
            "status" => "status",
            "spawn" => "spawn",
            other => other,
        });
        if action == "spawn" {
            let mut prompt = input["prompt"].as_str().unwrap_or("").to_string();
            if let Some(rest) = prompt.strip_prefix("spec:").map(str::trim) {
                let name = rest.split_whitespace().next().unwrap_or_default();
                if !name.is_empty() {
                    let home = factr_base::storage::factr_dir()?;
                    let store = factr_learn::entries::EntryStore::open_cached(&home)?;
                    if let Some(spec) = store.resolve_subagent_spec(&ctx.session_id, name)? {
                        prompt =
                            format!("{}\n\n{}", spec.content.trim(), rest[name.len()..].trim());
                    }
                }
            }
            if mapped
                .get("label")
                .and_then(Value::as_str)
                .is_none_or(str::is_empty)
            {
                mapped["label"] = json!("delegate");
            }
            mapped["prompt"] = json!(prompt);
            apply_delegation(&mut mapped, &factr_base::factr_config::current().delegation);
        }
        if action == "message" {
            let target = input["target_session"]
                .as_str()
                .or(input["to_session"].as_str());
            if let Some(t) = target {
                mapped["to_session"] = json!(t);
            }
            if let Some(body) = input["prompt"].as_str() {
                mapped["message"] = json!(body);
            }
        }
        if action == "read" {
            let target = input["target_session"].as_str().or(input["to_session"].as_str()).filter(|t| !t.is_empty());
            let Some(target) = target else { anyhow::bail!("target_session is required for read") };
            mapped["target_session"] = json!(target);
        }
        let output = self.inner.execute(mapped, ctx).await?;
        if action == "spawn"
            && let Some(cap) = factr_base::factr_config::current().delegation.max_iterations
            && let Some(child) = spawned_session(&output.output)
        {
            factr_base::factr_config::set_iteration_cap(child, cap);
        }
        Ok(output)
    }
}

/// Factr `delegation.model` / `provider` / `reasoning_effort` as the spawn's defaults: a value the
/// caller passed wins, and nothing is added when the setting is unset (the child inherits the parent).
fn apply_delegation(spawn: &mut Value, delegation: &factr_base::factr_config::Delegation) {
    let unset = |spawn: &Value, key: &str| spawn.get(key).and_then(Value::as_str).is_none_or(str::is_empty);
    if unset(spawn, "model")
        && let Some(spec) = delegation.model_spec()
    {
        spawn["model"] = json!(spec);
    }
    if unset(spawn, "effort")
        && let Some(effort) = delegation.reasoning_effort.as_deref()
    {
        spawn["effort"] = json!(effort);
    }
}

/// The child's session id from the swarm tool's "Spawned new agent: <id>" reply.
fn spawned_session(output: &str) -> Option<&str> {
    output.strip_prefix("Spawned new agent: ").map(str::trim).filter(|id| !id.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use factr_base::factr_config::FactrConfig;

    #[test]
    fn delegation_settings_default_the_spawn_and_never_override_the_caller() {
        let cfg = FactrConfig::parse("delegation: {model: cheap, provider: openrouter, reasoning_effort: low}\n").delegation;
        let mut spawn = json!({ "action": "spawn", "prompt": "x" });
        apply_delegation(&mut spawn, &cfg);
        assert_eq!((spawn["model"].as_str(), spawn["effort"].as_str()), (Some("openrouter:cheap"), Some("low")));
        let mut explicit = json!({ "model": "mine", "effort": "high" });
        apply_delegation(&mut explicit, &cfg);
        assert_eq!((explicit["model"].as_str(), explicit["effort"].as_str()), (Some("mine"), Some("high")));
    }

    #[test]
    fn unset_delegation_leaves_the_spawn_untouched() {
        let mut spawn = json!({ "action": "spawn", "prompt": "x" });
        apply_delegation(&mut spawn, &FactrConfig::parse("delegation: {model: '', provider: auto}").delegation);
        assert_eq!(spawn, json!({ "action": "spawn", "prompt": "x" }));
    }

    #[test]
    fn the_child_id_is_read_from_the_spawn_reply() {
        assert_eq!(spawned_session("Spawned new agent: sess-12\n"), Some("sess-12"));
        assert_eq!(spawned_session("Failed"), None);
    }
}
