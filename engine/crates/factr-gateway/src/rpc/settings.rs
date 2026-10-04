//! `config.get` / `config.set` for every key the desktop reads or writes through the gateway, owned by
//! the engine so a set reads back, survives a restart and needs no Python backend.
//!
//! Keys Factr also reads live in `$FACTR_CONFIG_HOME/config.yaml` (the one file the engine's own readers, the
//! approval hook, and Python share); per-session flags (`yolo`, `fast`) and keys nobody knows live
//! in `factr.db` (`engine_settings`). `checkpoints.*` are `config.yaml` keys too: the desktop's own
//! Settings save writes them there and the checkpoint store reads them from there.

mod factr_keys;

use super::RpcError;
use serde_json::{Value, json};
use factr_learn::entries::EntryStore;
use std::path::Path;

const APPROVAL: &[&str] = &["manual", "smart", "off"];
const EFFORTS: &[&str] = &["none", "minimal", "low", "medium", "high", "xhigh"];

fn bad(message: String) -> RpcError {
    RpcError { code: 4002, message, data: None }
}

fn word(v: &Value) -> String {
    match v {
        Value::String(s) => s.trim().to_ascii_lowercase(),
        Value::Null => String::new(),
        other => other.to_string().to_ascii_lowercase(),
    }
}

fn truth(w: &str) -> Option<bool> {
    match w {
        "1" | "true" | "on" | "yes" | "enable" | "enabled" => Some(true),
        "0" | "false" | "off" | "no" | "disable" | "disabled" => Some(false),
        _ => None,
    }
}

/// The Factr config dir (`$FACTR_CONFIG_HOME`, set by the entrypoint): `config.yaml`, `.env` and the hook
/// allowlist live here, never in the engine dir (`home`, which owns `factr.db`). `home` is only
/// the stand-in when the variable is unset (unit tests).
fn factr_dir(home: &Path) -> std::path::PathBuf {
    factr_base::factr_config::home().unwrap_or_else(|| home.to_path_buf())
}

fn config_file(home: &Path) -> std::path::PathBuf {
    factr_dir(home).join("config.yaml")
}

/// config.yaml as a mapping for a READ: a missing, empty or unparseable file reads as empty.
fn load(home: &Path) -> serde_yaml::Value {
    try_load(home).unwrap_or_else(|_| serde_yaml::Value::Mapping(Default::default()))
}

/// config.yaml as a mapping for a WRITE. A missing or blank file is an empty mapping; a file that
/// exists with content but does not parse as a mapping is an error, so the next save cannot wipe
/// what the user hand-edited (Factr's `require_readable_config_before_write` refuses the same way).
fn try_load(home: &Path) -> Result<serde_yaml::Value, String> {
    let file = config_file(home);
    let raw = match std::fs::read_to_string(&file) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(serde_yaml::Value::Mapping(Default::default())),
        Err(e) => return Err(format!("{} cannot be read: {e}", file.display())),
    };
    if raw.trim().is_empty() {
        return Ok(serde_yaml::Value::Mapping(Default::default()));
    }
    match serde_yaml::from_str::<serde_yaml::Value>(&raw) {
        Ok(value) if value.is_mapping() => Ok(value),
        Ok(_) => Err(format!("{} is not a YAML mapping; fix or remove it, the engine will not overwrite it", file.display())),
        Err(e) => Err(format!("{} does not parse ({e}); fix or remove it, the engine will not overwrite it", file.display())),
    }
}

/// The yaml value at a dotted path as JSON (`None` when absent).
pub(super) fn read(home: &Path, path: &str) -> Option<Value> {
    let mut node = load(home);
    for part in path.split('.') {
        node = node.get(part)?.clone();
    }
    serde_json::to_value(node).ok().filter(|v| !v.is_null())
}

/// Set a dotted yaml path and write the file atomically, merging with a concurrent save.
pub(super) fn write(home: &Path, path: &str, value: &Value) -> Result<(), RpcError> {
    let leaf = serde_yaml::to_value(value).map_err(|e| RpcError { code: 5000, message: format!("cannot write config.yaml: {e}"), data: None })?;
    update(home, |root| set_path(root, path, leaf.clone()))
}

/// A bare-string `model: name` becomes `{default: name}` so a sub-key can sit beside it without
/// dropping the name.
fn model_as_block(root: &mut serde_yaml::Value) {
    if let Some(name) = root.get("model").and_then(|m| m.as_str()).map(str::to_string) {
        set_path(root, "model", serde_yaml::Value::Mapping(Default::default()));
        if !name.trim().is_empty() {
            set_path(root, "model.default", serde_yaml::Value::from(name));
        }
    }
}

/// Put `leaf` at the dotted `path` of a yaml mapping, creating (or replacing a scalar with) the
/// mappings on the way.
fn set_path(root: &mut serde_yaml::Value, path: &str, leaf: serde_yaml::Value) {
    let parts: Vec<&str> = path.split('.').collect();
    let mut node = root;
    for part in &parts[..parts.len() - 1] {
        let key = serde_yaml::Value::from(*part);
        let map = node.as_mapping_mut().expect("mapping");
        if !map.get(&key).is_some_and(serde_yaml::Value::is_mapping) {
            map.insert(key.clone(), serde_yaml::Value::Mapping(Default::default()));
        }
        node = map.get_mut(&key).expect("just inserted");
    }
    node.as_mapping_mut().expect("mapping").insert(serde_yaml::Value::from(parts[parts.len() - 1]), leaf);
}

/// The leaves of a posted config object as `(dotted path, value)`. Arrays and scalars are leaves; the
/// `hooks` block is one leaf (it is validated and saved as a whole); empty objects add nothing.
fn leaves(prefix: &str, node: &serde_json::Map<String, Value>, out: &mut Vec<(String, Value)>) {
    for (key, value) in node {
        let path = if prefix.is_empty() { key.clone() } else { format!("{prefix}.{key}") };
        match value {
            Value::Object(inner) if path != "hooks" => leaves(&path, inner, out),
            _ => out.push((path, value.clone())),
        }
    }
}

/// Whether a config key name is a credential slot (`api_key`, `OPENAI_API_KEY`, `token`, `client_secret`,
/// `password`, ...): those live in `.env` (`/api/env`), never in `config.yaml`. A name that only says where
/// a credential is kept (`service_account_token_env` names an environment variable, `password_store` a
/// keychain backend) is not a slot.
fn is_secret_name(segment: &str) -> bool {
    let s = segment.to_ascii_lowercase();
    if s.ends_with("_env") || s.ends_with("_store") {
        return false;
    }
    s.contains("api_key") || s.contains("apikey") || s.split(['_', '-']).any(|w| matches!(w, "token" | "secret" | "password" | "passwd"))
}

/// Whether `value` holds any text: a flag (`show_token_analytics`), a number (`min_secret_chars`), an
/// empty string or null (a blank slot, which is how Factr lists a credential field by default) is not a
/// credential, only a non-empty string anywhere inside is.
fn has_text(value: &Value) -> bool {
    match value {
        Value::String(s) => !s.trim().is_empty(),
        Value::Object(map) => map.values().any(has_text),
        Value::Array(list) => list.iter().any(has_text),
        _ => false,
    }
}

/// The credential-named key at `path` (its last segment) or inside a list/object `value` under it, when
/// it is given a value to hold.
fn secret_in(path: &str, value: &Value) -> Option<String> {
    if path.rsplit('.').next().is_some_and(is_secret_name) && has_text(value) {
        return Some(path.to_string());
    }
    match value {
        Value::Object(map) => map.iter().find_map(|(k, v)| secret_in(&format!("{path}.{k}"), v)),
        Value::Array(list) => list.iter().find_map(|v| secret_in(path, v)),
        _ => None,
    }
}

/// Keys this engine validates as one value; a deeper path under one would replace the checked scalar
/// with a mapping. (`model` is the exception: its scalar-or-block form is merged by `put_config`.)
fn is_validated(key: &str) -> bool {
    matches!(key, "model_context_length" | "approvals.mode" | "agent.reasoning_effort") || factr_keys::known(key)
}

/// The validated key that is a strict prefix of `path`, if any.
fn validated_prefix(path: &str) -> Option<String> {
    let parts: Vec<&str> = path.split('.').collect();
    (1..parts.len()).map(|n| parts[..n].join(".")).find(|prefix| is_validated(prefix))
}

/// `PUT /api/config`: deep-merge the posted config object into `$FACTR_CONFIG_HOME/config.yaml`, the way
/// Factr does (keys not posted, known or not, stay as they are). Every value is checked first, so a
/// bad one rejects the whole save and writes nothing. The desktop's flat view shows `model` as the
/// model name and `model_context_length` beside it; both map back onto the `model:` block.
pub(crate) fn put_config(home: &Path, config: &Value) -> Result<(), RpcError> {
    let Value::Object(posted) = config else { return Err(bad("config must be an object".into())) };
    migrate_checkpoint_config(home);
    let mut found = Vec::new();
    leaves("", posted, &mut found);
    let mut plain = Vec::new();
    let (mut hooks, mut model, mut context) = (None, None, None);
    for (path, value) in found {
        if path.split('.').any(|seg| seg.starts_with('_')) {
            continue; // `_model_meta` and kin: view-only keys the GET never stores
        }
        if path != "hooks"
            && let Some(secret) = secret_in(&path, &value)
        {
            return Err(bad(format!("{secret} is a credential: use /api/env, config.yaml never holds one")));
        }
        if let Some(prefix) = validated_prefix(&path) {
            return Err(bad(format!("{prefix} takes a single value, not an object (got {path})")));
        }
        match path.as_str() {
            "hooks" => hooks = Some(value),
            "model" if value.is_string() => model = value.as_str().map(|m| m.trim().to_string()).filter(|m| !m.is_empty()),
            "model" => return Err(bad("model takes a model name".into())),
            "model_context_length" => {
                context = Some(value.as_i64().filter(|n| *n >= 0).ok_or_else(|| bad("model_context_length takes a whole number, 0 for automatic".into()))?)
            }
            "approvals.mode" => {
                let w = word(&value);
                if w == "smart" {
                    return Err(bad("approvals.mode smart is not supported by this engine (it has no guardian-model review step): use manual or off".into()));
                }
                if !["manual", "off"].contains(&w.as_str()) {
                    return Err(bad(format!("unknown approvals.mode value: {value}; pick one of manual|off")));
                }
                plain.push((path, json!(w)));
            }
            "agent.reasoning_effort" => {
                let w = word(&value);
                let ok = w.is_empty()
                    || EFFORTS.contains(&w.as_str())
                    || factr_provider_core::canonical_reasoning_effort(&w).is_some()
                    || factr_base::prompt::is_swarm_effort(&w);
                if !ok {
                    return Err(bad(format!("unknown agent.reasoning_effort value: {value}")));
                }
                plain.push((path, json!(w)));
            }
            _ => match factr_keys::normalize(&path, &value) {
                Some(done) => plain.push((path, done?)),
                None => plain.push((path, value)),
            },
        }
    }
    let hooks = hooks.as_ref().map(factr_keys::Hooks::parse).transpose()?;
    if plain.is_empty() && model.is_none() && context.is_none() && hooks.is_none() {
        return Ok(());
    }
    let plain: Vec<(String, serde_yaml::Value)> = plain
        .into_iter()
        .map(|(path, v)| serde_yaml::to_value(v).map(|y| (path, y)).map_err(|e| RpcError { code: 5000, message: format!("cannot write config.yaml: {e}"), data: None }))
        .collect::<Result<_, _>>()?;
    // Everything above only validated; this is the one write (hooks, plain keys and the model block together).
    let stored_hooks = std::cell::RefCell::new(Value::Null);
    update(home, |root| {
        if let Some(hooks) = &hooks {
            *stored_hooks.borrow_mut() = hooks.install(root);
        }
        for (path, value) in &plain {
            if path.starts_with("model.") {
                model_as_block(root);
            }
            set_path(root, path, value.clone());
        }
        if model.is_some() || context.is_some() {
            let key = serde_yaml::Value::from("model");
            let map = root.as_mapping_mut().expect("mapping");
            // A bare-string `model:` becomes `{default: <name>}` only when a sub-key must live beside it.
            let current = map.get(&key).cloned();
            if let Some(name) = &model {
                match current.as_ref().filter(|c| c.is_mapping()) {
                    Some(_) => set_path(root, "model.default", serde_yaml::Value::from(name.as_str())),
                    None => {
                        root.as_mapping_mut().expect("mapping").insert(key.clone(), serde_yaml::Value::from(name.as_str()));
                    }
                }
            }
            if let Some(n) = context {
                if n > 0 {
                    model_as_block(root);
                    set_path(root, "model.context_length", serde_yaml::Value::from(n));
                } else if let Some(block) = root.get_mut("model").and_then(|m| m.as_mapping_mut()) {
                    block.remove("context_length");
                }
            }
        }
    })?;
    if let Some(hooks) = &hooks {
        hooks.finish(home, stored_hooks.into_inner())?;
    }
    Ok(())
}

/// `GET /api/config`: the saved config (defaults beneath it when asked) as the desktop's flat view.
/// `model` is the name, `model_context_length` the override (0 = automatic); `_`-keys are hidden.
pub(crate) fn config_record(home: &Path, defaults: &Value, include_defaults: bool) -> Value {
    migrate_checkpoint_config(home);
    let saved = serde_json::to_value(load(home)).unwrap_or(Value::Null);
    let mut merged = match (include_defaults, defaults) {
        (true, Value::Object(_)) => defaults.clone(),
        _ => json!({}),
    };
    merge_json(&mut merged, &saved);
    let record = merged.as_object_mut().expect("object");
    let model = record.remove("model");
    let (name, context) = match &model {
        Some(Value::Object(block)) => (
            block.get("default").or_else(|| block.get("name")).cloned().unwrap_or(json!("")),
            block.get("context_length").and_then(Value::as_i64).unwrap_or(0),
        ),
        Some(other) => (other.clone(), 0),
        None => (json!(""), 0),
    };
    if model.is_some() || include_defaults {
        record.insert("model".into(), name);
        record.insert("model_context_length".into(), json!(context));
    }
    record.retain(|k, _| !k.starts_with('_'));
    // Factr's default `smart` needs a guardian model this engine does not have (a save of it is refused);
    // `words("approvals.mode")` reads an unset or unsupported value as manual, so the view says manual too.
    if let Some(mode) = record.get_mut("approvals").and_then(|a| a.get_mut("mode"))
        && word(mode) == "smart"
    {
        *mode = json!("manual");
    }
    merged
}

/// Deep merge: objects merge key by key, anything else in `over` replaces.
fn merge_json(base: &mut Value, over: &Value) {
    match (base, over) {
        (Value::Object(b), Value::Object(o)) => {
            for (k, v) in o {
                merge_json(b.entry(k.clone()).or_insert(Value::Null), v);
            }
        }
        (b, o) => *b = o.clone(),
    }
}

/// A config.yaml value at a dotted path, for `config.get` of a key the engine has no typed owner for.
pub(crate) fn saved_value(home: &Path, key: &str) -> Option<Value> {
    read(home, key)
}

/// An exclusive advisory lock on `config.yaml.lock`, held while a writer reads, changes and renames
/// `config.yaml`, so two engine writers (and any other `flock`-ing writer of the file) never interleave.
/// The kernel drops it with the descriptor, so a crashed holder leaves nothing stale.
struct ConfigLock(#[allow(dead_code)] std::fs::File);

/// Run a blocking wait without stalling a tokio worker: on a multi-thread runtime the worker hands its
/// queue to another thread for the duration; elsewhere (plain threads, current-thread tests) it just runs.
fn off_worker<T>(f: impl FnOnce() -> T) -> T {
    match tokio::runtime::Handle::try_current() {
        Ok(handle) if handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread => tokio::task::block_in_place(f),
        _ => f(),
    }
}

impl ConfigLock {
    fn acquire(file: &Path) -> Result<Self, String> {
        let lock = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(file.with_extension("yaml.lock")).map_err(|e| e.to_string())?;
        // std's `File::try_lock`: flock(2) on Unix, LockFileEx on Windows (the same advisory lock, so a
        // Python writer's flock still excludes us on Unix).
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while lock.try_lock().is_err() {
            if std::time::Instant::now() >= deadline {
                return Err("config.yaml is locked by another writer".into());
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        Ok(Self(lock))
    }
}

/// Apply `change` to the parsed config.yaml and write it atomically (temp file + rename) under the
/// config lock. The change is re-applied to a fresh read when a writer that does not take the lock
/// moved the file underneath us, so its save is merged, not lost.
fn update(home: &Path, mut change: impl FnMut(&mut serde_yaml::Value)) -> Result<(), RpcError> {
    let internal = |e: String| RpcError { code: 5000, message: format!("cannot write config.yaml: {e}"), data: None };
    let link = config_file(home);
    factr_base::real_home_guard::refuse_real_home(&link, cfg!(test));
    // A symlinked config.yaml stays a symlink: write through to its target.
    let file = match std::fs::symlink_metadata(&link) {
        Ok(meta) if meta.file_type().is_symlink() => std::fs::canonicalize(&link).map_err(|e| internal(format!("{} is a dangling symlink: {e}", link.display())))?,
        _ => link.clone(),
    };
    factr_base::real_home_guard::refuse_real_home(&file, cfg!(test));
    std::fs::create_dir_all(file.parent().unwrap_or(home)).map_err(|e| internal(e.to_string()))?;
    let _lock = off_worker(|| ConfigLock::acquire(&file)).map_err(internal)?;
    // Unique per call: two writers in this process must not share one temp file.
    static WRITES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let tmp = file.with_extension(format!("yaml.{}.{}.tmp", std::process::id(), WRITES.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
    for _ in 0..5 {
        let seen = std::fs::read_to_string(&file).unwrap_or_default();
        let mut root = try_load(home).map_err(internal)?;
        change(&mut root);
        write_private(&tmp, &serde_yaml::to_string(&root).map_err(|e| internal(e.to_string()))?).map_err(|e| internal(e.to_string()))?;
        if let Ok(meta) = std::fs::metadata(&file) {
            let _ = std::fs::set_permissions(&tmp, meta.permissions());
        }
        if std::fs::read_to_string(&file).unwrap_or_default() == seen {
            return std::fs::rename(&tmp, &file).map_err(|e| internal(e.to_string()));
        }
    }
    let _ = std::fs::remove_file(&tmp);
    Err(internal("the file kept changing".into()))
}

/// Write `text` to a fresh file that is owner-only (0600) from the moment it exists.
fn write_private(path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)?.write_all(text.as_bytes())
}

fn store(home: &Path) -> Result<std::sync::Arc<EntryStore>, RpcError> {
    EntryStore::open_cached(home).map_err(RpcError::internal)
}

/// Session flags live in the engine store; `""` is "no session".
fn flag_key(kind: &str, session: &str) -> String {
    format!("session.{kind}.{session}")
}

/// Whether `session` runs with YOLO (approval bypass): its own flag or a process-wide one, both in the
/// engine `store` (never a second `factr.db`), or `approvals.mode: off` in the Factr `config_dir`.
pub(crate) fn yolo_active(config_dir: Option<&Path>, store: Option<&EntryStore>, session: &str) -> bool {
    config_dir.is_some_and(|dir| read(dir, "approvals.mode").is_some_and(|m| m == "off"))
        || store.is_some_and(|s| [session, ""].iter().any(|id| s.setting(&flag_key("yolo", id)).as_deref() == Some("1")))
}

fn display_toggle(key: &str) -> bool {
    key.starts_with("display.") && !key["display.".len()..].is_empty()
}

/// Words keys: (yaml path, allowed words, default).
fn words(key: &str) -> Option<(&'static str, &'static [&'static str], &'static str)> {
    Some(match key {
        "approvals.mode" | "approval_mode" => ("approvals.mode", APPROVAL, "manual"),
        "busy" => ("display.busy_input_mode", &["queue", "steer", "interrupt"], "queue"),
        "theme" => ("display.tui_theme", &["auto", "light", "dark"], "auto"),
        "thinking_mode" => ("display.thinking_mode", &["collapsed", "truncated", "full"], "collapsed"),
        "voice.voice_chat_mode" => ("voice.voice_chat_mode", &["chained", "gpt-live"], "chained"),
        _ => return None,
    })
}

fn session_of(p: &Value) -> &str {
    p["session_id"].as_str().unwrap_or_default()
}

/// Checkpoint settings used to live in `<engine dir>/checkpoints/config.json`; `config.yaml` is the one
/// owner now. Move any such file into it (keys already set there win) and delete it. Kept on failure.
fn migrate_checkpoint_config(home: &Path) {
    let file = home.join("checkpoints").join("config.json");
    let Ok(raw) = std::fs::read_to_string(&file) else { return };
    if let Ok(Value::Object(old)) = serde_json::from_str::<Value>(&raw) {
        for key in ["enabled", "max_snapshots", "max_total_size_mb", "max_file_size_mb"] {
            let path = format!("checkpoints.{key}");
            if let (Some(value), None) = (old.get(key), read(home, &path))
                && write(home, &path, value).is_err()
            {
                return;
            }
        }
    }
    let _ = std::fs::remove_file(&file);
}

/// Save the user's last model pick as the default for new sessions, in the keys Factr uses
/// (`model.default`, `model.provider`; an empty provider clears a stale one). The next
/// `session.create`, `/api/model/info` and `model.options` read it back.
pub(super) fn save_default_model(home: &Path, model: &str, provider: &str) -> Result<(), RpcError> {
    // One write: a reader never sees the new model beside the old provider.
    update(home, |root| {
        model_as_block(root);
        set_path(root, "model.default", serde_yaml::Value::from(model));
        set_path(root, "model.provider", serde_yaml::Value::from(provider));
    })
}

/// `None`: not a key the engine owns.
pub(super) fn get(home: &Path, key: &str, p: &Value) -> Option<Result<Value, RpcError>> {
    migrate_checkpoint_config(home);
    if let Some(done) = factr_keys::get(home, key) {
        return Some(done);
    }
    if let Some((path, allowed, default)) = words(key) {
        let value = read(home, path).map(|v| word(&v)).filter(|w| allowed.contains(&w.as_str())).unwrap_or_else(|| default.into());
        return Some(Ok(json!({ "value": value })));
    }
    Some(match key {
        "reasoning" => {
            let effort = match read(home, "agent.reasoning_effort") {
                Some(Value::Bool(false)) => "none".to_string(),
                Some(v) if !word(&v).is_empty() => word(&v),
                _ => "medium".into(),
            };
            let show = read(home, "display.show_reasoning").and_then(|v| v.as_bool()).unwrap_or(true);
            Ok(json!({ "value": effort, "display": if show { "show" } else { "hide" } }))
        }
        "fast" => {
            let own = store(home).ok().and_then(|s| s.setting(&flag_key("fast", session_of(p))));
            let tier = own.or_else(|| read(home, "agent.service_tier").map(|v| word(&v)));
            Ok(json!({ "value": if matches!(tier.as_deref(), Some("fast" | "priority")) { "fast" } else { "normal" } }))
        }
        "yolo" => {
            let on = if p["scope"] == "global" {
                read(home, "approvals.mode").is_some_and(|m| m == "off")
            } else {
                yolo_active(Some(&factr_dir(home)), store(home).ok().as_deref(), session_of(p))
            };
            Ok(json!({ "value": if on { "1" } else { "0" } }))
        }
        k if display_toggle(k) => Ok(json!({ "value": read(home, k) })),
        _ => return None,
    })
}

/// `None`: not a key the engine owns.
pub(super) fn set(home: &Path, key: &str, p: &Value) -> Option<Result<Value, RpcError>> {
    migrate_checkpoint_config(home);
    let value = &p["value"];
    if let Some(done) = factr_keys::set(home, key, value) {
        return Some(done);
    }
    let w = word(value);
    if let Some((path, allowed, _)) = words(key) {
        // Factr `smart` asks a guardian model to approve, deny or escalate each flagged command
        // before the person is asked; nothing here reads the value, so accepting it would be a lie.
        if path == "approvals.mode" && w == "smart" {
            return Some(Err(bad("approvals.mode smart is not supported by this engine (it has no guardian-model review step): use manual or off".into())));
        }
        return Some(if allowed.contains(&w.as_str()) {
            write(home, path, &json!(w)).map(|_| json!({ "value": w }))
        } else {
            Err(bad(format!("unknown {key} value: {value}; pick one of {}", allowed.join("|"))))
        });
    }
    Some(match key {
        "reasoning" => (|| {
            for (accepted, reported, field, flag) in [
                (&["show", "on"][..], "show", "display.show_reasoning", true),
                (&["hide", "off"], "hide", "display.show_reasoning", false),
                (&["full", "all"], "full", "display.reasoning_full", true),
                (&["clamp", "collapse", "short"], "clamp", "display.reasoning_full", false),
            ] {
                if accepted.contains(&w.as_str()) {
                    write(home, field, &json!(flag))?;
                    return Ok(json!({ "value": reported }));
                }
            }
            if !EFFORTS.contains(&w.as_str()) && factr_provider_core::canonical_reasoning_effort(&w).is_none() && !factr_base::prompt::is_swarm_effort(&w) {
                return Err(bad(format!("unknown reasoning value: {value}")));
            }
            write(home, "agent.reasoning_effort", &json!(w))?;
            Ok(json!({ "value": w }))
        })(),
        "fast" => (|| {
            let current = get(home, "fast", p).unwrap()?["value"].as_str() == Some("fast");
            let tier = match w.as_str() {
                "fast" | "on" => "fast",
                "normal" | "off" => "normal",
                "" | "toggle" => if current { "normal" } else { "fast" },
                "status" => return Ok(json!({ "value": if current { "fast" } else { "normal" } })),
                _ => return Err(bad(format!("unknown fast mode: {value}"))),
            };
            match session_of(p) {
                "" => write(home, "agent.service_tier", &json!(tier))?,
                id => store(home)?.set_setting(&flag_key("fast", id), tier).map_err(RpcError::internal)?,
            }
            Ok(json!({ "value": tier }))
        })(),
        "yolo" => (|| {
            let global = p["scope"] == "global";
            let current = get(home, "yolo", p).unwrap()?["value"] == "1";
            let on = match w.as_str() {
                "" | "toggle" => !current,
                other => truth(other).ok_or_else(|| bad(format!("unknown yolo value: {value}")))?,
            };
            if global {
                write(home, "approvals.mode", &json!(if on { "off" } else { "manual" }))?;
            } else {
                store(home)?.set_setting(&flag_key("yolo", session_of(p)), if on { "1" } else { "0" }).map_err(RpcError::internal)?;
            }
            Ok(json!({ "value": if on { "1" } else { "0" }, "scope": if global { "global" } else { "session" } }))
        })(),
        k if display_toggle(k) => (|| {
            let on = truth(&w).ok_or_else(|| bad(format!("{k} takes true or false")))?;
            write(home, k, &json!(on))?;
            Ok(json!({ "value": on }))
        })(),
        _ => return None,
    })
}

/// Unknown keys the Python backend could not take (none running): kept in the engine store.
pub(super) fn stash(home: &Path, key: &str, value: &Value) -> Result<Value, RpcError> {
    store(home)?.set_setting(&format!("config.{key}"), &value.to_string()).map_err(RpcError::internal)?;
    Ok(json!({ "value": value }))
}

pub(super) fn stashed(home: &Path, key: &str) -> Option<Value> {
    store(home).ok()?.setting(&format!("config.{key}")).and_then(|s| serde_json::from_str(&s).ok())
}

#[cfg(test)]
mod tests {
    use crate::rpc::tests::test_conn;
    use serde_json::{Value, json};

    /// Every key the desktop reads or writes through the gateway, with a value and the value it reads back as.
    fn desktop_keys() -> Vec<(&'static str, Value, Value, Value)> {
        // (key, extra params, set value, expected get value)
        vec![
            ("approvals.mode", json!({}), json!("off"), json!("off")),
            ("approvals.mode", json!({}), json!("manual"), json!("manual")),
            ("reasoning", json!({}), json!("high"), json!("high")),
            ("fast", json!({}), json!("fast"), json!("fast")),
            ("fast", json!({ "session_id": "s1" }), json!("normal"), json!("normal")),
            ("yolo", json!({ "scope": "global" }), json!("1"), json!("1")),
            ("yolo", json!({ "session_id": "s1" }), json!("1"), json!("1")),
            ("display.message_reactions", json!({}), json!("false"), json!(false)),
            ("display.in_app_tips", json!({}), json!("true"), json!(true)),
            ("display.in_app_tours", json!({}), json!("false"), json!(false)),
            ("voice.voice_chat_mode", json!({}), json!("gpt-live"), json!("gpt-live")),
            ("learning.enabled", json!({}), json!(false), json!(false)),
            ("checkpoints.enabled", json!({}), json!(false), json!(false)),
            ("checkpoints.max_snapshots", json!({}), json!(7), json!(7)),
            ("agents.auto_verify", json!({}), json!(true), json!(true)),
            ("fallback_providers", json!({}), json!([{ "provider": "openrouter", "model": "a/b" }, "anthropic:claude-x"]),
                json!([{ "provider": "openrouter", "model": "a/b" }, { "provider": "anthropic", "model": "claude-x" }])),
            ("delegation.model", json!({}), json!("google/gemini-3-flash"), json!("google/gemini-3-flash")),
            ("delegation.provider", json!({}), json!("openrouter"), json!("openrouter")),
            ("delegation.max_iterations", json!({}), json!(40), json!(40)),
            ("delegation.reasoning_effort", json!({}), json!("Low"), json!("low")),
            ("auxiliary.background_review.provider", json!({}), json!("anthropic"), json!("anthropic")),
            ("auxiliary.background_review.model", json!({}), json!("claude-haiku"), json!("claude-haiku")),
            ("auxiliary.repl_sub.model", json!({}), json!("small"), json!("small")),
            ("hooks_auto_accept", json!({}), json!("true"), json!(true)),
            (
                "hooks",
                json!({}),
                json!({ "pre_tool_call": [{ "matcher": "terminal", "command": "./gate.sh", "timeout": 5 }], "on_session_end": [{ "command": "./end.sh" }], "subagent_stop": [{ "command": "./sub.sh" }] }),
                json!({ "pre_tool_call": [{ "matcher": "terminal", "command": "./gate.sh", "timeout": 5 }], "on_session_end": [{ "command": "./end.sh" }], "subagent_stop": [{ "command": "./sub.sh" }] }),
            ),
            ("compression.enabled", json!({}), json!("false"), json!(false)),
            ("compression.threshold", json!({}), json!(0.6), json!(0.6)),
            ("compression.target_ratio", json!({}), json!("0.25"), json!(0.25)),
            ("compression.protect_last_n", json!({}), json!(12), json!(12)),
            ("auxiliary.compression.provider", json!({}), json!("openrouter"), json!("openrouter")),
            ("auxiliary.compression.model", json!({}), json!("google/gemini-3-flash"), json!("google/gemini-3-flash")),
        ]
    }

    /// Splits the two homes (`FACTR_CONFIG_HOME` vs the engine dir) for one test; restores the variable on drop.
    struct SplitHomes {
        engine: std::path::PathBuf,
        factr: std::path::PathBuf,
        before: Option<std::ffi::OsString>,
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl SplitHomes {
        fn new(name: &str) -> Self {
            let lock = crate::factr_env::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let base = std::env::temp_dir().join(format!("{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&base);
            let factr = base.join("factr");
            let engine = factr.join("engine");
            std::fs::create_dir_all(&engine).unwrap();
            std::fs::create_dir_all(&factr).unwrap();
            let before = std::env::var_os("FACTR_CONFIG_HOME");
            // SAFETY: env is only touched under ENV_LOCK.
            unsafe { std::env::set_var("FACTR_CONFIG_HOME", &factr) };
            Self { engine, factr, before, _lock: lock }
        }
    }

    impl Drop for SplitHomes {
        fn drop(&mut self) {
            match self.before.take() {
                Some(v) => unsafe { std::env::set_var("FACTR_CONFIG_HOME", v) },
                None => unsafe { std::env::remove_var("FACTR_CONFIG_HOME") },
            }
            // Left in place: tests without the env lock may still be writing through FACTR_CONFIG_HOME.
        }
    }

    #[test]
    fn a_saved_hook_is_approved_in_factr_home_where_the_checker_reads_it() {
        let homes = SplitHomes::new("hook-approval");
        let value = json!({ "pre_tool_call": [{ "matcher": "terminal", "command": "./gate.sh" }] });
        super::set(&homes.engine, "hooks", &json!({ "value": value })).unwrap().unwrap();
        let allowlist = "shell-hooks-allowlist.json";
        let approved = std::fs::read_to_string(homes.factr.join(allowlist)).expect("the approval is in FACTR_CONFIG_HOME");
        assert!(approved.contains("./gate.sh") && approved.contains("pre_tool_call"), "{approved}");
        assert!(!homes.engine.join(allowlist).exists(), "nothing leaks into the engine dir");
        assert!(homes.factr.join("config.yaml").exists());
    }

    #[tokio::test]
    async fn session_yolo_is_read_from_the_engine_store_and_no_database_appears_in_factr_home() {
        let homes = SplitHomes::new("yolo-split");
        let set = json!({ "value": "1", "session_id": "s1" });
        super::set(&homes.engine, "yolo", &set).unwrap().unwrap();
        assert_eq!(super::get(&homes.engine, "yolo", &json!({ "session_id": "s1" })).unwrap().unwrap()["value"], "1");
        assert_eq!(super::get(&homes.engine, "yolo", &json!({ "session_id": "s2" })).unwrap().unwrap()["value"], "0");
        // The approval hub waves a risky command through for that session, on the same flag.
        let hub = std::sync::Arc::new(crate::approvals::Hub::default());
        hub.set_store(super::store(&homes.engine).unwrap()).await;
        assert_eq!(hub.decide("s1", "bash", "rm -rf x", "r").await, "once");
        assert!(homes.engine.join("factr.db").exists(), "the flag lives in the engine db");
        assert!(!homes.factr.join("factr.db").exists(), "no second factr.db in FACTR_CONFIG_HOME");
    }

    #[test]
    fn a_config_yaml_that_does_not_parse_is_refused_and_left_untouched() {
        let homes = SplitHomes::new("bad-yaml");
        let path = homes.factr.join("config.yaml");
        for broken in ["model: [unclosed\n  - x: : :", "- just\n- a list\n", "just a string"] {
            std::fs::write(&path, broken).unwrap();
            let err = super::write(&homes.engine, "display.compact", &json!(true)).unwrap_err();
            assert!(err.message.contains("will not overwrite"), "{}", err.message);
            assert_eq!(std::fs::read_to_string(&path).unwrap(), broken, "the file survived the refused write");
            assert!(super::save_default_model(&homes.engine, "m", "p").is_err());
        }
        // Reads still work (empty), and a blank file is just an empty config.
        assert_eq!(super::read(&homes.engine, "display.compact"), None);
        std::fs::write(&path, "  \n").unwrap();
        super::write(&homes.engine, "display.compact", &json!(true)).unwrap();
        assert_eq!(super::read(&homes.engine, "display.compact"), Some(json!(true)));
    }

    #[cfg(unix)]
    #[test]
    fn a_new_config_yaml_is_private_and_a_symlinked_one_stays_a_symlink() {
        use std::os::unix::fs::PermissionsExt;
        let homes = SplitHomes::new("cfg-perms");
        let path = homes.factr.join("config.yaml");
        super::write(&homes.engine, "display.compact", &json!(true)).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600, "created owner-only");
        std::fs::remove_file(&path).unwrap();

        let target = homes.factr.join("dotfiles").join("factr.yaml");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, "display:\n  compact: false\nkeep: me\n").unwrap();
        std::os::unix::fs::symlink(&target, &path).unwrap();
        super::write(&homes.engine, "display.compact", &json!(true)).unwrap();
        assert!(std::fs::symlink_metadata(&path).unwrap().file_type().is_symlink(), "still a symlink");
        let written = std::fs::read_to_string(&target).unwrap();
        assert!(written.contains("compact: true") && written.contains("keep: me"), "{written}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn waiting_for_the_config_lock_does_not_stall_the_async_worker() {
        let homes = SplitHomes::new("cfg-lock-wait");
        let file = homes.factr.join("config.yaml");
        let held = super::ConfigLock::acquire(&file).unwrap();
        let release = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(400));
            drop(held);
        });
        let ticks = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let ticker = {
            let ticks = ticks.clone();
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                    ticks.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
            })
        };
        tokio::time::sleep(std::time::Duration::from_millis(40)).await;
        let engine = homes.engine.clone();
        super::write(&engine, "display.compact", &json!(true)).unwrap();
        let seen = ticks.load(std::sync::atomic::Ordering::Relaxed);
        ticker.abort();
        release.join().unwrap();
        assert!(seen >= 8, "the only worker kept running other tasks while the lock was awaited ({seen} ticks)");
    }

    #[test]
    fn put_config_merges_every_desktop_toggle_into_config_yaml_and_keeps_what_it_does_not_know() {
        let homes = SplitHomes::new("put-config");
        let yaml_path = homes.factr.join("config.yaml");
        std::fs::write(&yaml_path, "custom_providers:\n  - name: mine\nmodel:\n  default: gpt-5.6-luna\n  provider: openai\n  base_url: http://x\nmemory:\n  provider: builtin\n").unwrap();
        let patch = json!({
            "display": { "show_reasoning": false },
            "workspace": { "repo_scan_enabled": false },
            "approvals": { "mode": "off", "mcp_reload_confirm": false },
            "browser": { "use_real_profile": true },
            "memory": { "memory_enabled": false, "user_profile_enabled": false },
            "voice": { "client_direct": true },
            "sessions": { "auto_archive": { "enabled": true, "days": 5 } },
            "agent": { "reasoning_effort": "High" },
            "checkpoints": { "max_snapshots": "9" },
            "_model_meta": { "ignored": true },
        });
        super::put_config(&homes.engine, &patch).unwrap();
        // A second, sparse save does not drop the first.
        super::put_config(&homes.engine, &json!({ "voice": { "client_direct": false } })).unwrap();
        let record = super::config_record(&homes.engine, &json!({ "memory": { "memory_enabled": true, "provider": "" }, "model": "default-model", "voice": { "record_key": "ctrl+b" } }), true);
        assert_eq!(record["display"]["show_reasoning"], false);
        assert_eq!(record["workspace"]["repo_scan_enabled"], false);
        assert_eq!(record["approvals"], json!({ "mode": "off", "mcp_reload_confirm": false }));
        assert_eq!(record["browser"]["use_real_profile"], true);
        assert_eq!(record["memory"], json!({ "memory_enabled": false, "user_profile_enabled": false, "provider": "builtin" }), "defaults sit beneath saved values");
        assert_eq!(record["voice"], json!({ "client_direct": false, "record_key": "ctrl+b" }));
        assert_eq!(record["sessions"]["auto_archive"], json!({ "enabled": true, "days": 5 }));
        assert_eq!(record["agent"]["reasoning_effort"], "high");
        assert_eq!(record["checkpoints"]["max_snapshots"], 9, "known keys are coerced like config.set");
        assert_eq!((&record["model"], &record["model_context_length"]), (&json!("gpt-5.6-luna"), &json!(0)));
        assert!(record.get("_model_meta").is_none());
        assert_eq!(record["custom_providers"][0]["name"], "mine", "unknown keys survive");
        let raw = super::config_record(&homes.engine, &json!({ "a": 1 }), false);
        assert!(raw.get("a").is_none() && raw["voice"] == json!({ "client_direct": false }), "{raw}");
        // config.get-side readers see the same file.
        assert_eq!(super::saved_value(&homes.engine, "memory.memory_enabled"), Some(json!(false)));
        assert_eq!(super::get(&homes.engine, "reasoning", &json!({})).unwrap().unwrap()["value"], "high");
        assert_eq!(super::get(&homes.engine, "approvals.mode", &json!({})).unwrap().unwrap()["value"], "off");
        assert_eq!(super::get(&homes.engine, "checkpoints.max_snapshots", &json!({})).unwrap().unwrap()["value"], 9);
        // A "restart" is a fresh read of the file; model block sub-keys stay.
        let yaml = std::fs::read_to_string(&yaml_path).unwrap();
        assert!(yaml.contains("base_url: http://x") && yaml.contains("provider: openai"), "{yaml}");
    }

    #[test]
    fn put_config_refuses_smart_and_unknown_modes_and_writes_nothing() {
        let homes = SplitHomes::new("put-config-bad");
        let yaml_path = homes.factr.join("config.yaml");
        std::fs::write(&yaml_path, "approvals:\n  mode: manual\n").unwrap();
        for bad_mode in ["smart", "sometimes"] {
            let err = super::put_config(&homes.engine, &json!({ "voice": { "client_direct": true }, "approvals": { "mode": bad_mode } })).unwrap_err();
            assert_eq!(err.code, 4002, "{bad_mode}");
        }
        assert!(super::put_config(&homes.engine, &json!({ "compression": { "threshold": 7 } })).is_err());
        assert!(super::put_config(&homes.engine, &json!([1])).is_err());
        assert_eq!(std::fs::read_to_string(&yaml_path).unwrap(), "approvals:\n  mode: manual\n", "a rejected save leaves the file alone");
    }

    #[test]
    fn put_config_never_stores_a_credential_and_refuses_objects_under_validated_keys() {
        let homes = SplitHomes::new("put-config-guard");
        let yaml_path = homes.factr.join("config.yaml");
        let before = "agent:\n  reasoning_effort: low\nmodel: gpt-5.6-luna\n";
        std::fs::write(&yaml_path, before).unwrap();
        for bad in [
            json!({ "OPENAI_API_KEY": "sk-x" }),
            json!({ "providers": { "x": { "api_key": "sk-x" } } }),
            json!({ "model": { "api_key": "sk-x" } }),
            json!({ "voice": { "client_direct": true }, "github_token": "t" }),
            json!({ "auth": { "client_secret": "s" } }),
            json!({ "custom_providers": [{ "name": "m", "api_key": "sk-x" }] }),
        ] {
            let err = super::put_config(&homes.engine, &bad).unwrap_err();
            assert!(err.code == 4002 && err.message.contains("/api/env"), "{bad}: {}", err.message);
        }
        // An underscore anywhere in the path is view-only, not stored.
        super::put_config(&homes.engine, &json!({ "_meta": { "x": 1 }, "a": { "_b": { "c": 1 } } })).unwrap();
        for bad in [json!({ "agent": { "reasoning_effort": { "x": 1 } } }), json!({ "approvals": { "mode": { "x": 1 } } }), json!({ "compression": { "threshold": { "x": 1 } } })] {
            assert_eq!(super::put_config(&homes.engine, &bad).unwrap_err().code, 4002, "{bad}");
        }
        assert_eq!(std::fs::read_to_string(&yaml_path).unwrap(), before, "nothing was written");
        // A sub-key over a bare-string model keeps the name.
        super::put_config(&homes.engine, &json!({ "model": { "provider": "openai" } })).unwrap();
        let record = super::config_record(&homes.engine, &json!({}), false);
        assert_eq!(record["model"], "gpt-5.6-luna");
        let yaml = std::fs::read_to_string(&yaml_path).unwrap();
        assert!(yaml.contains("provider: openai") && yaml.contains("default: gpt-5.6-luna"), "{yaml}");
        // Flags, numbers, blank slots and env-var names under credential-like names are not credentials (a
        // refused key would reject the whole autosave patch and every later one with it).
        super::put_config(
            &homes.engine,
            &json!({
                "dashboard": { "show_token_analytics": true, "drain_auth": { "min_secret_chars": 16 }, "basic_auth": { "password": "" } },
                "display": { "spinner_token_flow": false },
                "desktop": { "password_store": "basic" },
                "auxiliary": { "vision": { "api_key": "" } },
                "vault": { "onepassword": { "service_account_token_env": "OP_SERVICE_ACCOUNT_TOKEN" } },
            }),
        )
        .unwrap();
        // Max-token style keys are not credentials.
        super::put_config(&homes.engine, &json!({ "compression": { "protect_last_n": 3 }, "limits": { "max_tokens": 5 } })).unwrap();
    }

    #[test]
    fn a_save_with_hooks_and_a_bad_key_writes_nothing_and_a_good_one_writes_once() {
        let homes = SplitHomes::new("put-config-atomic");
        let yaml_path = homes.factr.join("config.yaml");
        std::fs::write(&yaml_path, "approvals:\n  mode: manual\n").unwrap();
        let hooks = json!({ "pre_tool_call": [{ "command": "./gate.sh" }] });
        let err = super::put_config(&homes.engine, &json!({ "hooks": hooks, "compression": { "threshold": 7 } })).unwrap_err();
        assert_eq!(err.code, 4002);
        assert_eq!(std::fs::read_to_string(&yaml_path).unwrap(), "approvals:\n  mode: manual\n", "no partial save: hooks were not written");
        assert!(!homes.factr.join("shell-hooks-allowlist.json").exists(), "and nothing was approved");
        super::put_config(&homes.engine, &json!({ "hooks": hooks, "compression": { "threshold": 0.5 }, "model": "m1" })).unwrap();
        let yaml = std::fs::read_to_string(&yaml_path).unwrap();
        assert!(yaml.contains("./gate.sh") && yaml.contains("threshold: 0.5") && yaml.contains("model: m1"), "{yaml}");
        assert!(std::fs::read_to_string(homes.factr.join("shell-hooks-allowlist.json")).unwrap().contains("./gate.sh"));
    }

    #[test]
    fn concurrent_config_writers_lose_nothing_and_the_default_model_never_reads_half_saved() {
        let homes = SplitHomes::new("config-lock");
        std::fs::write(homes.factr.join("config.yaml"), "model:\n  default: m0\n  provider: p0\n  base_url: http://x\n").unwrap();
        let engine = homes.engine.clone();
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let reader = {
            let (engine, stop) = (engine.clone(), stop.clone());
            std::thread::spawn(move || {
                let mut torn = 0;
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    let both = super::load(&engine);
                    let (m2, p2) = (both["model"]["default"].as_str().unwrap_or("").to_string(), both["model"]["provider"].as_str().unwrap_or("").to_string());
                    if m2[1..] != p2[1..] {
                        torn += 1;
                    }
                }
                torn
            })
        };
        let writers: Vec<_> = (0..6)
            .map(|i| {
                let engine = engine.clone();
                std::thread::spawn(move || {
                    for n in 0..15 {
                        super::save_default_model(&engine, &format!("m{i}-{n}"), &format!("p{i}-{n}")).unwrap();
                        super::write(&engine, &format!("custom.k{i}_{n}"), &json!(n)).unwrap();
                    }
                })
            })
            .collect();
        for w in writers {
            w.join().unwrap();
        }
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(reader.join().unwrap(), 0, "a reader saw a model with another save's provider");
        let saved = super::load(&engine);
        for i in 0..6 {
            for n in 0..15 {
                assert_eq!(saved["custom"][format!("k{i}_{n}")], n, "k{i}_{n} lost");
            }
        }
        assert_eq!(saved["model"]["base_url"], "http://x", "the model block's other keys stay");
        assert!(homes.factr.join("config.yaml.lock").exists(), "writers take the shared lock file");
    }

    #[test]
    fn put_config_model_name_and_context_length_map_onto_the_model_block() {
        let homes = SplitHomes::new("put-config-model");
        let yaml_path = homes.factr.join("config.yaml");
        std::fs::write(&yaml_path, "model: gpt-5.6-luna\n").unwrap();
        super::put_config(&homes.engine, &json!({ "model_context_length": 64000 })).unwrap();
        let record = super::config_record(&homes.engine, &json!({}), true);
        assert_eq!((&record["model"], &record["model_context_length"]), (&json!("gpt-5.6-luna"), &json!(64000)));
        super::put_config(&homes.engine, &json!({ "model": "gpt-5.6-terra", "model_context_length": 0 })).unwrap();
        let record = super::config_record(&homes.engine, &json!({}), true);
        assert_eq!((&record["model"], &record["model_context_length"]), (&json!("gpt-5.6-terra"), &json!(0)));
    }

    #[test]
    fn an_old_checkpoint_config_json_moves_into_config_yaml_once_and_is_removed() {
        let homes = SplitHomes::new("ckpt-migrate");
        std::fs::create_dir_all(homes.engine.join("checkpoints")).unwrap();
        let old = homes.engine.join("checkpoints/config.json");
        std::fs::write(&old, r#"{"enabled":false,"max_snapshots":9,"max_file_size_mb":3}"#).unwrap();
        std::fs::write(homes.factr.join("config.yaml"), "checkpoints:\n  max_snapshots: 4\n").unwrap();
        let got = |key: &str| super::get(&homes.engine, key, &json!({})).unwrap().unwrap()["value"].clone();
        assert_eq!(got("checkpoints.enabled"), json!(false), "carried over");
        assert_eq!(got("checkpoints.max_snapshots"), json!(4), "a value already in config.yaml wins");
        assert_eq!(got("checkpoints.max_file_size_mb"), json!(3));
        assert!(!old.exists(), "the private file is gone");
        let yaml = std::fs::read_to_string(homes.factr.join("config.yaml")).unwrap();
        assert!(yaml.contains("enabled: false") && yaml.contains("max_snapshots: 4"), "{yaml}");
        assert_eq!(got("checkpoints.max_total_size_mb"), json!(500), "unset numbers read as the engine default");
    }

    #[test]
    fn config_yaml_goes_to_factr_home_not_the_engine_home() {
        let _env = crate::factr_env::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let base = std::env::temp_dir().join(format!("settings-split-{}", std::process::id()));
        let hhome = base.join("factr");
        let jhome = hhome.join("engine");
        let _ = std::fs::remove_dir_all(&base);
        let before = std::env::var_os("FACTR_CONFIG_HOME");
        unsafe { std::env::set_var("FACTR_CONFIG_HOME", &hhome) };
        super::write(&jhome, "approvals.mode", &json!("off")).unwrap();
        let seen = (hhome.join("config.yaml").exists(), jhome.join("config.yaml").exists(), super::read(&jhome, "approvals.mode"));
        match before { Some(v) => unsafe { std::env::set_var("FACTR_CONFIG_HOME", v) }, None => unsafe { std::env::remove_var("FACTR_CONFIG_HOME") } }
        assert_eq!(seen, (true, false, Some(json!("off"))));
    }

    #[tokio::test]
    async fn every_desktop_key_reads_back_after_set_and_after_a_restart_with_no_python_backend() {
        let _env = crate::factr_env::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let jhome = std::env::temp_dir().join(format!("settings-jh-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&jhome);
        let before = std::env::var_os("FACTR_HOME");
        factr_app_core::env::set_var("FACTR_HOME", &jhome);
        // An inherited FACTR_CONFIG_HOME (a dev shell exporting ~/.factr) must not receive this test's keys.
        let factr_before = std::env::var_os("FACTR_CONFIG_HOME");
        factr_app_core::env::remove_var("FACTR_CONFIG_HOME");
        let conn = test_conn("settings-rt");
        // FACTR_CONFIG_HOME is unset here: config.yaml lives beside the engine's files (split homes are covered by the tests above).
        let hhome = std::path::PathBuf::from(&conn.config.home);
        assert!(conn.config.features.is_none(), "no Python backend");
        for (key, extra, value, expected) in desktop_keys() {
            let mut set = extra.clone();
            set["key"] = json!(key);
            set["value"] = value;
            let reply = conn.dispatch("config.set", &set).await.unwrap_or_else(|e| panic!("set {key}: {}", e.message));
            let mut get = extra.clone();
            get["key"] = json!(key);
            let read = conn.dispatch("config.get", &get).await.unwrap();
            assert_eq!(read["value"], expected, "get after set {key}: {reply}");
        }
        // New connection on the same home: everything persisted (the session-scoped ones are by session).
        let again = test_conn("settings-rt");
        for (key, extra, _, expected) in desktop_keys().into_iter().filter(|k| k.0 != "approvals.mode") {
            let mut get = extra.clone();
            get["key"] = json!(key);
            assert_eq!(again.dispatch("config.get", &get).await.unwrap()["value"], expected, "after restart {key}");
        }
        assert_eq!(again.dispatch("config.get", &json!({ "key": "approvals.mode" })).await.unwrap()["value"], "off", "global yolo wrote approvals.mode");
        assert_eq!(again.dispatch("config.get", &json!({ "key": "reasoning" })).await.unwrap()["display"], "show");
        let home = hhome.as_path();
        assert!(super::yolo_active(Some(home), None, "other"), "approvals.mode off bypasses prompts");
        // Factr's file carries the same values.
        let yaml = std::fs::read_to_string(hhome.join("config.yaml")).unwrap();
        assert!(yaml.contains("reasoning_effort: high") && yaml.contains("voice_chat_mode: gpt-live"), "{yaml}");
        // Checkpoint settings are config.yaml keys (the desktop saves them there too), not a file of the engine's own.
        let checkpoint_yaml = std::fs::read_to_string(jhome.join("config.yaml")).unwrap();
        assert!(checkpoint_yaml.contains("max_snapshots: 7"), "{checkpoint_yaml}");
        assert!(!jhome.join("checkpoints/config.json").exists());
        // The engine's own readers see the same file: the compaction limits and summary model follow.
        let seen = factr_base::factr_config::load_from(&hhome);
        let limits = factr_base::compaction::limits_from(&seen.compression);
        assert_eq!((limits.enabled, limits.soft, limits.keep_recent, limits.tail_ratio), (false, 0.6, 12, Some(0.25)));
        assert_eq!(
            seen.aux_model(factr_base::factr_config::AuxConsumer::Compaction).as_deref(),
            Some("openrouter:google/gemini-3-flash")
        );
        use factr_base::factr_config::AuxConsumer::*;
        assert_eq!(seen.aux_model(Learning).as_deref(), Some("anthropic:claude-haiku"));
        assert_eq!(seen.aux_model(MemoryExtraction).as_deref(), Some("anthropic:claude-haiku"));
        assert_eq!(seen.aux_model(ReplSub).as_deref(), Some("small"));
        assert_eq!(seen.delegation.model_spec().as_deref(), Some("openrouter:google/gemini-3-flash"));
        assert_eq!((seen.delegation.max_iterations, seen.delegation.reasoning_effort.as_deref()), (Some(40), Some("low")));
        assert_eq!(seen.hooks.len(), 3, "{:?}", seen.hooks);
        assert!(seen.hooks_auto_accept);
        let gate = seen.hooks.iter().find(|h| h.event == "pre_tool_call").expect("the gate hook");
        assert_eq!((gate.matcher.as_deref(), gate.timeout_s), (Some("terminal"), 5));
        // Saving a hook is the owner's consent: it is in Factr's allowlist; events the engine cannot run are reported.
        let allow = std::fs::read_to_string(hhome.join("shell-hooks-allowlist.json")).unwrap();
        assert!(allow.contains("./gate.sh") && allow.contains("on_session_end"), "{allow}");
        let saved = conn.dispatch("config.set", &json!({ "key": "hooks", "value": { "outbound": { "url": "x" }, "on_session_start": [{ "command": "./s.sh" }] } })).await.unwrap();
        assert_eq!(saved["unsupported"], json!([]));
        conn.dispatch("config.set", &json!({ "key": "hooks", "value": { "on_session_start": [{ "command": "./s.sh" }] } })).await.unwrap();
        assert_eq!(conn.dispatch("config.get", &json!({ "key": "hooks" })).await.unwrap()["value"]["outbound"], json!({ "url": "x" }), "reserved sub-sections survive a save");
        for bad_hooks in [json!({ "pre_tool_call": [{ "matcher": "x" }] }), json!({ "pre_tool_call": [{ "command": "x", "timeout": 999 }] }), json!({ "pre_tool_call": "x" }), json!("x")] {
            let err = conn.dispatch("config.set", &json!({ "key": "hooks", "value": bad_hooks })).await.unwrap_err();
            assert_eq!(err.code, 4002, "{bad_hooks}");
        }
        assert_eq!(seen.fallbacks.len(), 2, "the engine reads the saved fallback list: {:?}", seen.fallbacks);
        assert_eq!(seen.fallbacks[1].spec(), "anthropic:claude-x");
        let err = conn.dispatch("config.set", &json!({ "key": "fallback_providers", "value": [{ "provider": "openrouter" }] })).await.unwrap_err();
        assert_eq!(err.code, 4002, "an entry without a model is refused");
        // Out-of-range and non-numeric values are refused, not stored.
        for (key, bad_value) in [("compression.threshold", json!(7)), ("compression.protect_last_n", json!("many")), ("compression.enabled", json!("maybe")), ("delegation.max_iterations", json!(0)), ("delegation.reasoning_effort", json!("turbo"))] {
            let err = conn.dispatch("config.set", &json!({ "key": key, "value": bad_value })).await.unwrap_err();
            assert_eq!(err.code, 4002, "{key}");
        }
        assert_eq!(conn.dispatch("config.get", &json!({ "key": "compression.threshold" })).await.unwrap()["value"], 0.6);
        // Bad words are refused, not stored.
        let err = conn.dispatch("config.set", &json!({ "key": "approvals.mode", "value": "wild" })).await.unwrap_err();
        assert_eq!(err.code, 4002);
        let err = conn.dispatch("config.set", &json!({ "key": "approvals.mode", "value": "smart" })).await.unwrap_err();
        assert!(err.code == 4002 && err.message.contains("not supported"), "smart is refused, not stored: {}", err.message);
        match before {
            Some(v) => factr_app_core::env::set_var("FACTR_HOME", v),
            None => factr_app_core::env::remove_var("FACTR_HOME"),
        }
        if let Some(v) = factr_before {
            factr_app_core::env::set_var("FACTR_CONFIG_HOME", v);
        }
    }
}
