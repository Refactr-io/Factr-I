//! Gemini pure protocol types and helpers (compatibility shim).
//!
//! The Gemini provider *runtime* (`GeminiProvider`) now lives in the
//! downstream `factr-provider-gemini-runtime` crate so provider edits do not
//! rebuild the base -> app-core -> tui spine. The binary's composition root
//! registers it via [`crate::provider::external`]. This module keeps the pure
//! request/response types and helpers (from `factr-provider-gemini`)
//! importable at their historical `crate::provider::gemini::*` paths.

pub use factr_provider_gemini::{
    AVAILABLE_MODELS, DEFAULT_MODEL, GEMINI_API_ENDPOINT, GEMINI_API_VERSION, GeminiCandidate,
    GeminiContent, GeminiFunctionCall, GeminiFunctionCallingConfig, GeminiFunctionDeclaration,
    GeminiFunctionResponse, GeminiPart, GeminiPromptFeedback, GeminiRuntimeState, GeminiTool,
    GeminiToolConfig, GeminiUsageMetadata, GenerateContentEnvelope, InlineData,
    VertexGenerateContentRequest, VertexGenerateContentResponse, build_contents,
    build_system_instruction_with_tool_guard, build_tools, extract_gemini_model_ids,
    gemini_fallback_models, is_gemini_model_id, merge_gemini_model_lists,
};
