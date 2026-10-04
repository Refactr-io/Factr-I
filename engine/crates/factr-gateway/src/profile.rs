//! Defaults for the Factr profile selected by the desktop's `--profile` arg.
//! FACTR_CONFIG_HOME is scoped by the factr entrypoint before the gateway starts.

use serde_yaml::Value;
use std::path::PathBuf;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProfileDefaults {
    pub name: Option<String>,
    pub home: Option<PathBuf>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
    pub system_prompt: Option<String>,
}

pub fn current() -> ProfileDefaults {
    let name = std::env::var("FACTR_PROFILE")
        .ok()
        .filter(|s| !s.is_empty());
    let Some(home) = factr_base::factr_config::home() else {
        return ProfileDefaults { name, ..Default::default() };
    };
    let (provider, model, reasoning_effort) = std::fs::read_to_string(home.join("config.yaml"))
        .map(|raw| parse_config(&raw))
        .unwrap_or_default();
    let system_prompt = Some(persona(std::fs::read_to_string(home.join("SOUL.md")).ok().as_deref()));
    ProfileDefaults {
        name,
        home: Some(home),
        provider,
        model,
        reasoning_effort,
        system_prompt,
    }
}

/// The persona a chat starts with when SOUL.md is absent, empty, or still Factr's own seeded text.
pub const DEFAULT_PERSONA: &str = "You are Factr-I, a capable agent for coding and everyday work. Be direct: match the length of \
your reply to the weight of the ask, and report finished work briefly (what changed, what is verified, what is left). No filler, \
no restating the request. When unsure, say so plainly.";

/// The persona block for a SOUL.md body. Factr seeds SOUL.md on first start with "You are Factr-I, ..."
/// (or a comment-only scaffold); that text carries no user intent and would make
/// the agent introduce itself as someone else, so it is ignored in favour of [`DEFAULT_PERSONA`]. A
/// SOUL.md the user wrote is used as is.
pub fn persona(soul: Option<&str>) -> String {
    user_persona(soul).unwrap_or_else(|| DEFAULT_PERSONA.to_owned())
}

/// The persona a new chat starts with: interactive chats get [`persona`] (the user's SOUL.md, else
/// the Factr-I default); a headless run (`/api/agent/run`, cron, `FACTR_HEADLESS`) gets only a
/// SOUL.md the user wrote, and no persona block at all otherwise.
pub fn persona_for(headless: bool, soul: Option<&str>) -> Option<String> {
    if headless { user_persona(soul) } else { Some(persona(soul)) }
}

/// [`persona_for`] over the profile's SOUL.md.
pub fn new_session_persona(headless: bool) -> Option<String> {
    let soul = factr_base::factr_config::home().and_then(|home| std::fs::read_to_string(home.join("SOUL.md")).ok());
    persona_for(headless, soul.as_deref())
}

/// The SOUL.md text the user wrote; `None` when it is absent, empty, or Factr's own seeded text.
fn user_persona(soul: Option<&str>) -> Option<String> {
    let text = soul.map(str::trim).unwrap_or_default();
    let mut visible = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("<!--") {
        visible.push_str(&rest[..start]);
        match rest[start..].find("-->") {
            Some(end) => rest = &rest[start + end + 3..],
            None => rest = "",
        }
    }
    visible.push_str(rest);
    let visible = visible.trim();
    let seeded = visible.starts_with("You are Factr-I")
        || visible.lines().all(|l| l.trim().is_empty() || l.trim() == "# Factr-I Persona");
    (!seeded).then(|| text.to_owned())
}

/// The model and provider new sessions start on: the user's last saved pick (`model.default` /
/// `model.provider` in the profile's config.yaml, the keys Factr persists) when [`saved_pick_applies`],
/// else what the engine started with. The provider only follows the saved pick when it is one the
/// catalog knows, so a stale label never replaces a served one.
pub fn effective_default(config: &crate::Config) -> (String, String) {
    // The same file `config.set model` writes: FACTR_CONFIG_HOME's, else the engine home's stand-in.
    let dir = factr_base::factr_config::home().unwrap_or_else(|| PathBuf::from(&config.home));
    let (saved_provider, saved_model, _) = std::fs::read_to_string(dir.join("config.yaml")).map(|raw| parse_config(&raw)).unwrap_or_default();
    // Never the provider catalog's own default: with nothing saved, the model the engine started on.
    let startup = || (config.model.clone(), config.provider.clone());
    if !saved_pick_applies(config) {
        return startup();
    }
    match saved_model {
        Some(model) => {
            let provider = saved_provider
                .filter(|p| factr_base::provider_catalog::resolve_login_provider_loose(p).is_some())
                .unwrap_or_else(|| config.provider.clone());
            (model, provider)
        }
        None => startup(),
    }
}

/// Engine homes whose config.yaml pick was saved by THIS process (`persist_default_model`).
static PICKED_HERE: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// Record that this process saved the user's model pick, so later new chats follow it even when the
/// engine was started with an explicit model.
pub fn note_pick_saved(config: &crate::Config) {
    let mut picked = PICKED_HERE.lock().unwrap_or_else(|e| e.into_inner());
    if !picked.contains(&config.home) {
        picked.push(config.home.clone());
    }
}

/// Whether the profile's saved model is the user's to follow. An explicit `--model` / env model
/// (`profile_model_applies` false) outranks whatever config.yaml holds from an earlier run, so a
/// benchmark or private deployment always runs the model it was started with; only a pick the user
/// makes in THIS run (a later `config.set model`) replaces it. With no explicit model the saved
/// pick is the default, however it got there.
pub fn saved_pick_applies(config: &crate::Config) -> bool {
    config.profile_model_applies || PICKED_HERE.lock().unwrap_or_else(|e| e.into_inner()).contains(&config.home)
}

/// How a new chat's model was pinned.
#[derive(Debug, PartialEq, Eq)]
pub enum Pinned {
    /// The chosen model (and its provider, when known) is on.
    On { model: String, provider: Option<String> },
    /// A saved (not explicit) pick the engine could not serve: the chat runs on the startup model.
    FellBack { wanted: String, error: String, model: String, provider: String },
}

/// Pin a new chat to its model, the single owner of that decision. An explicit `model` (with its
/// `provider`) in `params` is the caller's; otherwise [`effective_default`] (the saved pick, else the
/// startup model). `set` sends the engine's `set_model` with the route-qualified name and is ALWAYS
/// called, so the chat never inherits whatever the provider happens to default to. An explicit pick
/// that fails is an error; a saved pick that fails falls back to the startup model.
pub async fn pin_new_chat_model<F, Fut>(config: &crate::Config, params: &serde_json::Value, mut set: F) -> anyhow::Result<Pinned>
where
    F: FnMut(String) -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<()>>,
{
    let route = |model: &str, provider: Option<&str>| {
        factr_base::provider::MultiProvider::model_switch_request_for_session_route(model, provider, params["route_api_method"].as_str())
    };
    let text = |key: &str| params[key].as_str().map(str::trim).filter(|v| !v.is_empty());
    if let Some(model) = text("model") {
        let provider = text("provider").or_else(|| factr_base::provider::provider_for_model(model));
        set(route(model, provider)).await?;
        return Ok(Pinned::On { model: model.to_string(), provider: provider.map(str::to_owned) });
    }
    let (model, provider) = effective_default(config);
    let error = match set(route(&model, Some(&provider))).await {
        Ok(()) => return Ok(Pinned::On { model, provider: Some(provider) }),
        Err(error) => error,
    };
    if model == config.model {
        return Err(error);
    }
    set(route(&config.model, Some(&config.provider)))
        .await
        .map_err(|startup| anyhow::anyhow!("the saved model {model} failed ({error:#}) and so did the startup model {}: {startup:#}", config.model))?;
    Ok(Pinned::FellBack { wanted: model, error: format!("{error:#}"), model: config.model.clone(), provider: config.provider.clone() })
}

/// What the profile may fill in at boot.
#[derive(Debug, PartialEq, Eq)]
pub struct BootPick<'a> {
    /// A provider from config.yaml that overrides the CLI's (`None`: keep the CLI's / auto).
    pub profile_provider: Option<&'a str>,
    /// `--model` without `--provider`: the provider that model belongs to. The saved provider never
    /// stands in for it, whatever config.yaml says (`None`: unknown model, the CLI's / auto).
    pub model_provider: Option<&'static str>,
    /// The model to start on (`None`: the provider's own choice).
    pub model: Option<&'a str>,
    /// Becomes `Config::profile_model_applies`.
    pub profile_model_applies: bool,
}

/// Boot-time precedence: an explicit CLI/env model always wins, and with an explicit provider too
/// the profile fills in nothing. Otherwise an explicit provider is not overridden by a profile that
/// only says "auto" (and then does not borrow that profile's model), and a profile provider/model
/// is what the user last picked. A bare `--model` takes its provider from the model id.
pub fn boot_pick<'a>(cli_explicit: bool, cli_model: Option<&'a str>, profile: &'a ProfileDefaults) -> BootPick<'a> {
    let cli_model = cli_model.filter(|m| !m.trim().is_empty());
    let profile_provider = profile
        .provider
        .as_deref()
        .filter(|p| !(cli_explicit && p.eq_ignore_ascii_case("auto")))
        .filter(|_| cli_model.is_none());
    let model_provider = cli_model
        .filter(|_| !cli_explicit)
        .and_then(factr_base::provider::provider_for_model);
    let model = if cli_model.is_some() || (cli_explicit && profile_provider.is_none()) {
        cli_model
    } else {
        profile.model.as_deref().or(cli_model)
    };
    BootPick {
        profile_provider,
        model_provider,
        model,
        profile_model_applies: !(cli_model.is_some() || (cli_explicit && profile_provider.is_none())),
    }
}

pub fn parse_config(raw: &str) -> (Option<String>, Option<String>, Option<String>) {
    let config = serde_yaml::from_str::<Value>(raw).unwrap_or(Value::Null);
    let model = &config["model"];
    let model_name = model["default"]
        .as_str()
        .or_else(|| model["model"].as_str())
        .or_else(|| model["name"].as_str())
        .or_else(|| model["default"]["model"].as_str())
        .or_else(|| model["default"]["default"].as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    let provider = model["provider"]
        .as_str()
        .or_else(|| model["default"]["provider"].as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    let effort = config["agent"]["reasoning_effort"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    (provider, model_name, effort)
}

/// The `GET /api/mcp/servers` body from `mcp_servers` in a Factr config.yaml, the
/// same map the engine's MCP client loads. Env values are never echoed.
pub fn mcp_servers_body(raw: &str) -> serde_json::Value {
    let config = serde_yaml::from_str::<Value>(raw).unwrap_or(Value::Null);
    let mut rows: Vec<_> = config["mcp_servers"]
        .as_mapping()
        .into_iter()
        .flatten()
        .filter_map(|(name, cfg)| Some((name.as_str()?.to_owned(), cfg)))
        .collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    let servers: Vec<_> = rows
        .into_iter()
        .map(|(name, cfg)| {
            let (url, command) = (cfg["url"].as_str(), cfg["command"].as_str());
            let env: serde_json::Map<_, _> = cfg["env"]
                .as_mapping()
                .into_iter()
                .flatten()
                .filter_map(|(k, _)| Some((k.as_str()?.to_owned(), "***".into())))
                .collect();
            serde_json::json!({
                "name": name,
                "transport": if url.is_some() { "http" } else if command.is_some() { "stdio" } else { "unknown" },
                "url": url, "command": command,
                "args": cfg["args"].as_sequence().map(|a| a.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>()).unwrap_or_default(),
                "env": env, "auth": cfg["auth"].as_str(),
                "enabled": !matches!(cfg["enabled"].as_bool(), Some(false)),
                "tools": cfg["tools"].as_sequence().map(|t| t.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>()),
                "source": "config", "plugin": null,
            })
        })
        .collect();
    serde_json::json!({ "servers": servers })
}

#[cfg(test)]
mod tests {
    use super::{effective_default, mcp_servers_body, parse_config, persona, persona_for, DEFAULT_PERSONA};

    #[test]
    fn a_headless_run_has_no_default_persona_but_honours_a_written_soul() {
        let seeded = "You are Factr-I, a helpful agent. Be direct.";
        for soul in [None, Some(""), Some(seeded), Some("# Factr-I Persona\n\n<!--\nedit me\n-->")] {
            assert_eq!(persona_for(true, soul), None, "no SOUL.md, no persona block");
            assert_eq!(persona_for(false, soul).as_deref(), Some(DEFAULT_PERSONA), "interactive keeps the default");
        }
        assert_eq!(persona_for(true, Some(" You are a pirate.\n")).as_deref(), Some("You are a pirate."));
        assert_eq!(persona_for(false, Some("You are a pirate.")).as_deref(), Some("You are a pirate."));
    }

    #[test]
    fn factr_seeded_soul_is_replaced_by_the_factr_i_persona() {
        let seeded = "You are Factr-I, a helpful agent. Be direct.";
        for soul in [None, Some(""), Some(seeded), Some("# Factr-I Persona\n\n<!--\nedit me\n-->")] {
            assert_eq!(persona(soul), DEFAULT_PERSONA);
        }
        assert_eq!(persona(Some("  You are a pirate.\n")), "You are a pirate.");
        assert!(DEFAULT_PERSONA.contains("Factr-I"));
    }

    /// A gateway config started with `--provider openai --model gpt-5.6-luna` (`explicit`) or with nothing explicit.
    fn config_in(dir: &std::path::Path, explicit: bool) -> crate::Config {
        crate::Config {
            bind: "127.0.0.1:0".parse().unwrap(), token: "t".repeat(32), version: "test".into(), legacy_socket: dir.join("s"),
            default_cwd: "/".into(), allow_non_loopback: false, provider: "openai".into(), model: "gpt-5.6-luna".into(), reasoning_efforts: Vec::new(),
            profile_model_applies: !explicit, home: dir.to_string_lossy().into(), complete: None, features: None, learning: None,
        }
    }

    /// Run `body` with FACTR_CONFIG_HOME at a fresh temp dir (under the env lock).
    fn with_home<T>(name: &str, body: impl FnOnce(&std::path::Path) -> T) -> T {
        let _env = crate::factr_env::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("profile-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let before = std::env::var_os("FACTR_CONFIG_HOME");
        // SAFETY: env is only touched under ENV_LOCK.
        unsafe { std::env::set_var("FACTR_CONFIG_HOME", &dir) };
        let out = body(&dir);
        match before { Some(v) => unsafe { std::env::set_var("FACTR_CONFIG_HOME", v) }, None => unsafe { std::env::remove_var("FACTR_CONFIG_HOME") } }
        out
    }

    #[test]
    fn an_explicit_model_is_the_default_until_this_process_saves_a_pick_whatever_config_yaml_holds() {
        with_home("explicit-default", |dir| {
            let config = config_in(dir, true);
            let luna = ("gpt-5.6-luna".to_string(), "openai".to_string());
            assert_eq!(effective_default(&config), luna, "nothing saved: the startup model, never the catalog default");
            // A leftover pick from an earlier run (it names its provider) does not override the flags.
            std::fs::write(dir.join("config.yaml"), "model:\n  default: gpt-6-astra\n  provider: openai\n").unwrap();
            assert_eq!(effective_default(&config), luna);
            // The user picks a model in this run: that pick is the default for later new chats.
            std::fs::write(dir.join("config.yaml"), "model:\n  default: gpt-5.6-sol\n  provider: openai\n").unwrap();
            super::note_pick_saved(&config);
            assert_eq!(effective_default(&config), ("gpt-5.6-sol".to_string(), "openai".to_string()));
        });
    }

    #[test]
    fn with_no_explicit_model_the_saved_pick_is_the_default_however_it_got_there() {
        with_home("saved-default", |dir| {
            let config = config_in(dir, false);
            std::fs::write(dir.join("config.yaml"), "model:\n  default: gpt-5.6-sol\n  provider: openai\n").unwrap();
            assert_eq!(effective_default(&config), ("gpt-5.6-sol".to_string(), "openai".to_string()));
        });
    }

    #[test]
    fn explicit_flags_win_at_boot_over_a_config_yaml_naming_another_model_and_provider() {
        use super::{BootPick, ProfileDefaults, boot_pick};
        let saved = ProfileDefaults { provider: Some("anthropic".into()), model: Some("claude-fable-5".into()), ..Default::default() };
        // --provider openai --model gpt-5.6-luna
        assert_eq!(
            boot_pick(true, Some("gpt-5.6-luna"), &saved),
            BootPick { profile_provider: None, model_provider: None, model: Some("gpt-5.6-luna"), profile_model_applies: false }
        );
        // --model alone: the model still wins and new chats do not follow the file
        let only_model = boot_pick(false, Some("gpt-5.6-luna"), &saved);
        assert_eq!((only_model.model, only_model.profile_model_applies), (Some("gpt-5.6-luna"), false));
        // ...and its provider comes from the model id, never from config.yaml's anthropic
        assert_eq!((only_model.profile_provider, only_model.model_provider), (None, Some("openai")));
        assert_eq!(boot_pick(false, Some("claude-fable-5"), &ProfileDefaults { provider: Some("openai".into()), ..Default::default() }).model_provider, Some("claude"));
        // a model id no provider claims leaves the provider to the CLI's default, not to config.yaml
        let unknown = boot_pick(false, Some("my-local-model"), &saved);
        assert_eq!((unknown.profile_provider, unknown.model_provider), (None, None));
        // an empty FACTR_MODEL is not an explicit model
        assert_eq!(boot_pick(false, Some("  "), &saved).model, Some("claude-fable-5"));
        // nothing explicit: the saved pick fills everything in
        assert_eq!(
            boot_pick(false, None, &saved),
            BootPick { profile_provider: Some("anthropic"), model_provider: None, model: Some("claude-fable-5"), profile_model_applies: true }
        );
        // an explicit provider with a profile that only says auto keeps its own model choice
        let auto = ProfileDefaults { provider: Some("auto".into()), model: Some("cloud-model".into()), ..Default::default() };
        assert_eq!(boot_pick(true, None, &auto), BootPick { profile_provider: None, model_provider: None, model: None, profile_model_applies: false });
    }

    /// A stand-in engine: records every `set_model` and refuses the models in `refuse`.
    fn engine(refuse: &'static [&'static str]) -> (std::sync::Arc<std::sync::Mutex<Vec<String>>>, impl FnMut(String) -> std::future::Ready<anyhow::Result<()>>) {
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = seen.clone();
        (seen, move |model: String| {
            log.lock().unwrap().push(model.clone());
            std::future::ready(if refuse.iter().any(|r| model.contains(r)) { Err(anyhow::anyhow!("Unsupported model {model}")) } else { Ok(()) })
        })
    }

    #[tokio::test]
    async fn a_new_chat_is_always_pinned_to_a_model_the_startup_one_when_nothing_is_saved() {
        let (config, dir) = with_home("pin-startup", |dir| (config_in(dir, true), dir.to_path_buf()));
        let _env = crate::factr_env::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        unsafe { std::env::set_var("FACTR_CONFIG_HOME", &dir) };
        let (seen, set) = engine(&[]);
        let pinned = super::pin_new_chat_model(&config, &serde_json::json!({}), set).await.unwrap();
        assert_eq!(pinned, super::Pinned::On { model: "gpt-5.6-luna".into(), provider: Some("openai".into()) });
        assert_eq!(seen.lock().unwrap().len(), 1, "set_model is sent even with nothing saved, never left to the provider default");
        assert!(seen.lock().unwrap()[0].contains("gpt-5.6-luna"));
        unsafe { std::env::remove_var("FACTR_CONFIG_HOME") };
    }

    #[tokio::test]
    async fn an_explicit_chat_model_wins_and_its_failure_is_an_error_not_a_substitution() {
        let (config, dir) = with_home("pin-explicit", |dir| (config_in(dir, true), dir.to_path_buf()));
        let _env = crate::factr_env::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        unsafe { std::env::set_var("FACTR_CONFIG_HOME", &dir) };
        std::fs::write(dir.join("config.yaml"), "model:\n  default: gpt-5.6-sol\n  provider: openai\n").unwrap();
        let (seen, set) = engine(&[]);
        let params = serde_json::json!({ "model": "gpt-5.6-terra", "provider": "openai" });
        let pinned = super::pin_new_chat_model(&config, &params, set).await.unwrap();
        assert!(matches!(pinned, super::Pinned::On { ref model, .. } if model == "gpt-5.6-terra"));
        assert_eq!(seen.lock().unwrap().len(), 1);
        let (seen, set) = engine(&["terra"]);
        assert!(super::pin_new_chat_model(&config, &params, set).await.is_err(), "no fallback for an explicit pick");
        assert_eq!(seen.lock().unwrap().len(), 1, "and no second model is ever tried");
        unsafe { std::env::remove_var("FACTR_CONFIG_HOME") };
    }

    #[tokio::test]
    async fn a_saved_pick_the_engine_cannot_serve_falls_back_to_the_startup_model() {
        let (config, dir) = with_home("pin-fallback", |dir| (config_in(dir, false), dir.to_path_buf()));
        let _env = crate::factr_env::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        unsafe { std::env::set_var("FACTR_CONFIG_HOME", &dir) };
        std::fs::write(dir.join("config.yaml"), "model:\n  default: gpt-5.6-sol\n  provider: openai\n").unwrap();
        let (seen, set) = engine(&["sol"]);
        match super::pin_new_chat_model(&config, &serde_json::json!({}), set).await.unwrap() {
            super::Pinned::FellBack { wanted, model, .. } => assert_eq!((wanted.as_str(), model.as_str()), ("gpt-5.6-sol", "gpt-5.6-luna")),
            other => panic!("{other:?}"),
        }
        assert_eq!(seen.lock().unwrap().len(), 2);
        // a startup model that fails too is an error, not a third guess
        let (_, set) = engine(&["sol", "luna"]);
        assert!(super::pin_new_chat_model(&config, &serde_json::json!({}), set).await.is_err());
        unsafe { std::env::remove_var("FACTR_CONFIG_HOME") };
    }

    #[test]
    fn mcp_list_mirrors_config_yaml_without_env_values() {
        let body = mcp_servers_body("mcp_servers:\n  b:\n    url: http://x/mcp\n    enabled: false\n  a:\n    command: /bin/a\n    args: [--x]\n    env: {K: secret}\n");
        assert_eq!(body["servers"][0]["name"], "a");
        assert_eq!(body["servers"][0]["transport"], "stdio");
        assert_eq!(body["servers"][0]["env"]["K"], "***");
        assert_eq!(body["servers"][1]["enabled"], false);
        assert_eq!(mcp_servers_body("")["servers"], serde_json::json!([]));
    }

    #[test]
    fn reads_factr_main_model_and_effort_shapes() {
        assert_eq!(
            parse_config(
                "model:\n  provider: ollama\n  default: qwen3\nagent:\n  reasoning_effort: low\n"
            ),
            (
                Some("ollama".into()),
                Some("qwen3".into()),
                Some("low".into())
            )
        );
        assert_eq!(
            parse_config("model:\n  default:\n    provider: ollama\n    model: qwen3\n"),
            (Some("ollama".into()), Some("qwen3".into()), None)
        );
    }
}
