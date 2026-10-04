use anyhow::Result;
use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::Path;

const FILES: &[(&str, &str)] = &[
    ("learn-goal/SKILL.md", include_str!("skills/goal/SKILL.md")),
    (
        "learn-goal/src/goal/__init__.py",
        include_str!("skills/goal/src/goal/__init__.py"),
    ),
    (
        "learn-refine/SKILL.md",
        include_str!("skills/refine/SKILL.md"),
    ),
    (
        "learn-refine/src/refine/__init__.py",
        include_str!("skills/refine/src/refine/__init__.py"),
    ),
    (
        "learn-rlm-heartbeat/SKILL.md",
        include_str!("skills/rlm-heartbeat/SKILL.md"),
    ),
    (
        "learn-rlm-heartbeat/src/rlm_heartbeat/__init__.py",
        include_str!("skills/rlm-heartbeat/src/rlm_heartbeat/__init__.py"),
    ),
    (
        "learn-agent-message/SKILL.md",
        include_str!("skills/agent-message/SKILL.md"),
    ),
    (
        "learn-agent-message/src/agent_message/__init__.py",
        include_str!("skills/agent-message/src/agent_message/__init__.py"),
    ),
    (
        "learn-agent-observe/SKILL.md",
        include_str!("skills/agent-observe/SKILL.md"),
    ),
    (
        "learn-agent-observe/src/agent_observe/__init__.py",
        include_str!("skills/agent-observe/src/agent_observe/__init__.py"),
    ),
    (
        "learn-compact/SKILL.md",
        include_str!("skills/compact/SKILL.md"),
    ),
    (
        "learn-compact/src/compact/__init__.py",
        include_str!("skills/compact/src/compact/__init__.py"),
    ),
    (
        "learn-websearch/SKILL.md",
        include_str!("skills/websearch/SKILL.md"),
    ),
    (
        "learn-websearch/src/websearch/__init__.py",
        include_str!("skills/websearch/src/websearch/__init__.py"),
    ),
    (
        "learn-websearch/src/websearch/websearch.py",
        include_str!("skills/websearch/src/websearch/websearch.py"),
    ),
    (
        "learn-skill-creator/SKILL.md",
        include_str!("skills/skill-creator/SKILL.md"),
    ),
    (
        "learn-skill-creator/src/skill_creator/__init__.py",
        include_str!("skills/skill-creator/src/skill_creator/__init__.py"),
    ),
    (
        "learn-skill-creator/references/python-skills.md",
        include_str!("skills/skill-creator/references/python-skills.md"),
    ),
];

/// Directory names of the skills this engine ships (the `learn-*` wrappers).
pub fn shipped_dirs() -> Vec<&'static str> {
    let mut dirs: Vec<&str> = FILES.iter().filter_map(|(relative, _)| relative.split('/').next()).collect();
    dirs.dedup();
    dirs
}

/// Install the shipped factr-learn wrappers without overwriting user-owned skills.
pub fn install(skills_root: &Path) -> Result<()> {
    let mut prepared = HashSet::new();
    let mut skipped = HashSet::new();
    for (relative, contents) in FILES {
        let skill_dir = relative.split('/').next().expect("skill path is nonempty");
        if skipped.contains(skill_dir) {
            continue;
        }
        if !prepared.contains(skill_dir) {
            let directory = skills_root.join(skill_dir);
            if directory.exists() {
                skipped.insert(skill_dir);
                continue;
            }
            fs::create_dir_all(&directory)?;
            prepared.insert(skill_dir);
        }
        let path = skills_root.join(relative);
        let parent = path.parent().expect("skill asset has a parent");
        fs::create_dir_all(parent)?;
        let mut file = match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        };
        file.write_all(contents.as_bytes())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::install;

    #[test]
    fn bundle_install_is_repeatable_and_preserves_user_skill_directories() {
        let root = std::env::temp_dir().join(format!("learn-skills-{}", uuid::Uuid::new_v4()));
        let user_skill = root.join("learn-goal");
        std::fs::create_dir_all(&user_skill).unwrap();
        std::fs::write(user_skill.join("SKILL.md"), "user-owned").unwrap();

        install(&root).unwrap();
        install(&root).unwrap();

        assert_eq!(
            std::fs::read_to_string(user_skill.join("SKILL.md")).unwrap(),
            "user-owned"
        );
        assert!(!user_skill.join("src/goal/__init__.py").exists());
        assert!(root.join("learn-refine/src/refine/__init__.py").is_file());
        std::fs::remove_dir_all(root).unwrap();
    }
}
