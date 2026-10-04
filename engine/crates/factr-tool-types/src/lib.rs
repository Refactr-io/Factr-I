#[derive(Debug, Clone)]
pub struct ToolOutput {
    pub output: String,
    pub title: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub images: Vec<ToolImage>,
}

#[derive(Debug, Clone)]
pub struct ToolImage {
    pub media_type: String,
    pub data: String,
    pub label: Option<String>,
}

impl ToolOutput {
    pub fn new(output: impl Into<String>) -> Self {
        Self {
            output: output.into(),
            title: None,
            metadata: None,
            images: Vec::new(),
        }
    }

    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = Some(metadata);
        self
    }

    pub fn with_image(mut self, media_type: impl Into<String>, data: impl Into<String>) -> Self {
        self.images.push(ToolImage {
            media_type: media_type.into(),
            data: data.into(),
            label: None,
        });
        self
    }

    pub fn with_labeled_image(
        mut self,
        media_type: impl Into<String>,
        data: impl Into<String>,
        label: impl Into<String>,
    ) -> Self {
        self.images.push(ToolImage {
            media_type: media_type.into(),
            data: data.into(),
            label: Some(label.into()),
        });
        self
    }
}

/// Resolve tool name aliases to their canonical internal names.
///
/// Providers can present tools with Claude Code aliases (e.g. `file_grep`,
/// `shell_exec`) or API namespace prefixes (e.g. `functions.bash`). Models can
/// repeat those names in sub-tool calls such as `batch`, while our registry
/// uses canonical internal names (`agentgrep`, `bash`). This mapping ensures
/// all of those forms resolve correctly.
///
/// This lives in `factr-tool-types` (rather than the tool `Registry`) so that
/// low-level crates such as config can normalize tool names without depending
/// on the full tool subsystem.
pub fn resolve_tool_name(name: &str) -> &str {
    // Some function-calling APIs expose a recipient such as `functions.bash`.
    // Models occasionally preserve that transport namespace when constructing
    // a nested tool call, especially inside `batch`.
    let name = name.strip_prefix("functions.").unwrap_or(name);
    ALIASES.iter().find(|(alias, _)| *alias == name).map_or(name, |(_, target)| target)
}

/// Every alias and the registered tool it resolves to (a test in `factr-app-core` checks that each
/// target is a registered tool, so an alias can never point at a tool that does not exist).
pub const ALIASES: &[(&str, &str)] = &[
    // Delegation is the one `delegate` tool; Claude Code's `Agent`/`Task` land there.
    ("task", "delegate"),
    ("task_runner", "delegate"),
    ("Agent", "delegate"),
    ("launch", "open"),
    ("shell", "bash"),
    ("shell_exec", "bash"),
    // Factr tool names (its prompts and learned skills use them).
    ("terminal", "bash"),
    ("search_files", "agentgrep"),
    ("web_extract", "webfetch"),
    ("web_search", "websearch"),
    ("read_file", "read"),
    ("file_read", "read"),
    ("write_file", "write"),
    ("file_write", "write"),
    ("edit_file", "edit"),
    ("file_edit", "edit"),
    // `multiedit` merged into `edit` (which accepts an `edits` array), and
    // `patch` merged into `apply_patch` (which accepts unified diffs).
    ("multiedit", "edit"),
    ("MultiEdit", "edit"),
    ("multi_edit", "edit"),
    ("patch", "apply_patch"),
    ("Patch", "apply_patch"),
    // The native grep tool was removed in favor of agentgrep, but models
    // still frequently call `grep` (and OAuth's `file_grep`). agentgrep's
    // grep mode accepts `pattern` as an alias for `query`, so these calls
    // work as-is.
    ("grep", "agentgrep"),
    ("file_grep", "agentgrep"),
    ("skill", "skill_manage"),
    ("Skill", "skill_manage"),
    ("todoread", "todo"),
    ("todowrite", "todo"),
    ("todo_read", "todo"),
    ("todo_write", "todo"),
    ("todos", "todo"),
    // The Anthropic OAuth surface advertises PascalCase tool names and
    // reverse-maps them provider-side for top-level calls, but nested
    // `batch` subcall names bypass that mapping and resolve here (issue
    // #486). Keep these in sync with anthropic_map_tool_name_from_oauth.
    ("Bash", "bash"),
    ("Read", "read"),
    ("Write", "write"),
    ("Edit", "edit"),
    ("Grep", "agentgrep"),
];

#[cfg(test)]
mod tests {
    use super::resolve_tool_name;

    #[test]
    fn resolve_tool_name_strips_function_namespace_before_alias_resolution() {
        assert_eq!(resolve_tool_name("functions.bash"), "bash");
        assert_eq!(resolve_tool_name("functions.shell_exec"), "bash");
        assert_eq!(resolve_tool_name("functions.file_grep"), "agentgrep");
    }

    #[test]
    fn resolve_tool_name_does_not_strip_unrecognized_namespaces() {
        assert_eq!(
            resolve_tool_name("mcp.functions.bash"),
            "mcp.functions.bash"
        );
    }

    #[test]
    fn resolve_tool_name_maps_pascalcase_oauth_aliases() {
        // Anthropic OAuth advertises PascalCase names; batch subcalls resolve
        // through here rather than the provider-side reverse map (issue #486).
        assert_eq!(resolve_tool_name("Read"), "read");
        assert_eq!(resolve_tool_name("Bash"), "bash");
        assert_eq!(resolve_tool_name("Write"), "write");
        assert_eq!(resolve_tool_name("Edit"), "edit");
        assert_eq!(resolve_tool_name("multiedit"), "edit");
        assert_eq!(resolve_tool_name("MultiEdit"), "edit");
        assert_eq!(resolve_tool_name("patch"), "apply_patch");
        assert_eq!(resolve_tool_name("Grep"), "agentgrep");
        assert_eq!(resolve_tool_name("Agent"), "delegate");
        assert_eq!(resolve_tool_name("task"), "delegate");
        assert_eq!(resolve_tool_name("ScheduleWakeup"), "ScheduleWakeup", "no schedule tool to land on");
        assert_eq!(resolve_tool_name("communicate"), "communicate", "no swarm tool to land on");
        assert_eq!(resolve_tool_name("Skill"), "skill_manage");
        assert_eq!(resolve_tool_name("functions.Read"), "read");
        assert_eq!(resolve_tool_name("terminal"), "bash");
        assert_eq!(resolve_tool_name("search_files"), "agentgrep");
        assert_eq!(resolve_tool_name("web_extract"), "webfetch");
        assert_eq!(resolve_tool_name("web_search"), "websearch");
    }
}
