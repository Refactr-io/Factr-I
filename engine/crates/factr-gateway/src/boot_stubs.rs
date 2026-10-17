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

/// True when `method` is a pet poll and the profile's config.yaml says the pet is off (the default),
/// so the answer is the same `{"enabled": false}` Python gives and the poll need not wake or keep
/// alive the Python backend. Anything unclear (unreadable file, odd value, session-bound call) is false.
pub(crate) fn pet_disabled(method: &str, params: &Value) -> bool {
    if !matches!(method, "pet.info" | "pet.info.meta") || params["session_id"].as_str().is_some_and(|s| !s.is_empty()) {
        return false;
    }
    // Python layers a managed-scope config over config.yaml; with one present the file alone is not the answer.
    let managed = std::env::var_os("FACTR_MANAGED_DIR").is_some_and(|d| !d.is_empty()) || std::path::Path::new("/etc/factr").is_dir();
    let Some(home) = factr_base::factr_config::home().filter(|_| !managed) else { return false };
    pet_disabled_in(&home, params["profile"].as_str().unwrap_or(""))
}

fn pet_disabled_in(home: &std::path::Path, profile: &str) -> bool {
    let profile = profile.trim();
    let dir = if profile.is_empty() {
        home.to_path_buf()
    } else if profile == "default" {
        root_home(home)
    } else if profile.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        root_home(home).join("profiles").join(profile)
    } else {
        return false;
    };
    let raw = match std::fs::read_to_string(dir.join("config.yaml")) {
        Ok(raw) => raw,
        Err(e) => return e.kind() == std::io::ErrorKind::NotFound,
    };
    let Ok(doc) = serde_yaml::from_str::<serde_yaml::Value>(&raw) else { return false };
    match doc.get("display").and_then(|d| d.get("pet")).and_then(|p| p.get("enabled")) {
        None | Some(serde_yaml::Value::Null) | Some(serde_yaml::Value::Bool(false)) => true,
        Some(serde_yaml::Value::String(s)) => matches!(s.trim().to_ascii_lowercase().as_str(), "" | "false" | "0" | "no" | "off"),
        _ => false,
    }
}

fn profile_row(name: &str, path: &str, is_default: bool) -> Value {
    json!({
        "name": name, "path": path, "is_default": is_default, "model": null, "provider": null,
        "description": "", "display_name": "", "skill_count": 0, "previous_names": [], "role": null,
        "has_avatar": false,
    })
}

/// The config root: a profile engine's home is `<root>/profiles/<name>`.
pub(crate) fn root_home(home: &std::path::Path) -> std::path::PathBuf {
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
    fn pet_polls_are_local_only_when_the_config_says_off() {
        let dir = std::env::temp_dir().join(format!("factr-pet-stub-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("profiles/scribe")).unwrap();
        assert!(pet_disabled_in(&dir, ""), "no config.yaml is the default: off");
        std::fs::write(dir.join("config.yaml"), "display:\n  pet:\n    enabled: false\n").unwrap();
        assert!(pet_disabled_in(&dir, "") && pet_disabled_in(&dir, "default"));
        std::fs::write(dir.join("config.yaml"), "display:\n  pet:\n    enabled: true\n").unwrap();
        assert!(!pet_disabled_in(&dir, ""), "an enabled pet is forwarded");
        std::fs::write(dir.join("config.yaml"), "display:\n  pet:\n    enabled: 'yes'\n").unwrap();
        assert!(!pet_disabled_in(&dir, ""));
        std::fs::write(dir.join("config.yaml"), ": : not yaml [").unwrap();
        assert!(!pet_disabled_in(&dir, ""), "unreadable config is forwarded");
        std::fs::write(dir.join("profiles/scribe/config.yaml"), "display:\n  pet:\n    enabled: true\n").unwrap();
        assert!(!pet_disabled_in(&dir, "scribe") && !pet_disabled_in(&dir, "../x"));
        assert!(!pet_disabled("pet.select", &json!({})) && !pet_disabled("pet.info", &json!({ "session_id": "s1" })));
        std::fs::remove_dir_all(&dir).unwrap();
    }

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
