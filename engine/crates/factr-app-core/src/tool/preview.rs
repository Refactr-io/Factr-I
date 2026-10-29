//! `preview`: show a URL or local file in the desktop app's in-app preview pane. Registered only
//! when the desktop app launched the engine (see `factr_base::prompt::desktop_client`).

use super::factr_bridge::{FactrHost, installed};
use super::{Tool, ToolContext, ToolOutput};
use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Value, json};
use std::sync::Arc;

pub struct PreviewTool {
    host: Option<Arc<dyn FactrHost>>,
}

impl PreviewTool {
    pub fn new() -> Self {
        Self { host: None }
    }
}

const FILE_EXTS: &[&str] = &["html", "htm", "pdf", "png", "jpg", "jpeg", "gif", "svg", "webp", "md", "txt"];

/// A bare host becomes a URL; schemes and absolute paths pass through; relative paths resolve
/// against the working directory.
fn normalize(raw: &str, cwd: Option<&std::path::Path>) -> String {
    let v = raw.trim().trim_matches('`').trim();
    if v.is_empty() || v.contains("://") || v.starts_with("file:") || v.starts_with('/') || v.starts_with('~') {
        return v.to_string();
    }
    let lower = v.to_ascii_lowercase();
    if ["localhost", "127.0.0.1", "0.0.0.0", "[::1]"]
        .iter()
        .any(|h| lower.strip_prefix(h).is_some_and(|r| r.is_empty() || r.starts_with(':') || r.starts_with('/')))
    {
        return format!("http://{v}");
    }
    let host = v.split(['/', ':']).next().unwrap_or("");
    let tld_ok = host.rsplit('.').next().is_some_and(|t| t.len() >= 2 && t.chars().all(|c| c.is_ascii_alphabetic()));
    let is_file = !v.contains('/')
        && std::path::Path::new(v).extension().is_some_and(|e| FILE_EXTS.contains(&e.to_string_lossy().to_lowercase().as_str()));
    if host.contains('.') && tld_ok && !is_file && !v.starts_with('.') {
        return format!("https://{v}");
    }
    match cwd {
        Some(dir) => dir.join(v.trim_start_matches("./")).to_string_lossy().into_owned(),
        None => v.to_string(),
    }
}

#[async_trait]
impl Tool for PreviewTool {
    fn name(&self) -> &str {
        "preview"
    }

    fn description(&self) -> &str {
        "Show a web page, a local dev server (for example http://localhost:5173) or a local file (HTML, image, PDF) in the app's preview pane beside the chat. Use it after starting a dev server or creating a viewable file. action=close closes the pane or the tab for `target`."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "intent": super::intent_schema_property(),
                "action": { "type": "string", "enum": ["open", "close"], "description": "Default open" },
                "target": { "type": "string", "description": "URL, host:port, or file path (required for open)" },
                "label": { "type": "string", "description": "Optional tab label" }
            }
        })
    }

    async fn execute(&self, input: Value, ctx: ToolContext) -> Result<ToolOutput> {
        let action = input["action"].as_str().unwrap_or("open");
        let target = normalize(input["target"].as_str().unwrap_or(""), ctx.working_dir.as_deref());
        let (event, payload) = match action {
            "open" => {
                anyhow::ensure!(!target.is_empty(), "`target` is required to open a preview");
                ("preview.open", json!({ "url": target, "label": input["label"].as_str().unwrap_or("").trim() }))
            }
            "close" => ("preview.close", json!({ "url": target })),
            other => anyhow::bail!("Unknown preview action: {other}. Valid actions: open, close"),
        };
        let Some(host) = self.host.clone().or_else(installed) else {
            return Ok(ToolOutput::new("The preview pane is not available here."));
        };
        Ok(ToolOutput::new(if host.emit_event(&ctx.session_id, event, payload).await {
            if action == "open" { format!("Opened {target} in the preview pane.") } else { "Closed the preview.".to_string() }
        } else {
            "No desktop window is showing this session, so the preview pane is not available. Tell the user the address instead.".to_string()
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::super::factr_bridge::ClarifyReply;
    use super::*;
    use factr_tool_core::ToolExecutionMode;
    use std::sync::Mutex;

    struct Rec(Mutex<Vec<(String, String, Value)>>);

    #[async_trait]
    impl FactrHost for Rec {
        async fn call(&self, _: &str, _: &str, _: Option<Value>) -> Result<Value> {
            anyhow::bail!("none")
        }
        async fn clarify(&self, _: &str, _: &str, _: &[String]) -> ClarifyReply {
            ClarifyReply::NoUser
        }
        async fn emit_event(&self, s: &str, e: &str, p: Value) -> bool {
            self.0.lock().unwrap().push((s.into(), e.into(), p));
            true
        }
    }

    fn ctx() -> ToolContext {
        ToolContext {
            session_id: "s1".into(),
            message_id: "m".into(),
            tool_call_id: "t".into(),
            working_dir: Some("/work".into()),
            stdin_request_tx: None,
            graceful_shutdown_signal: None,
            execution_mode: ToolExecutionMode::AgentTurn,
        }
    }

    #[test]
    fn normalizes_targets() {
        assert_eq!(normalize("localhost:5173", None), "http://localhost:5173");
        assert_eq!(normalize("www.cnn.com", None), "https://www.cnn.com");
        assert_eq!(normalize("http://x.dev/a", None), "http://x.dev/a");
        assert_eq!(normalize("index.html", Some(std::path::Path::new("/work"))), "/work/index.html");
        assert_eq!(normalize("out/a.html", Some(std::path::Path::new("/work"))), "/work/out/a.html");
        assert_eq!(normalize("/tmp/a.pdf", None), "/tmp/a.pdf");
    }

    #[tokio::test]
    async fn emits_preview_events() {
        let rec = Arc::new(Rec(Mutex::default()));
        let tool = PreviewTool { host: Some(rec.clone()) };
        tool.execute(json!({"target": "localhost:3000", "label": "app"}), ctx()).await.unwrap();
        tool.execute(json!({"action": "close"}), ctx()).await.unwrap();
        assert!(tool.execute(json!({}), ctx()).await.is_err());
        let got = rec.0.lock().unwrap();
        assert_eq!(got[0], ("s1".into(), "preview.open".into(), json!({"url": "http://localhost:3000", "label": "app"})));
        assert_eq!(got[1].1, "preview.close");
    }
}
