//! Read-only probes the desktop sends at launch (and polls) that Factr's Python backend answers
//! from its own state. Answered here with the "nothing there yet" shape while Python is not running,
//! so a chat-only session never starts it; once Python is up for something else, these are forwarded
//! and answered for real.

use serde_json::{Value, json};

/// The launch-time answer for `method`, or `None` when the method is not a boot probe.
pub(crate) fn answer(method: &str, _params: &Value) -> Option<Value> {
    Some(match method {
        "bot_relay.roster.sync" => json!({ "count": 0 }),
        "wake.status" => json!({
            "listening": false, "owned_by_caller": false, "owner_surface": null,
            "phrase": "", "provider": "", "configured_surface": "auto",
            "input_device": {}, "available": false, "hint": "", "enabled": false,
            "audio_silent": false, "capture": "auto", "local_input_available": false,
        }),
        "pet.info" | "pet.info.meta" => json!({ "enabled": false }),
        "profiles.list" => {
            let home = factr_base::factr_config::home();
            let root = home.as_deref().map(root_home);
            let path = root.as_ref().map(|h| h.to_string_lossy().into_owned()).unwrap_or_default();
            let mut profiles = vec![profile_row("default", &path, true)];
            // A bot created in this session lives on disk before Python has ever run for it: list the
            // profile folders so the roster shows it. Python's richer answer replaces this once it is up.
            if let Some(root) = &root {
                for name in profile_dirs(root) {
                    profiles.push(profile_row(&name, &root.join("profiles").join(&name).to_string_lossy(), false));
                }
            }
            json!({ "profiles": profiles, "bot_mode_protocol": true })
        }
        _ => return None,
    })
}

fn profile_row(name: &str, path: &str, is_default: bool) -> Value {
    json!({
        "name": name, "path": path, "is_default": is_default, "model": null, "provider": null,
        "description": "", "display_name": "", "skill_count": 0, "previous_names": [], "role": null,
        "has_avatar": false,
    })
}

/// The config root: a profile engine's home is `<root>/profiles/<name>`.
fn root_home(home: &std::path::Path) -> std::path::PathBuf {
    match home.parent() {
        Some(parent) if parent.file_name().is_some_and(|n| n == "profiles") => {
            parent.parent().map(std::path::Path::to_path_buf).unwrap_or_else(|| home.to_path_buf())
        }
        _ => home.to_path_buf(),
    }
}

/// Profile folder names under `<root>/profiles`, sorted; hidden and oddly named folders are skipped.
fn profile_dirs(root: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(root.join("profiles"))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| !n.is_empty() && n != "default" && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'))
        .collect();
    names.sort();
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_profile_folders_and_finds_the_root_from_a_profile_home() {
        let dir = std::env::temp_dir().join(format!("factr-boot-stub-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("profiles/scribe")).unwrap();
        std::fs::create_dir_all(dir.join("profiles/.hidden")).unwrap();
        std::fs::create_dir_all(dir.join("profiles/bad name")).unwrap();
        std::fs::write(dir.join("profiles/not-a-dir"), "x").unwrap();
        assert_eq!(profile_dirs(&dir), vec!["scribe".to_string()]);
        assert_eq!(root_home(&dir.join("profiles/scribe")), dir);
        assert_eq!(root_home(&dir), dir);
        assert!(profile_dirs(&dir.join("missing")).is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

/// Every method above, for the boot-sequence test.
#[cfg(all(test, unix))]
pub(crate) const METHODS: &[&str] = &[
    "bot_relay.roster.sync", "wake.status", "pet.info", "pet.info.meta",
    "profiles.list",
];
