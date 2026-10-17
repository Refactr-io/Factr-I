//! Read-only dashboard routes answered from files, so opening Settings or Scheduled jobs does not
//! start the Python backend (170-235 MB). Strictly additive: each handler either returns exactly
//! what the Python route returns for the same files, or `None`, and `None` means the request is
//! proxied to Python as before. Anything the port does not model (a managed-scope dir, `${VAR}`
//! references, a user plugin dir, a configured API key, a named profile, jobs on disk, platform
//! credentials) is `None`. Parity with Python: `backend/scripts/dump_ui_routes.py` writes the
//! goldens that `tests/ui_routes.rs` compares against.

use serde_json::{Value, json};
use std::path::Path;
use std::sync::OnceLock;

const SCHEMA: &str = include_str!("ui_golden/schema.json");
const BLUEPRINTS: &str = include_str!("ui_golden/cron_blueprints.json");
const MOA: &str = include_str!("ui_golden/model_moa.json");

/// `_AUX_TASK_SLOTS` in backend/factr_backend/web_server_config.py (the order the page shows).
const AUX_SLOTS: [&str; 11] = [
    "vision", "compression", "skills_hub", "approval", "mcp", "title_generation", "review",
    "triage_specifier", "kanban_decomposer", "profile_describer", "curator",
];

/// Env-var name fragments that make a messaging platform "connected" for the cron delivery options.
const PLATFORM_ENV: [&str; 24] = [
    "TELEGRAM", "DISCORD", "SLACK", "WHATSAPP", "SIGNAL", "MATRIX", "MATTERMOST", "EMAIL", "SMS", "TWILIO",
    "HOMEASSISTANT", "FEISHU", "DINGTALK", "WECOM", "WEIXIN", "QQ", "LINE_", "TEAMS", "WEBHOOK", "API_SERVER",
    "BLUEBUBBLES", "YUANBAO", "IRC", "MASTODON",
];

fn golden(cell: &'static OnceLock<Value>, text: &str) -> Value {
    cell.get_or_init(|| serde_json::from_str(text).expect("embedded golden is JSON")).clone()
}

/// The answer for `GET path` when the files make it knowable, else `None` (forward to Python).
pub(crate) fn answer(path: &str, query: &str, defaults: &Value) -> Option<Value> {
    static SCHEMA_V: OnceLock<Value> = OnceLock::new();
    static BLUEPRINTS_V: OnceLock<Value> = OnceLock::new();
    static MOA_V: OnceLock<Value> = OnceLock::new();
    if !matches!(
        path,
        "/api/config/schema" | "/api/model/auxiliary" | "/api/model/moa" | "/api/audio/elevenlabs/voices"
            | "/api/pairing" | "/api/cron/jobs" | "/api/cron/blueprints"
    ) {
        return None;
    }
    // `current` and `default` are this engine's own home; any other profile is Python's to resolve.
    let profile = crate::auth::query_param(query, "profile").unwrap_or_default();
    let scoped_ok = matches!(profile.as_str(), "" | "current" | "default") || (path == "/api/cron/jobs" && profile == "all");
    if !scoped_ok || managed_scope() {
        return None;
    }
    let home = factr_base::factr_config::home()?;
    let cfg = Config::load(&home)?;
    match path {
        "/api/config/schema" => schema_applies(&cfg, &home, defaults).then(|| golden(&SCHEMA_V, SCHEMA)),
        "/api/cron/blueprints" => cron_blueprints_apply(&cfg, &home).then(|| golden(&BLUEPRINTS_V, BLUEPRINTS)),
        "/api/model/moa" => cfg.yaml.get("moa").is_none().then(|| golden(&MOA_V, MOA)),
        "/api/model/auxiliary" => auxiliary(&cfg, defaults),
        "/api/audio/elevenlabs/voices" => no_elevenlabs_key(&cfg, &home).then(|| json!({ "available": false, "voices": [] })),
        "/api/pairing" => no_pairing_files(&home).then(|| json!({ "pending": [], "approved": [] })),
        "/api/cron/jobs" => no_cron_jobs(&home).then(|| json!([])),
        _ => None,
    }
}

/// Python layers a managed-scope config over config.yaml (`FACTR_MANAGED_DIR`, else `/etc/factr`).
fn managed_scope() -> bool {
    std::env::var_os("FACTR_MANAGED_DIR").is_some_and(|d| !d.is_empty()) || Path::new("/etc/factr").is_dir()
}

struct Config {
    yaml: Value,
}

impl Config {
    /// config.yaml as JSON; `None` when it holds `${VAR}` references, is unreadable or is not a plain mapping.
    fn load(home: &Path) -> Option<Self> {
        let yaml = match std::fs::read_to_string(home.join("config.yaml")) {
            Ok(text) => {
                if text.contains("${") {
                    return None;
                }
                match serde_yaml::from_str::<Value>(&text).ok()? {
                    Value::Null => json!({}),
                    v @ Value::Object(_) => v,
                    _ => return None,
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => json!({}),
            Err(_) => return None,
        };
        // External secret sources hydrate keys Python can see and this port cannot.
        if yaml.get("secrets").is_some() || yaml.get("vault").is_some() {
            return None;
        }
        Some(Config { yaml })
    }
}

fn deep_merge(base: &mut Value, over: &Value) {
    match (base, over) {
        (Value::Object(b), Value::Object(o)) => {
            for (k, v) in o {
                deep_merge(b.entry(k.clone()).or_insert(Value::Null), v);
            }
        }
        (b, o) => *b = o.clone(),
    }
}

/// Python `str(x or "")` for a JSON scalar; `None` for shapes whose Python spelling this does not model.
fn pystr(v: Option<&Value>) -> Option<String> {
    match v {
        None | Some(Value::Null) => Some(String::new()),
        Some(Value::String(s)) => Some(s.clone()),
        _ => None,
    }
}

/// `GET /api/model/auxiliary` from defaults + config.yaml (config only, no endpoint probing).
fn auxiliary(cfg: &Config, defaults: &Value) -> Option<Value> {
    if !defaults.is_object() || defaults.get("auxiliary").is_none() {
        return None;
    }
    // Root-level model keys and aliases that Python's loader rewrites are not modelled.
    let root = cfg.yaml.as_object()?;
    if ["provider", "base_url", "context_length", "max_turns"].iter().any(|k| root.contains_key(*k)) {
        return None;
    }
    match root.get("model") {
        None => {}
        Some(Value::Object(m)) => {
            if ["api_base", "model", "name"].iter().any(|k| m.contains_key(*k))
                || ["default", "provider"].iter().any(|k| m.get(*k).is_some_and(|v| !v.is_string() && !v.is_null()))
            {
                return None;
            }
        }
        Some(_) => return None,
    }
    let mut merged = defaults.clone();
    deep_merge(&mut merged, &cfg.yaml);
    let aux = merged.get("auxiliary").and_then(Value::as_object).cloned().unwrap_or_default();
    let mut tasks = Vec::new();
    for slot in AUX_SLOTS {
        let slot_cfg = aux.get(slot).and_then(Value::as_object).cloned().unwrap_or_default();
        // A pinned endpoint needs the local-address check; Python answers those.
        if !pystr(slot_cfg.get("base_url"))?.is_empty() {
            return None;
        }
        let provider = pystr(slot_cfg.get("provider"))?;
        let effort = pystr(slot_cfg.get("reasoning_effort"))?;
        tasks.push(json!({
            "task": slot,
            "provider": if provider.is_empty() { "auto".to_string() } else { provider },
            "model": pystr(slot_cfg.get("model"))?,
            "base_url": "",
            "reasoning_effort": if effort.is_empty() { Value::Null } else { json!(effort) },
            "local_endpoint": false,
        }));
    }
    let (model, provider) = match merged.get("model") {
        Some(Value::Object(m)) => (pystr(m.get("default").or_else(|| m.get("name")))?, pystr(m.get("provider"))?),
        Some(Value::String(s)) => (s.clone(), String::new()),
        _ => (String::new(), String::new()),
    };
    Some(json!({ "tasks": tasks, "main": { "provider": provider, "model": model } }))
}

/// No ElevenLabs key anywhere Python looks: the process env, the profile's `.env`.
fn no_elevenlabs_key(_cfg: &Config, home: &Path) -> bool {
    if std::env::var("ELEVENLABS_API_KEY").is_ok_and(|v| !v.trim().is_empty()) {
        return false;
    }
    match std::fs::read_to_string(home.join(".env")) {
        Ok(text) => !text.lines().any(|l| l.contains("ELEVENLABS_API_KEY")),
        Err(e) => e.kind() == std::io::ErrorKind::NotFound,
    }
}

fn dir_has_files(dir: &Path, suffix: &str) -> bool {
    std::fs::read_dir(dir).is_ok_and(|rd| rd.flatten().any(|e| e.file_name().to_string_lossy().ends_with(suffix)))
}

/// No pairing store files in either layout Python merges.
fn no_pairing_files(home: &Path) -> bool {
    [home.join("platforms/pairing"), home.join("pairing")]
        .iter()
        .all(|d| !dir_has_files(d, "-pending.json") && !dir_has_files(d, "-approved.json"))
}

/// The cron store of this home and of every profile under it has no jobs file.
fn no_cron_jobs(home: &Path) -> bool {
    let root = crate::boot_stubs::root_home(home);
    let mut homes = vec![root.clone()];
    if let Ok(rd) = std::fs::read_dir(root.join("profiles")) {
        homes.extend(rd.flatten().map(|e| e.path()));
    }
    homes.iter().all(|h| !h.join("cron/jobs.json").exists())
}

fn user_plugins(home: &Path) -> bool {
    std::fs::read_dir(home.join("plugins")).is_ok_and(|mut rd| rd.next().is_some())
}

/// The cron blueprints' delivery options come from the connected messaging platforms.
fn cron_blueprints_apply(cfg: &Config, home: &Path) -> bool {
    let env_key = |k: &str| PLATFORM_ENV.iter().any(|p| k.to_ascii_uppercase().contains(p));
    let env_text = std::fs::read_to_string(home.join(".env")).unwrap_or_default();
    let in_file = env_text.lines().filter_map(|l| l.split_once('=')).any(|(k, v)| env_key(k) && !v.trim().is_empty());
    let in_env = std::env::vars().any(|(k, v)| env_key(&k) && !v.trim().is_empty());
    let enabled = cfg.yaml.get("platforms").and_then(Value::as_object).is_some_and(|m| m.values().any(|p| p.get("enabled").is_some_and(|e| e != &json!(false))));
    !(in_file || in_env || enabled || user_plugins(home) || crate::features::messaging_enabled(home))
}

/// The schema is static except for provider choices Python discovers: command-type voice
/// providers, plugin memory providers, plugin terminal backends. A config that adds none is the baseline.
fn schema_applies(cfg: &Config, home: &Path, defaults: &Value) -> bool {
    if user_plugins(home) || !defaults.is_object() {
        return false;
    }
    let options = |key: &str| -> Vec<String> {
        let schema: &Value = SCHEMA_PARSED.get_or_init(|| serde_json::from_str(SCHEMA).expect("embedded golden is JSON"));
        schema["fields"][key]["options"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(|s| s.trim().to_lowercase())).collect()).unwrap_or_default()
    };
    for kind in ["tts", "stt"] {
        let Some(section) = cfg.yaml.get(kind) else { continue };
        let Some(section) = section.as_object() else { return false };
        // Any block that declares a `command` may be a command-type provider.
        let has_command = |m: &serde_json::Map<String, Value>| m.values().any(|v| v.as_object().is_some_and(|o| o.contains_key("command")));
        if has_command(section) || section.get("providers").is_some_and(|p| p.as_object().is_none_or(has_command)) {
            return false;
        }
        let chosen = match section.get("provider") {
            None | Some(Value::Null) => defaults[kind]["provider"].as_str().unwrap_or("").to_string(),
            Some(Value::String(s)) => s.clone(),
            Some(_) => return false,
        };
        if !chosen.trim().is_empty() && !options(&format!("{kind}.provider")).contains(&chosen.trim().to_lowercase()) {
            return false;
        }
    }
    match cfg.yaml.get("memory").and_then(|m| m.get("provider")) {
        None | Some(Value::Null) => {}
        Some(Value::String(s)) => {
            let s = s.trim().to_lowercase();
            if !s.is_empty() && s != "builtin" && !options("memory.provider").contains(&s) {
                return false;
            }
        }
        Some(_) => return false,
    }
    // A terminal backend added by a plugin is listed by Python; a value outside the baseline list may be one.
    if let Some(Value::String(b)) = cfg.yaml.get("terminal").and_then(|t| t.get("backend")) {
        if !options("terminal.backend").contains(&b.trim().to_lowercase()) {
            return false;
        }
    }
    true
}

static SCHEMA_PARSED: OnceLock<Value> = OnceLock::new();
