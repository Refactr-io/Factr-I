//! `/<skill-name> [instruction]`, expanded natively from the engine's skill registry (the one
//! `$FACTR_HOME/skills` dir) into the same user message Factr's `build_skill_invocation_message`
//! builds, so no Python starts to run a skill.

use serde_json::{Value, json};
use std::path::Path;

const SKILL_DIR_NOTE: &str = "Resolve any relative paths in this skill (e.g. `scripts/foo.js`, `templates/config.yaml`) against that directory, then run them with the terminal tool using the absolute path.";
const INSTRUCTION_MARKER: &str = "The user has provided the following instruction alongside the skill invocation: ";

fn key(name: &str) -> String {
    crate::slash_forward::skill_command(name.trim().trim_start_matches('/')).replace('_', "-")
}

fn find(name: &str) -> Option<factr_base::skill::Skill> {
    let wanted = key(name);
    if wanted.is_empty() {
        return None;
    }
    let disabled = factr_base::skill::disabled_skill_names();
    factr_base::skill::SkillRegistry::shared_snapshot()
        .list()
        .into_iter()
        .find(|s| key(&s.name) == wanted && !disabled.contains(&s.name))
        .cloned()
}

/// Skill-relative support files: `references`, `templates`, `scripts`, `assets` (regular files only).
fn supporting_files(dir: &Path) -> Vec<String> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(kind) = entry.file_type() else { continue };
            if kind.is_symlink() {
                continue;
            }
            if kind.is_dir() {
                walk(root, &path, out);
            } else if let Ok(rel) = path.strip_prefix(root) {
                out.push(rel.to_string_lossy().into_owned());
            }
        }
    }
    let mut out = Vec::new();
    for sub in ["references", "templates", "scripts", "assets"] {
        let mut found = Vec::new();
        walk(dir, &dir.join(sub), &mut found);
        found.sort();
        out.extend(found);
    }
    out
}

/// The model-facing message for one skill invocation (Factr's wording, byte for byte where the
/// skill has no config vars or setup notes).
pub(crate) fn message(skill: &factr_base::skill::Skill, instruction: &str, session_id: &str, skills_root: Option<&Path>) -> Option<String> {
    let text = std::fs::read_to_string(&skill.path).ok()?;
    let dir = skill.path.parent()?;
    let content = text
        .replace("${FACTR_SKILL_DIR}", &dir.to_string_lossy())
        .replace("${FACTR_SESSION_ID}", if session_id.is_empty() { "${FACTR_SESSION_ID}" } else { session_id });
    let mut parts = vec![
        format!("[IMPORTANT: The user has invoked the \"{}\" skill, indicating they want you to follow its instructions. The full skill content is loaded below.]", skill.name),
        String::new(),
        content.trim().to_string(),
        String::new(),
        format!("[Skill directory: {}]", dir.display()),
        SKILL_DIR_NOTE.to_string(),
    ];
    let files = supporting_files(dir);
    if !files.is_empty() {
        let target = skills_root
            .and_then(|root| dir.strip_prefix(root).ok())
            .map(|p| p.to_string_lossy().into_owned())
            .or_else(|| dir.file_name().map(|n| n.to_string_lossy().into_owned()))?;
        parts.push(String::new());
        parts.push("[This skill has supporting files (paths relative to the skill directory above):]".into());
        parts.extend(files.iter().map(|f| format!("- {f}")));
        parts.push(format!(
            "\nLoad any of these with skill_view(name=\"{target}\", file_path=\"<path>\"), or run scripts directly by absolute path (e.g. `node {}/scripts/foo.js`).",
            dir.display()
        ));
    }
    if !instruction.is_empty() {
        parts.push(String::new());
        parts.push(format!("{INSTRUCTION_MARKER}{instruction}"));
    }
    Some(parts.join("\n"))
}

/// `command.dispatch` / `slash.exec` answer for `/<name> <arg>` when `name` is a skill, else `None`
/// (an unknown name, a disabled skill, or a stacked `/a /b ...` invocation, which Factr answers).
pub(crate) fn expand(name: &str, arg: &str, session_id: Option<&str>) -> Option<Value> {
    let skill = find(name)?;
    let arg = arg.trim();
    if arg.strip_prefix('/').and_then(|rest| rest.split_whitespace().next()).is_some_and(|next| find(next).is_some()) {
        return None;
    }
    let root = factr_base::storage::factr_dir().ok().map(|d| d.join("skills"));
    let body = message(&skill, arg, session_id.unwrap_or_default(), root.as_deref())?;
    let shown = arg.split_whitespace().collect::<Vec<_>>().join(" ");
    let display = if shown.is_empty() { format!("/{}", skill.name) } else { format!("/{} {shown}", skill.name) };
    Some(json!({ "status": "ok", "type": "skill", "message": body, "name": skill.name, "display": display }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_skill_command_expands_natively_to_factrs_message() {
        let _env = crate::factr_env::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let home = std::env::temp_dir().join(format!("skill-slash-{}", std::process::id()));
        let skill = home.join("skills/Release Notes");
        std::fs::create_dir_all(skill.join("scripts")).unwrap();
        std::fs::write(skill.join("scripts/run.sh"), "echo").unwrap();
        std::fs::write(
            skill.join("SKILL.md"),
            "---\nname: Release Notes\ndescription: Write notes\n---\nUse ${FACTR_SKILL_DIR} for session ${FACTR_SESSION_ID}.\n",
        )
        .unwrap();
        let saved = ["FACTR_HOME", "FACTR_CONFIG_HOME"].map(|k| (k, std::env::var_os(k)));
        // SAFETY: serialized by lock_test_env; restored below.
        unsafe {
            std::env::set_var("FACTR_HOME", &home);
            std::env::remove_var("FACTR_CONFIG_HOME");
        }
        let done = expand("release-notes", "  cut   v2 ", Some("sess-1"));
        let missing = expand("no-such-skill", "", None);
        let stacked = expand("release-notes", "/release-notes again", None);
        for (k, v) in saved {
            unsafe {
                match v {
                    Some(v) => std::env::set_var(k, v),
                    None => std::env::remove_var(k),
                }
            }
        }
        let done = done.expect("a skill in the one dir expands");
        assert_eq!(done["type"], "skill");
        assert_eq!(done["name"], "Release Notes");
        assert_eq!(done["display"], "/Release Notes cut v2");
        let body = done["message"].as_str().unwrap();
        assert!(body.starts_with("[IMPORTANT: The user has invoked the \"Release Notes\" skill, indicating they want you to follow its instructions. The full skill content is loaded below.]\n\n---\nname: Release Notes"), "{body}");
        assert!(body.contains(&format!("Use {} for session sess-1.", skill.display())), "{body}");
        assert!(body.contains(&format!("[Skill directory: {}]", skill.display())));
        assert!(body.contains("- scripts/run.sh") && body.contains("skill_view(name=\"Release Notes\", file_path=\"<path>\")"));
        assert!(body.ends_with("The user has provided the following instruction alongside the skill invocation: cut   v2"), "{body}");
        assert!(missing.is_none());
        assert!(stacked.is_none(), "stacked invocations stay Factr's");
    }
}
