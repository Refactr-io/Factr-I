use factr_message_types::{
    ContentBlock, Message, Role, TOOL_OUTPUT_MISSING_TEXT, ToolDefinition, sanitize_tool_id,
};
use factr_provider_core::anthropic_map_tool_name_for_oauth as map_tool_name_for_oauth;
use serde::Serialize;
use serde_json::{Value, json};

/// Claude Code billing attribution text observed in the official CLI's system
/// prompt blocks.
pub const OAUTH_BILLING_HEADER: &str = "cc_version=2.1.280; cc_entrypoint=sdk-cli; cch=33f85;";

const CLAUDE_CODE_IDENTITY: &str = "You are a Claude agent, built on Anthropic's Claude Agent SDK.";

/// Minimal user turn appended when a formatted conversation would otherwise end
/// on an assistant message, which Anthropic rejects on non-prefill models.
pub(crate) const CONTINUATION_USER_TURN: &str = "Continue.";

pub fn format_messages(messages: &[Message], is_oauth: bool) -> Vec<ApiMessage> {
    use std::collections::HashSet;

    // Pre-pass: drop duplicate tool_results for the same tool_use_id.
    //
    // Anthropic rejects the whole request (400 "unexpected `tool_use_id` found
    // in `tool_result` blocks") when a tool_use_id appears twice, because after
    // same-role merging only the first result lines up with the tool_use in the
    // preceding assistant message. Duplicates are produced by the missing
    // tool-output repair racing a still-running tool: the repair inserts a
    // synthetic placeholder result, then the real result lands moments later,
    // and the conversation is permanently unsendable. Prefer the real output
    // over the synthetic placeholder, and otherwise keep the first occurrence.
    let messages = &dedupe_tool_results(messages);

    // First pass: collect all tool_use IDs and tool_result IDs
    let mut tool_use_ids: HashSet<String> = HashSet::new();
    let mut tool_result_ids: HashSet<String> = HashSet::new();

    for msg in messages {
        for block in &msg.content {
            match block {
                ContentBlock::ToolUse { id, .. } => {
                    tool_use_ids.insert(id.clone());
                }
                ContentBlock::ToolResult { tool_use_id, .. } => {
                    tool_result_ids.insert(tool_use_id.clone());
                }
                _ => {}
            }
        }
    }

    // Find dangling tool_uses (no matching tool_result)
    let dangling: HashSet<_> = tool_use_ids.difference(&tool_result_ids).cloned().collect();
    if !dangling.is_empty() {
        factr_logging::info(&format!(
            "[anthropic] Repairing {} dangling tool_use(s) by injecting synthetic tool_results",
            dangling.len()
        ));
    }

    // Second pass: build messages, injecting synthetic tool_results after assistant messages
    // that have dangling tool_uses
    let mut result: Vec<ApiMessage> = Vec::new();

    for msg in messages {
        let role = match msg.role {
            Role::User => "user",
            Role::Assistant => "assistant",
        };

        let content = format_content_blocks(&msg.content, is_oauth);

        if !content.is_empty() {
            result.push(ApiMessage {
                role: role.to_string(),
                content,
            });
        }

        // If this is an assistant message with dangling tool_uses, inject synthetic results
        if matches!(msg.role, Role::Assistant) {
            let mut synthetic_results: Vec<ApiContentBlock> = Vec::new();
            for block in &msg.content {
                if let ContentBlock::ToolUse { id, .. } = block
                    && dangling.contains(id)
                {
                    synthetic_results.push(ApiContentBlock::ToolResult {
                        tool_use_id: sanitize_tool_id(id),
                        content: ToolResultContent::Text(
                            "[Session interrupted before tool execution completed]".to_string(),
                        ),
                        is_error: true,
                        cache_control: None,
                    });
                }
            }
            if !synthetic_results.is_empty() {
                result.push(ApiMessage {
                    role: "user".to_string(),
                    content: synthetic_results,
                });
            }
        }
    }

    // Third pass: merge consecutive messages of the same role
    // Anthropic API requires strictly alternating user/assistant messages
    let pre_merge_count = result.len();
    let mut merged: Vec<ApiMessage> = Vec::new();
    for msg in result {
        if let Some(last) = merged.last_mut()
            && last.role == msg.role
        {
            last.content.extend(msg.content);
            continue;
        }
        merged.push(msg);
    }

    if merged.len() != pre_merge_count {
        factr_logging::info(&format!(
            "[anthropic] Merged {} consecutive same-role messages",
            pre_merge_count - merged.len()
        ));
    }

    // Anthropic rejects a request whose final message is an assistant turn on
    // models that do not support assistant prefill ("This model does not support
    // assistant message prefill. The conversation must end with a user message.").
    // factr never intends to prefill, so a trailing assistant turn here is always
    // an upstream accident: the reload auto-resume path starts a turn with empty
    // user content and delivers its continuation as a system reminder, leaving the
    // transcript ending on the interrupted assistant turn. Repair the shape at the
    // last formatting step. See issue #600.
    if merged.last().is_some_and(|last| last.role == "assistant") {
        factr_logging::warn(
            "[anthropic] Conversation ended with an assistant message; appending a \
             continuation user turn to avoid a model prefill rejection (400)",
        );
        merged.push(ApiMessage {
            role: "user".to_string(),
            content: vec![ApiContentBlock::Text {
                text: CONTINUATION_USER_TURN.to_string(),
                cache_control: None,
            }],
        });
    }

    // Validate: check each assistant message with tool_use has matching tool_result in next user message
    for (i, msg) in merged.iter().enumerate() {
        if msg.role == "assistant" {
            let tool_uses: Vec<&String> = msg
                .content
                .iter()
                .filter_map(|b| {
                    if let ApiContentBlock::ToolUse { id, .. } = b {
                        Some(id)
                    } else {
                        None
                    }
                })
                .collect();

            if !tool_uses.is_empty() {
                // Check next message
                if let Some(next) = merged.get(i + 1) {
                    if next.role != "user" {
                        factr_logging::warn(&format!(
                            "[anthropic] Message {} has tool_use but next message is {} (should be user)",
                            i, next.role
                        ));
                    } else {
                        let tool_results: std::collections::HashSet<&String> = next
                            .content
                            .iter()
                            .filter_map(|b| {
                                if let ApiContentBlock::ToolResult { tool_use_id, .. } = b {
                                    Some(tool_use_id)
                                } else {
                                    None
                                }
                            })
                            .collect();

                        for tu_id in &tool_uses {
                            if !tool_results.contains(*tu_id) {
                                factr_logging::warn(&format!(
                                    "[anthropic] Message {} has tool_use {} but no matching tool_result in message {}",
                                    i,
                                    tu_id,
                                    i + 1
                                ));
                            }
                        }
                    }
                } else {
                    factr_logging::warn(&format!(
                        "[anthropic] Message {} has tool_use but no next message",
                        i
                    ));
                }
            }
        }
    }

    merged
}

/// Returns true when a tool_result body is one of the synthetic placeholders
/// injected by the missing tool-output repair paths rather than real output.
fn is_placeholder_tool_result(content: &str, is_error: Option<bool>) -> bool {
    is_error.unwrap_or(false)
        && (content.contains(TOOL_OUTPUT_MISSING_TEXT)
            || content.contains("[Session interrupted before tool execution completed]"))
}

/// Remove duplicate `tool_result` blocks so each `tool_use_id` is answered
/// exactly once, preferring real output over a synthetic placeholder.
/// Messages left with no content at all are dropped by the caller's
/// `!content.is_empty()` guard.
fn dedupe_tool_results(messages: &[Message]) -> Vec<Message> {
    use std::collections::HashMap;

    // Winner position per tool_use_id: the first real result if one exists,
    // otherwise the first occurrence at all.
    let mut winner: HashMap<&str, (usize, usize)> = HashMap::new();
    let mut winner_is_real: HashMap<&str, bool> = HashMap::new();
    let mut duplicate_seen = false;

    for (mi, msg) in messages.iter().enumerate() {
        for (bi, block) in msg.content.iter().enumerate() {
            let ContentBlock::ToolResult {
                tool_use_id,
                content,
                is_error,
            } = block
            else {
                continue;
            };
            let real = !is_placeholder_tool_result(content, *is_error);
            match winner_is_real.get(tool_use_id.as_str()) {
                None => {
                    winner.insert(tool_use_id, (mi, bi));
                    winner_is_real.insert(tool_use_id, real);
                }
                Some(false) if real => {
                    // Upgrade a placeholder winner to the real output.
                    winner.insert(tool_use_id, (mi, bi));
                    winner_is_real.insert(tool_use_id, true);
                    duplicate_seen = true;
                }
                Some(_) => duplicate_seen = true,
            }
        }
    }

    if !duplicate_seen {
        return messages.to_vec();
    }

    let dropped = std::cell::Cell::new(0usize);
    let out: Vec<Message> = messages
        .iter()
        .enumerate()
        .map(|(mi, msg)| {
            let mut msg = msg.clone();
            let mut bi = 0usize;
            msg.content.retain(|block| {
                let index = bi;
                bi += 1;
                let ContentBlock::ToolResult { tool_use_id, .. } = block else {
                    return true;
                };
                let keep = winner.get(tool_use_id.as_str()) == Some(&(mi, index));
                if !keep {
                    dropped.set(dropped.get() + 1);
                }
                keep
            });
            msg
        })
        .collect();

    if dropped.get() > 0 {
        factr_logging::warn(&format!(
            "[anthropic] Dropped {} duplicate tool_result block(s); each tool_use_id may be \
             answered only once",
            dropped.get()
        ));
    }
    out
}

/// Convert our ContentBlock to Anthropic API format
pub fn format_content_blocks(blocks: &[ContentBlock], is_oauth: bool) -> Vec<ApiContentBlock> {
    let mut result: Vec<ApiContentBlock> = Vec::new();
    for block in blocks {
        match block {
            ContentBlock::Text { text, .. } => {
                // A text block that immediately follows an image-bearing tool_result is the
                // "[Attached image associated with the preceding tool result: ...]" label
                // emitted alongside image tool outputs. The Anthropic API requires every
                // tool_result for a parallel tool-call turn to be contiguous in the next user
                // message; a sibling text block wedged between tool_results makes the API
                // report later tool_use ids as missing their tool_result. Fold the label into
                // the tool_result's content blocks so the tool_results stay contiguous.
                if let Some(ApiContentBlock::ToolResult {
                    content: ToolResultContent::Blocks(blocks),
                    ..
                }) = result.last_mut()
                    && blocks
                        .iter()
                        .any(|b| matches!(b, ToolResultContentBlock::Image { .. }))
                {
                    blocks.push(ToolResultContentBlock::Text { text: text.clone() });
                } else {
                    result.push(ApiContentBlock::Text {
                        text: text.clone(),
                        cache_control: None,
                    });
                }
            }
            ContentBlock::AnthropicThinking {
                thinking,
                signature,
            } => {
                result.push(ApiContentBlock::Thinking {
                    thinking: thinking.clone(),
                    signature: signature.clone(),
                });
            }
            ContentBlock::ToolUse {
                id, name, input, ..
            } => {
                result.push(ApiContentBlock::ToolUse {
                    id: sanitize_tool_id(id),
                    name: if is_oauth {
                        map_tool_name_for_oauth(name)
                    } else {
                        name.clone()
                    },
                    input: if input.is_object() {
                        input.clone()
                    } else {
                        serde_json::json!({})
                    },
                    cache_control: None,
                });
            }
            ContentBlock::ToolResult {
                tool_use_id,
                content,
                is_error,
            } => {
                result.push(ApiContentBlock::ToolResult {
                    tool_use_id: sanitize_tool_id(tool_use_id),
                    content: ToolResultContent::Text(content.clone()),
                    is_error: is_error.unwrap_or(false),
                    cache_control: None,
                });
            }
            ContentBlock::Image { media_type, data } => {
                let img_block = ToolResultContentBlock::Image {
                    source: ApiImageSource {
                        kind: "base64".to_string(),
                        media_type: media_type.clone(),
                        data: data.clone(),
                    },
                };
                if let Some(ApiContentBlock::ToolResult { content, .. }) = result.last_mut() {
                    match content {
                        ToolResultContent::Text(text) => {
                            let text_block = ToolResultContentBlock::Text {
                                text: std::mem::take(text),
                            };
                            *content = ToolResultContent::Blocks(vec![text_block, img_block]);
                        }
                        ToolResultContent::Blocks(blocks) => {
                            blocks.push(img_block);
                        }
                    }
                } else {
                    result.push(ApiContentBlock::Image {
                        source: ApiImageSource {
                            kind: "base64".to_string(),
                            media_type: media_type.clone(),
                            data: data.clone(),
                        },
                    });
                }
            }
            _ => {}
        }
    }
    result
}

/// Convert tool definitions to Anthropic API format
/// Adds cache_control to the last tool for prompt caching
/// Local tool names that are represented by the curated Claude-Code builtin
/// definitions in OAuth mode. These keep their hand-tuned schemas/descriptions
/// (which the Anthropic subscription endpoint expects) instead of the raw
/// registry definitions; every other tool is forwarded as-is (see #409).
/// Local tool names that already have a hand-tuned curated OAuth definition
/// above, so the registry pass must not forward them a second time.
///
/// `schedule` is deliberately absent: its curated `ScheduleWakeup` schema had
/// drifted from the real tool (it advertised `delaySeconds`/`reason`/`prompt`
/// while the handler requires `task` + `wake_in_minutes`/`wake_at`), so every
/// call failed with "task is required for action=create" (#706). Forwarding the
/// real schema under the remapped name keeps the two in sync by construction.
/// `bash` is likewise forwarded: its curated schema omitted timeout units and
/// execution options (#1223). Only its OAuth name changes, not its definition.
const OAUTH_BUILTIN_LOCAL_TOOLS: &[&str] = &[
    "subagent",
    "edit",
    "glob",
    "grep",
    "read",
    "skill_manage",
    "write",
];

/// Normalize a tool schema for Anthropic's `input_schema`.
///
/// Anthropic accepts JSON Schema combinators inside object properties but
/// rejects `oneOf`/`anyOf`/`allOf` at the top level, and requires an object
/// schema with a `properties` map. The subset and the rewrites live in
/// `factr-schema-dialect` so every provider shares one implementation and one
/// set of regression tests.
///
/// Widening a top-level combiner loses the per-branch constraint, which is
/// intended: runtime tool deserialization remains the authority on which
/// combination is actually valid.
fn anthropic_input_schema(schema: &Value) -> Value {
    factr_schema_dialect::normalize(schema, &factr_schema_dialect::registry::ANTHROPIC)
}

pub fn format_tools(tools: &[ToolDefinition], is_oauth: bool, cache_ttl_1h: bool) -> Vec<ApiTool> {
    if is_oauth {
        // A curated builtin may only be advertised when at least one backing
        // local tool is actually registered. Otherwise the model calls e.g.
        // `Agent`/`Glob`, the reverse mapping resolves to `subagent`/`glob`,
        // and the registry lookup fails with "Unknown tool" (see #572).
        let has_backing = |candidates: &[&str]| {
            candidates
                .iter()
                .any(|candidate| tools.iter().any(|tool| tool.name == *candidate))
        };
        // Curated Claude-Code builtin tool definitions. These remain hand-tuned
        // because the Anthropic OAuth (subscription) endpoint expects the
        // builtin names with compatible schemas. Anything not represented here
        // is appended from the real registry below so OAuth users keep the full
        // toolset (websearch, webfetch, browser, codesearch, memory, ...).
        let curated: Vec<(&[&str], ApiTool)> = vec![
            (
                &["subagent"],
                ApiTool {
                    name: "Agent".to_string(),
                    description: "Launch a new agent to handle complex, multi-step tasks."
                        .to_string(),
                    input_schema: json!({"type":"object","properties":{"description":{"type":"string"},"prompt":{"type":"string"},"subagent_type":{"type":"string"},"run_in_background":{"type":"boolean"}},"required":["description","prompt"],"additionalProperties":false}),
                    cache_control: None,
                },
            ),
            (
                &["edit"],
                ApiTool {
                    name: "Edit".to_string(),
                    description: "Performs exact string replacements in files.".to_string(),
                    input_schema: json!({"type":"object","properties":{"file_path":{"type":"string"},"old_string":{"type":"string"},"new_string":{"type":"string"},"replace_all":{"type":"boolean","default":false}},"required":["file_path","old_string","new_string"],"additionalProperties":false}),
                    cache_control: None,
                },
            ),
            (
                &["glob"],
                ApiTool {
                    name: "Glob".to_string(),
                    description: "Fast file pattern matching tool.".to_string(),
                    input_schema: json!({"type":"object","properties":{"pattern":{"type":"string"},"path":{"type":"string"}},"required":["pattern"],"additionalProperties":false}),
                    cache_control: None,
                },
            ),
            (
                &["grep"],
                ApiTool {
                    name: "Grep".to_string(),
                    description: "A powerful search tool built on ripgrep.".to_string(),
                    input_schema: json!({"type":"object","properties":{"pattern":{"type":"string"},"path":{"type":"string"},"glob":{"type":"string"},"output_mode":{"type":"string","enum":["content","files_with_matches","count"]},"-B":{"type":"number"},"-A":{"type":"number"},"-C":{"type":"number"},"context":{"type":"number"},"-n":{"type":"boolean"},"-i":{"type":"boolean"},"type":{"type":"string"},"head_limit":{"type":"number"},"offset":{"type":"number"},"multiline":{"type":"boolean"}},"required":["pattern"],"additionalProperties":false}),
                    cache_control: None,
                },
            ),
            (
                &["read"],
                ApiTool {
                    name: "Read".to_string(),
                    description: "Reads a file from the local filesystem.".to_string(),
                    input_schema: json!({"type":"object","properties":{"file_path":{"type":"string"},"offset":{"type":"integer","minimum":0},"limit":{"type":"integer","exclusiveMinimum":0},"pages":{"type":"string"}},"required":["file_path"],"additionalProperties":false}),
                    cache_control: None,
                },
            ),
            (
                &["skill_manage"],
                ApiTool {
                    name: "Skill".to_string(),
                    description: "Execute a skill within the main conversation".to_string(),
                    input_schema: json!({"type":"object","properties":{"skill":{"type":"string"},"args":{"type":"string"}},"required":["skill"],"additionalProperties":false}),
                    cache_control: None,
                },
            ),
            (
                &["write"],
                ApiTool {
                    name: "Write".to_string(),
                    description: "Writes a file to the local filesystem.".to_string(),
                    input_schema: json!({"type":"object","properties":{"file_path":{"type":"string"},"content":{"type":"string"}},"required":["file_path","content"],"additionalProperties":false}),
                    cache_control: None,
                },
            ),
        ];
        let mut out: Vec<ApiTool> = curated
            .into_iter()
            .filter(|(backing, _)| has_backing(backing))
            .map(|(_, tool)| tool)
            .collect();

        // Forward every other registered tool, remapping its name to the
        // OAuth-accepted form. This restores websearch/webfetch/browser/
        // codesearch/memory/swarm/multiedit/open/etc. for subscription users,
        // matching the documented "remap names, keep the full toolset" behavior.
        for tool in tools {
            if OAUTH_BUILTIN_LOCAL_TOOLS.contains(&tool.name.as_str()) {
                continue;
            }
            out.push(ApiTool {
                name: map_tool_name_for_oauth(&tool.name),
                description: tool.description.clone(),
                input_schema: anthropic_input_schema(&tool.input_schema),
                cache_control: None,
            });
        }

        // Move the prompt-cache breakpoint to the final tool in the list.
        if let Some(last) = out.last_mut() {
            last.cache_control = Some(CacheControlParam::ephemeral(cache_ttl_1h));
        }

        return out;
    }

    let len = tools.len();
    tools
        .iter()
        .enumerate()
        .map(|(i, tool)| ApiTool {
            name: tool.name.clone(),
            description: tool.description.clone(),
            input_schema: anthropic_input_schema(&tool.input_schema),
            cache_control: if i == len - 1 {
                Some(CacheControlParam::ephemeral(cache_ttl_1h))
            } else {
                None
            },
        })
        .collect()
}

#[derive(Serialize, Clone)]
pub struct ApiRequest {
    pub model: String,
    pub max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<ApiSystem>,
    pub messages: Vec<ApiMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ApiTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<ApiMetadata>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking: Option<ApiThinking>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_config: Option<ApiOutputConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<String>,
    pub stream: bool,
}

#[derive(Serialize, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ApiThinking {
    Adaptive {
        #[serde(skip_serializing_if = "Option::is_none")]
        display: Option<&'static str>,
        #[serde(skip_serializing_if = "Option::is_none")]
        block_binding: Option<ApiThinkingBlockBinding>,
    },
    Enabled {
        budget_tokens: u32,
    },
}

/// Permit the API to discard stale signed reasoning after compaction or a
/// changed system prompt/tool schema instead of rejecting the whole request.
#[derive(Serialize, Clone)]
pub struct ApiThinkingBlockBinding {
    pub prefix_mismatch_behavior: &'static str,
}

#[derive(Serialize, Clone)]
pub struct ApiOutputConfig {
    pub effort: String,
}

#[derive(Serialize, Clone)]
pub struct ApiMetadata {
    pub user_id: String,
}

#[derive(Serialize, Clone)]
#[serde(untagged)]
pub enum ApiSystem {
    Blocks(Vec<ApiSystemBlock>),
}

/// Cache control for prompt caching
#[derive(Serialize, Clone)]
pub struct CacheControlParam {
    #[serde(rename = "type")]
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ttl: Option<&'static str>,
}

impl CacheControlParam {
    fn ephemeral(cache_ttl_1h: bool) -> Self {
        if cache_ttl_1h {
            Self::ephemeral_1h()
        } else {
            Self {
                kind: "ephemeral",
                ttl: None,
            }
        }
    }

    fn ephemeral_1h() -> Self {
        Self {
            kind: "ephemeral",
            ttl: Some("1h"),
        }
    }
}

#[derive(Serialize, Clone)]
pub struct ApiSystemBlock {
    #[serde(rename = "type")]
    pub block_type: &'static str,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<CacheControlParam>,
}

pub fn build_system_param(system: &str, is_oauth: bool, cache_ttl_1h: bool) -> Option<ApiSystem> {
    build_system_param_split(system, "", is_oauth, cache_ttl_1h)
}

/// Build system param with split static/dynamic content for better caching
pub fn build_system_param_split(
    static_part: &str,
    dynamic_part: &str,
    is_oauth: bool,
    cache_ttl_1h: bool,
) -> Option<ApiSystem> {
    if is_oauth {
        let mut blocks = Vec::new();
        blocks.push(ApiSystemBlock {
            block_type: "text",
            text: format!("x-anthropic-billing-header: {}", OAUTH_BILLING_HEADER),
            cache_control: None,
        });
        blocks.push(ApiSystemBlock {
            block_type: "text",
            text: CLAUDE_CODE_IDENTITY.to_string(),
            cache_control: None,
        });
        // Static content - CACHED (instruction files, base prompt, skills)
        if !static_part.is_empty() {
            blocks.push(ApiSystemBlock {
                block_type: "text",
                text: static_part.to_string(),
                cache_control: Some(CacheControlParam::ephemeral(cache_ttl_1h)),
            });
        }
        // Dynamic content - NOT cached (date, git status, memory)
        if !dynamic_part.is_empty() {
            blocks.push(ApiSystemBlock {
                block_type: "text",
                text: dynamic_part.to_string(),
                cache_control: None,
            });
        }
        return Some(ApiSystem::Blocks(blocks));
    }

    // Non-OAuth: use block format with cache control for static part only
    let has_static = !static_part.is_empty();
    let has_dynamic = !dynamic_part.is_empty();

    if !has_static && !has_dynamic {
        None
    } else {
        let mut blocks = Vec::new();
        if has_static {
            blocks.push(ApiSystemBlock {
                block_type: "text",
                text: static_part.to_string(),
                cache_control: Some(CacheControlParam::ephemeral(cache_ttl_1h)),
            });
        }
        if has_dynamic {
            blocks.push(ApiSystemBlock {
                block_type: "text",
                text: dynamic_part.to_string(),
                cache_control: None,
            });
        }
        Some(ApiSystem::Blocks(blocks))
    }
}

pub fn format_messages_with_identity(
    messages: Vec<ApiMessage>,
    _is_oauth: bool,
    cache_ttl_1h: bool,
) -> Vec<ApiMessage> {
    let mut out = messages;

    // Add cache breakpoints for both OAuth and non-OAuth paths
    add_message_cache_breakpoint(&mut out, cache_ttl_1h);

    out
}

/// A block the agent adds to the end of one request only (memory, nudges, the turn reminder). It is
/// not in the stored history, so a breakpoint on it would never be read back.
fn is_ephemeral_block(block: &ApiContentBlock) -> bool {
    matches!(block, ApiContentBlock::Text { text, .. } if text.trim_start().starts_with("<system-reminder>"))
}

/// Add cache_control to messages for conversation caching.
///
/// Strategy: sliding two-marker window, both on persisted content
///   - the last persisted block (the newest tool_result or user text) -> WRITE marker, so the whole
///     history up to the newest result is cached for the next request;
///   - the last cacheable block of the message before it -> READ marker, where the previous
///     request's write sat.
///
/// Trailing ephemeral blocks are skipped: they differ every request and are billed as fresh input.
///
/// Budget: system (1) + tools (1) + messages (2) = 4 total, within Anthropic's limit.
pub fn add_message_cache_breakpoint(messages: &mut [ApiMessage], cache_ttl_1h: bool) {
    if messages.len() < 3 {
        // Need at least: user + assistant + user to be worth caching
        return;
    }

    let mut anchors = messages.iter().enumerate().rev().flat_map(|(mi, msg)| {
        msg.content
            .iter()
            .enumerate()
            .rev()
            .filter(|(_, block)| {
                !is_ephemeral_block(block)
                    && matches!(
                        block,
                        ApiContentBlock::Text { .. }
                            | ApiContentBlock::ToolUse { .. }
                            | ApiContentBlock::ToolResult { .. }
                    )
            })
            .map(move |(bi, _)| (mi, bi))
    });
    let Some(write) = anchors.next() else {
        return;
    };
    let read = anchors.find(|(mi, _)| *mi < write.0);

    for (mi, bi) in std::iter::once(write).chain(read) {
        match &mut messages[mi].content[bi] {
            ApiContentBlock::Text { cache_control, .. }
            | ApiContentBlock::ToolUse { cache_control, .. }
            | ApiContentBlock::ToolResult { cache_control, .. } => {
                *cache_control = Some(CacheControlParam::ephemeral(cache_ttl_1h));
            }
            _ => unreachable!("anchors are Text, ToolUse or ToolResult blocks"),
        }
    }
}

#[derive(Serialize, Clone)]
pub struct ApiMessage {
    pub role: String,
    pub content: Vec<ApiContentBlock>,
}

#[derive(Serialize, Clone)]
#[serde(tag = "type")]
pub enum ApiContentBlock {
    #[serde(rename = "text")]
    Text {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<CacheControlParam>,
    },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<CacheControlParam>,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: ToolResultContent,
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        is_error: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<CacheControlParam>,
    },
    #[serde(rename = "thinking")]
    Thinking { thinking: String, signature: String },
    #[serde(rename = "image")]
    Image { source: ApiImageSource },
}

#[derive(Serialize, Clone)]
#[serde(untagged)]
pub enum ToolResultContent {
    Text(String),
    Blocks(Vec<ToolResultContentBlock>),
}

#[derive(Serialize, Clone)]
#[serde(tag = "type")]
pub enum ToolResultContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image")]
    Image { source: ApiImageSource },
}

#[derive(Serialize, Clone)]
pub struct ApiImageSource {
    #[serde(rename = "type")]
    pub kind: String,
    pub media_type: String,
    pub data: String,
}

#[derive(Serialize, Clone)]
pub struct ApiTool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<CacheControlParam>,
}

#[cfg(test)]
mod cache_prefix_invariant_tests {
    //! Anthropic caching is strict-prefix: a `cache_control` breakpoint caches every token up to
    //! and including the block it sits on. `add_message_cache_breakpoint` anchors the WRITE
    //! breakpoint on the last persisted block (the newest tool_result or user text) and the READ
    //! breakpoint on the message before it. The ephemeral tail the agent appends to a request
    //! (memory, nudges, the turn reminder: `<system-reminder>` user messages) must never move them.

    use super::*;
    use factr_message_types::{ContentBlock, Message, Role};

    fn text_msg(role: Role, text: &str) -> Message {
        Message {
            role,
            content: vec![ContentBlock::Text {
                text: text.to_string(),
                cache_control: None,
            }],
            timestamp: None,
            tool_duration_ms: None,
        }
    }

    fn blocks_msg(role: Role, content: Vec<ContentBlock>) -> Message {
        Message { role, content, timestamp: None, tool_duration_ms: None }
    }

    /// A warm agent conversation: a user prompt, then two tool rounds, ending on a tool result.
    fn base_conversation() -> Vec<Message> {
        let call = |id: &str| {
            blocks_msg(
                Role::Assistant,
                vec![ContentBlock::ToolUse {
                    id: id.into(),
                    name: "bash".into(),
                    input: json!({"command": "ls"}),
                    thought_signature: None,
                }],
            )
        };
        let result = |id: &str| {
            blocks_msg(
                Role::User,
                vec![ContentBlock::ToolResult {
                    tool_use_id: id.into(),
                    content: "output".into(),
                    is_error: None,
                }],
            )
        };
        vec![text_msg(Role::User, "Q1"), call("c1"), result("c1"), call("c2"), result("c2")]
    }

    /// (message index, block index) of every block carrying a cache_control breakpoint.
    fn breakpoint_anchors(messages: &[ApiMessage]) -> Vec<(usize, usize)> {
        messages
            .iter()
            .enumerate()
            .flat_map(|(mi, msg)| {
                msg.content.iter().enumerate().filter_map(move |(bi, block)| {
                    matches!(
                        block,
                        ApiContentBlock::Text { cache_control: Some(_), .. }
                            | ApiContentBlock::ToolUse { cache_control: Some(_), .. }
                            | ApiContentBlock::ToolResult { cache_control: Some(_), .. }
                    )
                    .then_some((mi, bi))
                })
            })
            .collect()
    }

    /// Serialize only the prefix up to and including the last breakpoint. This is the exact span
    /// Anthropic caches; if it is byte-identical across two requests, the cache is reused.
    fn cached_prefix_json(messages: &[ApiMessage]) -> String {
        let (mi, bi) = *breakpoint_anchors(messages).last().expect("expected a cache breakpoint");
        let mut prefix = messages[..=mi].to_vec();
        prefix[mi].content.truncate(bi + 1);
        serde_json::to_string(&prefix).expect("serialize cached prefix")
    }

    fn formatted_with_breakpoints(messages: &[Message]) -> Vec<ApiMessage> {
        let mut api = format_messages(messages, false);
        add_message_cache_breakpoint(&mut api, false);
        api
    }

    #[test]
    fn the_write_marker_sits_on_the_newest_tool_result_and_the_read_marker_one_message_earlier() {
        let api = formatted_with_breakpoints(&base_conversation());
        assert_eq!(api.len(), 5);
        // Read marker: the previous message (the assistant tool call); write: the last tool result.
        assert_eq!(breakpoint_anchors(&api), vec![(3, 0), (4, 0)]);
        assert!(matches!(api[4].content[0], ApiContentBlock::ToolResult { cache_control: Some(_), .. }));
        let json = serde_json::to_value(&api[4]).unwrap();
        assert_eq!(json["content"][0]["cache_control"]["type"], "ephemeral");
    }

    #[test]
    fn an_ephemeral_tail_does_not_move_the_breakpoints() {
        let base = base_conversation();
        let base_anchors = breakpoint_anchors(&formatted_with_breakpoints(&base));
        for tail in [
            vec!["<system-reminder>\nrecall\n</system-reminder>"],
            vec![
                "<system-reminder>turn reminder</system-reminder>",
                "<system-reminder>\nmemory\n</system-reminder>",
            ],
        ] {
            let mut with_tail = base.clone();
            for text in tail {
                with_tail.push(text_msg(Role::User, text));
            }
            let api = formatted_with_breakpoints(&with_tail);
            // The tail merges into the last user message, after the marked tool_result.
            assert_eq!(breakpoint_anchors(&api), base_anchors);
        }
    }

    #[test]
    fn cached_prefix_is_byte_identical_whatever_the_ephemeral_tail() {
        let base = base_conversation();
        let cached = cached_prefix_json(&formatted_with_breakpoints(&base));
        for tail in ["", "<system-reminder>recall A</system-reminder>", "<system-reminder>a different, longer recall B</system-reminder>"] {
            let mut msgs = base.clone();
            if !tail.is_empty() {
                msgs.push(text_msg(Role::User, tail));
            }
            assert_eq!(cached, cached_prefix_json(&formatted_with_breakpoints(&msgs)), "tail {tail:?}");
        }
    }

    #[test]
    fn a_plain_user_prompt_is_the_write_anchor_when_no_tool_ran() {
        let msgs = vec![text_msg(Role::User, "Q1"), text_msg(Role::Assistant, "A1"), text_msg(Role::User, "Q2")];
        assert_eq!(breakpoint_anchors(&formatted_with_breakpoints(&msgs)), vec![(1, 0), (2, 0)]);
    }

    fn tool_def(name: &str) -> ToolDefinition {
        ToolDefinition {
            name: name.to_string(),
            description: format!("{name} description"),
            input_schema: json!({"type":"object","properties":{}}),
        }
    }

    #[test]
    fn format_tools_removes_top_level_combinators_for_anthropic_api() {
        let tool = ToolDefinition {
            name: "custom".to_string(),
            description: "schema compatibility regression".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "action": {"type": "string"},
                    "nested_union": {
                        "anyOf": [{"type": "string"}, {"type": "array", "items": {"type": "string"}}]
                    }
                },
                "required": ["action"],
                "oneOf": [
                    {"type": "object", "properties": {"label": {"type": "string"}}},
                    {"type": "object", "properties": {"task_id": {"type": "string"}}}
                ],
                "allOf": [
                    {"type": "object", "properties": {"intent": {"type": "string"}}, "required": ["intent"]}
                ]
            }),
        };

        let formatted = format_tools(&[tool], false, false);
        let schema = &formatted[0].input_schema;
        for keyword in ["oneOf", "anyOf", "allOf"] {
            assert!(
                schema.get(keyword).is_none(),
                "Anthropic rejects top-level {keyword}: {schema}"
            );
        }
        for property in ["action", "nested_union", "label", "task_id", "intent"] {
            assert!(
                schema["properties"].get(property).is_some(),
                "missing merged property {property}: {schema}"
            );
        }
        assert!(
            schema["properties"]["nested_union"].get("anyOf").is_some(),
            "nested combinators remain supported and should not be flattened"
        );
        assert_eq!(schema["required"], json!(["action", "intent"]));
    }

    #[test]
    fn oauth_format_tools_keeps_full_custom_toolset() {
        // Registry includes builtins (remapped) plus extra tools that must survive.
        let registry = vec![
            tool_def("bash"),
            tool_def("read"),
            tool_def("write"),
            tool_def("edit"),
            tool_def("glob"),
            tool_def("grep"),
            tool_def("subagent"),
            tool_def("websearch"),
            tool_def("webfetch"),
            tool_def("browser"),
            tool_def("codesearch"),
            tool_def("memory"),
        ];

        let formatted = format_tools(&registry, true, false);
        let names: Vec<&str> = formatted.iter().map(|t| t.name.as_str()).collect();

        // Curated builtins are present under their OAuth names.
        for builtin in ["Bash", "Read", "Agent", "Write", "Edit", "Glob", "Grep"] {
            assert!(
                names.contains(&builtin),
                "missing builtin {builtin} in {names:?}"
            );
        }
        // The previously-dropped custom tools are now forwarded.
        for custom in ["websearch", "webfetch", "browser", "codesearch", "memory"] {
            assert!(
                names.contains(&custom),
                "custom tool {custom} was dropped on OAuth; got {names:?}"
            );
        }
        // No duplicate Agent/Bash/Read from the registry remap.
        assert_eq!(names.iter().filter(|n| **n == "Agent").count(), 1);
        assert_eq!(names.iter().filter(|n| **n == "Bash").count(), 1);
        assert_eq!(names.iter().filter(|n| **n == "Read").count(), 1);
    }

    #[test]
    fn oauth_format_tools_drops_builtins_missing_from_registry() {
        // subagent and glob no longer exist in the registry, so the curated
        // Agent/Glob builtins must not be advertised (see #572).
        let registry = vec![
            tool_def("bash"),
            tool_def("read"),
            tool_def("write"),
            tool_def("edit"),
            tool_def("agentgrep"),
        ];
        let formatted = format_tools(&registry, true, false);
        let names: Vec<&str> = formatted.iter().map(|t| t.name.as_str()).collect();

        for ghost in ["Agent", "Glob", "Grep", "Skill"] {
            assert!(
                !names.contains(&ghost),
                "advertised ghost builtin {ghost} without a backing registry tool: {names:?}"
            );
        }
        for present in ["Bash", "Read", "Write", "Edit", "agentgrep"] {
            assert!(names.contains(&present), "missing {present} in {names:?}");
        }
    }

    #[test]
    fn oauth_format_tools_places_single_cache_breakpoint_on_last_tool() {
        let registry = vec![tool_def("bash"), tool_def("websearch")];
        let formatted = format_tools(&registry, true, false);
        let with_cache: Vec<&str> = formatted
            .iter()
            .filter(|t| t.cache_control.is_some())
            .map(|t| t.name.as_str())
            .collect();
        assert_eq!(with_cache.len(), 1, "expected exactly one cache breakpoint");
        assert_eq!(
            formatted.last().map(|t| t.name.as_str()),
            with_cache.first().copied(),
            "cache breakpoint must be on the final tool"
        );
    }
}

#[cfg(test)]
#[path = "oauth_tool_schema_tests.rs"]
mod oauth_tool_schema_tests;

#[cfg(test)]
#[path = "trailing_assistant_repair_tests.rs"]
mod trailing_assistant_repair_tests;

#[cfg(test)]
#[path = "duplicate_tool_result_tests.rs"]
mod duplicate_tool_result_tests;

#[cfg(test)]
#[path = "wedge_fixture_check.rs"]
mod wedge_fixture_check;
