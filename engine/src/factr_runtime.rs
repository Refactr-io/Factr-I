//! Headless runtime used by the Factr desktop's `factr` binary.

use anyhow::Result;
use clap::ValueEnum;
use std::time::Instant;

use crate::provider::{self, Provider};
use crate::server;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum ProviderChoice {
    /// Native Claude (Anthropic OAuth/API). `claude-subprocess` is kept as a
    /// hidden alias for old scripts; the Claude Code CLI subprocess transport
    /// has been removed.
    /// `anthropic` / `claude-code`: the ids the Factr runtime saves in config.yaml for this login.
    #[value(alias = "claude-subprocess", alias = "anthropic", alias = "claude-code")]
    Claude,
    #[value(alias = "claude-api", alias = "anthropic-key", alias = "claude-key")]
    AnthropicApi,
    /// `openai-codex` / `chatgpt`: the ids the Factr runtime saves in config.yaml for the ChatGPT login.
    #[value(alias = "openai-codex", alias = "chatgpt", alias = "chatgpt-codex")]
    Openai,
    #[value(
        alias = "openai-key",
        alias = "openai-apikey",
        alias = "openai-platform"
    )]
    OpenaiApi,
    Openrouter,
    #[value(alias = "aws-bedrock", alias = "aws_bedrock")]
    Bedrock,
    #[value(alias = "azure-openai", alias = "aoai")]
    Azure,
    #[value(alias = "opencode-zen", alias = "zen")]
    Opencode,
    #[value(alias = "opencodego")]
    OpencodeGo,
    #[value(alias = "z.ai", alias = "z-ai", alias = "zai-coding")]
    Zai,
    #[value(
        alias = "kimi-code",
        alias = "kimi-coding",
        alias = "kimi-coding-plan",
        alias = "kimi-for-coding",
        alias = "moonshot-coding"
    )]
    Kimi,
    #[value(alias = "302.ai", alias = "302ai")]
    Ai302,
    Baseten,
    #[value(alias = "conifer-api")]
    Conifer,
    Cortecs,
    #[value(alias = "cgc", alias = "comtegra-gpu-cloud")]
    Comtegra,
    Deepseek,
    #[value(alias = "fpt-ai", alias = "fptcloud", alias = "fpt-cloud")]
    Fpt,
    Firmware,
    #[value(alias = "hugging-face", alias = "hf", alias = "huggingface")]
    HuggingFace,
    #[value(alias = "moonshot", alias = "moonshotai")]
    MoonshotAi,
    Nebius,
    Scaleway,
    Stackit,
    Groq,
    #[value(alias = "mistralai")]
    Mistral,
    #[value(alias = "pplx")]
    Perplexity,
    #[value(alias = "together", alias = "together-ai", alias = "togetherai")]
    TogetherAi,
    #[value(alias = "deep-infra")]
    Deepinfra,
    #[value(alias = "fireworks-ai", alias = "fireworks.ai")]
    Fireworks,
    #[value(alias = "novita-ai", alias = "novita.ai")]
    Novita,
    #[value(alias = "minimax-ai", alias = "minimaxi")]
    Minimax,
    #[value(alias = "x.ai", alias = "x-ai", alias = "grok")]
    Xai,
    /// Grok Build subscription via the authenticated Grok CLI ACP transport.
    #[value(name = "grok-build")]
    GrokBuild,
    #[value(alias = "nvidia", alias = "nim")]
    NvidiaNim,
    #[value(alias = "xiaomi", alias = "mimo", alias = "xiaomi-mimo-api")]
    XiaomiMimo,
    #[value(
        alias = "meta",
        alias = "muse",
        alias = "muse-spark",
        alias = "meta-model-api",
        alias = "meta-ai"
    )]
    MetaMuse,
    #[value(alias = "celeris-ai", alias = "celeris1", alias = "celeris-1")]
    Celeris,
    YoloAuto,
    #[value(alias = "lm-studio")]
    Lmstudio,
    Ollama,
    Chutes,
    #[value(alias = "cerebrascode", alias = "cerberascode")]
    Cerebras,
    #[value(alias = "belvedir.ai", alias = "belvedir-ai")]
    Belvedir,
    #[value(alias = "orca-router")]
    Orcarouter,
    #[value(
        alias = "bailian",
        alias = "aliyun-bailian",
        alias = "coding-plan",
        alias = "alibaba-coding"
    )]
    AlibabaCodingPlan,
    #[value(alias = "compat", alias = "custom")]
    OpenaiCompatible,
    Cursor,
    Copilot,
    Gemini,
    #[value(
        alias = "gemini-key",
        alias = "gemini-apikey",
        alias = "google-ai-studio",
        alias = "ai-studio"
    )]
    GeminiApi,
    Google,
    Auto,
}

impl ProviderChoice {
    #[allow(deprecated)]
    pub fn as_arg_value(&self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::AnthropicApi => "anthropic-api",
            Self::Openai => "openai",
            Self::OpenaiApi => "openai-api",
            Self::Openrouter => "openrouter",
            Self::Bedrock => "bedrock",
            Self::Azure => "azure",
            Self::Opencode => "opencode",
            Self::OpencodeGo => "opencode-go",
            Self::Zai => "zai",
            Self::Kimi => "kimi",
            Self::Ai302 => "302ai",
            Self::Baseten => "baseten",
            Self::Conifer => "conifer",
            Self::Cortecs => "cortecs",
            Self::Comtegra => "comtegra",
            Self::Deepseek => "deepseek",
            Self::Fpt => "fpt",
            Self::Firmware => "firmware",
            Self::HuggingFace => "huggingface",
            Self::MoonshotAi => "moonshotai",
            Self::Nebius => "nebius",
            Self::Scaleway => "scaleway",
            Self::Stackit => "stackit",
            Self::Groq => "groq",
            Self::Mistral => "mistral",
            Self::Perplexity => "perplexity",
            Self::TogetherAi => "togetherai",
            Self::Deepinfra => "deepinfra",
            Self::Fireworks => "fireworks",
            Self::Novita => "novita",
            Self::Minimax => "minimax",
            Self::Xai => "xai",
            Self::GrokBuild => "grok-build",
            Self::NvidiaNim => "nvidia-nim",
            Self::XiaomiMimo => "xiaomi-mimo",
            Self::MetaMuse => "meta-muse",
            Self::Celeris => "celeris",
            Self::YoloAuto => "yolo-auto",
            Self::Lmstudio => "lmstudio",
            Self::Ollama => "ollama",
            Self::Chutes => "chutes",
            Self::Cerebras => "cerebras",
            Self::Belvedir => "belvedir",
            Self::Orcarouter => "orcarouter",
            Self::AlibabaCodingPlan => "alibaba-coding-plan",
            Self::OpenaiCompatible => "openai-compatible",
            Self::Cursor => "cursor",
            Self::Copilot => "copilot",
            Self::Gemini => "gemini",
            Self::GeminiApi => "gemini-api",
            Self::Google => "google",
            Self::Auto => "auto",
        }
    }
}

fn register_external_provider_runtimes() {
    crate::provider::external::register_external_provider(
        crate::provider::external::GROK_BUILD_RUNTIME,
        || std::sync::Arc::new(factr_provider_grok_build_runtime::GrokBuildProvider::new()),
    );
    crate::provider::external::register_external_provider(
        crate::provider::external::GEMINI_RUNTIME,
        || std::sync::Arc::new(factr_provider_gemini_runtime::GeminiProvider::new()),
    );
    crate::provider::external::register_external_provider(
        crate::provider::external::CURSOR_RUNTIME,
        || std::sync::Arc::new(factr_provider_cursor_runtime::CursorCliProvider::new()),
    );
    crate::provider::external::register_external_provider(
        crate::provider::external::ANTHROPIC_RUNTIME,
        || std::sync::Arc::new(factr_provider_anthropic_runtime::AnthropicProvider::new()),
    );
    // OpenRouter serves several identities (aggregator, pinned API-key
    // runtime, direct OpenAI-compatible profiles, named config profiles)
    // through one concrete type, so it registers a parameterized factory.
    crate::provider::external::register_openrouter_factory(|spec| {
        use crate::provider::external::OpenRouterRuntimeSpec;
        use factr_provider_openrouter_runtime::OpenRouterProvider;
        let provider: std::sync::Arc<dyn crate::provider::Provider> = match spec {
            OpenRouterRuntimeSpec::Default => std::sync::Arc::new(OpenRouterProvider::new()?),
            OpenRouterRuntimeSpec::OpenRouterApiKey => {
                std::sync::Arc::new(OpenRouterProvider::new_openrouter_api_key_runtime()?)
            }
            OpenRouterRuntimeSpec::CompatibleProfile(profile) => std::sync::Arc::new(
                OpenRouterProvider::new_openai_compatible_profile_runtime(profile)?,
            ),
            OpenRouterRuntimeSpec::NamedProfile { name, config } => std::sync::Arc::new(
                OpenRouterProvider::new_named_openai_compatible(&name, &config)?,
            ),
        };
        Ok(provider)
    });
    crate::provider::external::register_profile_catalog_refresh(
        factr_provider_openrouter_runtime::maybe_schedule_openai_compatible_profile_catalog_refresh,
    );
    crate::provider::external::register_standard_openrouter_catalog_refresh(
        factr_provider_openrouter_runtime::maybe_schedule_standard_openrouter_catalog_refresh,
    );
    // API-backed OpenAI routes use Codex/platform credentials. The runtime is
    // still registered without them so a later login upgrades it in place.
    crate::provider::external::register_external_provider_fallible(
        crate::provider::external::OPENAI_RUNTIME,
        || {
            let provider = match crate::auth::codex::load_credentials() {
                Ok(credentials) => {
                    factr_gateway::clear_missing_login();
                    factr_provider_openai_runtime::OpenAIProvider::new(credentials)
                }
                Err(err) => {
                    factr_gateway::note_missing_login("openai", &format!("{err:#}"));
                    factr_provider_openai_runtime::OpenAIProvider::new_without_credentials()
                }
            };
            Some(std::sync::Arc::new(provider) as std::sync::Arc<dyn crate::provider::Provider>)
        },
    );
    // Copilot's constructor is fallible (needs a GitHub token) and the runtime
    // wants tier detection scheduled right after construction, eagerly for
    // interactive sessions and deferred for non-interactive ones. That policy
    // lives here in the composition root so base stays provider-agnostic.
    crate::provider::external::register_external_provider_fallible(
        crate::provider::external::COPILOT_RUNTIME,
        || {
            let provider = std::sync::Arc::new(
                factr_provider_copilot_runtime::CopilotApiProvider::new().ok()?,
            );
            let eager_tier_detection = std::env::var("FACTR_NON_INTERACTIVE").is_err();
            if eager_tier_detection && tokio::runtime::Handle::try_current().is_ok() {
                let p_clone = std::sync::Arc::clone(&provider);
                tokio::spawn(async move {
                    p_clone.detect_tier_and_set_default().await;
                });
            } else {
                provider.complete_init_without_tier_detection();
            }
            Some(provider as std::sync::Arc<dyn crate::provider::Provider>)
        },
    );
}


/// A token the launcher supplied (stdin or dev env) must be usable: a short one would lock the
/// caller out of a gateway that quietly used another token, so refuse to start instead.
fn supplied_token(launch: Option<String>) -> anyhow::Result<Option<String>> {
    match launch {
        Some(token) if token.len() < 32 => {
            anyhow::bail!("the supplied gateway token is {} characters; it must be at least 32", token.len())
        }
        other => Ok(other),
    }
}

async fn server_is_running_at(path: &std::path::Path) -> bool {
    // Check liveness before performing a protocol handshake. On Windows the
    // named pipe may be busy while another client is connecting; that already
    // proves a daemon exists, while a handshake connect can otherwise wait in
    // the transport's ERROR_PIPE_BUSY retry loop and block server startup.
    server::has_live_listener(path).await || server::is_server_ready(path).await
}

/// What rows, costs and `/api/model/info` call the provider. OpenAI-compatible profiles
/// (groq, deepseek, a named profile) all run in the OpenRouter runtime, whose own name is
/// "openrouter"; the label is the profile the user picked.
fn served_provider_label(choice: ProviderChoice, runtime_name: &str) -> String {
    if let Ok(named) = std::env::var("FACTR_NAMED_PROVIDER_PROFILE")
        && !named.trim().is_empty()
    {
        return named.trim().to_string();
    }
    crate::provider_catalog::resolve_openai_compatible_profile_selection(choice.as_arg_value())
        .map_or_else(|| runtime_name.to_string(), |profile| profile.id.to_string())
}

#[allow(deprecated)]
async fn init_provider_for_serve(
    choice: ProviderChoice,
    model: Option<&str>,
) -> Result<Arc<dyn provider::Provider>> {
    register_external_provider_runtimes();
    if let Ok(profile_name) = std::env::var("FACTR_PROVIDER_PROFILE_NAME")
        && !profile_name.trim().is_empty()
    {
        crate::provider_catalog::apply_named_provider_profile_env(profile_name.trim())?;
        crate::env::set_var("FACTR_PROVIDER_PROFILE_ACTIVE", "1");
    }
    let profile = crate::provider_catalog::resolve_openai_compatible_profile_selection(
        choice.as_arg_value(),
    );
    let provider: Arc<dyn provider::Provider> = if let Some(profile) = profile {
        if std::env::var_os("FACTR_NAMED_PROVIDER_PROFILE").is_none() {
            crate::provider_catalog::force_apply_openai_compatible_profile_env(Some(profile));
        }
        let named_profile = std::env::var("FACTR_NAMED_PROVIDER_PROFILE").ok();
        let runtime_model = if let Some(name) = named_profile.as_deref() {
            let cfg = crate::config::config();
            cfg
                .providers
                .get(name)
                .ok_or_else(|| anyhow::anyhow!("Unknown provider profile '{name}'"))?
                .default_model
                .clone()
        } else {
            crate::provider_catalog::resolve_openai_compatible_profile(profile).default_model
        };
        provider::activation::apply_openai_compatible_runtime(runtime_model)?;
        if let Some(name) = named_profile {
            let cfg = crate::config::config();
            let profile = cfg
                .providers
                .get(&name)
                .ok_or_else(|| anyhow::anyhow!("Unknown provider profile '{name}'"))?;
            Arc::new(
                factr_provider_openrouter_runtime::OpenRouterProvider::new_named_openai_compatible(
                    &name, profile,
                )?,
            )
        } else {
            Arc::new(factr_provider_openrouter_runtime::OpenRouterProvider::new()?)
        }
    } else {
        let multi = || Arc::new(provider::MultiProvider::new_fast());
        match choice {
            ProviderChoice::Claude => {
                provider::activation::select_initial_runtime_provider_key("claude");
                Arc::new(provider::MultiProvider::with_preference_fast(false))
            }
            ProviderChoice::AnthropicApi => {
                provider::activation::select_initial_runtime_provider_key("claude");
                Arc::new(provider::MultiProvider::with_preference_fast(false))
            }
            ProviderChoice::Openai | ProviderChoice::OpenaiApi => {
                provider::activation::select_initial_runtime_provider_key("openai");
                Arc::new(provider::MultiProvider::with_preference_fast(true))
            }
            ProviderChoice::Openrouter => {
                provider::activation::select_initial_runtime_provider_key("openrouter");
                multi()
            }
            ProviderChoice::Bedrock => {
                provider::activation::select_initial_runtime_provider_key("bedrock");
                multi()
            }
            ProviderChoice::Azure => {
                let azure_model = provider::activation::apply_azure_openai_runtime()?;
                let provider = multi();
                if let Some(model) = azure_model {
                    let _ = provider.set_model(&model);
                }
                provider
            }
            ProviderChoice::Cursor => {
                crate::env::set_var("FACTR_ACTIVE_PROVIDER", "cursor");
                Arc::new(factr_provider_cursor_runtime::CursorCliProvider::new())
            }
            ProviderChoice::Copilot => {
                provider::activation::select_initial_runtime_provider_key("copilot");
                multi()
            }
            ProviderChoice::Gemini => {
                crate::env::set_var("FACTR_ACTIVE_PROVIDER", "gemini");
                Arc::new(factr_provider_gemini_runtime::GeminiProvider::new())
            }
            ProviderChoice::GrokBuild => {
                crate::provider::external::instantiate_external_provider(
                    crate::provider::external::GROK_BUILD_RUNTIME,
                )
                .ok_or_else(|| anyhow::anyhow!("Grok Build runtime is not registered"))?
            }
            ProviderChoice::Google | ProviderChoice::Auto => {
                let auto = provider::MultiProvider::from_auth_status(
                    crate::auth::AuthStatus::check_fast(),
                );
                crate::env::set_var("FACTR_ACTIVE_PROVIDER", auto.name().to_lowercase());
                Arc::new(auto)
            }
            ProviderChoice::YoloAuto => unreachable!("YoloAuto is an OpenAI-compatible profile"),
            _ => anyhow::bail!("unsupported factr provider {}", choice.as_arg_value()),
        }
    };
    if matches!(choice, ProviderChoice::AnthropicApi | ProviderChoice::OpenaiApi) {
        provider.set_credential_mode(provider::CredentialMode::ApiKey)?;
    }
    if let Some(model) = model {
        // `--model` is the user's explicit pick: the backend, not a possibly stale catalog, decides.
        provider::trust_explicit_openai_model(model);
        provider.set_model(model)?;
    }
    Ok(provider)
}

/// What `serve` starts on, before any provider is built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupResolution {
    pub provider: ProviderChoice,
    pub model: Option<String>,
    /// A config.yaml provider that names one of Factr's provider profiles (`FACTR_NAMED_PROVIDER_PROFILE`).
    pub named_profile: Option<String>,
    /// Becomes `Config::profile_model_applies` (see `profile::boot_pick`).
    pub profile_model_applies: bool,
    /// config.yaml's provider when the engine cannot serve it: it starts on none rather than another.
    pub unsupported: Option<String>,
    /// The provider config.yaml chose (`boot_pick` leaves it empty for any start with an explicit
    /// `--provider`/`--model`): only then does boot apply a saved custom endpoint.
    pub profile_provider: Option<String>,
}

/// Boot precedence: an explicit `--provider`/`--model` (or FACTR_*) outranks what config.yaml holds;
/// the profile only fills what the flags left absent (see `profile::boot_pick`). A saved provider id
/// the parser does not know is resolved through the login catalog (ids, aliases, display names); one
/// that still does not resolve is reported as `unsupported`, never replaced by another provider.
pub fn resolve_startup(
    cli: ProviderChoice,
    model: Option<&str>,
    profile: &factr_gateway::profile::ProfileDefaults,
    is_named_profile: impl Fn(&str) -> bool,
) -> StartupResolution {
    // An explicit `--provider` is not overridden by a profile that only says "auto"; that profile's
    // model belongs to its own provider, so it is not borrowed either.
    let cli_explicit = !matches!(cli, ProviderChoice::Auto);
    let boot = factr_gateway::profile::boot_pick(cli_explicit, model, profile);
    let mut resolution = StartupResolution {
        provider: cli,
        model: boot.model.map(str::to_owned),
        named_profile: None,
        profile_model_applies: boot.profile_model_applies,
        unsupported: None,
        profile_provider: boot.profile_provider.map(str::to_owned),
    };
    // A bare `--model` names its own provider: derived from the model id, never from config.yaml.
    if let Some(choice) = boot.model_provider.and_then(|name| ProviderChoice::from_str(name, true).ok()) {
        resolution.provider = choice;
    }
    if let Some(provider) = boot.profile_provider {
        match ProviderChoice::from_str(provider, true) {
            Ok(choice) => resolution.provider = choice,
            Err(_) if is_named_profile(provider) => {
                resolution.named_profile = Some(provider.to_string());
                resolution.provider = ProviderChoice::OpenaiCompatible;
            }
            Err(_) => match loose_provider_choice(provider) {
                Some(choice) => resolution.provider = choice,
                None => resolution.unsupported = Some(provider.to_string()),
            },
        }
    }
    resolution
}

/// A provider id, alias or display name the login catalog knows, as the engine's choice.
pub fn loose_provider_choice(value: &str) -> Option<ProviderChoice> {
    ProviderChoice::from_str(value, true).ok().or_else(|| {
        crate::provider_catalog::resolve_login_provider_loose(value)
            .and_then(|d| ProviderChoice::from_str(d.id, true).ok())
    })
}

/// The local Ollama server: `OLLAMA_HOST` (`host`, `host:port` or a URL; a bind-all host means this
/// machine), else `http://127.0.0.1:11434`.
pub fn ollama_base_url() -> String {
    ollama_base_url_from(std::env::var("OLLAMA_HOST").ok().as_deref())
}

fn ollama_base_url_from(host: Option<&str>) -> String {
    let host = host.map(str::trim).filter(|h| !h.is_empty()).unwrap_or("127.0.0.1:11434");
    let (scheme, rest) = match host.split_once("://") {
        Some((scheme, rest)) => (scheme, rest),
        None => ("http", host),
    };
    let rest = rest.trim_end_matches('/');
    let (name, port) = match rest.rsplit_once(':') {
        Some((name, port)) if !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()) => (name, port),
        _ => (rest, "11434"),
    };
    let name = match name {
        "" | "0.0.0.0" | "[::]" | "::" => "127.0.0.1",
        other => other,
    };
    format!("{scheme}://{name}:{port}")
}

/// The models a local Ollama lists (`GET /api/tags`), within a short timeout. `None`: not reachable.
pub async fn local_ollama_models(base_url: &str) -> Option<Vec<String>> {
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_millis(300))
        .timeout(std::time::Duration::from_millis(1500))
        .no_proxy()
        .build()
        .ok()?;
    let body: serde_json::Value = client
        .get(format!("{}/api/tags", base_url.trim_end_matches('/')))
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?
        .json()
        .await
        .ok()?;
    Some(
        body["models"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|m| m["name"].as_str().or_else(|| m["model"].as_str()))
            .map(str::to_owned)
            .collect(),
    )
}

/// Whether boot applies config.yaml's custom endpoint (`model.base_url`): only when config.yaml itself
/// chose the provider (`profile_provider`, which `boot_pick` leaves empty for any start with an
/// explicit `--model`) and that provider is the generic OpenAI-compatible route (not a named profile).
pub fn saved_custom_endpoint_applies(profile_provider: Option<&str>, effective: ProviderChoice, named_profile: bool) -> bool {
    profile_provider.is_some() && matches!(effective, ProviderChoice::OpenaiCompatible) && !named_profile
}

pub async fn run_gateway(
    provider_choice: &ProviderChoice,
    model: Option<&str>,
    host: &str,
    port: u16,
    allow_remote: bool,
    unsupported_env: Option<&str>,
) -> Result<()> {
    // Before any provider is built: the Factr login must be readable by the startup init.
    factr_gateway::register_credentials();
    let profile_defaults = factr_gateway::profile::current();
    let resolved = resolve_startup(*provider_choice, model, &profile_defaults, |name| {
        crate::config::config().providers.contains_key(name)
    });
    if let Some(name) = &resolved.named_profile {
        crate::env::set_var("FACTR_NAMED_PROVIDER_PROFILE", name);
    }
    // A provider the engine cannot serve (an unknown FACTR_PROVIDER, or a saved one): start so the UI
    // is reachable, but on no provider of our choosing; the gateway reports why (see `profile`).
    let unsupported = match (unsupported_env, &resolved.unsupported) {
        (Some(value), _) => Some((value.to_string(), true)),
        (None, Some(value)) => Some((value.clone(), false)),
        (None, None) => None,
    };
    if let Some((value, from_env)) = &unsupported {
        factr_gateway::profile::note_unsupported_provider(value, *from_env);
    }
    let mut effective_provider = resolved.provider;
    // Nothing is served for an unsupported provider, so no model is claimed for it either.
    let mut effective_model = resolved.model.clone().filter(|_| unsupported.is_none());
    // The last step of the auto chain: nothing configured, nothing signed in, a local Ollama with models.
    if unsupported.is_none()
        && matches!(effective_provider, ProviderChoice::Auto)
        && effective_model.is_none()
        && profile_defaults.provider.is_none()
        && std::env::var_os("FACTR_PROVIDER").is_none()
        && std::env::var_os("FACTR_NAMED_PROVIDER_PROFILE").is_none()
        && crate::config::config().provider.default_provider.is_none()
    {
        let anything_configured = factr_gateway::any_provider_configured();
        let installed = if anything_configured {
            Vec::new()
        } else {
            local_ollama_models(&ollama_base_url()).await.unwrap_or_default()
        };
        if let Some(model) = factr_gateway::auto_local_ollama_model(anything_configured, &installed) {
            eprintln!("factr: no provider configured or signed in; using local Ollama ({model})");
            effective_provider = ProviderChoice::Ollama;
            effective_model = Some(model);
        }
    }
    if matches!(effective_provider, ProviderChoice::OpenaiCompatible)
        && std::env::var_os("FACTR_NAMED_PROVIDER_PROFILE").is_none()
        && let Some(provider) = crate::config::config()
            .provider
            .default_provider
            .as_deref()
            .filter(|name| crate::config::config().providers.contains_key(*name))
    {
        // Factr owns provider selection; Factr's named profile owns endpoint
        // details and credentials for the selected OpenAI-compatible transport.
        crate::env::set_var("FACTR_NAMED_PROVIDER_PROFILE", provider);
    }
    // A saved custom / local endpoint (`model.base_url`, from the desktop's custom-endpoint
    // onboarding) is the OpenAI-compatible route's base URL. Only when config.yaml chose the
    // provider: a start with an explicit `--model` (a benchmark) never reads it.
    if saved_custom_endpoint_applies(resolved.profile_provider.as_deref(), effective_provider, std::env::var_os("FACTR_NAMED_PROVIDER_PROFILE").is_some())
        && let Some(home) = profile_defaults.home.as_deref()
    {
        factr_gateway::profile::apply_saved_custom_endpoint(home);
    }
    let effective_model = effective_model.as_deref();
    let socket =
        crate::storage::runtime_dir().join(format!("factr-{}.sock", std::process::id()));
    server::set_socket_path(&socket.to_string_lossy());

    let launch = factr_gateway::auth::launch_token()
        .map(str::to_owned)
        .or_else(|| std::env::var("FACTR_DASHBOARD_SESSION_TOKEN").ok());
    let token = match supplied_token(launch)? {
        Some(token) => token,
        None => {
            let token = factr_gateway::auth::generate_token();
            let path = crate::storage::factr_dir()?.join("factr-gateway.token");
            write_private_file(&path, &token)?;
            eprintln!("factr: token written to {}", path.display());
            token
        }
    };
    let bind: std::net::SocketAddr = format!("{host}:{port}")
        .parse()
        .or_else(|_| format!("[{host}]:{port}").parse())
        .map_err(|_| anyhow::anyhow!("invalid --host {host}"))?;

    // Ollama's OpenAI-compat `/v1` path ignores per-request num_ctx and reloads
    // the base model at its trained window (262k here), wiping a warm load.
    // Pin num_ctx on a local alias (`factr/…`) so warm-up and chat share
    // one serving size, then load it for the life of this process.
    let mut serve_model = effective_model.map(str::to_owned);
    if matches!(effective_provider, ProviderChoice::Ollama)
        || std::env::var("FACTR_PROVIDER").ok().as_deref() == Some("ollama")
    {
        // `--provider ollama` without `--model`: the preferred default when installed, else an
        // installed model; a server listing none is said plainly (the warm below then fails).
        let picked = match effective_model {
            Some(_) => None,
            None => match local_ollama_models(&ollama_base_url()).await {
                Some(installed) if installed.is_empty() => {
                    eprintln!(
                        "factr: error: Ollama at {} has no models installed; run `ollama pull {}` or pass --model",
                        ollama_base_url(),
                        factr_gateway::PREFERRED_LOCAL_OLLAMA_MODEL
                    );
                    None
                }
                Some(installed) => factr_gateway::pick_local_ollama_model(&installed),
                None => None,
            },
        };
        let base = effective_model
            .or(picked.as_deref())
            .unwrap_or(factr_gateway::PREFERRED_LOCAL_OLLAMA_MODEL);
        match warm_ollama_serving_context(base).await {
            Some(alias) => {
                serve_model = Some(alias);
            }
            None => {
                eprintln!("factr: Ollama warm failed; continuing with {base}");
            }
        }
    }
    // A provider with no key yet must not stop the gateway: the desktop needs it
    // up to show key setup, and a saved key hot-initialises via auth-changed.
    let provider = if unsupported.is_some() {
        // The same not-ready state; new chats are refused until a provider is chosen (see `profile`).
        Arc::new(provider::MultiProvider::new_fast())
    } else {
        match init_provider_for_serve(effective_provider, serve_model.as_deref()).await {
            Ok(provider) => provider,
            Err(err) => {
                eprintln!("factr: model provider not ready ({err:#}); serving until a key is added");
                Arc::new(provider::MultiProvider::new_fast())
            }
        }
    };
    // Catalog enrichment (GET /api/ps) only runs on fetch_models. Until then
    // Ollama's context_window() hard-falls back to 4096 and every tool-heavy
    // turn emergency-compacts. Refresh now that the model is warm.
    if matches!(effective_provider, ProviderChoice::Ollama)
        || std::env::var("FACTR_PROVIDER").ok().as_deref() == Some("ollama")
    {
        match provider.refresh_model_catalog().await {
            Ok(_) => {
                eprintln!(
                    "factr: Ollama context_window={} after catalog refresh",
                    provider.context_window()
                );
            }
            Err(err) => {
                eprintln!(
                    "factr: Ollama catalog refresh failed ({err}); context may stay at 4k"
                );
            }
        }
    }
    // The model asked for at startup is the default new chats start on. `provider.model()` is only
    // the fallback: before the account's catalog loads it can answer the provider's catalog default
    // (gpt-6-astra) for a model that was set explicitly (gpt-5.6-luna).
    let provider_model = serve_model.clone().filter(|m| !m.trim().is_empty()).unwrap_or_else(|| provider.model());
    let provider_name = match &unsupported {
        // Reported as the provider the user configured, never the stand-in's name.
        Some((value, _)) => value.clone(),
        None => served_provider_label(effective_provider, provider.name()),
    };
    let refine_provider = provider.clone();
    let startup_model = provider_model.clone();
    let complete: factr_gateway::Complete =
        std::sync::Arc::new(move |system: String, user: String| {
            // The chat the call is for (set by the gateway around it), else startup. A chat whose model
            // cannot be resolved, or a startup provider that is not on the model that was chosen (the
            // boot-time switch failed), fails the call: no hidden fallback model. The consumer the
            // gateway named decides which `auxiliary.*` override applies (learning by default; a
            // one-shot takes none), on a fork so the session's model is never switched.
            let provider = crate::provider::aux_provider(&refine_provider, &startup_model);
            Box::pin(async move { provider?.complete_simple_with_usage(&user, &system).await })
        });
    let learning = Some(factr_learning());
    // Read before the server takes the provider: the catalog reports these.
    let reasoning_efforts: Vec<String> = provider.available_efforts().into_iter().map(str::to_owned).collect();
    let server = server::Server::new_with_name(provider, Some("factr".to_string()));

    let default_cwd = std::env::var("FACTR_DESKTOP_CWD")
        .or_else(|_| std::env::var("TERMINAL_CWD"))
        .unwrap_or_else(|_| {
            std::env::current_dir()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|_| ".".into())
        });

    let ollama_unload = serve_model.clone().filter(|_| {
        matches!(effective_provider, ProviderChoice::Ollama)
            || std::env::var("FACTR_PROVIDER").ok().as_deref() == Some("ollama")
    });

    let features = factr_feature_command()
        .map(|cmd| std::sync::Arc::new(factr_gateway::features::Features::new(cmd)));

    let gateway = async {
        let deadline = Instant::now() + std::time::Duration::from_secs(30);
        while !server_is_running_at(&socket).await {
            if Instant::now() > deadline {
                anyhow::bail!("engine server did not start");
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        let gateway = factr_gateway::Gateway::bind(factr_gateway::Config {
            bind,
            token: token.clone(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            legacy_socket: socket.clone(),
            default_cwd,
            allow_non_loopback: allow_remote,
            provider: provider_name,
            model: provider_model,
            reasoning_efforts,
            profile_model_applies: resolved.profile_model_applies,
            home: crate::storage::factr_dir()?.to_string_lossy().into_owned(),
            complete: Some(complete),
            features: features.clone(),
            learning: learning.clone(),
        })
        .await?;
        let port = gateway.local_addr().port();
        if let Some(features) = &features {
            features.set_engine_env(format!("http://127.0.0.1:{port}"), token.clone());
            crate::tool::set_browser_bridge(
                format!("http://127.0.0.1:{port}/api/browser/act"),
                token.clone(),
            );
        }
        if let Ok(ready_file) = std::env::var("FACTR_DESKTOP_READY_FILE") {
            write_private_file(
                std::path::Path::new(&ready_file),
                &format!("{{\"port\":{port}}}"),
            )?;
        }
        // The exact line the Factr desktop waits for.
        println!("FACTR_BACKEND_READY port={port}");
        use std::io::Write as _;
        std::io::stdout().flush()?;
        gateway.serve().await
    };
    let result = tokio::select! {
        result = server.run() => result,
        result = gateway => result,
        _ = shutdown_signal() => Ok(()),
    };
    // SIGTERM/SIGINT end up here too: drain spans and stop commands before the process can exit
    // (the signal thread runs the same once-only cleanup; whichever is second waits for it).
    let _ = tokio::task::spawn_blocking(crate::shutdown::cleanup).await;
    // The main socket, the debug socket and both `.hash` files.
    server::cleanup_socket_files(&socket);
    crate::power_inhibit::release_all();
    if let Some(model) = ollama_unload {
        unload_ollama_model(&model).await;
    }
    result
}

async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();
    #[cfg(unix)]
    {
        let mut term =
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                Ok(s) => s,
                Err(_) => {
                    let _ = ctrl_c.await;
                    return;
                }
            };
        tokio::select! {
            _ = ctrl_c => {}
            _ = term.recv() => {}
        }
        return;
    }
    #[cfg(not(unix))]
    {
        let _ = ctrl_c.await;
    }
}

fn ollama_num_ctx() -> u64 {
    std::env::var("FACTR_OLLAMA_NUM_CTX")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|n| *n >= 8192)
        .unwrap_or(32_768)
}

fn ollama_alias_name(base_model: &str) -> String {
    // Ollama model names: namespace/name; strip tags' ':' for the alias leaf.
    // Always include `:latest` so it matches `/v1/models` / catalog ids —
    // without it context_window() misses the cache and falls back to 4096.
    let leaf = base_model.replace(':', "-");
    format!("factr/{leaf}:latest")
}

/// Pin `num_ctx` on a local Ollama alias and load it for the life of this
/// process (`keep_alive: -1`). Returns the alias to use for chat, or `None`
/// if Ollama is unreachable (deferred auth / later failure).
async fn warm_ollama_serving_context(base_model: &str) -> Option<String> {
    let num_ctx = ollama_num_ctx();
    let alias = ollama_alias_name(base_model);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .ok()?;

    // Integer num_ctx required — a string value makes later loads fail with
    // `option "num_ctx" must be of type integer`.
    let create = serde_json::json!({
        "model": alias,
        "from": base_model,
        "stream": false,
        "parameters": { "num_ctx": num_ctx },
    });
    match client
        .post("http://127.0.0.1:11434/api/create")
        .json(&create)
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => {}
        Ok(resp) => {
            eprintln!(
                "factr: Ollama create {} returned HTTP {}; chat may reload at a different num_ctx",
                alias,
                resp.status()
            );
            return None;
        }
        Err(err) => {
            eprintln!("factr: Ollama create skipped ({err})");
            return None;
        }
    }

    // -1 = stay loaded until we send keep_alive:0 (app quit). Do not use a
    // wall-clock TTL that keeps the model resident after Factr exits.
    let body = serde_json::json!({
        "model": alias,
        "prompt": ".",
        "stream": false,
        "keep_alive": -1,
    });
    match client
        .post("http://127.0.0.1:11434/api/generate")
        .json(&body)
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => {
            eprintln!(
                "factr: warmed Ollama {alias} (from {base_model}) with num_ctx={num_ctx}, keep_alive=-1"
            );
            if let Ok(home) = crate::storage::factr_dir() {
                let meta =
                    serde_json::json!({ "model": alias, "base": base_model, "num_ctx": num_ctx, "pid": std::process::id() });
                let _ =
                    write_private_file(&home.join("factr-ollama-warm.json"), &meta.to_string());
            }
            Some(alias)
        }
        Ok(resp) => {
            eprintln!(
                "factr: Ollama warm returned HTTP {}; chat may emergency-compact until num_ctx is pinned",
                resp.status()
            );
            None
        }
        Err(err) => {
            eprintln!("factr: Ollama warm skipped ({err})");
            None
        }
    }
}

async fn unload_ollama_model(model: &str) {
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
    {
        Ok(c) => c,
        Err(_) => return,
    };
    let body = serde_json::json!({ "model": model, "keep_alive": 0 });
    match client
        .post("http://127.0.0.1:11434/api/generate")
        .json(&body)
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => {
            eprintln!("factr: unloaded Ollama {model}");
        }
        Ok(resp) => {
            eprintln!("factr: Ollama unload HTTP {}", resp.status());
        }
        Err(err) => {
            eprintln!("factr: Ollama unload skipped ({err})");
        }
    }
    if let Ok(home) = crate::storage::factr_dir() {
        let _ = std::fs::remove_file(home.join("factr-ollama-warm.json"));
    }
}

/// Command that starts Factr's Python backend for the features the Rust
/// harness does not own: `FACTR_BACKEND_CMD` (empty disables), else the
/// engine-managed install. Never a `factr` found on PATH: it may be a different program.
/// `FACTR_BACKEND_CMD`: a program plus leading args, or one path to a program (which may hold spaces).
fn split_factr_cmd(cmd: &str) -> Option<Vec<String>> {
    if std::path::Path::new(cmd).is_file() {
        return Some(vec![cmd.to_owned()]);
    }
    let parts: Vec<String> = cmd.split_whitespace().map(str::to_owned).collect();
    (!parts.is_empty()).then_some(parts)
}

pub fn factr_feature_command() -> Option<Vec<String>> {
    factr_command_from(|key| std::env::var(key).ok(), dirs::home_dir().as_deref())
}

/// Precedence: `FACTR_BACKEND_PYTHON` (it does not name the REPL's interpreter, see `FACTR_REPL_PYTHON`),
/// then `FACTR_BACKEND_CMD`, else the managed install under the home dir. An empty python disables.
fn factr_command_from(env: impl Fn(&str) -> Option<String>, home: Option<&std::path::Path>) -> Option<Vec<String>> {
    if let Some(python) = env("FACTR_BACKEND_PYTHON") {
        return (!python.is_empty()).then(|| vec![python, "-m".into(), "factr_backend.main".into()]);
    }
    if let Some(cmd) = env("FACTR_BACKEND_CMD") {
        return split_factr_cmd(&cmd);
    }
    managed_factr_command(home?)
}

#[cfg(test)]
mod factr_command_tests {
    use super::factr_command_from;

    fn cmd(vars: &[(&str, &str)]) -> Option<Vec<String>> {
        factr_command_from(|key| vars.iter().find(|(k, _)| *k == key).map(|(_, v)| v.to_string()), None)
    }

    #[test]
    fn the_refresh_command_python_is_factr_backend_python() {
        let module = |python: &str| Some(vec![python.to_string(), "-m".into(), "factr_backend.main".into()]);
        assert_eq!(cmd(&[("FACTR_BACKEND_PYTHON", "/new/py")]), module("/new/py"));
        assert_eq!(cmd(&[("FACTR_BACKEND_PYTHON", "")]), None, "an empty value disables");
        // Nothing set and no managed install: no refresh command (the engine never refreshes itself).
        assert_eq!(cmd(&[]), None);
    }
}

fn managed_factr_command(home: &std::path::Path) -> Option<Vec<String>> {
    // A venv puts its scripts in `bin/` on Unix and `Scripts` (with an `.exe` launcher) on Windows.
    let managed = home.join(".factr/factr-backend/venv");
    let managed = if cfg!(windows) { managed.join("Scripts/factr.exe") } else { managed.join("bin/factr") };
    managed.is_file().then(|| vec![managed.to_string_lossy().into_owned()])
}

/// Write a file readable only by the current user (tokens, ready files).
fn write_private_file(path: &std::path::Path, contents: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    // A pre-existing tmp file would keep its old permissions; start fresh.
    let _ = std::fs::remove_file(&tmp);
    {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        use std::io::Write as _;
        let mut file = options.open(&tmp)?;
        file.write_all(contents.as_bytes())?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}



fn factr_learning() -> factr_gateway::learn::Learning {
    // factr-learn's `settings.autoRefine.turnInterval` / `.cooldownMs` (defaults
    // 25 turns / 20 minutes); no idle wait.
    let turn_interval = std::env::var("FACTR_LEARN_TURN_INTERVAL")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(25);
    let cooldown_ms = std::env::var("FACTR_LEARN_COOLDOWN_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(20 * 60_000);
    factr_gateway::learn::Learning {
        turn_interval,
        cooldown: std::time::Duration::from_millis(cooldown_ms),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_provider_ids_the_factr_runtime_saves_parse_as_engine_providers() {
        use super::ProviderChoice;
        use clap::ValueEnum;
        // config.yaml's `model.provider` is written by the Factr runtime; at the next boot the engine
        // reads it back. An id it cannot parse made the gateway refuse to start.
        for (saved, engine) in [
            ("openai-codex", ProviderChoice::Openai),
            ("chatgpt", ProviderChoice::Openai),
            ("anthropic", ProviderChoice::Claude),
            ("claude-code", ProviderChoice::Claude),
            ("openai-api", ProviderChoice::OpenaiApi),
            ("muse", ProviderChoice::MetaMuse),
            ("auto", ProviderChoice::Auto),
            ("yolo-auto", ProviderChoice::YoloAuto),
        ] {
            assert_eq!(ProviderChoice::from_str(saved, true), Ok(engine), "{saved}");
        }
    }

    /// `run_gateway`'s provider resolution exactly as it was before `resolve_startup` (main 8aa455d),
    /// kept verbatim as the baseline the isolation tests compare against. `Err`: the old engine exited.
    fn legacy_resolution(
        provider_choice: ProviderChoice,
        model: Option<&str>,
        profile_defaults: &factr_gateway::profile::ProfileDefaults,
        is_named_profile: impl Fn(&str) -> bool,
    ) -> anyhow::Result<(ProviderChoice, Option<String>, Option<String>, bool)> {
        let mut named = None;
        let mut effective_provider = provider_choice;
        let cli_explicit = !matches!(provider_choice, ProviderChoice::Auto);
        let boot = factr_gateway::profile::boot_pick(cli_explicit, model, profile_defaults);
        let profile_provider = boot.profile_provider;
        if let Some(choice) = boot.model_provider.and_then(|name| ProviderChoice::from_str(name, true).ok()) {
            effective_provider = choice;
        }
        if let Some(provider) = profile_provider {
            effective_provider = match ProviderChoice::from_str(provider, true) {
                Ok(choice) => choice,
                Err(_) if is_named_profile(provider) => {
                    named = Some(provider.to_string());
                    ProviderChoice::OpenaiCompatible
                }
                Err(_) => {
                    anyhow::bail!("Factr profile selects unsupported engine provider `{provider}`")
                }
            };
        }
        Ok((effective_provider, boot.model.map(str::to_owned), named, boot.profile_model_applies))
    }

    fn saved(provider: Option<&str>, model: Option<&str>) -> factr_gateway::profile::ProfileDefaults {
        factr_gateway::profile::ProfileDefaults {
            provider: provider.map(str::to_owned),
            model: model.map(str::to_owned),
            ..Default::default()
        }
    }

    /// The scored benchmarks ran `--provider openai --model gpt-6-luna serve ...`. Whatever config.yaml
    /// holds, the resolved provider, model, named profile and model-follow flag are what the old engine
    /// resolved, and nothing is reported unsupported.
    #[test]
    fn the_benchmark_argv_resolves_exactly_as_before_under_any_config_yaml() {
        let profiles = [
            saved(None, None),
            saved(Some("openai-codex"), Some("gpt-6-astra")),
            saved(Some("openai-codex"), None),
            saved(Some("huggingface"), Some("some/model")),
            saved(Some("garbage-provider"), Some("x")),
            saved(Some("anthropic"), Some("claude-fable-5")),
            saved(Some("auto"), Some("cloud-model")),
            saved(Some("ollama"), Some("qwen3.8:27b")),
            saved(None, Some("gpt-5.6-sol")),
        ];
        for profile in &profiles {
            for named in [false, true] {
                let is_named = |_: &str| named;
                let old = legacy_resolution(ProviderChoice::Openai, Some("gpt-6-luna"), profile, is_named).unwrap();
                let new = resolve_startup(ProviderChoice::Openai, Some("gpt-6-luna"), profile, is_named);
                assert_eq!(
                    (new.provider, new.model.clone(), new.named_profile.clone(), new.profile_model_applies),
                    old,
                    "{profile:?}"
                );
                assert_eq!(new.provider, ProviderChoice::Openai);
                assert_eq!(new.model.as_deref(), Some("gpt-6-luna"));
                assert_eq!(new.unsupported, None);
            }
        }
    }

    /// Off the benchmark path too, every case the old engine started on resolves the same; only the
    /// cases where it exited now start, on no provider, with the saved id reported.
    #[test]
    fn resolution_only_differs_where_the_old_engine_exited() {
        let providers = [None, Some("openai-codex"), Some("anthropic"), Some("auto"), Some("ollama"), Some("my-profile"),
            Some("huggingface"), Some("kimi-coding-cn"), Some("azure-foundry"), Some("garbage")];
        let models = [None, Some("gpt-5.6-sol"), Some("local-model")];
        let clis = [ProviderChoice::Auto, ProviderChoice::Openai, ProviderChoice::Ollama, ProviderChoice::Claude];
        for p in providers {
            for m in models {
                let profile = saved(p, m);
                for cli in clis {
                    for cli_model in [None, Some("gpt-6-luna")] {
                        let is_named = |name: &str| name == "my-profile";
                        let new = resolve_startup(cli, cli_model, &profile, is_named);
                        match legacy_resolution(cli, cli_model, &profile, is_named) {
                            Ok(old) => {
                                // huggingface: the old engine could not parse its own id; the only
                                // other change is that it now does (see the aliases on ProviderChoice).
                                if p == Some("huggingface") && old.0 != new.provider {
                                    panic!("huggingface parsed by the old engine too: {old:?}");
                                }
                                assert_eq!((new.provider, new.model.clone(), new.named_profile.clone(), new.profile_model_applies), old, "{cli:?} {cli_model:?} {profile:?}");
                                assert_eq!(new.unsupported, None);
                            }
                            Err(_) => {
                                let id = p.unwrap();
                                if id == "huggingface" {
                                    assert_eq!(new.provider, ProviderChoice::HuggingFace);
                                    assert_eq!(new.unsupported, None);
                                } else {
                                    assert_eq!(new.unsupported.as_deref(), Some(id), "{cli:?} {profile:?}");
                                    assert_eq!(new.provider, cli, "never another provider");
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// Audit A: every id in backend/factr_backend/auth.py `_REGISTRY_ROWS` can land in config.yaml's
    /// `model.provider`. The engine starts on each: on that provider when it serves it, otherwise on
    /// none, with the id reported (it never exits and never picks another provider).
    #[test]
    fn every_provider_id_the_python_runtime_can_save_starts_the_engine_without_another_provider() {
        let python_ids = [
            "openai-codex", "openai-api", "qwen-oauth", "lmstudio", "copilot", "gemini", "zai",
            "kimi-coding", "kimi-coding-cn", "stepfun", "arcee", "gmi", "actual", "minimax",
            "anthropic", "alibaba", "alibaba-coding-plan", "minimax-cn", "deepseek", "xai", "nvidia",
            "ai-gateway", "opencode-zen", "opencode-go", "kilocode", "huggingface", "xiaomi",
            "tencent-tokenhub", "tencent-tokenplan", "ollama-cloud", "bedrock", "vertex", "azure-foundry",
        ];
        let mut unsupported = Vec::new();
        for id in python_ids {
            let r = resolve_startup(ProviderChoice::Auto, None, &saved(Some(id), Some("m")), |_| false);
            match r.unsupported {
                Some(u) => {
                    assert_eq!(u, id);
                    assert_eq!(r.provider, ProviderChoice::Auto, "{id}: no provider of the engine's choosing is resolved");
                    unsupported.push(id);
                }
                None => assert_ne!(r.provider, ProviderChoice::Auto, "{id}"),
            }
        }
        eprintln!("saved ids the engine starts without a provider on: {unsupported:?}");
        assert!(!unsupported.contains(&"huggingface"), "the engine's own provider");
    }

    /// Every id the engine itself saves in config.yaml (`config.set model` -> `runtime_saved_provider`)
    /// reads back as the same engine provider at the next boot.
    #[test]
    fn every_provider_id_the_engine_saves_reads_back_as_the_same_provider() {
        let _lock = crate::storage::lock_test_env();
        for choice in ProviderChoice::value_variants().iter().copied().filter(|c| *c != ProviderChoice::Auto) {
            let saved_id = factr_gateway::runtime_saved_provider(choice.as_arg_value());
            let back = loose_provider_choice(&saved_id);
            // `openai-api` is the API-key half of the engine's `openai`; both are the same served route.
            let same = back == Some(choice)
                || (choice == ProviderChoice::Openai && back == Some(ProviderChoice::OpenaiApi))
                || (choice == ProviderChoice::AnthropicApi && back == Some(ProviderChoice::Claude));
            assert!(same, "{:?} saved as `{saved_id}` reads back as {back:?}", choice);
        }
    }

    /// The Python runtime's provider ids and aliases (registry rows, `_PROVIDER_ALIASES`, model-provider
    /// plugin names and aliases), read from its source.
    fn python_provider_ids() -> std::collections::BTreeSet<String> {
        let backend = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../backend");
        let quoted = |text: &str| -> Vec<String> {
            text.split('"').skip(1).step_by(2).map(str::to_owned).collect()
        };
        let mut ids: std::collections::BTreeSet<String> = ["openrouter", "custom", "auto"].map(str::to_owned).into();
        let auth = std::fs::read_to_string(backend.join("factr_backend/auth.py")).unwrap();
        let rows = &auth[auth.find("_REGISTRY_ROWS: Tuple").unwrap()..auth.find("PROVIDER_REGISTRY: Dict").unwrap()];
        let mut after_config = false;
        for line in rows.lines().map(str::trim) {
            if let Some(rest) = line.strip_prefix("(\"").or_else(|| line.split_once("ProviderConfig(\"").map(|(_, r)| r)) {
                ids.insert(rest.split('"').next().unwrap().to_owned());
            } else if after_config && line.starts_with('"') {
                ids.insert(quoted(line)[0].clone());
            }
            after_config = line.ends_with("ProviderConfig(");
        }
        let aliases = &auth[auth.find("_PROVIDER_ALIASES: Dict").unwrap()..];
        ids.extend(quoted(&aliases[..aliases.find('}').unwrap()]));
        for plugin in std::fs::read_dir(backend.join("plugins/model-providers")).unwrap().flatten() {
            let Ok(src) = std::fs::read_to_string(plugin.path().join("__init__.py")) else { continue };
            for part in src.split("name=\"").skip(1) {
                ids.insert(part.split('"').next().unwrap().to_owned());
            }
            for part in src.split("aliases=(").skip(1) {
                ids.extend(quoted(&part[..part.find(')').unwrap_or(part.len())]));
            }
        }
        ids
    }

    /// Requirement C: every provider id the engine writes into config.yaml parses in the Python
    /// runtime too. The engine-only providers (no route in the runtime: `runtime_provider_id` is
    /// `None`, the model picker marks them not settable) are the one listed exception.
    #[test]
    fn every_provider_id_the_engine_saves_parses_in_the_python_runtime() {
        let _lock = crate::storage::lock_test_env();
        let python = python_provider_ids();
        assert!(python.contains("openai-codex") && python.contains("huggingface") && python.contains("google-ai-studio"), "{python:?}");
        let mut engine_only = Vec::new();
        for choice in ProviderChoice::value_variants().iter().copied().filter(|c| *c != ProviderChoice::Auto) {
            let id = choice.as_arg_value();
            let saved_id = factr_gateway::runtime_saved_provider(id);
            if python.contains(saved_id.to_ascii_lowercase().as_str()) {
                continue;
            }
            assert!(
                factr_gateway::runtime_provider_id(id).is_none(),
                "the engine saves `{saved_id}` for {choice:?}, which the Python runtime does not parse"
            );
            engine_only.push(saved_id);
        }
        eprintln!("engine-only providers saved under their engine id: {engine_only:?}");
    }

    #[test]
    fn ollama_host_is_read_like_ollama_reads_it() {
        assert_eq!(ollama_base_url_from(None), "http://127.0.0.1:11434");
        assert_eq!(ollama_base_url_from(Some("")), "http://127.0.0.1:11434");
        assert_eq!(ollama_base_url_from(Some("0.0.0.0")), "http://127.0.0.1:11434");
        assert_eq!(ollama_base_url_from(Some("0.0.0.0:12000")), "http://127.0.0.1:12000");
        assert_eq!(ollama_base_url_from(Some("10.0.0.8:11500")), "http://10.0.0.8:11500");
        assert_eq!(ollama_base_url_from(Some("https://box.local/")), "https://box.local:11434");
        assert_eq!(ollama_base_url_from(Some("http://box.local:9/")), "http://box.local:9");
    }

    /// A stub Ollama on a random port (never the real one): one canned reply per connection.
    async fn stub_ollama(body: &'static str) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            while let Ok((mut socket, _)) = listener.accept().await {
                let mut buf = [0u8; 4096];
                let _ = socket.read(&mut buf).await;
                let reply = format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len());
                let _ = socket.write_all(reply.as_bytes()).await;
            }
        });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn the_ollama_probe_lists_installed_models_and_is_none_when_unreachable() {
        let url = stub_ollama(r#"{"models":[{"name":"llama3:8b"},{"name":"qwen3.8:27b"}]}"#).await;
        let models = local_ollama_models(&url).await.unwrap();
        assert_eq!(models, ["llama3:8b", "qwen3.8:27b"]);
        assert_eq!(factr_gateway::auto_local_ollama_model(false, &models).as_deref(), Some("qwen3.8:27b"));
        assert_eq!(factr_gateway::auto_local_ollama_model(true, &models), None);
        let empty = stub_ollama(r#"{"models":[]}"#).await;
        assert_eq!(local_ollama_models(&empty).await, Some(Vec::new()));
        // A port nothing listens on: unreachable, quickly.
        let closed = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap().local_addr().unwrap();
        let started = std::time::Instant::now();
        assert_eq!(local_ollama_models(&format!("http://{closed}")).await, None);
        assert!(started.elapsed() < std::time::Duration::from_secs(3));
    }

    #[test]
    fn the_factr_command_may_be_a_path_with_spaces_or_a_program_with_args() {
        let dir = std::env::temp_dir().join(format!("factr cmd {}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let bin = dir.join("factr");
        std::fs::write(&bin, "").unwrap();
        let bin = bin.to_string_lossy().into_owned();
        assert_eq!(super::split_factr_cmd(&bin), Some(vec![bin.clone()]));
        assert_eq!(super::split_factr_cmd("python3 -m factr_backend.main"), Some(vec!["python3".into(), "-m".into(), "factr_backend.main".into()]));
        assert_eq!(super::split_factr_cmd("  "), None);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn only_the_managed_factr_is_used_never_one_on_path() {
        let dir = std::env::temp_dir().join(format!("factr-managed-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(super::managed_factr_command(&dir), None);
        let (sub, exe) = if cfg!(windows) { ("Scripts", "factr.exe") } else { ("bin", "factr") };
        let bin = dir.join(".factr/factr-backend/venv").join(sub);
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join(exe), "").unwrap();
        assert_eq!(super::managed_factr_command(&dir), Some(vec![bin.join(exe).to_string_lossy().into_owned()]));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_short_supplied_token_is_refused_not_replaced() {
        assert!(super::supplied_token(Some("short".into())).is_err());
        assert_eq!(super::supplied_token(None).unwrap(), None);
        let ok = "x".repeat(32);
        assert_eq!(super::supplied_token(Some(ok.clone())).unwrap(), Some(ok));
    }

    use super::*;

    #[test]
    fn a_groq_run_is_labelled_groq_not_openrouter() {
        let _lock = crate::storage::lock_test_env();
        assert_eq!(served_provider_label(ProviderChoice::Groq, "openrouter"), "groq");
        assert_eq!(served_provider_label(ProviderChoice::Ollama, "ollama"), "ollama");
    }
}
