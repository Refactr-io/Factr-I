//! Tool descriptions that steer reads, searches and edits away from the shell
//! (`LoopShape::tool_steering`). Descriptions only, no behaviour. Every sentence names only tools
//! in the final list, and definitions a client supplied (SDK custom tools) are left as given.

use crate::message::ToolDefinition;
use std::collections::HashSet;

/// (file tool, the shell commands it replaces)
const READ_TOOLS: [(&str, &str); 3] = [("read", "cat/head/tail"), ("agentgrep", "grep/find"), ("ls", "ls")];
const CHANGE_TOOLS: [&str; 3] = ["edit", "write", "apply_patch"];

fn join(items: &[&str]) -> String {
    match items {
        [] => String::new(),
        [one] => one.to_string(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// Run on the final, filtered tool list (after allowlist, SDK overrides, disabling and deferral).
pub(super) fn steer(tools: &mut [ToolDefinition], custom: &HashSet<String>) {
    let present: HashSet<String> = tools.iter().map(|t| t.name.clone()).collect();
    let has = |name: &str| present.contains(name);
    let readers: Vec<_> = READ_TOOLS.iter().filter(|(t, _)| has(t)).collect();
    let changers: Vec<&str> = CHANGE_TOOLS.iter().copied().filter(|t| has(t)).collect();
    let mut instead: Vec<&str> = readers.iter().map(|(_, shell)| *shell).collect();
    if !changers.is_empty() {
        instead.push("sed/heredocs");
    }
    let bash_note = {
        let read_part = (!readers.is_empty())
            .then(|| format!("{} for reading and searching files", join(&readers.iter().map(|(t, _)| *t).collect::<Vec<_>>())));
        let change_part = (!changers.is_empty()).then(|| format!("{} for changing them", changers.join("/")));
        let uses: Vec<String> = read_part.into_iter().chain(change_part).collect();
        (!uses.is_empty()).then(|| format!("Use {}, instead of {}.", uses.join(", and "), join(&instead)))
    };
    for tool in tools.iter_mut().filter(|t| !custom.contains(&t.name)) {
        let note = match tool.name.as_str() {
            "bash" => bash_note.clone(),
            "read" if has("bash") => Some("Use this instead of cat/head/tail in bash.".to_string()),
            "agentgrep" if has("bash") => Some("Use this instead of grep/rg/find in bash.".to_string()),
            "write" => Some(
                "A successful result means the file is on disk exactly as sent; there is no need to read it back just to confirm the save."
                    .to_string(),
            ),
            _ => None,
        };
        if let Some(note) = note {
            tool.description = format!("{} {note}", tool.description.trim_end());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defs(names: &[&str]) -> Vec<ToolDefinition> {
        names
            .iter()
            .map(|n| ToolDefinition { name: n.to_string(), description: format!("{n} tool."), input_schema: serde_json::json!({}) })
            .collect()
    }

    fn desc<'a>(tools: &'a [ToolDefinition], name: &str) -> &'a str {
        &tools.iter().find(|t| t.name == name).unwrap().description
    }

    #[test]
    fn full_toolset_gets_every_sentence() {
        let mut tools = defs(&["agentgrep", "apply_patch", "bash", "edit", "ls", "read", "write"]);
        steer(&mut tools, &HashSet::new());
        assert_eq!(
            desc(&tools, "bash"),
            "bash tool. Use read, agentgrep and ls for reading and searching files, and edit/write/apply_patch for changing them, instead of cat/head/tail, grep/find, ls and sed/heredocs."
        );
        assert_eq!(desc(&tools, "read"), "read tool. Use this instead of cat/head/tail in bash.");
        assert_eq!(desc(&tools, "agentgrep"), "agentgrep tool. Use this instead of grep/rg/find in bash.");
        assert!(desc(&tools, "write").ends_with("no need to read it back just to confirm the save."));
        assert_eq!(desc(&tools, "edit"), "edit tool.");
    }

    #[test]
    fn a_restricted_toolset_names_only_present_tools() {
        let mut tools = defs(&["bash", "read"]);
        steer(&mut tools, &HashSet::new());
        assert_eq!(desc(&tools, "bash"), "bash tool. Use read for reading and searching files, instead of cat/head/tail.");
        let mut no_shell = defs(&["read", "agentgrep"]);
        steer(&mut no_shell, &HashSet::new());
        assert_eq!(desc(&no_shell, "read"), "read tool.", "no bash, nothing to steer away from");
        let mut only_bash = defs(&["bash"]);
        steer(&mut only_bash, &HashSet::new());
        assert_eq!(desc(&only_bash, "bash"), "bash tool.");
    }

    #[test]
    fn a_client_supplied_definition_is_left_as_given() {
        let mut tools = defs(&["bash", "read", "write"]);
        steer(&mut tools, &HashSet::from(["write".to_string(), "read".to_string()]));
        assert_eq!(desc(&tools, "write"), "write tool.");
        assert_eq!(desc(&tools, "read"), "read tool.");
        assert!(desc(&tools, "bash").contains("Use read for reading"), "still a present tool");
    }
}
