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
            let path = factr_base::factr_config::home().map(|h| h.to_string_lossy().into_owned()).unwrap_or_default();
            json!({
                "profiles": [{
                    "name": "default", "path": path, "is_default": true, "model": null, "provider": null,
                    "description": "", "display_name": "", "skill_count": 0, "previous_names": [], "role": null,
                    "has_avatar": false,
                }],
                "bot_mode_protocol": true,
            })
        }
        _ => return None,
    })
}

/// Every method above, for the boot-sequence test.
#[cfg(all(test, unix))]
pub(crate) const METHODS: &[&str] = &[
    "bot_relay.roster.sync", "wake.status", "pet.info", "pet.info.meta",
    "profiles.list",
];
