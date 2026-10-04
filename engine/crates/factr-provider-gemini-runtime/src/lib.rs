//! Gemini provider runtime (official Gemini Developer API key from Google AI
//! Studio), moved out of `factr-base` so provider edits compile only this crate
//! plus a binary relink instead of rebuilding the base -> app-core -> tui
//! spine. The binary's composition root registers [`GeminiProvider`] with
//! `factr_base::provider::external` at startup.

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::Utc;
use factr_base::auth::gemini as gemini_auth;
use factr_message_types::{ConnectionPhase, Message, StreamEvent, ToolDefinition};
use factr_provider_core::{EventStream, Provider};
pub use factr_provider_gemini::{
    AVAILABLE_MODELS, GenerateContentEnvelope, DEFAULT_MODEL, GEMINI_API_ENDPOINT,
    GEMINI_API_VERSION, GeminiCandidate, GeminiContent, GeminiFunctionCall,
    GeminiFunctionCallingConfig, GeminiFunctionDeclaration, GeminiFunctionResponse, GeminiPart,
    GeminiPromptFeedback, GeminiRuntimeState, GeminiTool, GeminiToolConfig, GeminiUsageMetadata,
    InlineData, VertexGenerateContentRequest, VertexGenerateContentResponse, build_contents,
    build_system_instruction_with_tool_guard, build_tools, extract_gemini_model_ids,
    gemini_fallback_models, is_gemini_model_id, merge_gemini_model_lists,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::sync::{Mutex, mpsc};
use tokio_stream::wrappers::ReceiverStream;
use uuid::Uuid;

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
struct PersistedCatalog {
    models: Vec<String>,
    fetched_at_rfc3339: String,
}

pub struct GeminiProvider {
    client: reqwest::Client,
    model: Arc<RwLock<String>>,
    state: Arc<Mutex<Option<GeminiRuntimeState>>>,
    fetched_models: Arc<RwLock<Vec<String>>>,
}

impl GeminiProvider {
    fn persisted_catalog_path() -> Result<std::path::PathBuf> {
        Ok(factr_base::storage::app_config_dir()?.join("gemini_models_cache.json"))
    }

    fn load_persisted_catalog() -> Option<PersistedCatalog> {
        let path = Self::persisted_catalog_path().ok()?;
        factr_base::storage::read_json(&path)
            .ok()
            .filter(|catalog: &PersistedCatalog| !catalog.models.is_empty())
    }

    fn persist_catalog(models: &[String]) {
        if models.is_empty() {
            return;
        }
        let Ok(path) = Self::persisted_catalog_path() else {
            return;
        };
        let payload = PersistedCatalog {
            models: models.to_vec(),
            fetched_at_rfc3339: Utc::now().to_rfc3339(),
        };
        if let Err(error) = factr_base::storage::write_json(&path, &payload) {
            factr_base::logging::warn(&format!(
                "Failed to persist Gemini model catalog {}: {}",
                path.display(),
                error
            ));
        }
    }

    fn seed_cached_catalog(&self) {
        if let Some(catalog) = Self::load_persisted_catalog()
            && let Ok(mut models) = self.fetched_models.write()
        {
            *models = catalog.models;
        }
    }

    pub fn new() -> Self {
        let model = std::env::var("FACTR_GEMINI_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.into());
        let provider = Self {
            client: gemini_http_client(),
            model: Arc::new(RwLock::new(model)),
            state: Arc::new(Mutex::new(None)),
            fetched_models: Arc::new(RwLock::new(Vec::new())),
        };
        provider.seed_cached_catalog();
        provider
    }

    /// Base URL for the official Gemini Developer API (Google AI Studio).
    fn developer_api_base_url() -> String {
        let endpoint = std::env::var("GEMINI_API_ENDPOINT")
            .unwrap_or_else(|_| GEMINI_API_ENDPOINT.to_string());
        let version =
            std::env::var("GEMINI_API_VERSION").unwrap_or_else(|_| GEMINI_API_VERSION.to_string());
        format!(
            "{}/{}",
            endpoint.trim_end_matches('/'),
            version.trim_matches('/')
        )
    }

    /// The configured Gemini Developer API key (Google AI Studio).
    fn api_key() -> Result<String> {
        gemini_auth::api_key().ok_or_else(|| {
            anyhow::anyhow!(
                "Gemini API key not configured. Set GEMINI_API_KEY (Google AI Studio) or run `factr login gemini-api`."
            )
        })
    }

    async fn ensure_state(&self) -> Result<GeminiRuntimeState> {
        Self::api_key()?;
        let mut guard = self.state.lock().await;
        if let Some(state) = guard.as_ref() {
            return Ok(state.clone());
        }
        // The Developer API key path is stateless: there is no project or
        // onboarding handshake, so synthesize a lightweight session.
        let state = GeminiRuntimeState {
            project_id: String::new(),
            session_id: Uuid::new_v4().to_string(),
        };
        *guard = Some(state.clone());
        Ok(state)
    }

    async fn refresh_available_models(&self) -> Result<Vec<String>> {
        let api_key = Self::api_key()?;
        self.refresh_available_models_api_key(&api_key).await
    }

    /// Discover models via the official Developer API `ListModels` endpoint.
    /// Returned names look like `models/gemini-2.5-pro`, so strip the prefix
    /// before normalizing through the shared catalog merge.
    async fn refresh_available_models_api_key(&self, api_key: &str) -> Result<Vec<String>> {
        let url = format!("{}/models", Self::developer_api_base_url());
        let response: Value = match self.get_json_api_key(&url, api_key, "ListModels").await {
            Ok(response) => response,
            Err(err) => {
                factr_base::logging::info(&format!(
                    "Gemini Developer API model discovery failed: {err:#}"
                ));
                return Ok(Vec::new());
            }
        };

        let raw: Vec<String> = response
            .get("models")
            .and_then(|models| models.as_array())
            .map(|models| {
                models
                    .iter()
                    .filter_map(|model| model.get("name").and_then(|name| name.as_str()))
                    .map(|name| name.trim_start_matches("models/").to_string())
                    .collect()
            })
            .unwrap_or_default();

        let models = merge_gemini_model_lists(raw);
        if !models.is_empty() {
            factr_base::logging::info(&format!(
                "Discovered Gemini Developer API models: {}",
                models.join(", ")
            ));
            if let Ok(mut guard) = self.fetched_models.write() {
                *guard = models.clone();
            }
            Self::persist_catalog(&models);
        }
        Ok(models)
    }

    /// Send a request with a single transient-error retry, transparently
    /// rebuilding the HTTP client on the second attempt. The `make` closure
    /// produces a fully-configured (auth + body) request builder for each try.
    async fn send_with_retry<F>(&self, make: F, url: &str) -> Result<reqwest::Response>
    where
        F: Fn(reqwest::Client) -> reqwest::RequestBuilder,
    {
        let mut last_error: Option<anyhow::Error> = None;
        for attempt in 0..2 {
            let client = if attempt == 0 {
                self.client.clone()
            } else {
                gemini_http_client()
            };
            match make(client).send().await {
                Ok(response) => return Ok(response),
                Err(err) if attempt == 0 && is_transient_gemini_transport_error(&err) => {
                    last_error = Some(err.into());
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
                Err(err) => {
                    return Err(err).with_context(|| format!("Gemini request to {} failed", url));
                }
            }
        }
        let err = last_error.unwrap_or_else(|| anyhow::anyhow!("Gemini request failed"));
        Err(err).with_context(|| format!("Gemini request to {} failed", url))
    }

    /// POST a JSON body to the official Gemini Developer API, authenticating
    /// with an `x-goog-api-key` header.
    async fn post_json_api_key<T: DeserializeOwned>(
        &self,
        url: &str,
        api_key: &str,
        body: &impl Serialize,
        label: &str,
    ) -> Result<T> {
        let body_value =
            serde_json::to_value(body).context("Failed to serialize Gemini request body")?;
        // Short-window burst limits (429) and transient 5xx clear within
        // seconds, so retry them with backoff instead of failing the turn.
        let mut attempt: u32 = 0;
        loop {
            let resp = self
                .send_with_retry(
                    |client| {
                        client
                            .post(url)
                            .header("x-goog-api-key", api_key)
                            .header(reqwest::header::CONTENT_TYPE, "application/json")
                            .json(&body_value)
                    },
                    url,
                )
                .await?;

            if !resp.status().is_success() {
                let status = resp.status();
                let retry_after = factr_provider_core::retry_after::retry_after(resp.headers());
                let body = factr_base::util::http_error_body(resp, "HTTP error").await;
                if let Some(delay) = gemini_http_retry_delay(
                    label,
                    status,
                    &body,
                    attempt,
                    retry_after.map(|hint| hint.remaining()),
                ) {
                    factr_base::logging::warn(&format!(
                        "Gemini {} hit transient HTTP {} (attempt {}/{}); retrying in {:?}",
                        label,
                        status.as_u16(),
                        attempt + 1,
                        MAX_HTTP_RETRIES,
                        delay
                    ));
                    attempt += 1;
                    tokio::time::sleep(delay).await;
                    continue;
                }
                return Err(factr_provider_core::retry_after::error_with_retry_after(
                    format!(
                        "Gemini request {} failed (HTTP {}): {}",
                        label,
                        status,
                        body.trim()
                    ),
                    retry_after,
                ));
            }

            return resp
                .json()
                .await
                .with_context(|| format!("Failed to parse Gemini {} response", label));
        }
    }

    /// GET a JSON resource from the official Gemini Developer API using an
    /// `x-goog-api-key` header.
    async fn get_json_api_key<T: DeserializeOwned>(
        &self,
        url: &str,
        api_key: &str,
        label: &str,
    ) -> Result<T> {
        let resp = self
            .send_with_retry(
                |client| {
                    client
                        .get(url)
                        .header("x-goog-api-key", api_key)
                        .header(reqwest::header::CONTENT_TYPE, "application/json")
                },
                url,
            )
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = factr_base::util::http_error_body(resp, "HTTP error").await;
            anyhow::bail!("Gemini {} failed (HTTP {}): {}", label, status, body.trim());
        }

        resp.json()
            .await
            .with_context(|| format!("Failed to parse Gemini {} response", label))
    }

    /// Recover from a tool-schema rejection by learning what
    /// `generateContent` refused and re-sending the turn without it.
    ///
    /// factr advertises every tool on every request, so one construct the
    /// endpoint dislikes 400s the whole session rather than one tool. The
    /// historical fix was to append the keyword to a deny-list and ship a
    /// release (#754, #655); this recovers in the same turn instead, and
    /// `factr-schema-dialect` remembers it so later requests never send it.
    ///
    /// Returns `None` when the error is not a recoverable schema rejection,
    /// so the caller falls through to its normal error handling. The quirk
    /// store reports a construct as newly-learned only once, which is what
    /// bounds this to a single retry per distinct construct.
    #[expect(
        clippy::too_many_arguments,
        reason = "mirrors generate_content so the retry re-sends an identical turn"
    )]
    async fn retry_after_schema_rejection(
        &self,
        error: &str,
        state: &GeminiRuntimeState,
        model: &str,
        messages: &[Message],
        tools: &[ToolDefinition],
        system: &str,
        resume_session_id: Option<&str>,
    ) -> Option<Result<GenerateContentEnvelope>> {
        let dialect = &factr_schema_dialect::registry::GEMINI;
        match factr_schema_dialect::recover_from_error(error, dialect) {
            factr_schema_dialect::RecoveryAction::NotSchemaRelated => None,
            factr_schema_dialect::RecoveryAction::Unrecoverable { hint } => {
                factr_base::logging::warn(&format!("Gemini tool-schema rejection: {hint}"));
                None
            }
            factr_schema_dialect::RecoveryAction::RetryWithoutConstruct { description } => {
                factr_base::logging::warn(&format!("Gemini {description}"));
                Some(
                    self.generate_content(state, model, messages, tools, system, resume_session_id)
                        .await,
                )
            }
        }
    }

    async fn generate_content(
        &self,
        state: &GeminiRuntimeState,
        model: &str,
        messages: &[Message],
        tools: &[ToolDefinition],
        system: &str,
        _resume_session_id: Option<&str>,
    ) -> Result<GenerateContentEnvelope> {
        let api_key = Self::api_key()?;
        let request = VertexGenerateContentRequest {
            contents: build_contents(messages),
            system_instruction: build_system_instruction_with_tool_guard(system, !tools.is_empty()),
            tools: build_tools(tools),
            tool_config: if tools.is_empty() {
                None
            } else {
                Some(GeminiToolConfig {
                    function_calling_config: GeminiFunctionCallingConfig { mode: "AUTO" },
                })
            },
            // The Developer API has no session concept.
            session_id: None,
        };

        let contents_value = serde_json::to_value(&request.contents).unwrap_or(Value::Null);
        let content_items = contents_value.as_array().cloned().unwrap_or_default();
        let system_value = request
            .system_instruction
            .as_ref()
            .and_then(|system| serde_json::to_value(system).ok());
        let tools_value = request
            .tools
            .as_ref()
            .and_then(|tools| serde_json::to_value(tools).ok());
        let payload = json!({
            "model": model,
            "contents": contents_value,
            "system_instruction": system_value.as_ref(),
            "tools": tools_value.as_ref(),
            "tool_config": &request.tool_config,
        });
        factr_provider_core::fingerprint::log_provider_canonical_input(
            "gemini",
            model,
            "gemini_generate_content",
            &payload,
            &content_items,
            system_value.as_ref(),
            tools_value.as_ref(),
            request.tools.as_ref().map(|tools| tools.len()),
            &[("session_id_present", (!state.session_id.is_empty()).to_string())],
        );

        let url = format!(
            "{}/models/{}:generateContent",
            Self::developer_api_base_url(),
            model
        );
        let response: VertexGenerateContentResponse = self
            .post_json_api_key(&url, &api_key, &request, "generateContent")
            .await
            .context("Gemini generateContent failed")?;
        Ok(GenerateContentEnvelope {
            trace_id: None,
            response: Some(response),
        })
    }
}

impl Default for GeminiProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Provider for GeminiProvider {
    async fn complete(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
        system: &str,
        resume_session_id: Option<&str>,
    ) -> Result<EventStream> {
        let model = self.model();
        let messages = messages.to_vec();
        let tools = tools.to_vec();
        let system = system.to_string();
        let resume_session_id = resume_session_id.map(|value| value.to_string());
        let state_cache = self.state.clone();
        let provider = self.clone();
        let (tx, rx) = mpsc::channel::<Result<StreamEvent>>(100);

        tokio::spawn(async move {
            let _ = tx
                .send(Ok(StreamEvent::ConnectionType {
                    connection: "https".to_string(),
                }))
                .await;
            let _ = tx
                .send(Ok(StreamEvent::ConnectionPhase {
                    phase: ConnectionPhase::Authenticating,
                }))
                .await;

            let state = {
                let provider = GeminiProvider {
                    client: provider.client.clone(),
                    model: provider.model.clone(),
                    state: state_cache.clone(),
                    fetched_models: provider.fetched_models.clone(),
                };
                match provider.ensure_state().await {
                    Ok(state) => state,
                    Err(err) => {
                        let _ = tx.send(Err(err)).await;
                        return;
                    }
                }
            };

            let _ = tx
                .send(Ok(StreamEvent::SessionId(
                    resume_session_id
                        .clone()
                        .unwrap_or_else(|| state.session_id.clone()),
                )))
                .await;
            let _ = tx
                .send(Ok(StreamEvent::ConnectionPhase {
                    phase: ConnectionPhase::SendingRequest,
                }))
                .await;
            let _ = tx
                .send(Ok(StreamEvent::ConnectionPhase {
                    phase: ConnectionPhase::WaitingForResponse,
                }))
                .await;

            let response = match provider
                .generate_content(
                    &state,
                    &model,
                    &messages,
                    &tools,
                    &system,
                    resume_session_id.as_deref(),
                )
                .await
            {
                Ok(response) => response,
                Err(err) if is_gemini_model_not_found_error(&err) => {
                    let mut fallback_response = None;
                    let mut last_err = err;
                    for fallback_model in gemini_fallback_models(&model) {
                        factr_base::logging::warn(&format!(
                            "Gemini model '{}' was not found; retrying with fallback '{}'",
                            model, fallback_model
                        ));
                        match provider
                            .generate_content(
                                &state,
                                fallback_model,
                                &messages,
                                &tools,
                                &system,
                                resume_session_id.as_deref(),
                            )
                            .await
                        {
                            Ok(response) => {
                                let _ = provider.set_model(fallback_model);
                                fallback_response = Some(response);
                                break;
                            }
                            Err(err) => {
                                last_err = err;
                            }
                        }
                    }

                    match fallback_response {
                        Some(response) => response,
                        None => {
                            let _ = tx.send(Err(last_err)).await;
                            return;
                        }
                    }
                }
                Err(err) => {
                    // A tool schema `generateContent` rejects 400s every turn,
                    // so the provider is unusable until factr ships a new
                    // keyword. Learn the rejected construct from the error,
                    // persist it, and retry this turn without it. See
                    // `factr-schema-dialect`.
                    match provider
                        .retry_after_schema_rejection(
                            &err.to_string(),
                            &state,
                            &model,
                            &messages,
                            &tools,
                            &system,
                            resume_session_id.as_deref(),
                        )
                        .await
                    {
                        Some(Ok(response)) => response,
                        Some(Err(retry_err)) => {
                            let _ = tx.send(Err(retry_err)).await;
                            return;
                        }
                        None => {
                            let _ = tx.send(Err(err)).await;
                            return;
                        }
                    }
                }
            };

            let _ = tx
                .send(Ok(StreamEvent::ConnectionPhase {
                    phase: ConnectionPhase::Streaming,
                }))
                .await;

            if let Some(usage) = response
                .response
                .as_ref()
                .and_then(|response| response.usage_metadata.as_ref())
            {
                let _ = tx
                    .send(Ok(StreamEvent::TokenUsage {
                        input_tokens: usage.prompt_token_count,
                        output_tokens: usage.candidates_token_count,
                        cache_read_input_tokens: usage.cached_content_token_count,
                        cache_creation_input_tokens: None,
                    }))
                    .await;
            }

            let response_body = response.response;

            let candidate = response_body
                .as_ref()
                .and_then(|response| response.candidates.as_ref())
                .and_then(|candidates| candidates.first())
                .cloned();

            if candidate.is_none() {
                if let Some(feedback) = response_body
                    .as_ref()
                    .and_then(|response| response.prompt_feedback.as_ref())
                {
                    let block_reason = feedback.block_reason.as_deref().unwrap_or("unspecified");
                    let detail = feedback
                        .block_reason_message
                        .as_deref()
                        .filter(|msg| !msg.trim().is_empty())
                        .map(|msg| format!(": {}", msg.trim()))
                        .unwrap_or_default();
                    factr_base::logging::warn(&format!(
                        "Gemini blocked the prompt ({}){}",
                        block_reason, detail
                    ));
                    let _ = tx
                        .send(Ok(StreamEvent::MessageEnd {
                            stop_reason: Some(
                                factr_provider_core::refusal::REFUSAL_STOP_REASON.to_string(),
                            ),
                        }))
                        .await;
                    return;
                }

                let _ = tx
                    .send(Err(anyhow::anyhow!(
                        "Gemini returned no candidates for generateContent"
                    )))
                    .await;
                return;
            }

            let mut stop_reason = None;
            if let Some(candidate) = candidate {
                stop_reason = candidate
                    .finish_reason
                    .clone()
                    .map(|reason| reason.to_lowercase())
                    .map(factr_provider_core::refusal::normalize_stop_reason);
                if candidate.content.is_none()
                    && candidate.finish_reason.as_deref() == Some("RECITATION")
                {
                    let reason = candidate.finish_reason.as_deref().unwrap_or("unknown");
                    let detail = candidate
                        .finish_message
                        .as_deref()
                        .filter(|msg| !msg.trim().is_empty())
                        .map(|msg| format!(": {}", msg.trim()))
                        .unwrap_or_default();
                    let _ = tx
                        .send(Err(anyhow::anyhow!(
                            "Gemini stopped without content ({}){}",
                            reason,
                            detail
                        )))
                        .await;
                    return;
                }
                // Track whether this candidate produced any usable output (text or
                // a tool call). Gemini-3 thinking models intermittently emit
                // Python-style pseudo-code instead of a clean functionCall and
                // finish with `MALFORMED_FUNCTION_CALL` and empty content; surface
                // that as a retryable error below rather than a silent empty turn.
                let mut produced_output = false;
                if let Some(content) = candidate.content {
                    // Gemini 3 attaches a `thoughtSignature` to function-call
                    // parts (and occasionally to a standalone preceding part).
                    // Replay it via a ToolUseSignature event so it is persisted
                    // on the ToolUse block and resent on later turns; the API
                    // rejects follow-up turns whose functionCall omits it
                    // ("Function call is missing a thought_signature").
                    let mut pending_signature: Option<String> = None;
                    for part in content.parts {
                        let part_signature = part
                            .thought_signature
                            .as_ref()
                            .filter(|sig| !sig.is_empty())
                            .cloned();
                        if let Some(text) = part.text
                            && !text.is_empty()
                        {
                            produced_output = true;
                            let _ = tx.send(Ok(StreamEvent::TextDelta(text))).await;
                        }
                        if let Some(function_call) = part.function_call {
                            produced_output = true;
                            let signature =
                                part_signature.clone().or_else(|| pending_signature.take());
                            let raw_call_id = function_call
                                .id
                                .clone()
                                .unwrap_or_else(|| Uuid::new_v4().to_string());
                            let call_id = factr_message_types::sanitize_tool_id(&raw_call_id);
                            let _ = tx
                                .send(Ok(StreamEvent::ToolUseStart {
                                    id: call_id,
                                    name: function_call.name,
                                }))
                                .await;
                            let _ = tx
                                .send(Ok(StreamEvent::ToolInputDelta(
                                    function_call.args.to_string(),
                                )))
                                .await;
                            let _ = tx.send(Ok(StreamEvent::ToolUseEnd)).await;
                            if let Some(signature) = signature {
                                let _ = tx.send(Ok(StreamEvent::ToolUseSignature(signature))).await;
                            }
                        } else if let Some(signature) = part_signature {
                            // Standalone signature part; remember it for the next
                            // function call in this candidate.
                            pending_signature = Some(signature);
                        }
                    }
                    // A thought signature not consumed by a following function
                    // call (e.g. a pure-text reasoning turn) is still an opaque
                    // reasoning signal. Surface it as a ThinkingSignatureDelta
                    // instead of dropping it.
                    if let Some(signature) = pending_signature.take() {
                        let _ = tx
                            .send(Ok(StreamEvent::ThinkingSignatureDelta(signature)))
                            .await;
                    }
                }

                // An abnormal finish (typically Gemini-3's intermittent
                // `MALFORMED_FUNCTION_CALL`) that yielded no text and no tool call
                // is a dead turn: surface it as a retryable error instead of a
                // silent empty `MessageEnd`. `STOP`/`MAX_TOKENS` are normal.
                if !produced_output {
                    let abnormal = candidate
                        .finish_reason
                        .as_deref()
                        .map(|reason| {
                            !factr_provider_core::refusal::is_refusal_reason(reason)
                                && !matches!(
                                    reason.to_ascii_uppercase().as_str(),
                                    "STOP" | "MAX_TOKENS" | "FINISH_REASON_UNSPECIFIED" | ""
                                )
                        })
                        .unwrap_or(false);
                    if abnormal {
                        let reason = candidate.finish_reason.as_deref().unwrap_or("unknown");
                        let detail = candidate
                            .finish_message
                            .as_deref()
                            .filter(|msg| !msg.trim().is_empty())
                            .map(|msg| {
                                format!(": {}", factr_base::util::truncate_str(msg.trim(), 300))
                            })
                            .unwrap_or_default();
                        let _ = tx
                            .send(Err(anyhow::anyhow!(
                                "Gemini returned no usable output (finish_reason={reason}){detail}"
                            )))
                            .await;
                        return;
                    }
                }
            }

            let _ = tx.send(Ok(StreamEvent::MessageEnd { stop_reason })).await;
        });

        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    fn name(&self) -> &'static str {
        "gemini"
    }

    fn model(&self) -> String {
        self.model
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn supports_image_input(&self) -> bool {
        true
    }

    fn set_model(&self, model: &str) -> Result<()> {
        // See `strip_own_model_prefix`: `--provider gemini` routes through this
        // runtime directly, so session restore hands it `gemini:<model>`.
        let trimmed = factr_provider_core::strip_own_model_prefix(model, "gemini:");
        if trimmed.is_empty() {
            anyhow::bail!("Gemini model cannot be empty");
        }
        *self
            .model
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = trimmed.to_string();
        Ok(())
    }

    fn available_models(&self) -> Vec<&'static str> {
        AVAILABLE_MODELS.to_vec()
    }

    fn available_models_display(&self) -> Vec<String> {
        let discovered = self
            .fetched_models
            .read()
            .map(|guard| guard.clone())
            .unwrap_or_default();
        if discovered.is_empty() {
            return vec![self.model()];
        }

        merge_gemini_model_lists(
            discovered
                .into_iter()
                .chain(std::iter::once(self.model()))
                .collect(),
        )
    }

    fn available_models_for_switching(&self) -> Vec<String> {
        self.available_models_display()
    }

    fn model_routes(&self) -> Vec<factr_provider_core::ModelRoute> {
        self.available_models_display()
            .into_iter()
            .map(|model| factr_provider_core::ModelRoute {
                model,
                provider: "Gemini".to_string(),
                api_method: "gemini-api-key".to_string(),
                available: true,
                detail: String::new(),
                usage: None,
                cheapness: None,
            })
            .collect()
    }

    async fn prefetch_models(&self) -> Result<()> {
        let _ = self.refresh_available_models().await?;
        Ok(())
    }

    fn supports_compaction(&self) -> bool {
        // No native server-side compaction exists for this provider, so factr's
        // own summary compaction is the only thing standing between a long
        // session and a hard context-limit rejection. Returning `false` here
        // disabled the entire compaction block in `Agent::messages_for_provider`
        // — including the emergency hard-compact and payload truncation at the
        // critical threshold — leaving these sessions with no safety net at all.
        true
    }

    fn fork(&self) -> Arc<dyn Provider> {
        Arc::new(Self {
            client: self.client.clone(),
            model: Arc::new(RwLock::new(self.model())),
            state: self.state.clone(),
            fetched_models: self.fetched_models.clone(),
        })
    }

    async fn invalidate_credentials(&self) {
        let mut guard = self.state.lock().await;
        *guard = None;
    }
}

impl Clone for GeminiProvider {
    fn clone(&self) -> Self {
        Self {
            client: self.client.clone(),
            model: self.model.clone(),
            state: self.state.clone(),
            fetched_models: self.fetched_models.clone(),
        }
    }
}

const MAX_HTTP_RETRIES: u32 = 5;

fn gemini_http_retry_delay(
    method: &str,
    status: reqwest::StatusCode,
    body: &str,
    attempt: u32,
    retry_after: Option<Duration>,
) -> Option<Duration> {
    if attempt >= MAX_HTTP_RETRIES || method != "generateContent" {
        return None;
    }
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        let lower = body.to_ascii_lowercase();
        if [
            "daily",
            "per day",
            "per_day",
            "perday",
            "billing",
            "insufficient_quota",
            "quota_exhausted",
        ]
        .iter()
        .any(|reason| lower.contains(reason))
        {
            return None;
        }
    } else if !matches!(status.as_u16(), 500 | 502 | 503 | 504) {
        return None;
    }
    Some(factr_provider_core::retry_after::retry_delay(
        attempt,
        1500,
        retry_after,
    ))
}

fn gemini_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(format!("factr/{} (gemini)", env!("CARGO_PKG_VERSION")))
        .http1_only()
        .connect_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(90))
        .pool_max_idle_per_host(0)
        .tcp_keepalive(Some(Duration::from_secs(30)))
        .build()
        .unwrap_or_else(|_| factr_provider_core::shared_http_client())
}

fn is_transient_gemini_transport_error(err: &reqwest::Error) -> bool {
    // Delegate to the shared transport classifier so Gemini recognizes the
    // same transient faults as every other provider (close_notify, connection
    // reset, DNS, HTTP/2 stream errors, ...), plus reqwest's structured
    // connect/timeout flags which don't always surface in the message text.
    err.is_connect()
        || err.is_timeout()
        || factr_provider_core::is_transient_transport_error(&err.to_string())
}

fn is_gemini_model_not_found_error(err: &anyhow::Error) -> bool {
    let lower = format!("{err:#}").to_ascii_lowercase();
    lower.contains("http 404")
        || lower.contains("\"status\": \"not_found\"")
        || lower.contains("requested entity was not found")
}

#[cfg(test)]
#[path = "gemini_tests.rs"]
mod tests;
