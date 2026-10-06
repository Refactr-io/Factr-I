//! The destructive-command gate for the `bash` tool (issue #604).
//!
//! Kept in its own file so the policy seam is easy to find and review: this is
//! the only thing standing between a model's `rm -rf` and the user's data.

/// What the destructive-command gate makes of a shell command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BashVerdict {
    /// Plainly safe: runs without a prompt.
    Run,
    /// Destructive but allowed to run (bounded, or a justified `Confirm`): a person should decide.
    /// Carries the reason to show them.
    Ask(String),
    /// Refused as issued: a reflection prompt or an outright deny. Carries the text for the model.
    Refuse(String),
}

/// The ONE deterministic destructive-command gate: the `bash` tool enforces it and the gateway's
/// approval hook reads the same verdict, so the two cannot disagree.
///
/// Stage 1 is a pure blast-radius assessment; stage 2 turns a `Confirm` verdict
/// into a reflection prompt that a blind retry cannot satisfy. Catastrophic
/// targets (`/`, `$HOME`, credential stores, device nodes) are denied outright.
/// See issue #604.
pub fn bash_verdict(
    command: &str,
    justification: Option<&str>,
    working_dir: Option<std::path::PathBuf>,
) -> BashVerdict {
    let mut risk_ctx = factr_command_risk::RiskContext::from_env(working_dir);
    // Assess the same scratch path that the child shell actually receives.
    #[cfg(not(windows))]
    {
        risk_ctx.scratch_dir = super::tool_scratch_dir();
    }
    let assessment = factr_command_risk::assess(command, &risk_ctx);
    let reason = || {
        assessment
            .findings
            .first()
            .map(|f| f.reason.clone())
            .unwrap_or_else(|| "potentially destructive command".into())
    };
    match assessment.level {
        factr_command_risk::RiskLevel::Safe => return BashVerdict::Run,
        factr_command_risk::RiskLevel::Low => return BashVerdict::Ask(reason()),
        _ => {}
    }

    let justification = factr_command_risk::Justification {
        text: justification.map(str::to_string),
    };
    match factr_command_risk::gate(&assessment, &justification) {
        factr_command_risk::GateOutcome::Allow => BashVerdict::Ask(reason()),
        factr_command_risk::GateOutcome::Deny { reason } => {
            crate::logging::warn(&format!("[bash] denied destructive command: {command}"));
            BashVerdict::Refuse(reason)
        }
        factr_command_risk::GateOutcome::Reflect { prompt } => {
            crate::logging::info(&format!(
                "[bash] destructive command held for justification: {command}"
            ));
            BashVerdict::Refuse(prompt)
        }
    }
}

/// The refusal text when the command must not run as-issued.
pub(super) fn destructive_command_refusal(
    command: &str,
    justification: Option<&str>,
    working_dir: Option<std::path::PathBuf>,
) -> Option<String> {
    match bash_verdict(command, justification, working_dir) {
        BashVerdict::Refuse(text) => Some(text),
        _ => None,
    }
}

/// The `bash` tool's JSON schema, including the `justification` field the
/// destructive-command gate consumes.
///
/// Lives beside the gate so the schema and the policy that reads it stay in
/// sync, and so bash.rs stays inside the code-size budget.
/// `FACTR_GUARD_BG_DESC=1` selects the newer `run_in_background` wording and adds the server hint
/// to the background-start result (off by default: the v0.0.1 text). Read once per process (tool schemas are frozen per session).
pub(super) fn bg_desc_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| factr_base::prompt::opt_in_switch("BG_DESC"))
}

pub(super) fn bg_start_hint(on: bool) -> &'static str {
    if on { "For a server or watcher: `bg` wait with until=<ready regex> returns when the ready line prints.\n" } else { "" }
}

fn run_in_background_description(new_wording: bool) -> &'static str {
    if new_wording {
        "Long builds/servers: run here, then `bg` wait until=<ready line>. May print `FACTR_PROGRESS {json}`."
    } else {
        "Run in background. Emit `FACTR_PROGRESS {json}` lines for progress reporting."
    }
}

pub(super) fn bash_parameters_schema() -> serde_json::Value {
    let cmd_desc = if cfg!(windows) {
        "The Windows command to execute via cmd.exe. Use cmd.exe syntax and quoting, not Bash syntax."
    } else {
        "The bash command to execute. Put large temp files under `$FACTR_SCRATCH_DIR`, not `/tmp`."
    };
    serde_json::json!({
        "type": "object",
        "required": ["command"],
        "properties": {
            "intent": crate::tool::intent_schema_property(),
            "command": {
                "type": "string",
                "minLength": 1,
                "description": cmd_desc
            },
            "timeout": {
                "type": "integer",
                "description": "MILLISECONDS (default 120000, max 600000). On expiry it keeps running as a background task."
            },
            "run_in_background": {
                "type": "boolean",
                "description": run_in_background_description(bg_desc_on())
            },
            "notify": {
                "type": "boolean",
                "description": "Notify on completion."
            },
            "wake": {
                "type": "boolean",
                "description": "Wake on completion."
            },
            "stall_wake_seconds": {
                "type": "integer",
                "description": "With run_in_background: wake after N seconds without output/progress (min 30). For jobs that may hang."
            },
            "justification": {
                "type": "string",
                "description": "Only when re-issuing a command the destructive gate refused; explain which user request it serves."
            }
        }
    })
}

#[cfg(test)]
#[test]
fn timeout_schema_describes_background_promotion() {
    let d = bash_parameters_schema()["properties"]["timeout"]["description"].as_str().unwrap().to_string();
    assert!(d.contains("background") && !d.contains("124") && !d.contains("no timeout"), "{d}");
}

#[cfg(all(test, not(windows)))]
mod tests {
    use super::destructive_command_refusal;

    #[test]
    fn scratch_log_and_backup_commands_do_not_require_justification() {
        let cwd = std::env::current_dir().ok();
        for command in [
            "cargo test --lib > \"$FACTR_SCRATCH_DIR/tests.log\" 2>&1",
            "git diff > \"${FACTR_SCRATCH_DIR}/before.patch\"",
            "env | grep FACTR",
            "command -v sudo && sudo -n true",
            "find /sys -type l -exec readlink {} \\;",
            "find /etc -type f -exec sed -n '1,10p' {} \\;",
        ] {
            assert!(
                destructive_command_refusal(command, None, cwd.clone()).is_none(),
                "{command}"
            );
        }
    }

    #[test]
    fn protected_writes_and_unknown_variables_remain_blocked() {
        for command in [
            "rm -rf /etc",
            "echo bad > /etc/passwd",
            "find /etc -type f -exec rm {} \\;",
            "echo test > \"$UNKNOWN/tests.log\"",
        ] {
            assert!(
                destructive_command_refusal(command, None, std::env::current_dir().ok()).is_some(),
                "{command}"
            );
        }
    }
}

#[cfg(test)]
mod bg_desc_tests {
    #[test]
    fn the_earlier_text_is_the_default_and_the_new_text_opt_in() {
        assert_eq!(
            super::run_in_background_description(false),
            "Run in background. Emit `FACTR_PROGRESS {json}` lines for progress reporting."
        );
        assert!(super::run_in_background_description(true).contains("until=<ready line>"));
        assert!(super::bg_start_hint(true).contains("until=<ready regex>") && super::bg_start_hint(false).is_empty());
        assert!(!super::bg_desc_on(), "opt-in: off by default");
    }
}
