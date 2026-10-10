//! `setup.status`, `setup.runtime_check` and `model.options` from the engine's real
//! credential state (Factr `.env` first, then factr's own auth), in Factr's shapes.

use factr_base::auth::AuthStatus;
use factr_base::provider_catalog::{
    LoginProviderDescriptor, LoginProviderTarget, load_api_key, login_providers, resolve_login_provider_loose,
};
use serde_json::{Value, json};

/// The factr-only row (the credential importer) is not an engine provider a
/// user picks a model from: one provider catalog, the one Factr logs in to.
fn is_factr_only(d: LoginProviderDescriptor) -> bool {
    matches!(d.target, LoginProviderTarget::AutoImport)
}

/// Whether Factr (which owns login and the credential pool) has this provider, under its own id or an
/// alias. Only these are offered to sign in to; a provider already signed in stays listed regardless.
fn factr_knows(d: LoginProviderDescriptor) -> bool {
    matches!(
        d.id,
        "claude" | "anthropic-api" | "openai" | "openai-api" | "openrouter" | "copilot" | "gemini" | "gemini-api" | "google"
            | "bedrock" | "azure" | "opencode" | "opencode-go" | "zai" | "kimi" | "moonshotai" | "deepseek" | "fireworks"
            | "huggingface" | "lmstudio" | "minimax" | "nebius" | "novita" | "xai" | "nvidia-nim" | "xiaomi-mimo"
            | "meta-muse" | "alibaba-coding-plan" | "deepinfra" | "ollama" | "openai-compatible"
    )
}

/// The provider id the Factr runtime (which owns `/api/model/set` and config.yaml's saved pick) takes
/// for an engine catalog id: a different spelling for the few that differ, `None` for an engine
/// provider the runtime has no route for (chats can still use it, but it cannot be saved as a
/// default there), and any other id unchanged.
pub(crate) fn runtime_provider_id(id: &str) -> Option<&str> {
    match id.trim().to_ascii_lowercase().as_str() {
        "openai" => Some("openai-codex"),
        "claude" | "anthropic-api" => Some("anthropic"),
        "gemini-api" => Some("gemini"),
        "meta-muse" => Some("muse"),
        "302ai" | "baseten" | "belvedir" | "celeris" | "cerebras" | "chutes" | "comtegra" | "conifer"
        | "cortecs" | "cursor" | "firmware" | "fpt" | "grok-build" | "groq" | "mistral" | "moonshotai"
        | "openai-compatible" | "orcarouter" | "perplexity" | "scaleway" | "stackit" | "togetherai"
        | "yolo-auto" => None,
        _ => Some(id),
    }
}

/// The provider id to save in config.yaml for an engine pick: the runtime's id, so the Factr runtime
/// can read the saved default too. The engine's `openai` covers the ChatGPT login and an API key; with
/// only a key it is the runtime's `openai-api`. An engine-only provider keeps its engine id (the runtime
/// has no route for it either way). The engine reads every one of these back (catalog aliases), at boot too.
pub(crate) fn runtime_saved_provider(id: &str) -> String {
    if id.trim().eq_ignore_ascii_case("openai") {
        let status = AuthStatus::check_fast();
        let chatgpt = resolve_login_provider_loose("openai").is_some_and(|d| has_login(d, &status));
        if !chatgpt && load_api_key("OPENAI_API_KEY").is_some() {
            return "openai-api".to_string();
        }
    }
    // `gemini-api` stays: the runtime's `gemini` boots the engine's Gemini CLI runtime instead.
    if id.trim().eq_ignore_ascii_case("gemini-api") {
        return id.to_string();
    }
    runtime_provider_id(id).unwrap_or(id).to_string()
}

/// The catalog the engine offers: `login_providers()` without the factr-only rows.
fn offered() -> Vec<LoginProviderDescriptor> {
    login_providers().iter().copied().filter(|d| !is_factr_only(*d)).collect()
}

fn authenticated(d: LoginProviderDescriptor, status: &AuthStatus) -> bool {
    match d.target {
        LoginProviderTarget::OpenAiCompatible(p) => {
            !p.requires_api_key || load_api_key(p.api_key_env).is_some()
        }
        // The served OpenAI / Claude provider runs on either credential: a login or the API key.
        LoginProviderTarget::OpenAi => has_login(d, status) || load_api_key("OPENAI_API_KEY").is_some(),
        LoginProviderTarget::Claude => has_login(d, status) || load_api_key("ANTHROPIC_API_KEY").is_some(),
        _ => has_login(d, status),
    }
}

fn has_login(d: LoginProviderDescriptor, status: &AuthStatus) -> bool {
    status.state_for_provider(d) != factr_base::auth::AuthState::NotConfigured
}

fn default_model(d: LoginProviderDescriptor) -> Option<&'static str> {
    match d.target {
        LoginProviderTarget::OpenAiCompatible(p) => p.default_model,
        _ => None,
    }
}

/// Every model the engine can call for this provider, from the same catalog a live
/// session lists (`known_*_model_ids`: the account's cached catalog, else the static
/// list). Only providers with a real catalog are listed here; a provider the engine
/// has no model list for stays empty rather than advertising something it cannot call.
fn catalog_models(d: LoginProviderDescriptor) -> Vec<String> {
    match d.target {
        LoginProviderTarget::OpenAi | LoginProviderTarget::OpenAiApiKey => factr_base::provider::known_openai_model_ids(),
        LoginProviderTarget::Claude | LoginProviderTarget::ClaudeApiKey => factr_base::provider::known_anthropic_model_ids(),
        _ => default_model(d).map(str::to_string).into_iter().collect(),
    }
}

fn without_swarm(efforts: impl IntoIterator<Item = impl AsRef<str>>) -> Vec<String> {
    efforts.into_iter().map(|e| e.as_ref().to_string()).filter(|e| !factr_base::prompt::is_swarm_effort(e)).collect()
}

/// Efforts the engine can state exactly for `model` (OpenAI and Anthropic, the two
/// providers whose per-model ladders live in the engine); `None` for any other route,
/// where only the live provider knows.
pub(super) fn known_efforts(provider: &str, model: &str) -> Option<Vec<String>> {
    let family = match resolve_login_provider_loose(provider).map(|d| d.target) {
        Some(LoginProviderTarget::OpenAi | LoginProviderTarget::OpenAiApiKey) => "openai",
        Some(LoginProviderTarget::Claude | LoginProviderTarget::ClaudeApiKey) => "anthropic",
        _ => return None,
    };
    let canonical = factr_provider_core::model_id::canonical(model);
    if family == "anthropic" {
        return Some(without_swarm(factr_provider_core::inferred_reasoning_efforts(Some("anthropic"), Some(&canonical))));
    }
    // The OpenAI provider's own ladders: platform-only GPT Pro models are pinned
    // (gpt-5-pro takes only `high`), others use what the account's catalog advertises.
    if factr_provider_core::is_openai_api_only_pro_model(&canonical) {
        return Some(if canonical.starts_with("gpt-5-pro") { vec!["high".into()] } else { vec!["medium".into(), "high".into(), "xhigh".into()] });
    }
    // The cache is keyed by the id the account catalog served; compare both sides in the one
    // normalised form so a differently-cased or `[1m]`-suffixed key still hits.
    let advertised = factr_base::provider::cached_openai_reasoning_efforts()
        .and_then(|m| m.into_iter().find(|(k, _)| factr_provider_core::model_id::canonical(k) == canonical).map(|(_, v)| v));
    Some(match advertised {
        Some(list) => list.iter().filter_map(|e| factr_provider_core::canonical_reasoning_effort(e)).map(str::to_string).collect(),
        // No catalog yet (fresh home): the gpt-6 and gpt-5.6 families take the provider's own
        // ladder (a benchmark may set xhigh before any catalog is cached); any other model gets
        // only the levels every reasoning model accepts. `minimal` and `none` stay out of both.
        None if canonical.starts_with("gpt-6") || canonical.starts_with("gpt-5.6") => FULL_OPENAI_EFFORTS.iter().map(|e| e.to_string()).collect(),
        None => CONSERVATIVE_OPENAI_EFFORTS.iter().map(|e| e.to_string()).collect(),
    })
}

/// What an OpenAI reasoning model takes when the account catalog has not told us more.
const CONSERVATIVE_OPENAI_EFFORTS: [&str; 3] = ["low", "medium", "high"];

/// The ladder the account catalog advertises for the gpt-6 and gpt-5.6 families.
const FULL_OPENAI_EFFORTS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];

/// One entry per model: ids that differ only by case or the `[1m]` suffix are the same
/// model. The id the engine serves wins its slot, else the first spelling listed.
fn dedupe_models(models: Vec<String>, served: &str) -> Vec<String> {
    use factr_provider_core::model_id::canonical;
    let mut out: Vec<String> = Vec::with_capacity(models.len());
    for m in models {
        match out.iter().position(|o| canonical(o) == canonical(&m)) {
            Some(i) => {
                if m == served {
                    out[i] = m;
                }
            }
            None => out.push(m),
        }
    }
    out
}

/// The efforts `model` takes on `provider`: the exact ladder when the engine knows it,
/// else what the served provider reported for the served model, else an inference.
pub(super) fn efforts_for_model(provider: &str, model: &str, served_model: &str, served: &[String]) -> Vec<String> {
    known_efforts(provider, model).unwrap_or_else(|| {
        if model == served_model {
            without_swarm(served)
        } else {
            without_swarm(factr_provider_core::inferred_reasoning_efforts(Some(provider), Some(model)))
        }
    })
}

/// `Err(message)` when `value` is an effort the engine knows `model` cannot take.
/// Routes it cannot judge pass (the live provider rejects them itself).
pub(super) fn check_effort(provider: &str, model: &str, value: &str) -> Result<(), String> {
    let Some(efforts) = known_efforts(provider, model) else { return Ok(()) };
    let swarm = factr_base::prompt::is_swarm_effort(value);
    let wanted = factr_provider_core::canonical_reasoning_effort(value);
    if (swarm && !efforts.is_empty()) || wanted.is_some_and(|w| efforts.iter().any(|e| e == w)) {
        return Ok(());
    }
    Err(if efforts.is_empty() {
        format!("{model} does not take a reasoning effort")
    } else {
        format!("reasoning effort '{}' is not supported by {model} (supported: {})", value.trim(), efforts.join(", "))
    })
}

/// The served provider has usable credentials. A label the catalog does not know
/// (a named profile, an external runtime) counts when anything is configured.
pub(super) fn configured(provider: &str) -> bool {
    let status = AuthStatus::check_fast();
    match resolve_login_provider_loose(provider) {
        Some(d) => authenticated(d, &status),
        None => offered().iter().any(|d| authenticated(*d, &status)),
    }
}

fn other_configured(provider: &str) -> bool {
    let status = AuthStatus::check_fast();
    let current = resolve_login_provider_loose(provider).map(|d| d.id);
    offered().iter().any(|d| Some(d.id) != current && authenticated(*d, &status))
}

pub(super) fn setup_status(provider: &str) -> Value {
    json!({
        "provider_configured": configured(provider), "ready": true,
        "other_providers": other_configured(provider), "inference_provider": provider,
    })
}

/// `requested`: an explicit provider to check strictly, else the served one.
pub(super) fn runtime_check(provider: &str, model: &str, requested: Option<&str>) -> Value {
    let (provider, model) = match requested {
        Some(r) if resolve_login_provider_loose(r).map(|d| d.id) != resolve_login_provider_loose(provider).map(|d| d.id) => {
            (r, resolve_login_provider_loose(r).and_then(default_model).unwrap_or(model))
        }
        _ => (provider, model),
    };
    if configured(provider) {
        json!({ "ok": true, "provider": provider, "model": model, "source": "engine" })
    } else {
        json!({ "ok": false, "provider": provider, "model": model, "source": "engine", "error": format!("No usable credentials found for {provider}.") })
    }
}

/// The session's catalog lists every route's models; a first-party row keeps only its own family's, so
/// Claude ids never show under OpenAI (or the reverse). The served model always stays.
fn only_own_family(d: LoginProviderDescriptor, models: Vec<String>, served: &str) -> Vec<String> {
    use factr_provider_core::model_id::canonical;
    let foreign = match d.target {
        LoginProviderTarget::OpenAi | LoginProviderTarget::OpenAiApiKey => factr_base::provider::known_anthropic_model_ids(),
        LoginProviderTarget::Claude | LoginProviderTarget::ClaudeApiKey => factr_base::provider::known_openai_model_ids(),
        _ => return models,
    };
    let claude_row = matches!(d.target, LoginProviderTarget::Claude | LoginProviderTarget::ClaudeApiKey);
    models
        .into_iter()
        .filter(|m| {
            m == served
                || !(foreign.iter().any(|f| canonical(f) == canonical(m)) || (!claude_row && m.to_lowercase().starts_with("claude")))
        })
        .collect()
}

/// Provider rows: the served one (with `current_models` when known), every provider with
/// credentials, and with `include_unconfigured` the rest of the catalog.
pub(super) fn model_options(provider: &str, model: &str, current_models: Vec<String>, include_unconfigured: bool, efforts: &[String], served_model: &str) -> Value {
    let status = AuthStatus::check_fast();
    let current_id = resolve_login_provider_loose(provider).filter(|d| !is_factr_only(*d)).map(|d| d.id);
    let mut rows: Vec<Value> = offered()
        .iter()
        .filter_map(|d| {
            let (is_current, auth) = (Some(d.id) == current_id, authenticated(*d, &status));
            if !(is_current || auth || (include_unconfigured && factr_knows(*d))) {
                return None;
            }
            let models: Vec<String> = dedupe_models(
                if is_current && !current_models.is_empty() {
                    only_own_family(*d, current_models.clone(), model)
                } else {
                    let mut listed = catalog_models(*d);
                    if is_current && !listed.iter().any(|m| m == model) {
                        listed.insert(0, model.to_string());
                    }
                    listed
                },
                model,
            );
            // `settable`: a pick here can be saved as the default (`/api/model/set`); pickers that save hide the rest.
            let mut row = json!({ "slug": d.id, "name": d.display_name, "total_models": models.len(), "models": models, "is_current": is_current, "authenticated": auth, "settable": runtime_provider_id(d.id).is_some() });
            if !models.is_empty() {
                row["capabilities"] = served_capabilities(d.id, &models, efforts, served_model);
            }
            Some(row)
        })
        .collect();
    if current_id.is_none() {
        // Named profile or runtime the catalog has no row for: it is what the engine serves.
        let models = dedupe_models(if current_models.is_empty() { vec![model.to_string()] } else { current_models }, model);
        let capabilities = served_capabilities(provider, &models, efforts, served_model);
        rows.insert(0, json!({ "slug": provider, "name": provider, "total_models": models.len(), "models": models, "is_current": true, "authenticated": configured(provider), "capabilities": capabilities }));
    }
    rows.sort_by_key(|r| r["is_current"] != true);
    json!({ "providers": rows, "model": model, "provider": provider })
}

/// What each model of a row can honour: reasoning (and switching it off) only when it
/// accepts effort levels, with the exact `efforts` it takes. One boolean for every model
/// made the desktop offer all seven levels everywhere, and a level the route could not
/// serve was reported as set while the session kept the old one.
fn served_capabilities(provider: &str, models: &[String], served: &[String], served_model: &str) -> Value {
    Value::Object(
        models
            .iter()
            .map(|m| {
                let efforts = efforts_for_model(provider, m, served_model, served);
                let caps = json!({ "fast": false, "reasoning": !efforts.is_empty(), "can_disable_reasoning": efforts.iter().any(|e| e == "none"), "efforts": efforts });
                (m.clone(), caps)
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn isolated<T>(f: impl FnOnce() -> T) -> T {
        let _lock = crate::factr_env::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("provider-state-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("factr")).unwrap();
        // SAFETY: env is only touched under ENV_LOCK.
        unsafe {
            std::env::set_var("FACTR_HOME", dir.join("factr"));
            std::env::set_var("FACTR_CONFIG_HOME", dir.join("factr"));
            std::env::remove_var("GROQ_API_KEY");
        }
        crate::factr_env::register();
        factr_base::provider::models::reset_model_catalog_services_for_tests();
        let out = f();
        factr_base::provider::models::reset_model_catalog_services_for_tests();
        unsafe {
            std::env::remove_var("FACTR_HOME");
            std::env::remove_var("FACTR_CONFIG_HOME");
        }
        let _ = std::fs::remove_dir_all(dir);
        out
    }

    #[test]
    fn rows_say_whether_a_pick_can_be_saved_as_the_default() {
        isolated(|| {
            // Fresh install of an older build: the served row is an engine-only API-key provider.
            let options = model_options("yolo-auto", "yolo", Vec::new(), true, &[], "yolo");
            let row = |slug: &str| options["providers"].as_array().unwrap().iter().find(|r| r["slug"] == slug).cloned();
            assert_eq!(row("yolo-auto").unwrap()["settable"], json!(false));
            assert_eq!(row("openai").unwrap()["settable"], json!(true));
            assert_eq!(row("claude").unwrap()["settable"], json!(true));
        });
        assert_eq!(runtime_provider_id("openai"), Some("openai-codex"));
        assert_eq!(runtime_provider_id("OpenAI"), Some("openai-codex"));
        assert_eq!(runtime_provider_id("cursor"), None);
        assert_eq!(runtime_provider_id("openrouter"), Some("openrouter"));
    }

    #[test]
    fn a_saved_default_uses_the_runtimes_id_and_the_engine_reads_it_back() {
        isolated(|| {
            let key = std::env::var_os("OPENAI_API_KEY");
            // SAFETY: env is only touched under ENV_LOCK (held by `isolated`).
            unsafe { std::env::remove_var("OPENAI_API_KEY") };
            let codex = runtime_saved_provider("openai");
            unsafe { std::env::set_var("OPENAI_API_KEY", "sk-test-not-real") };
            let key_only = runtime_saved_provider("OpenAI");
            match key {
                Some(key) => unsafe { std::env::set_var("OPENAI_API_KEY", key) },
                None => unsafe { std::env::remove_var("OPENAI_API_KEY") },
            }
            assert_eq!((codex.as_str(), key_only.as_str()), ("openai-codex", "openai-api"));
            assert_eq!(runtime_saved_provider("Claude"), "anthropic");
            assert_eq!(runtime_saved_provider("groq"), "groq", "engine-only: kept, the runtime cannot use it anyway");
            // Every id written here resolves to the engine provider it came from.
            assert_eq!(runtime_saved_provider("gemini-api"), "gemini-api");
            for (saved, engine) in [("openai-codex", "openai"), ("openai-api", "openai-api"), ("anthropic", "claude"), ("muse", "meta-muse")] {
                assert_eq!(resolve_login_provider_loose(saved).map(|d| d.id), Some(engine), "{saved}");
            }
        });
    }

    #[test]
    fn the_served_row_reports_reasoning_only_when_the_provider_takes_efforts() {
        isolated(|| {
            let none = model_options("ollama", "qwen3", vec!["qwen3".into()], false, &[], "qwen3");
            let caps = &none["providers"][0]["capabilities"]["qwen3"];
            assert_eq!((caps["reasoning"].clone(), caps["can_disable_reasoning"].clone()), (json!(false), json!(false)));

            let efforts = ["low".to_string(), "high".to_string()];
            let some = model_options("ollama", "qwen3", vec!["qwen3".into()], false, &efforts, "qwen3");
            assert_eq!(some["providers"][0]["capabilities"]["qwen3"]["reasoning"], json!(true));
        });
    }

    #[test]
    fn a_sessions_mixed_catalog_never_puts_claude_models_under_openai() {
        isolated(|| {
            let mixed = vec!["gpt-5.6-luna".to_string(), "claude-opus-5-5".to_string(), "claude-sonnet-4-6".to_string(), "gpt-5.6-sol".to_string()];
            let options = model_options("openai", "gpt-5.6-luna", mixed, false, &[], "gpt-5.6-luna");
            let models: Vec<&str> = options["providers"][0]["models"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
            assert_eq!(models, ["gpt-5.6-luna", "gpt-5.6-sol"]);
        });
    }

    #[test]
    fn a_draft_chat_lists_the_providers_whole_catalog_with_per_model_efforts() {
        isolated(|| {
            let options = model_options("openai", "gpt-5.6-luna", vec![], false, &[], "gpt-5.6-luna");
            let row = &options["providers"][0];
            let models: Vec<&str> = row["models"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
            assert!(models.len() > 5 && models.contains(&"gpt-5.6-luna") && models.contains(&"gpt-5.6-sol"), "{models:?}");
            // Per model, never the swarm sentinels, and a pinned Pro ladder differs from the rest.
            let caps = &row["capabilities"];
            assert_eq!(caps["gpt-5-pro"]["efforts"], json!(["high"]));
            let luna = caps["gpt-5.6-luna"]["efforts"].as_array().unwrap();
            assert!(luna.contains(&json!("high")) && !luna.iter().any(|e| e.as_str().unwrap().starts_with("swarm")));
            assert_eq!(caps["gpt-5.6-luna"]["reasoning"], json!(true));
        });
    }

    #[test]
    fn the_effort_slider_is_given_exactly_the_levels_the_accounts_catalog_lists_for_the_model() {
        isolated(|| {
            let mut catalog = factr_base::provider::OpenAIModelCatalog::default();
            catalog.available_models = vec!["gpt-5.6-luna".into()];
            catalog.reasoning_efforts.insert("gpt-5.6-luna".into(), ["low", "medium", "high", "xhigh", "max"].map(String::from).to_vec());
            factr_base::provider::persist_openai_model_catalog(&catalog);
            let options = model_options("openai", "gpt-5.6-luna", vec![], false, &[], "gpt-5.6-luna");
            // The desktop reads providers[].capabilities[model].efforts (array of level names).
            let caps = &options["providers"][0]["capabilities"]["gpt-5.6-luna"];
            assert_eq!(caps["efforts"], json!(["low", "medium", "high", "xhigh", "max"]));
            assert_eq!(caps["can_disable_reasoning"], json!(false));
        });
    }

    #[test]
    fn a_cache_keyed_by_a_differently_cased_id_still_sets_the_ladder_and_each_model_has_one_row() {
        isolated(|| {
            let mut catalog = factr_base::provider::OpenAIModelCatalog::default();
            catalog.available_models = vec!["GPT-5.6-Luna".into(), "gpt-5.6-luna".into(), "gpt-5.6-sol".into()];
            catalog.reasoning_efforts.insert("GPT-5.6-Luna".into(), ["low", "medium", "high", "xhigh", "max"].map(String::from).to_vec());
            factr_base::provider::persist_openai_model_catalog(&catalog);
            let options = model_options("openai", "gpt-5.6-luna", vec![], false, &[], "gpt-5.6-luna");
            eprintln!("model.options payload: {options}");
            let row = &options["providers"][0];
            let luna: Vec<&Value> = row["models"].as_array().unwrap().iter().filter(|m| m.as_str().unwrap().eq_ignore_ascii_case("gpt-5.6-luna")).collect();
            assert_eq!(luna, vec![&json!("gpt-5.6-luna")], "one row, spelled as the engine serves it");
            assert_eq!(row["capabilities"]["gpt-5.6-luna"]["efforts"], json!(["low", "medium", "high", "xhigh", "max"]));
            assert_eq!(known_efforts("openai", "GPT-5.6-LUNA").unwrap(), ["low", "medium", "high", "xhigh", "max"]);
        });
    }

    #[test]
    fn with_no_catalog_cache_gpt_6_and_gpt_5_6_take_the_full_ladder_others_the_conservative_one_never_minimal() {
        isolated(|| {
            let full = json!(["low", "medium", "high", "xhigh", "max"]);
            let options = model_options("openai", "gpt-6-luna", vec![], false, &[], "gpt-6-luna");
            let caps = &options["providers"][0]["capabilities"];
            assert_eq!(caps["gpt-6-luna"]["efforts"], full, "{options}");
            assert_eq!(caps["gpt-6-sol"]["efforts"], full);
            assert_eq!(caps["gpt-5.6-luna"]["efforts"], full);
            assert_eq!(caps["gpt-5.5"]["efforts"], json!(["low", "medium", "high"]));
            assert!(check_effort("openai", "gpt-6-luna", "xhigh").is_ok());
            assert!(check_effort("openai", "gpt-6-luna", "max").is_ok());
            assert!(check_effort("openai", "gpt-5.5", "xhigh").is_err());
            assert!(check_effort("openai", "gpt-6-luna", "minimal").is_err());
            // gpt-6-luna is listed exactly once.
            let models = options["providers"][0]["models"].as_array().unwrap();
            assert_eq!(models.iter().filter(|m| **m == json!("gpt-6-luna")).count(), 1);
        });
    }

    #[test]
    fn a_signed_in_claude_provider_lists_its_catalog_and_an_unserved_one_stays_empty() {
        isolated(|| {
            let options = model_options("openai", "gpt-5.6-luna", vec![], true, &[], "gpt-5.6-luna");
            let row = |slug: &str| options["providers"].as_array().unwrap().iter().find(|r| r["slug"] == slug).cloned().unwrap();
            let claude = row("claude");
            assert!(claude["models"].as_array().unwrap().contains(&json!("claude-opus-5-5")), "{claude}");
            // Efforts follow the model: Haiku takes none of the ladder Opus does.
            assert_ne!(claude["capabilities"]["claude-haiku-4-5"]["efforts"], claude["capabilities"]["claude-opus-5-5"]["efforts"]);
            // A provider with no engine-side catalog advertises nothing it cannot call.
            assert_eq!(row("copilot")["models"], json!([]));
        });
    }

    #[test]
    fn the_picker_has_one_catalog_without_factr_only_rows_or_providers_factr_cannot_sign_in() {
        isolated(|| {
            let options = model_options("openai", "gpt-5.6-luna", vec![], true, &[], "gpt-5.6-luna");
            let slugs: Vec<&str> = options["providers"].as_array().unwrap().iter().filter_map(|r| r["slug"].as_str()).collect();
            for gone in ["factr", "auto-import", "cursor", "grok-build", "mistral", "togetherai"] {
                assert!(!slugs.contains(&gone), "{gone} in {slugs:?}");
            }
            for kept in ["openai", "claude", "openrouter", "openai-compatible", "ollama", "copilot", "xai"] {
                assert!(slugs.contains(&kept), "{kept} missing from {slugs:?}");
            }
            assert!(slugs.len() < 40, "{}", slugs.len());
        });
    }

    #[test]
    fn an_effort_the_model_cannot_take_is_refused_with_what_it_can() {
        let err = check_effort("openai", "gpt-5-pro", "low").unwrap_err();
        assert!(err.contains("not supported by gpt-5-pro") && err.contains("high"), "{err}");
        assert!(check_effort("openai", "gpt-5-pro", "high").is_ok());
        assert!(check_effort("openai", "gpt-5-pro", "swarm").is_ok());
        assert!(check_effort("claude", "claude-sonnet-4-5", "max").is_err());
        // A route the engine cannot judge is left to the live provider.
        assert!(check_effort("ollama", "qwen3", "low").is_ok());
    }

    #[test]
    fn an_openai_or_claude_api_key_alone_makes_the_served_provider_ready() {
        isolated(|| {
            for (label, var) in [("OpenAI", "OPENAI_API_KEY"), ("openai", "OPENAI_API_KEY"), ("Claude", "ANTHROPIC_API_KEY")] {
                assert_eq!(setup_status(label)["provider_configured"], false, "{label} with no key");
                assert_eq!(runtime_check(label, "m", None)["ok"], false, "{label} with no key");
                let env = std::path::Path::new(&std::env::var("FACTR_CONFIG_HOME").unwrap()).join(".env");
                std::fs::write(&env, format!("{var}=test-key\n")).unwrap();
                assert_eq!(setup_status(label)["provider_configured"], true, "{label} with {var}");
                assert_eq!(runtime_check(label, "m", None)["ok"], true, "{label} with {var}");
                std::fs::remove_file(env).unwrap();
            }
        });
    }

    #[test]
    fn a_missing_key_asks_for_one_and_a_factr_env_key_satisfies_it() {
        isolated(|| {
            assert_eq!(setup_status("groq")["provider_configured"], false);
            let check = runtime_check("groq", "llama-3.3-70b", None);
            assert_eq!((check["ok"].clone(), check["error"].as_str()), (json!(false), Some("No usable credentials found for groq.")));
            let options = model_options("groq", "llama-3.3-70b", vec![], false, &[], "llama-3.3-70b");
            assert_eq!((options["providers"][0]["slug"].as_str(), options["providers"][0]["authenticated"].clone()), (Some("groq"), json!(false)));

            std::fs::write(std::path::Path::new(&std::env::var("FACTR_CONFIG_HOME").unwrap()).join(".env"), "GROQ_API_KEY=test-key\n").unwrap();
            assert_eq!(setup_status("groq")["provider_configured"], true);
            assert_eq!(runtime_check("groq", "llama-3.3-70b", None)["ok"], true);
            let options = model_options("groq", "llama-3.3-70b", vec!["llama-3.3-70b".into(), "qwen3".into()], false, &[], "llama-3.3-70b");
            let row = &options["providers"][0];
            assert_eq!((row["is_current"].clone(), row["authenticated"].clone(), row["total_models"].clone()), (json!(true), json!(true), json!(2)));
        });
    }
}
