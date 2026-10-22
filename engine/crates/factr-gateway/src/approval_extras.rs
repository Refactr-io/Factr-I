//! Extra approval questions the user can switch on. They only ever ADD a question: a match turns an
//! "allow" into "ask" and never touches a refusal.
//!
//! * `approvals.always_ask`: the user's own regular expressions. A command matching one is asked about every
//!   time; allowlist and session grants do not apply to it.
//! * `approvals.gui_automation_ask`: commands that drive other apps (AppleScript, `open -a`, xdotool,
//!   cliclick, PowerShell UI automation, ...) and commands that change the agent's own approval settings.
//!
//! Fail-open: nothing configured, or a pattern that does not compile, means the existing behaviour.

use fancy_regex::Regex;

/// Starts the reason of a user-pattern match; the hub does not honour grants for these.
pub(crate) const USER_PREFIX: &str = "matches your always-ask pattern";

/// Where a program may appear: start of a segment (after `;`, `&`, `|`, newline, `(`, backtick, `$(`),
/// optionally behind sudo/env/wrappers and a directory path.
const PREFIX: &str = r"(?:^|[\n;&|`(]|\$\()\s*(?:\w+=\S*\s+)*(?:sudo\s+(?:-[^\s]+\s+(?:[^-\s][^\s]*\s+)?)*)?(?:env\s+(?:-\S+\s+|\w+=\S*\s+)*)?(?:(?:exec|nohup|setsid|time|command|builtin|caffeinate)\s+(?:-\S+\s+)*)*(?:[\w.~-]*[/\\])*";

fn gui_rules() -> Vec<(String, &'static str)> {
    let at_start = |prog: &str| format!(r"{PREFIX}(?:{prog})(?:\.exe)?(?=[\s;&|)]|$)");
    vec![
        (at_start("osascript|automator|shortcuts|xdotool|ydotool|cliclick|wmctrl|nircmd|autohotkey|ahk"), "drives other apps (GUI automation)"),
        (format!(r"{PREFIX}open\s+(?:-[^\s]*\s+)*-[a-z]*[ab]\b"), "opens or switches to another app"),
        (r"\b(?:powershell|pwsh)(?:\.exe)?\b[^\n]*(?:sendkeys|appactivate|system\.windows\.automation|uiautomation|sendinput|mouse_event)".into(), "drives other apps (PowerShell UI automation)"),
        // The agent's own approval settings: a command that names an approvals key and writes.
        (r"\bapprovals\.(?:mode|cron_mode|unattended_mode|command_allowlist|always_ask|gui_automation_ask)\b".into(), "changes the agent's own approval settings"),
        (r"\bfactr\b[^\n]*\bconfig\b[^\n]*\bset\b[^\n]*\b(?:approvals|yolo)\b".into(), "changes the agent's own approval settings"),
        (r"(?:~|\$home|\$\{home\})/\.factr/(?:profiles/[^/\s]+/)?(?:engine/)?(?:config\.(?:yaml|toml)|approvals[\w.-]*)(?=[\s;&|<>]|$)[^\n]*".into(), "touches the agent's own config"),
    ]
}

/// Why `command` deserves an extra question, if it does.
pub(crate) fn extra_ask_reason(command: &str) -> Option<String> {
    let cfg = factr_base::factr_config::current();
    let extras = &cfg.approval_extras;
    if !extras.gui_automation_ask && extras.always_ask.is_empty() {
        return None;
    }
    let variants = [command.to_string(), crate::dangerous::normalize(command)];
    let hit = |re: &Regex| variants.iter().any(|v| re.is_match(v).unwrap_or(false));
    for pattern in &extras.always_ask {
        match Regex::new(&format!("(?is){pattern}")) {
            Ok(re) if hit(&re) => return Some(format!("{USER_PREFIX}: {pattern}")),
            Ok(_) => {}
            Err(error) => eprintln!("[factr] approvals.always_ask: ignoring invalid pattern {pattern:?}: {error}"),
        }
    }
    if extras.gui_automation_ask {
        // Quotes around a program name do not change what runs: also look at the command with them removed.
        let dequoted = command.replace(['"', '\'', '\\'], "");
        for (pattern, description) in gui_rules() {
            if Regex::new(&format!("(?is){pattern}")).is_ok_and(|re| hit(&re) || re.is_match(&dequoted).unwrap_or(false)) {
                return Some(description.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_config<T>(yaml: &str, body: impl FnOnce() -> T) -> T {
        let _env = crate::factr_env::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("approval-extras-{}-{}", std::process::id(), rand::random::<u64>()));
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("config.yaml"), yaml).unwrap();
        let before = std::env::var_os("FACTR_CONFIG_HOME");
        unsafe { std::env::set_var("FACTR_CONFIG_HOME", &dir) };
        let out = body();
        match before {
            Some(v) => unsafe { std::env::set_var("FACTR_CONFIG_HOME", v) },
            None => unsafe { std::env::remove_var("FACTR_CONFIG_HOME") },
        }
        let _ = std::fs::remove_dir_all(&dir);
        out
    }

    const GUI_ON: &str = "approvals: {gui_automation_ask: true}\n";

    #[test]
    fn nothing_configured_adds_no_question() {
        with_config("approvals: {mode: manual}\n", || {
            for cmd in ["osascript -e 'tell application \"Keynote\" to quit'", "open -a Safari", "xdotool key ctrl+q", "cargo publish"] {
                assert_eq!(extra_ask_reason(cmd), None, "{cmd}");
            }
        });
    }

    #[test]
    fn gui_automation_is_asked_about_including_the_usual_dodges() {
        with_config(GUI_ON, || {
            for cmd in [
                "osascript -e 'tell application \"Keynote\" to quit'",
                "/usr/bin/osascript -e 'beep'",
                "\"osascript\" -e 'beep'",
                "osa''script -e beep",
                "A=1 osascript -e beep",
                "env -i A=1 osascript -e beep",
                "sudo -u root -- /usr/bin/osascript x.scpt",
                "echo hi; osascript -e beep",
                "true & osascript -e beep",
                "true && xdotool key a",
                "ls | cliclick c:100,100",
                "open -a Slack",
                "open -na 'Google Chrome' --args x",
                "nohup osascript -e beep",
                "PoWeRsHeLl.EXE -c \"(New-Object -com wscript.shell).SendKeys('x')\"",
                "powershell -Command \"[System.Windows.Automation.AutomationElement]::RootElement\"",
                "sed -i 's/manual/off/' x && factr config set approvals.mode off",
                "yq -i '.approvals.mode = \"off\"' config.yaml",
                "echo 'approvals:\\n  mode: off' >> ~/.factr/config.yaml",
            ] {
                assert!(extra_ask_reason(cmd).is_some(), "{cmd}");
            }
        });
    }

    #[test]
    fn negative_controls_do_not_trip_it() {
        with_config(GUI_ON, || {
            for cmd in [
                "printf '%s' 'osascript -e beep'",
                "echo osascript",
                "ls /usr/bin/osascript-docs",
                "open README.md",
                "open https://example.com",
                "cat notes.txt # osascript",
                "grep -r approvals docs/",
            ] {
                assert_eq!(extra_ask_reason(cmd), None, "{cmd}");
            }
        });
    }

    #[test]
    fn user_patterns_ask_and_a_bad_pattern_is_skipped() {
        with_config("approvals:\n  always_ask: ['(unclosed', '^cargo publish', 'kubectl\\s+delete']\n", || {
            let reason = extra_ask_reason("cargo publish --dry-run").unwrap();
            assert!(reason.starts_with(USER_PREFIX), "{reason}");
            assert!(extra_ask_reason("echo ok; kubectl   delete pod x").is_some());
            assert_eq!(extra_ask_reason("cargo build"), None);
        });
    }
}
