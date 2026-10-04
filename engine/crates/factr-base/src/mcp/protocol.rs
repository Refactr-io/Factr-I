//! MCP Protocol types (JSON-RPC 2.0)

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// JSON-RPC request
#[derive(Debug, Clone, Serialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: &'static str,
    pub id: u64,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

impl JsonRpcRequest {
    pub fn new(id: u64, method: impl Into<String>, params: Option<Value>) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            method: method.into(),
            params,
        }
    }
}

/// JSON-RPC notification (a request without an `id`).
#[derive(Debug, Clone, Serialize)]
pub struct JsonRpcNotification {
    pub jsonrpc: &'static str,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

impl JsonRpcNotification {
    pub fn new(method: impl Into<String>, params: Option<Value>) -> Self {
        Self {
            jsonrpc: "2.0",
            method: method.into(),
            params,
        }
    }
}

/// JSON-RPC response
#[derive(Debug, Clone, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Option<u64>,
    #[serde(default)]
    pub result: Option<Value>,
    #[serde(default)]
    pub error: Option<JsonRpcError>,
}

/// JSON-RPC error
#[derive(Debug, Clone, Deserialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(default)]
    pub data: Option<Value>,
}

/// MCP Initialize params
#[derive(Debug, Clone, Serialize)]
pub struct InitializeParams {
    #[serde(rename = "protocolVersion")]
    pub protocol_version: String,
    pub capabilities: ClientCapabilities,
    #[serde(rename = "clientInfo")]
    pub client_info: ClientInfo,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct ClientCapabilities {}

#[derive(Debug, Clone, Serialize)]
pub struct ClientInfo {
    pub name: String,
    pub version: String,
}

/// MCP Initialize result
#[derive(Debug, Clone, Deserialize)]
pub struct InitializeResult {
    #[serde(rename = "protocolVersion")]
    pub protocol_version: String,
    pub capabilities: ServerCapabilities,
    #[serde(rename = "serverInfo")]
    pub server_info: Option<ServerInfo>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ServerCapabilities {
    #[serde(default)]
    pub tools: Option<ToolsCapability>,
    #[serde(default)]
    pub resources: Option<ResourcesCapability>,
    #[serde(default)]
    pub prompts: Option<PromptsCapability>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ToolsCapability {
    #[serde(rename = "listChanged", default)]
    pub list_changed: bool,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ResourcesCapability {
    #[serde(default)]
    pub subscribe: bool,
    #[serde(rename = "listChanged", default)]
    pub list_changed: bool,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct PromptsCapability {
    #[serde(rename = "listChanged", default)]
    pub list_changed: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerInfo {
    pub name: String,
    #[serde(default)]
    pub version: Option<String>,
}

/// MCP Tool definition from server
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolDef {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}

/// tools/list result
#[derive(Debug, Clone, Deserialize)]
pub struct ToolsListResult {
    pub tools: Vec<McpToolDef>,
}

/// tools/call params
#[derive(Debug, Clone, Serialize)]
pub struct ToolCallParams {
    pub name: String,
    pub arguments: Value,
}

/// tools/call result
#[derive(Debug, Clone, Deserialize)]
pub struct ToolCallResult {
    pub content: Vec<ContentBlock>,
    #[serde(rename = "isError", default)]
    pub is_error: bool,
}

/// Content block in tool result
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
pub enum ContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image")]
    Image {
        data: String,
        #[serde(rename = "mimeType")]
        mime_type: String,
    },
    #[serde(rename = "resource")]
    Resource { resource: ResourceContent },
}

#[derive(Debug, Clone, Deserialize)]
pub struct ResourceContent {
    pub uri: String,
    #[serde(rename = "mimeType")]
    pub mime_type: Option<String>,
    pub text: Option<String>,
    pub blob: Option<String>,
}

/// MCP server configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct McpServerConfig {
    /// Command for stdio servers. Empty for remote servers.
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
    /// Whether this server can be shared across sessions (default: true).
    /// Stateless API wrappers (Todoist, Canvas) should be shared.
    /// Stateful servers (Playwright browser) should not be shared.
    #[serde(default = "default_shared")]
    pub shared: bool,
    /// Transport type from MCP configs ("stdio", "http", "sse").
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub transport: Option<String>,
    /// URL for streamable HTTP servers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Additional headers for streamable HTTP servers.
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub headers: std::collections::HashMap<String, String>,
    /// Whether this server is enabled (default: true). Disabled servers stay
    /// registered in config but are not spawned or connected at load time
    /// until re-enabled (issue #436). opencode-style `"enabled": false`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// Claude Code style alias: `"disabled": true`. Wins over `enabled` when
    /// both are present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled: Option<bool>,
    /// Per-request reply timeout in seconds for this server (tools/call,
    /// tools/list, initialize). Absent keeps the default of 30s. Servers whose
    /// tools legitimately run long (multi-engine web search, browser fetch, PDF
    /// extraction) can raise it here (issues #802, #1174).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
}

impl McpServerConfig {
    /// A config entry is stdio when it has a command and is not a remote
    /// transport.
    pub fn is_stdio(&self) -> bool {
        if let Some(t) = &self.transport {
            let t = t.to_ascii_lowercase();
            if t == "http" || t == "sse" || t == "streamable-http" {
                return false;
            }
        }
        !self.command.trim().is_empty()
    }

    pub fn is_http(&self) -> bool {
        self.url.as_deref().is_some_and(|url| !url.is_empty())
            && !self
                .transport
                .as_deref()
                .is_some_and(|transport| transport.eq_ignore_ascii_case("sse"))
    }

    /// Whether this server should be spawned/connected automatically.
    /// Defaults to true. `"disabled": true` (Claude Code style) wins over
    /// `"enabled"` (opencode style) when both are present. Disabled servers
    /// stay in config and can still be connected on demand by name.
    pub fn is_enabled(&self) -> bool {
        if let Some(disabled) = self.disabled {
            return !disabled;
        }
        self.enabled.unwrap_or(true)
    }
}

fn default_shared() -> bool {
    true
}

/// The MCP servers the engine may start, loaded from Factr's config.yaml.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct McpConfig {
    /// Server map (`servers`, or the Claude-style `mcpServers` spelling).
    #[serde(default, alias = "mcpServers")]
    pub servers: std::collections::HashMap<String, McpServerConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct UnresolvedEnvironmentVariable {
    server: String,
    variable: String,
}

fn valid_environment_variable_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some('_' | 'A'..='Z' | 'a'..='z'))
        && chars.all(|ch| matches!(ch, '_' | 'A'..='Z' | 'a'..='z' | '0'..='9'))
}

/// Expand Claude Code's documented `${VAR}` and `${VAR:-default}` syntax in a
/// single config string. Unsupported/malformed expressions are preserved.
fn expand_environment_string<F>(
    value: &str,
    lookup: &F,
    unresolved: &mut std::collections::BTreeSet<String>,
) -> String
where
    F: Fn(&str) -> Option<String>,
{
    let mut output = String::with_capacity(value.len());
    let mut remainder = value;

    while let Some(start) = remainder.find("${") {
        output.push_str(&remainder[..start]);
        let expression_start = start + 2;
        let Some(relative_end) = remainder[expression_start..].find('}') else {
            output.push_str(&remainder[start..]);
            return output;
        };
        let end = expression_start + relative_end;
        let expression = &remainder[expression_start..end];
        let (variable, default) = match expression.split_once(":-") {
            Some((variable, default)) => (variable, Some(default)),
            None => (expression, None),
        };
        let literal = &remainder[start..=end];

        if !valid_environment_variable_name(variable) {
            output.push_str(literal);
        } else if let Some(expanded) = lookup(variable) {
            output.push_str(&expanded);
        } else if let Some(default) = default {
            output.push_str(default);
        } else {
            unresolved.insert(variable.to_string());
            output.push_str(literal);
        }

        remainder = &remainder[end + 1..];
    }

    output.push_str(remainder);
    output
}

impl McpConfig {
    /// Expand environment references only after all config sources have been
    /// merged. This avoids warning about shadowed definitions and ensures every
    /// downstream consumer, including the tool-schema cache, sees the exact
    /// values that will be passed to the MCP process.
    fn expand_environment_variables_with<F>(
        &mut self,
        lookup: F,
    ) -> Vec<UnresolvedEnvironmentVariable>
    where
        F: Fn(&str) -> Option<String>,
    {
        let mut warnings = Vec::new();

        for (server_name, config) in &mut self.servers {
            let mut unresolved = std::collections::BTreeSet::new();
            config.command = expand_environment_string(&config.command, &lookup, &mut unresolved);
            for arg in &mut config.args {
                *arg = expand_environment_string(arg, &lookup, &mut unresolved);
            }
            for value in config.env.values_mut() {
                *value = expand_environment_string(value, &lookup, &mut unresolved);
            }
            if let Some(url) = &mut config.url {
                *url = expand_environment_string(url, &lookup, &mut unresolved);
            }
            for value in config.headers.values_mut() {
                *value = expand_environment_string(value, &lookup, &mut unresolved);
            }

            warnings.extend(
                unresolved
                    .into_iter()
                    .map(|variable| UnresolvedEnvironmentVariable {
                        server: server_name.clone(),
                        variable,
                    }),
            );
        }

        warnings.sort();
        warnings
    }

    fn expand_environment_variables(&mut self) {
        let warnings =
            self.expand_environment_variables_with(|variable| std::env::var(variable).ok());
        for warning in warnings {
            crate::logging::warn(&format!(
                "MCP: Server '{}' references unset environment variable '{}'; leaving '${{{}}}' unexpanded",
                warning.server, warning.variable, warning.variable
            ));
        }
    }

    /// Load the MCP servers the engine may start.
    ///
    /// Factr owns MCP settings: `FACTR_CONFIG_HOME/config.yaml` (`mcp_servers`) is the
    /// only source, so every server the engine calls is one Settings can edit and
    /// adding/removing one there affects the next session.
    pub fn load() -> Self {
        let mut merged = Self::default();

        if let Some(factr_home) = crate::factr_config::home() {
            let path = factr_home.join("config.yaml");
            if let Ok(contents) = std::fs::read_to_string(path)
                && let Ok(root) = serde_yaml::from_str::<serde_yaml::Value>(&contents)
                && let Some(servers) = root.get("mcp_servers")
                && let Ok(servers) = serde_yaml::from_value::<
                    std::collections::HashMap<String, McpServerConfig>,
                >(servers.clone())
            {
                merged.servers.extend(servers);
            }
        }

        // Expand environment references.
        merged.expand_environment_variables();

        // Keep supported stdio and streamable HTTP servers. Legacy SSE uses a
        // different transport and is not supported here.
        merged.servers.retain(|name, cfg| {
            let keep = cfg.is_stdio() || cfg.is_http();
            if !keep {
                crate::logging::info(&format!(
                    "MCP: Skipping MCP server '{}' ({}); no stdio command or HTTP URL is configured",
                    name,
                    cfg.transport.as_deref().unwrap_or("http")
                ));
            }
            keep
        });

        merged
    }
}

#[cfg(test)]
#[path = "protocol_tests.rs"]
mod protocol_tests;
