//! Skill tool - load, list, reload, and read skills

use super::{Tool, ToolContext, ToolOutput};
use crate::skill::SkillRegistry;
use anyhow::Result;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct SkillTool {
    registry: Arc<RwLock<SkillRegistry>>,
}

impl SkillTool {
    pub fn new(registry: Arc<RwLock<SkillRegistry>>) -> Self {
        Self { registry }
    }
}

#[derive(Deserialize)]
struct SkillInput {
    /// Action to perform: load (default), list, reload, reload_all, read, create.
    /// `list` shows both loaded skills and the factr-endorsed catalog.
    #[serde(default = "default_action")]
    action: String,
    /// Skill name (required for load, reload, read)
    #[serde(alias = "skill")]
    #[serde(default)]
    name: Option<String>,
    /// Optional Claude-compatible Skill wrapper argument. The skill loader only
    /// needs to load the prompt, so args are currently accepted and ignored.
    #[serde(default)]
    args: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    instructions: Option<String>,
    #[serde(default)]
    package_name: Option<String>,
    #[serde(default)]
    package_code: Option<String>,
    /// Supporting file (relative to the skill) for write_file / remove_file.
    #[serde(default)]
    file_path: Option<String>,
    /// Full SKILL.md for edit, or the file body for write_file.
    #[serde(default)]
    file_content: Option<String>,
    /// Exact text to replace (patch) and its replacement.
    #[serde(default)]
    old_string: Option<String>,
    #[serde(default)]
    new_string: Option<String>,
}

fn default_action() -> String {
    "load".to_string()
}

#[async_trait]
impl Tool for SkillTool {
    fn name(&self) -> &str {
        "skill_manage"
    }

    fn description(&self) -> &str {
        "Manage skills."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "intent": super::intent_schema_property(),
                "action": {
                    "type": "string",
                    "enum": ["load", "list", "reload", "reload_all", "read", "create", "patch", "edit", "write_file", "remove_file"],
                    "description": "Action."
                },
                "name": {
                    "type": "string",
                    "description": "Skill name."
                },
                "description": {"type":"string", "description":"Short skill summary for create."},
                "instructions": {"type":"string", "description":"SKILL.md body for create."},
                "package_name": {"type":"string", "description":"Optional Python module name for create."},
                "package_code": {"type":"string", "description":"Optional Python package __init__.py for create."},
                "file_path": {"type":"string", "description":"File under the skill (write_file/remove_file)."},
                "file_content": {"type":"string", "description":"Full SKILL.md (edit) or file body (write_file)."},
                "old_string": {"type":"string", "description":"Text to replace (patch)."},
                "new_string": {"type":"string", "description":"Replacement (patch)."}
            }
        })
    }

    async fn execute(&self, input: Value, ctx: ToolContext) -> Result<ToolOutput> {
        let params: SkillInput = serde_json::from_value(input)?;
        let action_label = params.action.clone();
        let name_label = params.name.clone().unwrap_or_else(|| "<none>".to_string());
        let _args = params.args.as_deref();

        match params.action.as_str() {
            "load" => {
                self.load_skill(params.name)
                    .await
            }
            "list" => self.list_skills().await,
            "reload" => self.reload_skill(params.name).await,
            "reload_all" => self.reload_all_skills().await,
            "read" => {
                self.read_skill(params.name)
                    .await
            }
            "create" => self.create_skill(params).await,
            "patch" | "edit" | "write_file" | "remove_file" => self.modify_skill(params).await,
            _ => Ok(ToolOutput::new(format!(
                "Unknown action: {}. Use 'load', 'list', 'reload', 'reload_all', 'read', 'create', 'patch', 'edit', 'write_file', or 'remove_file'.",
                params.action
            ))),
        }
        .map_err(|err| {
            crate::logging::warn(&format!(
                "[tool:skill_manage] action failed action={} skill={} session_id={} error={}",
                action_label, name_label, ctx.session_id, err
            ));
            err
        })
    }
}

impl SkillTool {
    async fn create_skill(&self, params: SkillInput) -> Result<ToolOutput> {
        let name = params.name.clone().unwrap_or_else(|| "skill".to_string());
        let (path, note) = create_skill_with_registry(&self.registry, params).await?;
        let message = match &note {
            Some(note) => note.clone(),
            None => format!("Created skill '{}' at {}", name, path.display()),
        };
        Ok(ToolOutput::new(message).with_title(format!("Skills: Created {name}")))
    }

    async fn modify_skill(&self, params: SkillInput) -> Result<ToolOutput> {
        let name = normalize_skill_name(params.name.clone(), &params.action)?;
        let dir = {
            let registry = self.registry.read().await;
            let skill = registry
                .get(&name)
                .ok_or_else(|| anyhow::anyhow!("Skill '{name}' not found"))?;
            skill.path.parent().map(|p| p.to_path_buf())
        }
        .ok_or_else(|| anyhow::anyhow!("Skill '{name}' has no directory"))?;
        let message = modify_skill_files(&dir, &params)?;
        self.registry.write().await.reload_global()?;
        Ok(ToolOutput::new(message).with_title(format!("Skills: {} {name}", params.action)))
    }

    async fn load_skill(
        &self,
        name: Option<String>,
    ) -> Result<ToolOutput> {
        let name = normalize_skill_name(name, "load")?;

        let registry = self.registry.read().await.clone();
        // A skill switched off in Settings is not offered to the model.
        let skill = registry
            .get(&name)
            .filter(|s| !crate::skill::disabled_skill_names().contains(&s.name)).ok_or_else(|| {
            // Endorsed skills are advertised in `list` but are not bundled;
            // a bare "not found" here reads like a bug (issue #445). Point at
            // the actual install command instead.
            if let Some(endorsed) = crate::skill::endorsed_skills()
                .iter()
                .find(|endorsed| endorsed.name == name)
            {
                match endorsed.install {
                    Some(install) => anyhow::anyhow!(
                        "Skill '{}' is endorsed but not installed. Install it with `{}`, then run skill_manage reload_all.",
                        name,
                        install
                    ),
                    None => anyhow::anyhow!(
                        "Skill '{}' is endorsed but not installed (source: {}). Install it into ~/.factr/engine/skills/{}/SKILL.md, then run skill_manage reload_all.",
                        name,
                        endorsed.source,
                        name
                    ),
                }
            } else {
                anyhow::anyhow!("Skill '{}' not found", name)
            }
        })?;

        let base_dir = skill
            .path
            .parent()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| ".".to_string());

        Ok(ToolOutput::new(format!(
            "## Skill: {}\n\n**Base directory**: {}\n\n{}",
            skill.name,
            base_dir,
            skill.get_prompt()
        ))
        .with_title(format!("skill: {}", skill.name)))
    }

    async fn list_skills(&self) -> Result<ToolOutput> {
        let registry = self.registry.read().await.clone();
        let mut skills = registry.list();
        skills.sort_by(|a, b| a.name.cmp(&b.name));

        let installed: std::collections::HashSet<&str> =
            skills.iter().map(|s| s.name.as_str()).collect();
        let disabled = crate::skill::disabled_skill_names();
        skills.retain(|s| !disabled.contains(&s.name));

        let mut output = if skills.is_empty() {
            "No skills loaded.\n\n\
            Skills live in one directory, <skills dir>/<skill-name>/SKILL.md.\n\n\
            Create a SKILL.md file with YAML frontmatter:\n\
            ---\n\
            name: my-skill\n\
            description: What this skill does\n\
            allowed-tools: bash, read, write\n\
            ---\n\n\
            # Skill content here\n"
                .to_string()
        } else {
            let mut output = format!("Loaded skills: {}\n\n", skills.len());
            for skill in &skills {
                output.push_str(&format!("## /{}\n", skill.name));
                output.push_str(&format!("  {}\n", skill.description));
                output.push_str(&format!("  Path: {}\n", skill.path.display()));
                if let Some(ref tools) = skill.allowed_tools {
                    output.push_str(&format!("  Tools: {}\n", tools.join(", ")));
                }
                output.push('\n');
            }
            output
        };

        append_endorsed_skills(&mut output, &installed);

        Ok(ToolOutput::new(output).with_title("Skills: List"))
    }

    async fn reload_skill(&self, name: Option<String>) -> Result<ToolOutput> {
        let name = normalize_skill_name(name, "reload")?;

        let mut registry = self.registry.write().await;

        match registry.reload(&name) {
            Ok(true) => {
                // Re-read to get updated info
                if let Some(skill) = registry.get(&name) {
                    Ok(ToolOutput::new(format!(
                        "Reloaded skill '{}'\n\nDescription: {}\nPath: {}",
                        name,
                        skill.description,
                        skill.path.display()
                    ))
                    .with_title(format!("Skills: Reloaded {}", name)))
                } else {
                    Ok(ToolOutput::new(format!("Reloaded skill '{}'", name))
                        .with_title(format!("Skills: Reloaded {}", name)))
                }
            }
            Ok(false) => Ok(ToolOutput::new(format!(
                "Skill '{}' not found or was deleted.\n\nUse 'list' to see available skills.",
                name
            ))
            .with_title("Skills: Not found")),
            Err(e) => {
                crate::logging::warn(&format!(
                    "[tool:skill_manage] reload failed skill={} error={}",
                    name, e
                ));
                Ok(
                    ToolOutput::new(format!("Failed to reload skill '{}': {}", name, e))
                        .with_title("Skills: Reload failed"),
                )
            }
        }
    }

    async fn reload_all_skills(&self) -> Result<ToolOutput> {
        // Reload the shared GLOBAL registry only; the project-local overlay is
        // session-scoped and re-read from disk on every access, so reloading
        // it here would leak this session's project skills to other sessions
        // (issue #457).
        let reloaded = {
            let mut registry = self.registry.write().await;
            registry.reload_global()
        };

        match reloaded {
            Ok(global_count) => {
                let effective = self.registry.read().await.clone();
                let skills = effective.list();
                let mut output = format!(
                    "Reloaded {} global skills ({} effective for this session)\n\n",
                    global_count,
                    skills.len()
                );

                for skill in skills {
                    output.push_str(&format!("- /{}: {}\n", skill.name, skill.description));
                }

                Ok(
                    ToolOutput::new(output)
                        .with_title(format!("Skills: Reloaded {}", global_count)),
                )
            }
            Err(e) => {
                crate::logging::warn(&format!(
                    "[tool:skill_manage] reload_all failed error={}",
                    e
                ));
                Ok(ToolOutput::new(format!("Failed to reload skills: {}", e))
                    .with_title("Skills: Reload failed"))
            }
        }
    }

    async fn read_skill(
        &self,
        name: Option<String>,
    ) -> Result<ToolOutput> {
        let name = normalize_skill_name(name, "read")?;

        let registry = self.registry.read().await.clone();

        if let Some(skill) = registry.get(&name) {
            let mut output = format!("# Skill: {}\n\n", skill.name);
            output.push_str(&format!("**Description:** {}\n", skill.description));
            output.push_str(&format!("**Path:** {}\n", skill.path.display()));
            if let Some(ref tools) = skill.allowed_tools {
                output.push_str(&format!("**Allowed tools:** {}\n", tools.join(", ")));
            }
            output.push_str("\n---\n\n");
            output.push_str(&skill.content);

            Ok(ToolOutput::new(output).with_title(format!("Skills: {}", name)))
        } else {
            Ok(ToolOutput::new(format!(
                "Skill '{}' not found.\n\nUse 'list' to see available skills.",
                name
            ))
            .with_title("Skills: Not found"))
        }
    }
}

/// Append the curated factr-endorsed skill catalog to `output`, grouped by
/// category and marked with installed/not-installed status. `installed` is the
/// set of skill names currently loaded in the registry.
fn append_endorsed_skills(output: &mut String, installed: &std::collections::HashSet<&str>) {
    let endorsed = crate::skill::endorsed_skills();
    if endorsed.is_empty() {
        return;
    }

    output.push_str("\nEndorsed skills (recommended by Factr-I)\n");

    // Group by category, preserving first-seen order.
    let mut category_order: Vec<&str> = Vec::new();
    for skill in endorsed {
        if !category_order.contains(&skill.category) {
            category_order.push(skill.category);
        }
    }

    for category in category_order {
        let in_category: Vec<_> = endorsed.iter().filter(|e| e.category == category).collect();
        let installed_count = in_category
            .iter()
            .filter(|e| installed.contains(e.name))
            .count();
        output.push_str(&format!(
            "\n  {} ({}/{} installed)\n",
            category,
            installed_count,
            in_category.len()
        ));
        for skill in in_category {
            let is_installed = installed.contains(skill.name);
            let status = if is_installed {
                "installed"
            } else {
                "not installed"
            };
            output.push_str(&format!("  - /{} [{}]\n", skill.name, status));
            output.push_str(&format!("      {}\n", skill.description));
            output.push_str(&format!("      source: {}\n", skill.source));
            if !is_installed && let Some(install) = skill.install {
                output.push_str(&format!("      install: {}\n", install));
            }
        }
    }

    output.push_str(
        "\nActivate a loaded skill by loading it with skill_manage (action=load) or typing its slash command.\n",
    );
    output.push_str(
        "NVIDIA CUDA-X skills come from the official catalog at https://github.com/NVIDIA/skills.\n",
    );
}

pub(crate) async fn create_skill_for_repl(input: Value) -> Result<std::path::PathBuf> {
    let params: SkillInput = serde_json::from_value(input)?;
    Ok(create_skill_with_registry(&SkillRegistry::shared_registry(), params).await?.0)
}

async fn create_skill_with_registry(
    registry: &Arc<RwLock<SkillRegistry>>,
    params: SkillInput,
) -> Result<(std::path::PathBuf, Option<String>)> {
    let name = params
        .name
        .ok_or_else(|| anyhow::anyhow!("'name' is required for create action"))?;
    let description = params
        .description
        .ok_or_else(|| anyhow::anyhow!("'description' is required for create action"))?;
    let instructions = params
        .instructions
        .ok_or_else(|| anyhow::anyhow!("'instructions' is required for create action"))?;
    let root = factr_base::storage::factr_dir()?.join("skills");
    let made = create_or_merge_skill(
        &root,
        &name,
        &description,
        &instructions,
        params.package_name.as_deref(),
        params.package_code.as_deref(),
    )?;
    registry.write().await.reload_global()?;
    Ok(made)
}

fn create_skill_files(
    skills_root: &std::path::Path,
    name: &str,
    description: &str,
    instructions: &str,
    package_name: Option<&str>,
    package_code: Option<&str>,
) -> Result<std::path::PathBuf> {
    let valid_slug = |value: &str| {
        !value.is_empty()
            && value.len() <= 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
            && value.as_bytes()[0].is_ascii_lowercase()
    };
    anyhow::ensure!(
        valid_slug(name),
        "skill name must be a lowercase slug of at most 64 characters"
    );
    anyhow::ensure!(
        !description.trim().is_empty()
            && description.len() <= 500
            && !description.contains('\n')
            && !description.contains('\r'),
        "description must be a single line of at most 500 bytes"
    );
    anyhow::ensure!(
        !instructions.trim().is_empty() && instructions.len() <= 20_000,
        "instructions must contain 1-20000 bytes"
    );
    anyhow::ensure!(
        package_name.is_some() == package_code.is_some(),
        "package_name and package_code must be provided together"
    );
    if let (Some(module), Some(code)) = (package_name, package_code) {
        anyhow::ensure!(
            !module.is_empty()
                && module.len() <= 64
                && module.as_bytes()[0].is_ascii_alphabetic()
                && module
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_'),
            "package_name must be a Python identifier"
        );
        anyhow::ensure!(code.len() <= 65_536, "package_code exceeds 64 KiB");
    }

    std::fs::create_dir_all(skills_root)?;
    let skill_dir = skills_root.join(name);
    std::fs::create_dir(&skill_dir)
        .map_err(|error| anyhow::anyhow!("cannot create skill '{}': {error}", name))?;
    let result = (|| -> Result<()> {
        let parsed = parse_skill_text(instructions);
        std::fs::write(skill_dir.join("SKILL.md"), render_skill(name, description, &parsed.extras, 1, &parsed.body))?;
        if let (Some(module), Some(code)) = (package_name, package_code) {
            let package_dir = skill_dir.join("src").join(module);
            std::fs::create_dir_all(&package_dir)?;
            std::fs::write(package_dir.join("__init__.py"), code)?;
        }
        Ok(())
    })();
    if let Err(error) = result {
        let _ = std::fs::remove_dir_all(&skill_dir);
        return Err(error);
    }
    Ok(skill_dir)
}

/// A SKILL.md split into its pieces. Whatever the model passes as `instructions` (a bare body, or a
/// whole SKILL.md with its own front matter, even twice over) comes out as one body and the extra
/// front-matter keys, so the file written has exactly one well-formed front matter.
struct ParsedSkill {
    name: Option<String>,
    description: Option<String>,
    /// Other front-matter lines (top-level `key: value`), kept in order.
    extras: Vec<String>,
    version: u32,
    body: String,
}

fn parse_skill_text(text: &str) -> ParsedSkill {
    let mut parsed = ParsedSkill { name: None, description: None, extras: Vec::new(), version: 1, body: String::new() };
    let mut rest = text.trim_start_matches('\u{feff}').trim_start();
    // Strip every leading front-matter block (a doubled one is the malformed case).
    while rest.starts_with("---") && rest.lines().next().is_some_and(|l| l.trim() == "---") {
        let mut lines = rest.lines();
        lines.next();
        let mut consumed = 4; // "---\n"
        let mut closed = false;
        for line in lines.by_ref() {
            consumed += line.len() + 1;
            if line.trim() == "---" {
                closed = true;
                break;
            }
            let Some((key, value)) = line.trim().split_once(':') else { continue };
            let value = value.trim().trim_matches('"').trim_matches('\'').to_string();
            match key.trim() {
                "name" if parsed.name.is_none() => parsed.name = Some(value),
                "description" if parsed.description.is_none() => parsed.description = Some(value),
                "name" | "description" => {}
                "version" => parsed.version = value.parse().unwrap_or(1),
                key if !line.starts_with(char::is_whitespace) && !key.is_empty() => parsed.extras.push(line.trim_end().to_string()),
                _ => {}
            }
        }
        if !closed {
            break;
        }
        rest = rest.get(consumed.min(rest.len())..).unwrap_or("").trim_start();
    }
    parsed.body = rest.trim().to_string();
    parsed
}

fn render_skill(name: &str, description: &str, extras: &[String], version: u32, body: &str) -> String {
    let quoted_name = serde_json::to_string(name).unwrap_or_default();
    let quoted_description = serde_json::to_string(description).unwrap_or_default();
    let extras: String = extras.iter().map(|l| format!("{l}\n")).collect();
    let version = if version > 1 { format!("version: {version}\n") } else { String::new() };
    format!("---\nname: {quoted_name}\ndescription: {quoted_description}\n{extras}{version}---\n\n{body}\n")
}

struct SkillOnDisk {
    slug: String,
    parsed: ParsedSkill,
    learned: bool,
}

fn read_skill_on_disk(root: &std::path::Path, slug: &str) -> Option<SkillOnDisk> {
    let text = std::fs::read_to_string(root.join(slug).join("SKILL.md")).ok()?;
    Some(SkillOnDisk { slug: slug.to_string(), parsed: parse_skill_text(&text), learned: factr_learn::skill_files::is_learned(root, slug) })
}

fn skill_dirs(root: &std::path::Path) -> Vec<String> {
    let mut slugs: Vec<String> = std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().join("SKILL.md").is_file())
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    slugs.sort();
    slugs
}

/// Whether two skills are the same one: same slug, or a matching name, or the memory store's
/// near-duplicate overlap on name + description + body.
fn same_skill(slug_a: &str, a: &ParsedSkill, slug_b: &str, b: &ParsedSkill) -> bool {
    let text = |p: &ParsedSkill, slug: &str| format!("{}\n{}\n{}", p.name.clone().unwrap_or_else(|| slug.to_string()).replace('-', " "), p.description.clone().unwrap_or_default(), p.body);
    let title = |p: &ParsedSkill, slug: &str| p.name.clone().unwrap_or_else(|| slug.to_string()).replace('-', " ");
    let (ta, tb) = (title(a, slug_a), title(b, slug_b));
    slug_a == slug_b
        || (ta.split_whitespace().count() >= 2 && factr_base::near_duplicate(&ta, &tb).is_some())
        || factr_base::near_duplicate(&text(a, slug_a), &text(b, slug_b)).is_some()
}

/// `a` is more complete than `b`: longer content, else a more specific (longer) description.
fn more_complete(a: &ParsedSkill, b: &ParsedSkill) -> bool {
    (a.body.len(), a.description.as_deref().map_or(0, str::len)) > (b.body.len(), b.description.as_deref().map_or(0, str::len))
}

/// Save the file's current text under `<skill>/.history/` so the previous state can be restored.
fn back_up_skill(root: &std::path::Path, slug: &str, version: u32) {
    let dir = root.join(slug);
    let history = dir.join(".history");
    if std::fs::create_dir_all(&history).is_ok() {
        let _ = std::fs::copy(dir.join("SKILL.md"), history.join(format!("SKILL.md.v{version}")));
    }
}

/// `create`, but a skill that matches an existing one is not added a second time: the better one is
/// kept (more complete content, else the more specific description; a tie keeps the existing one and
/// bumps its version). The previous text of a replaced skill is saved in its `.history`. The note says
/// what happened; `None` means a new skill was created.
fn create_or_merge_skill(
    root: &std::path::Path,
    name: &str,
    description: &str,
    instructions: &str,
    package_name: Option<&str>,
    package_code: Option<&str>,
) -> Result<(std::path::PathBuf, Option<String>)> {
    let incoming = {
        let mut p = parse_skill_text(instructions);
        p.name = Some(name.to_string());
        p.description = Some(description.to_string());
        p
    };
    let existing = skill_dirs(root).into_iter().filter_map(|slug| read_skill_on_disk(root, &slug)).find(|e| same_skill(&e.slug, &e.parsed, name, &incoming));
    let Some(old) = existing else {
        return Ok((create_skill_files(root, name, description, instructions, package_name, package_code)?, None));
    };
    let path = root.join(&old.slug);
    let simple = package_name.is_none() && !old.learned;
    if simple && more_complete(&incoming, &old.parsed) {
        back_up_skill(root, &old.slug, old.parsed.version);
        let mut extras = old.parsed.extras.clone();
        let added: Vec<String> = incoming.extras.iter().filter(|l| !extras.contains(l)).cloned().collect();
        extras.extend(added);
        let version = old.parsed.version + 1;
        std::fs::write(path.join("SKILL.md"), render_skill(&old.slug, description, &extras, version, &incoming.body))?;
        return Ok((path, Some(format!("Skill '{name}' matches existing skill '{}': kept the more complete new content in it (version {version}, previous text in .history); no second skill was created.", old.slug))));
    }
    if !old.learned {
        back_up_skill(root, &old.slug, old.parsed.version);
        let version = old.parsed.version + 1;
        let body = old.parsed.body.clone();
        let desc = old.parsed.description.clone().unwrap_or_default();
        std::fs::write(path.join("SKILL.md"), render_skill(&old.slug, &desc, &old.parsed.extras, version, &body))?;
        return Ok((path, Some(format!("Skill '{name}' matches existing skill '{}': kept the existing one (version {version}); no second skill was created.", old.slug))));
    }
    Ok((path, Some(format!("Skill '{name}' matches the learned skill '{}': kept it; no second skill was created.", old.slug))))
}

/// One-shot pass over skills already on disk (`memory audit`): where two are the same skill, keep the
/// better and move the other to `skills.deduped/` (never deleted). Without `apply` it only reports.
/// Skills the learning loop wrote are never moved.
pub fn dedupe_skills_on_disk(root: &std::path::Path, apply: bool) -> Vec<String> {
    let mut report = Vec::new();
    let mut gone: Vec<String> = Vec::new();
    let slugs = skill_dirs(root);
    for (i, a_slug) in slugs.iter().enumerate() {
        if gone.contains(a_slug) {
            continue;
        }
        for b_slug in &slugs[i + 1..] {
            if gone.contains(b_slug) || gone.contains(a_slug) {
                continue;
            }
            let (Some(a), Some(b)) = (read_skill_on_disk(root, a_slug), read_skill_on_disk(root, b_slug)) else { continue };
            if !same_skill(&a.slug, &a.parsed, &b.slug, &b.parsed) {
                continue;
            }
            let (keep, drop) = match (a.learned, b.learned) {
                (true, false) => (&a, &b),
                (false, true) => (&b, &a),
                (true, true) => continue,
                _ if more_complete(&b.parsed, &a.parsed) => (&b, &a),
                _ => (&a, &b),
            };
            report.push(format!("skill '{}' duplicates '{}': kept '{}'{}", drop.slug, keep.slug, keep.slug, if apply { format!(", moved '{}' to skills.deduped/", drop.slug) } else { " (dry run)".into() }));
            if apply {
                let target = root.parent().unwrap_or(root).join("skills.deduped");
                if std::fs::create_dir_all(&target).is_ok() && std::fs::rename(root.join(&drop.slug), target.join(&drop.slug)).is_ok() {
                    gone.push(drop.slug.clone());
                }
            } else {
                gone.push(drop.slug.clone());
            }
        }
    }
    report
}

/// patch/edit/write_file/remove_file on one skill directory. A skill the learning loop wrote
/// (factr-learn's marker) is regenerated from its memory row, so its SKILL.md is not edited here.
fn modify_skill_files(dir: &std::path::Path, p: &SkillInput) -> Result<String> {
    use std::path::Component;
    let md = dir.join("SKILL.md");
    let learned = || {
        dir.file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|slug| {
                factr_learn::skill_files::is_learned(dir.parent().unwrap_or(dir), slug)
            })
    };
    let need = |v: &Option<String>, what: &str| -> Result<String> {
        v.clone()
            .ok_or_else(|| anyhow::anyhow!("'{what}' is required for {} action", p.action))
    };
    let file = |rel: &str| -> Result<std::path::PathBuf> {
        let rel = std::path::Path::new(rel);
        anyhow::ensure!(
            rel.components().all(|c| matches!(c, Component::Normal(_)))
                && rel != std::path::Path::new("SKILL.md"),
            "file_path must be a relative path inside the skill (not SKILL.md)"
        );
        Ok(dir.join(rel))
    };
    let guard_learned = || {
        anyhow::ensure!(
            !learned(),
            "this skill was learned by the learning loop; change its memory entry instead (SKILL.md is regenerated from it)"
        );
        Ok::<(), anyhow::Error>(())
    };
    match p.action.as_str() {
        "edit" => {
            guard_learned()?;
            let body = need(&p.file_content, "file_content")?;
            anyhow::ensure!(
                body.starts_with("---"),
                "SKILL.md must start with YAML frontmatter"
            );
            std::fs::write(&md, body)?;
            Ok("Rewrote SKILL.md".into())
        }
        "patch" => {
            let target = match &p.file_path {
                Some(rel) => file(rel)?,
                None => {
                    guard_learned()?;
                    md
                }
            };
            let old = need(&p.old_string, "old_string")?;
            let new = need(&p.new_string, "new_string")?;
            let text = std::fs::read_to_string(&target)?;
            anyhow::ensure!(
                !old.is_empty() && text.matches(&old).count() == 1,
                "old_string must match exactly once"
            );
            std::fs::write(&target, text.replacen(&old, &new, 1))?;
            Ok(format!("Patched {}", target.display()))
        }
        "write_file" => {
            let target = file(&need(&p.file_path, "file_path")?)?;
            let body = need(&p.file_content, "file_content")?;
            anyhow::ensure!(body.len() <= 1_048_576, "file_content exceeds 1 MiB");
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&target, body)?;
            Ok(format!("Wrote {}", target.display()))
        }
        _ => {
            let target = file(&need(&p.file_path, "file_path")?)?;
            std::fs::remove_file(&target)?;
            Ok(format!("Removed {}", target.display()))
        }
    }
}

fn normalize_skill_name(name: Option<String>, action: &str) -> Result<String> {
    let name = name.ok_or_else(|| anyhow::anyhow!("'name' is required for {} action", action))?;
    let trimmed = name.trim().trim_start_matches('/').to_string();
    if trimmed.is_empty() {
        anyhow::bail!("'name' is required for {} action", action);
    }
    Ok(trimmed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(action: &str) -> SkillInput {
        serde_json::from_value(json!({"action": action, "name": "s"})).unwrap()
    }

    #[test]
    fn modifies_user_skill_and_refuses_learned_or_escaping_paths() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("s");
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("SKILL.md"), "---\nname: s\n---\nhello world\n").unwrap();
        let mut p = input("patch");
        p.old_string = Some("hello".into());
        p.new_string = Some("bye".into());
        modify_skill_files(&dir, &p).unwrap();
        assert!(
            std::fs::read_to_string(dir.join("SKILL.md"))
                .unwrap()
                .contains("bye world")
        );
        let mut w = input("write_file");
        w.file_path = Some("references/a.md".into());
        w.file_content = Some("x".into());
        modify_skill_files(&dir, &w).unwrap();
        assert!(dir.join("references/a.md").is_file());
        let mut r = input("remove_file");
        r.file_path = Some("references/a.md".into());
        modify_skill_files(&dir, &r).unwrap();
        assert!(!dir.join("references/a.md").exists());
        w.file_path = Some("../x".into());
        assert!(modify_skill_files(&dir, &w).is_err());
        w.file_path = Some("SKILL.md".into());
        assert!(modify_skill_files(&dir, &w).is_err());
        std::fs::write(
            dir.join("SKILL.md"),
            "---\nname: s\nfactr-learned: true\n---\nbody\n",
        )
        .unwrap();
        assert!(modify_skill_files(&dir, &p).is_err());
        let mut e = input("edit");
        e.file_content = Some("---\nx".into());
        assert!(modify_skill_files(&dir, &e).is_err());
    }

    #[test]
    fn create_writes_one_well_formed_front_matter_whatever_the_model_passes() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("skills");
        let messy = "---\nname: project-codeword\ndescription: old\n---\n---\n description: stray\n---\nlicense: mit\nWhen asked for the project codeword answer BLUEFIN-7";
        let (path, note) = create_or_merge_skill(&root, "project-codeword", "Answer the project codeword", messy, None, None).unwrap();
        assert!(note.is_none());
        let text = std::fs::read_to_string(path.join("SKILL.md")).unwrap();
        assert_eq!(text.matches("---").count(), 2, "{text}");
        assert!(text.starts_with("---\nname: \"project-codeword\"\ndescription: \"Answer the project codeword\"\n---\n"), "{text}");
        assert!(text.contains("BLUEFIN-7") && !text.contains("stray"), "{text}");
        let again = parse_skill_text(&text);
        assert_eq!((again.name.as_deref(), again.version), (Some("project-codeword"), 1));
    }

    #[test]
    fn a_matching_skill_is_merged_not_duplicated_and_the_better_one_kept() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("skills");
        create_or_merge_skill(&root, "run-unit-tests", "Run tests", "Run the unit tests before saying done", None, None).unwrap();
        // Tie or worse: the existing stays, its version is bumped.
        let (path, note) = create_or_merge_skill(&root, "run-the-unit-tests", "Run tests", "Run the unit tests before saying done", None, None).unwrap();
        assert!(note.unwrap().contains("kept the existing one (version 2)"));
        assert_eq!(skill_dirs(&root), ["run-unit-tests"], "no second skill");
        assert!(std::fs::read_to_string(path.join("SKILL.md")).unwrap().contains("version: 2"));
        // Better: the new content replaces it, the old text is saved.
        let (_, note) = create_or_merge_skill(&root, "run-unit-tests", "Run the unit tests", "Run the unit tests before saying done and list failing tests", None, None).unwrap();
        assert!(note.unwrap().contains("more complete"));
        let text = std::fs::read_to_string(path.join("SKILL.md")).unwrap();
        assert!(text.contains("list failing tests") && text.contains("version: 3"), "{text}");
        assert!(path.join(".history/SKILL.md.v2").is_file());
        assert_eq!(skill_dirs(&root).len(), 1);
        // An unrelated skill is untouched.
        create_or_merge_skill(&root, "deploy-app", "Deploy the app", "Run the deploy script", None, None).unwrap();
        assert_eq!(skill_dirs(&root).len(), 2);
    }

    #[test]
    fn the_one_shot_pass_reports_and_moves_duplicates_already_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("skills");
        for (slug, body) in [("run-unit-tests", "Run the unit tests before saying done"), ("run-the-unit-tests", "Run the unit tests before saying done and list failing tests"), ("deploy-app", "Run the deploy script")] {
            std::fs::create_dir_all(root.join(slug)).unwrap();
            std::fs::write(root.join(slug).join("SKILL.md"), format!("---\nname: {slug}\ndescription: d\n---\n\n{body}\n")).unwrap();
        }
        let dry = dedupe_skills_on_disk(&root, false);
        assert_eq!(dry.len(), 1);
        assert!(dry[0].contains("kept 'run-the-unit-tests'") && dry[0].contains("dry run"), "{dry:?}");
        assert_eq!(skill_dirs(&root).len(), 3);
        dedupe_skills_on_disk(&root, true);
        assert_eq!(skill_dirs(&root), ["deploy-app", "run-the-unit-tests"]);
        assert!(dir.path().join("skills.deduped/run-unit-tests/SKILL.md").is_file());
    }

    fn create_test_tool() -> SkillTool {
        let registry = Arc::new(RwLock::new(SkillRegistry::default()));
        SkillTool::new(registry)
    }

    #[test]
    fn creates_python_package_skill_with_confined_paths() {
        let dir = tempfile::tempdir().unwrap();
        let created = create_skill_files(
            &dir.path().join("skills"),
            "demo-skill",
            "A test skill",
            "Use the demo package.",
            Some("demo_skill"),
            Some("def value():\n    return 42\n"),
        )
        .unwrap();
        assert!(created.join("SKILL.md").is_file());
        assert_eq!(
            std::fs::read_to_string(created.join("src/demo_skill/__init__.py")).unwrap(),
            "def value():\n    return 42\n"
        );
        assert!(
            create_skill_files(
                &dir.path().join("skills"),
                "../escape",
                "bad",
                "bad",
                None,
                None,
            )
            .is_err()
        );
        // The same slug is one skill: create_skill_files itself still refuses to overwrite.
        assert!(
            create_skill_files(
                &dir.path().join("skills"),
                "demo-skill",
                "overwrite",
                "overwrite",
                None,
                None,
            )
            .is_err()
        );
    }

    fn create_test_tool_with_skill(name: &str) -> (SkillTool, tempfile::TempDir) {
        let temp_dir = tempfile::tempdir().unwrap();
        let skill_dir = temp_dir.path().join(".factr/engine").join("skills").join(name);
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            format!(
                "---\nname: {name}\ndescription: Test skill\n---\n\n# Test Skill\n\nUse this test skill."
            ),
        )
        .unwrap();

        let registry = SkillRegistry::from_dir(&temp_dir.path().join(".factr/engine").join("skills")).unwrap();
        let tool = SkillTool::new(Arc::new(RwLock::new(registry)));
        (tool, temp_dir)
    }

    fn create_test_context() -> ToolContext {
        ToolContext {
            session_id: "test-session".to_string(),
            message_id: "test-message".to_string(),
            tool_call_id: "test-tool-call".to_string(),
            working_dir: None,
            stdin_request_tx: None,
            graceful_shutdown_signal: None,
            execution_mode: crate::tool::ToolExecutionMode::Direct,
        }
    }

    #[test]
    fn test_tool_name() {
        let tool = create_test_tool();
        assert_eq!(tool.name(), "skill_manage");
    }

    #[test]
    fn test_tool_description() {
        let tool = create_test_tool();
        assert!(tool.description().contains("skill"));
    }

    #[test]
    fn test_parameters_schema() {
        let tool = create_test_tool();
        let schema = tool.parameters_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["action"].is_object());
        assert!(schema["properties"]["name"].is_object());
    }

    #[tokio::test]
    async fn test_list_empty() {
        let tool = create_test_tool();
        let ctx = create_test_context();
        let input = json!({"action": "list"});

        let result = tool.execute(input, ctx).await.unwrap();
        assert!(result.output.contains("No skills loaded"));
        // Even with no skills loaded, the endorsed catalog should be listed.
        assert!(result.output.contains("Endorsed skills"));
    }

    #[tokio::test]
    async fn test_list_includes_endorsed_skills() {
        let tool = create_test_tool();
        let ctx = create_test_context();
        let input = json!({"action": "list"});

        let result = tool.execute(input, ctx).await.unwrap();
        // Every endorsed skill should appear with an install-status marker.
        for endorsed in crate::skill::endorsed_skills() {
            assert!(
                result.output.contains(&format!("/{}", endorsed.name)),
                "expected endorsed skill /{} in:\n{}",
                endorsed.name,
                result.output
            );
        }
        // No skills are loaded in this tool, so they should be "not installed".
        assert!(result.output.contains("[not installed]"));
    }

    #[tokio::test]
    async fn test_load_missing_name() {
        let tool = create_test_tool();
        let ctx = create_test_context();
        let input = json!({"action": "load"});

        let result = tool.execute(input, ctx).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("name"));
    }

    #[tokio::test]
    async fn test_load_accepts_skill_alias_and_args() {
        // `disabled_skill_is_not_listed_or_loadable` points FACTR_CONFIG_HOME at a config that disables this skill.
        let _env = factr_base::storage::lock_test_env();
        let (tool, _temp_dir) = create_test_tool_with_skill("optimization");
        let ctx = create_test_context();
        let input = json!({"skill": "optimization", "args": "optimize this"});

        let result = tool.execute(input, ctx).await.unwrap();
        assert!(result.output.contains("## Skill: optimization"));
        assert_eq!(result.title.as_deref(), Some("skill: optimization"));
    }

    #[tokio::test]
    async fn test_load_strips_leading_slash_from_name() {
        // `disabled_skill_is_not_listed_or_loadable` points FACTR_CONFIG_HOME at a config that disables this skill.
        let _env = factr_base::storage::lock_test_env();
        let (tool, _temp_dir) = create_test_tool_with_skill("optimization");
        let ctx = create_test_context();
        let input = json!({"action": "load", "name": "/optimization"});

        let result = tool.execute(input, ctx).await.unwrap();
        assert!(result.output.contains("## Skill: optimization"));
    }

    #[tokio::test]
    async fn disabled_skill_is_not_listed_or_loadable() {
        let (_, temp_dir) = create_test_tool_with_skill("optimization");
        let _env = factr_base::storage::lock_test_env();
        let factr = tempfile::tempdir().unwrap();
        std::fs::write(
            factr.path().join("config.yaml"),
            "skills:\n  disabled: [optimization]\n",
        )
        .unwrap();
        // SAFETY: serialized by lock_test_env; restored below.
        unsafe { std::env::set_var("FACTR_CONFIG_HOME", factr.path()) };
        let registry = SkillRegistry::from_dir(&temp_dir.path().join(".factr/engine").join("skills")).unwrap();
        let tool = SkillTool::new(Arc::new(RwLock::new(registry)));

        let listed = tool
            .execute(json!({"action": "list"}), create_test_context())
            .await
            .unwrap();
        assert!(!listed.output.contains("## /optimization"));
        let loaded = tool
            .execute(
                json!({"action": "load", "name": "optimization"}),
                create_test_context(),
            )
            .await;
        assert!(loaded.is_err());

        // Enabling it in the same config makes it visible again, no restart.
        std::fs::write(
            factr.path().join("config.yaml"),
            "skills:\n  disabled: []\n",
        )
        .unwrap();
        let listed = tool
            .execute(json!({"action": "list"}), create_test_context())
            .await
            .unwrap();
        unsafe { std::env::remove_var("FACTR_CONFIG_HOME") };
        assert!(listed.output.contains("## /optimization"));
    }

    #[tokio::test]
    async fn test_reload_missing_name() {
        let tool = create_test_tool();
        let ctx = create_test_context();
        let input = json!({"action": "reload"});

        let result = tool.execute(input, ctx).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("name"));
    }

    #[tokio::test]
    async fn test_read_missing_name() {
        let tool = create_test_tool();
        let ctx = create_test_context();
        let input = json!({"action": "read"});

        let result = tool.execute(input, ctx).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("name"));
    }

    #[tokio::test]
    async fn test_reload_nonexistent() {
        let tool = create_test_tool();
        let ctx = create_test_context();
        let input = json!({"action": "reload", "name": "nonexistent"});

        let result = tool.execute(input, ctx).await.unwrap();
        assert!(result.output.contains("not found"));
    }

    #[tokio::test]
    async fn test_unknown_action() {
        let tool = create_test_tool();
        let ctx = create_test_context();
        let input = json!({"action": "invalid"});

        let result = tool.execute(input, ctx).await.unwrap();
        assert!(result.output.contains("Unknown action"));
    }

    #[tokio::test]
    async fn test_reload_all() {
        let tool = create_test_tool();
        let ctx = create_test_context();
        let input = json!({"action": "reload_all"});

        let result = tool.execute(input, ctx).await.unwrap();
        // The output format is "Reloaded N skills" where N is any number
        // (depends on what skills exist on the system)
        assert!(
            result.output.contains("Reloaded"),
            "Expected 'Reloaded' in output, got: {}",
            result.output
        );
        assert!(
            result.output.contains("skills"),
            "Expected 'skills' in output, got: {}",
            result.output
        );
    }

    fn context_with_working_dir(dir: &std::path::Path) -> ToolContext {
        ToolContext {
            working_dir: Some(dir.to_path_buf()),
            ..create_test_context()
        }
    }

    fn write_project_skill(root: &std::path::Path, name: &str) {
        let skill_dir = root.join(".agents").join("skills").join(name);
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: Project skill {name}\n---\n\nBody."),
        )
        .unwrap();
    }

    /// One skills dir: a project's `.agents/skills` (or `.factr/engine`/`.claude`) is not a skill source.
    #[tokio::test]
    async fn project_local_skill_dirs_are_not_read() {
        let tool = create_test_tool();
        let repo = tempfile::tempdir().unwrap();
        write_project_skill(repo.path(), "repo-skill");
        let ctx = context_with_working_dir(repo.path());
        let list = tool.execute(json!({"action": "list"}), ctx).await.unwrap();
        assert!(!list.output.contains("repo-skill"), "{}", list.output);
    }
}
