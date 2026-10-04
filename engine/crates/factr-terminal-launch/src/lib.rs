use anyhow::Result;
use std::process::Command;


/// Terminal/window-manager environment variables that identify *which*
/// terminal, multiplexer, or display a client is attached to.
///
/// The factr server process is long-lived and captures these at *its* startup,
/// so once a client connects from a different terminal/tmux/zellij session the
/// server's copies are stale. Spawn and focus hooks executed by the server then
/// target the wrong terminal (see issue #405). To fix this, clients snapshot
/// these vars from their own environment and send them to the server, which
/// re-exports them to spawn/focus hooks so the hook places windows in the
/// terminal the user is actually looking at.
///
/// This intentionally covers terminal multiplexers (tmux, screen, zellij),
/// terminal emulators (kitty, wezterm, ghostty, iTerm, ...), and the display
/// server (X11 `DISPLAY`, Wayland `WAYLAND_DISPLAY`) so window placement and
/// routing all follow the connecting client.
pub const CLIENT_TERMINAL_ENV_VARS: &[&str] = &[
    // Terminal multiplexers
    "ZELLIJ",
    "ZELLIJ_SESSION_NAME",
    "ZELLIJ_PANE_ID",
    "TMUX",
    "TMUX_PANE",
    "STY",
    // herdr terminal multiplexer (https://herdr.dev), see issue #405
    "HERDR_ENV",
    "HERDR_SOCKET_PATH",
    "HERDR_PANE_ID",
    "HERDR_TAB_ID",
    "HERDR_WORKSPACE_ID",
    "HERDR_BIN_PATH",
    "HERDR_SESSION",
    "HERDR_AGENT",
    // Terminal emulators
    "TERM",
    "TERM_PROGRAM",
    "TERM_PROGRAM_VERSION",
    "COLORTERM",
    "KITTY_PID",
    "KITTY_WINDOW_ID",
    "KITTY_LISTEN_ON",
    "WEZTERM_PANE",
    "WEZTERM_EXECUTABLE",
    "WEZTERM_UNIX_SOCKET",
    "ALACRITTY_WINDOW_ID",
    "ALACRITTY_SOCKET",
    "GHOSTTY_RESOURCES_DIR",
    "GHOSTTY_BIN_DIR",
    "ITERM_SESSION_ID",
    "WINDOWID",
    "HANDTERM_SESSION",
    "HANDTERM_PID",
    "WT_SESSION",
    "WT_PROFILE_ID",
    // Display / window manager
    "DISPLAY",
    "WAYLAND_DISPLAY",
];

/// Snapshot the current process's terminal-identifying env vars (see
/// [`CLIENT_TERMINAL_ENV_VARS`]). Only vars that are actually set are included,
/// so the map is empty when nothing identifies the terminal.
pub fn snapshot_client_terminal_env() -> Vec<(String, String)> {
    CLIENT_TERMINAL_ENV_VARS
        .iter()
        .filter_map(|&key| {
            std::env::var(key)
                .ok()
                .map(|value| (key.to_string(), value))
        })
        .collect()
}

/// Replace inherited terminal identity with an authoritative client snapshot.
///
/// Removing every known key first is important for a shared server: an empty
/// client snapshot must not leak the pane that happened to start the server.
/// Aliases let integrations explicitly distinguish client values from other
/// process environment while native names preserve existing hook behavior.
pub fn apply_client_terminal_env(cmd: &mut Command, env: &[(String, String)]) {
    for key in CLIENT_TERMINAL_ENV_VARS {
        cmd.env_remove(key);
        cmd.env_remove(format!("FACTR_CLIENT_{key}"));
    }
    for (key, value) in env {
        if CLIENT_TERMINAL_ENV_VARS.contains(&key.as_str()) {
            cmd.env(key, value);
            cmd.env(format!("FACTR_CLIENT_{key}"), value);
        }
    }
}


pub fn sh_escape(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\"'\"'"))
}


/// Parse an external spawn-hook command line into argv parts.
///
/// Supports basic POSIX-style word splitting: whitespace separates arguments,
/// single and double quotes group words, and backslash escapes the next
/// character (outside single quotes). Errors on empty input, unterminated
/// quotes, and trailing escapes.
pub fn parse_hook_command(raw: &str) -> Result<Vec<String>> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut token_started = false;

    for ch in raw.chars() {
        if escaped {
            current.push(ch);
            token_started = true;
            escaped = false;
            continue;
        }

        if let Some(quote_ch) = quote {
            if ch == quote_ch {
                quote = None;
            } else if ch == '\\' && quote_ch == '"' {
                escaped = true;
            } else {
                current.push(ch);
                token_started = true;
            }
            continue;
        }

        match ch {
            '\\' => {
                escaped = true;
                token_started = true;
            }
            '\'' | '"' => {
                quote = Some(ch);
                token_started = true;
            }
            ch if ch.is_whitespace() => {
                if token_started {
                    parts.push(std::mem::take(&mut current));
                    token_started = false;
                }
            }
            ch => {
                current.push(ch);
                token_started = true;
            }
        }
    }

    if escaped {
        anyhow::bail!("spawn hook command ends with an escape character");
    }
    if quote.is_some() {
        anyhow::bail!("spawn hook command has an unterminated quote");
    }
    if token_started {
        parts.push(current);
    }
    if parts.is_empty() {
        anyhow::bail!("spawn hook command is empty");
    }

    Ok(parts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn client_terminal_env_replaces_inherited_identity_and_exports_aliases() {
        let mut command = Command::new("hook");
        command.env("HERDR_PANE_ID", "stale-pane");
        command.env("TMUX_PANE", "stale-tmux");
        apply_client_terminal_env(
            &mut command,
            &[
                ("HERDR_PANE_ID".to_string(), "client-pane".to_string()),
                ("UNTRUSTED_CLIENT_VAR".to_string(), "ignored".to_string()),
            ],
        );
        let env = command
            .get_envs()
            .map(|(key, value)| {
                (
                    key.to_string_lossy().into_owned(),
                    value.map(|value| value.to_string_lossy().into_owned()),
                )
            })
            .collect::<std::collections::HashMap<_, _>>();
        assert_eq!(env["HERDR_PANE_ID"].as_deref(), Some("client-pane"));
        assert_eq!(
            env["FACTR_CLIENT_HERDR_PANE_ID"].as_deref(),
            Some("client-pane")
        );
        assert_eq!(env["TMUX_PANE"], None);
        assert!(!env.contains_key("UNTRUSTED_CLIENT_VAR"));
        assert!(!env.contains_key("FACTR_CLIENT_UNTRUSTED_CLIENT_VAR"));
    }


    #[test]
    #[cfg(unix)]
    fn snapshot_client_terminal_env_captures_set_vars_only() {
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe {
            std::env::set_var("ZELLIJ_SESSION_NAME", "snapshot-test");
            std::env::remove_var("TMUX");
        }
        let snapshot = snapshot_client_terminal_env();
        assert!(
            snapshot
                .iter()
                .any(|(k, v)| k == "ZELLIJ_SESSION_NAME" && v == "snapshot-test")
        );
        assert!(!snapshot.iter().any(|(k, _)| k == "TMUX"));
        unsafe {
            std::env::remove_var("ZELLIJ_SESSION_NAME");
        }
    }


    #[test]
    fn parse_hook_command_splits_words_and_quotes() {
        assert_eq!(
            parse_hook_command("tmux new-window --").unwrap(),
            vec!["tmux", "new-window", "--"]
        );
        assert_eq!(
            parse_hook_command("my-hook --label 'two words'").unwrap(),
            vec!["my-hook", "--label", "two words"]
        );
        assert_eq!(
            parse_hook_command(r#"hook "a \"b\" c""#).unwrap(),
            vec!["hook", r#"a "b" c"#]
        );
    }

    #[test]
    fn parse_hook_command_rejects_bad_input() {
        assert!(parse_hook_command("").is_err());
        assert!(parse_hook_command("   ").is_err());
        assert!(parse_hook_command("hook 'unterminated").is_err());
        assert!(parse_hook_command("hook trailing\\").is_err());
    }
}
