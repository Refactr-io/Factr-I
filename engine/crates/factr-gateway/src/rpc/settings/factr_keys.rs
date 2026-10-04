//! Factr config keys the engine acts on (`compression.*`, `auxiliary.<task>.*`, `fallback_providers`,
//! `delegation.*`, `checkpoints.*`, `hooks`): validated here, stored in `config.yaml` through the settings layer's
//! writer, read back by `factr_base::factr_config` on the next turn. One store; this is only the
//! typed front door, so a Settings screen `config.set` is checked and takes effect with no restart.

use super::{RpcError, bad, read, truth, word, write};
use serde_json::{Value, json};
use std::path::Path;

enum Kind {
    Bool(bool),
    Float(f64, f64),
    Int(i64, i64),
    Text,
    /// One of a fixed set of words (`""` clears).
    Words(&'static [&'static str]),
    /// An ordered list of `{provider, model}` entries.
    Providers,
    /// The `hooks:` block: `{event: [{matcher?, command, timeout?, fail_closed?}]}`.
    Hooks,
}

fn kind(key: &str) -> Option<Kind> {
    Some(match key {
        "checkpoints.enabled" => Kind::Bool(true),
        "checkpoints.max_snapshots" => Kind::Int(1, 100_000),
        "checkpoints.max_total_size_mb" | "checkpoints.max_file_size_mb" => Kind::Int(0, 10_000_000),
        "compression.enabled" => Kind::Bool(true),
        "compression.threshold" => Kind::Float(0.10, 1.0),
        "compression.target_ratio" => Kind::Float(0.05, 0.80),
        "compression.protect_last_n" => Kind::Int(0, 500),
        "fallback_providers" => Kind::Providers,
        "hooks" => Kind::Hooks,
        "hooks_auto_accept" => Kind::Bool(false),
        "delegation.model" | "delegation.provider" => Kind::Text,
        "delegation.max_iterations" => Kind::Int(1, 1000),
        "delegation.reasoning_effort" => Kind::Words(&["", "none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra"]),
        _ => {
            // Only the slots the engine itself consumes (the table in factr_base::factr_config);
            // Factr's other slots (vision, approval, ...) stay Python's to answer.
            let (task, field) = key.strip_prefix("auxiliary.")?.split_once('.')?;
            factr_base::factr_config::AUX_MAP.iter().find(|(t, _)| *t == task)?;
            matches!(field, "provider" | "model").then_some(Kind::Text)?
        }
    })
}

/// Whether `key` is one of the keys this module validates.
pub(super) fn known(key: &str) -> bool {
    kind(key).is_some()
}

/// `None`: not one of these keys.
pub(super) fn get(home: &Path, key: &str) -> Option<Result<Value, RpcError>> {
    let kind = kind(key)?;
    let stored = read(home, key);
    let engine_default = || {
        let c = factr_base::checkpoint_store::Config::default();
        match key {
            "checkpoints.max_snapshots" => Some(json!(c.max_snapshots)),
            "checkpoints.max_total_size_mb" => Some(json!(c.max_total_size_mb)),
            "checkpoints.max_file_size_mb" => Some(json!(c.max_file_size_mb)),
            _ => None,
        }
    };
    let value = match (&kind, stored) {
        (Kind::Int(..), None) if engine_default().is_some() => engine_default().unwrap_or(Value::Null),
        (Kind::Bool(default), None) => json!(default),
        (Kind::Providers, None) => json!([]),
        (Kind::Hooks, None) => json!({}),
        (_, stored) => stored.unwrap_or(Value::Null),
    };
    Some(Ok(json!({ "value": value })))
}

/// `None`: not one of these keys.
pub(super) fn set(home: &Path, key: &str, value: &Value) -> Option<Result<Value, RpcError>> {
    if key == "hooks" {
        return Some(set_hooks(home, value));
    }
    let parsed = normalize(key, value)?;
    Some(parsed.and_then(|v| write(home, key, &v).map(|_| json!({ "value": v }))))
}

/// The validated, coerced value `set` would store for `key`; nothing is written. `None`: not one of
/// these keys, or `hooks` (which validates and records approvals as it saves).
pub(super) fn normalize(key: &str, value: &Value) -> Option<Result<Value, RpcError>> {
    let kind = kind(key)?;
    let number = || value.as_f64().or_else(|| value.as_str().and_then(|s| s.trim().parse().ok()));
    Some(match kind {
        Kind::Bool(_) => truth(&word(value)).map(|b| json!(b)).ok_or_else(|| bad(format!("{key} takes true or false"))),
        Kind::Float(lo, hi) => number()
            .filter(|n| n.is_finite() && (lo..=hi).contains(n))
            .map(|n| json!(n))
            .ok_or_else(|| bad(format!("{key} takes a number from {lo} to {hi}"))),
        Kind::Int(lo, hi) => number()
            .filter(|n| n.fract() == 0.0 && (lo as f64..=hi as f64).contains(n))
            .map(|n| json!(n as i64))
            .ok_or_else(|| bad(format!("{key} takes a whole number from {lo} to {hi}"))),
        Kind::Words(allowed) => {
            let w = word(value);
            allowed.contains(&w.as_str()).then(|| json!(w)).ok_or_else(|| bad(format!("{key} takes one of: {}", allowed.join(", "))))
        }
        Kind::Providers => providers(value).map(Value::Array),
        Kind::Hooks => return None,
        Kind::Text => match value {
            Value::String(s) => Ok(json!(s.trim())),
            Value::Null => Ok(json!("")),
            _ => Err(bad(format!("{key} takes text"))),
        },
    })
}

/// `[{provider, model}]` (or `"provider:model"` strings) as a normalized list; an entry needs both.
fn providers(value: &Value) -> Result<Vec<Value>, RpcError> {
    let list = match value {
        Value::Null => return Ok(Vec::new()),
        Value::Array(list) => list,
        _ => return Err(bad("fallback_providers takes a list of {provider, model} entries".into())),
    };
    list.iter()
        .map(|entry| {
            let (provider, model) = match entry {
                Value::String(s) => s.split_once(':').map(|(p, m)| (p.to_string(), m.to_string())).unwrap_or_default(),
                other => (other["provider"].as_str().unwrap_or_default().to_string(), other["model"].as_str().unwrap_or_default().to_string()),
            };
            let (provider, model) = (provider.trim().to_string(), model.trim().to_string());
            if provider.is_empty() || provider.eq_ignore_ascii_case("auto") || model.is_empty() {
                return Err(bad(format!("a fallback needs a provider and a model: {entry}")));
            }
            Ok(json!({ "provider": provider, "model": model }))
        })
        .collect()
}

/// Factr sub-sections of `hooks:` that are not events; a save of the events keeps them.
const RESERVED: &[&str] = &["output_spill", "outbound"];

/// A validated `hooks:` block, not yet written: the `(event, command)` pairs to approve and the block
/// as posted (the reserved sub-sections already saved are merged in by [`Hooks::install`]).
pub(super) struct Hooks {
    block: serde_json::Map<String, Value>,
    pairs: Vec<(String, String)>,
}

impl Hooks {
    /// Validate `value` (`{event: [{command, ...}]}`); nothing is written.
    pub(super) fn parse(value: &Value) -> Result<Self, RpcError> {
        let events = match value {
            Value::Null => &serde_json::Map::new(),
            Value::Object(events) => events,
            _ => return Err(bad("hooks takes {event: [{command, ...}]}".into())),
        };
        let mut block = serde_json::Map::new();
        let mut pairs = Vec::new();
        for (event, entries) in events {
            if RESERVED.contains(&event.as_str()) {
                block.insert(event.clone(), entries.clone());
                continue;
            }
            let list = match entries {
                Value::Null => continue,
                Value::Array(list) => list,
                _ => return Err(bad(format!("hooks.{event} takes a list of hook entries"))),
            };
            let mut kept = Vec::new();
            for entry in list {
                let command = entry["command"].as_str().map(str::trim).filter(|c| !c.is_empty());
                let Some(command) = command else { return Err(bad(format!("a hooks.{event} entry needs a command"))) };
                let mut out = serde_json::Map::new();
                out.insert("command".into(), json!(command));
                if let Some(m) = entry["matcher"].as_str().map(str::trim).filter(|m| !m.is_empty()) {
                    out.insert("matcher".into(), json!(m));
                }
                if !entry["timeout"].is_null() {
                    let t = entry["timeout"].as_i64().filter(|t| (1..=300).contains(t)).ok_or_else(|| bad(format!("hooks.{event} timeout takes 1 to 300 seconds")))?;
                    out.insert("timeout".into(), json!(t));
                }
                if !entry["fail_closed"].is_null() {
                    let b = entry["fail_closed"].as_bool().ok_or_else(|| bad(format!("hooks.{event} fail_closed takes true or false")))?;
                    out.insert("fail_closed".into(), json!(b));
                }
                pairs.push((event.clone(), command.to_string()));
                kept.push(Value::Object(out));
            }
            block.insert(event.clone(), Value::Array(kept));
        }
        Ok(Self { block, pairs })
    }

    /// Set `hooks:` in the parsed config, keeping the reserved sub-sections already there unless this
    /// save carries its own. Returns the block as stored.
    pub(super) fn install(&self, root: &mut serde_yaml::Value) -> Value {
        let mut block = self.block.clone();
        if let Some(old) = root.get("hooks").and_then(|h| serde_json::to_value(h).ok()).and_then(|h| h.as_object().cloned()) {
            for key in RESERVED {
                if !block.contains_key(*key) && let Some(v) = old.get(*key) {
                    block.insert((*key).into(), v.clone());
                }
            }
        }
        let block = Value::Object(block);
        if let Ok(yaml) = serde_yaml::to_value(&block) {
            super::set_path(root, "hooks", yaml);
        }
        block
    }

    /// After the block is saved: approve each hook in Factr's allowlist and report the events the
    /// engine cannot run.
    pub(super) fn finish(&self, home: &Path, block: Value) -> Result<Value, RpcError> {
        // The checker (`factr_hooks`) reads the allowlist from the Factr config dir, not the engine's.
        let factr_dir = super::factr_dir(home);
        for (event, command) in &self.pairs {
            factr_base::factr_hooks::record_approval(&factr_dir, event, command)
                .map_err(|e| RpcError { code: 5000, message: format!("cannot record the hook approval: {e}"), data: None })?;
        }
        let unsupported: Vec<&String> = block.as_object().into_iter().flatten().map(|(k, _)| k).filter(|k| !RESERVED.contains(&k.as_str()) && factr_base::factr_hooks::engine_event(k).is_none()).collect();
        Ok(json!({ "value": block, "unsupported": unsupported }))
    }
}

/// Validate and save the `hooks:` block. Saving it through the authenticated Settings call is the
/// owner's consent: each `(event, command)` pair goes in Factr's allowlist, so it runs without the
/// first-use prompt a headless engine cannot show. Events the engine has no counterpart for are kept
/// (the Python backend runs those) and reported back as `unsupported`.
fn set_hooks(home: &Path, value: &Value) -> Result<Value, RpcError> {
    let hooks = Hooks::parse(value)?;
    let stored = std::cell::RefCell::new(Value::Null);
    super::update(home, |root| *stored.borrow_mut() = hooks.install(root))?;
    hooks.finish(home, stored.into_inner())
}
