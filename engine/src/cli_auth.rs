//! `factr login|logout|auth`: Factr's credential commands under Factr-I's name. Public mode runs
//! the bundled Factr Python command, so the flows and the one credential store
//! (`$FACTR_CONFIG_HOME/auth.json` + `.env`) are exactly Factr's, shared with the desktop UI.
//! Private deployment refuses every credential change; `auth status` is native and prints names only.

use std::path::{Path, PathBuf};

use factr_gateway::deployment::PRIVATE_MSG;

#[derive(Debug, PartialEq)]
pub enum Plan {
    /// Run `factr <args>`.
    Factr(Vec<String>),
    /// Print which providers are connected (names only).
    Status,
    /// Private deployment: refuse with the one-line message.
    Refuse,
    /// A Factr command that would run a second agent loop, memory, session store or ticker:
    /// refuse with a pointer to the engine/desktop equivalent.
    Deny(String),
}

/// Factr's id for what the user typed (`openai`, `openai-api` and `chatgpt` are the old names).
pub fn canonical(provider: &str) -> &str {
    match provider {
        "openai" | "openai-api" | "chatgpt" | "chatgpt-codex" | "codex" => "openai-codex",
        other => other,
    }
}

/// `args` follow the binary name. `None`: not an auth command.
pub fn plan(args: &[String], private: bool) -> Option<Plan> {
    let rest = args.get(1..).unwrap_or_default();
    let canon = |v: &[String]| -> Vec<String> {
        v.iter().enumerate().map(|(i, a)| if i == 0 && !a.starts_with('-') { canonical(a).to_string() } else { a.clone() }).collect()
    };
    let factr = |v: Vec<String>| Some(if private { Plan::Refuse } else { Plan::Factr(v) });
    match args.first()?.as_str() {
        // `factr login` is deprecated upstream; `factr login` is `auth add`.
        "login" => {
            let mut v = vec!["auth".to_string(), "add".to_string()];
            v.extend(canon(rest));
            factr(v)
        }
        "logout" => {
            let mut v = vec!["logout".to_string()];
            match rest.first() {
                Some(p) if !p.starts_with('-') => v.extend(["--provider".into(), canonical(p).into()]),
                _ => v.extend(rest.iter().cloned()),
            }
            factr(v)
        }
        "auth" => match rest.first().map(String::as_str) {
            None => Some(Plan::Status),
            Some("list" | "status") if rest.len() == 1 => Some(Plan::Status),
            Some(sub @ ("add" | "list" | "remove" | "reset" | "priority" | "refresh" | "status" | "logout")) => {
                if private && !matches!(sub, "list" | "status") {
                    return Some(Plan::Refuse);
                }
                if private {
                    return Some(Plan::Status);
                }
                let mut v = vec!["auth".to_string(), sub.to_string()];
                v.extend(canon(&rest[1..]));
                Some(Plan::Factr(v))
            }
            Some(_) => factr(["auth".to_string()].into_iter().chain(rest.iter().cloned()).collect()),
        },
        _ => None,
    }
}

/// Commands the engine owns; every other first word goes to the bundled Factr CLI.
const ENGINE_COMMANDS: &[&str] = &["serve", "login", "logout", "auth", "__version"];
/// Factr commands that only configure (no agent loop, memory, session store or ticker). Everything
/// else is refused: the engine owns the agent loop, memory, approvals, sessions, cron and learning.
const SETUP_COMMANDS: &[&str] = &[
    "auth", "config", "mcp", "plugins", "profile", "doctor", "backup", "logs", "update", "version", "help", "login",
    "logout", "setup", "model", "secrets", "vault", "claw", "migrate", "import", "import-agent", "pairing",
    "slack", "whatsapp", "whatsapp-cloud", "uninstall", "cron", "kanban",
];

/// Why a Factr command is refused, with the engine/desktop equivalent. `None`: allowed.
fn deny_reason(words: &[String]) -> Option<String> {
    let w = words.first().map(String::as_str)?;
    let sub = words.get(1).map(String::as_str);
    let pointer = match w {
        "chat" => "chat runs in the Factr-I engine; use the desktop app (or `factr serve`)",
        "sessions" => "sessions live in the engine; use the desktop app",
        "memory" => "memory lives in the engine; use the desktop app (`factr memory audit` checks it)",
        "gateway" => "the engine is the only runtime and cron ticker; run `factr serve`",
        "skills" => "skills are managed by the engine; use the desktop app",
        "cron" if matches!(sub, Some("run" | "tick")) => "cron jobs fire from the engine; use the desktop app",
        "kanban" if matches!(sub, Some("dispatch" | "daemon")) => "kanban workers run through the engine; use the desktop app",
        _ if SETUP_COMMANDS.contains(&w) => return None,
        _ => "this command would run a second agent runtime; use the desktop app or `factr serve`",
    };
    Some(format!("factr: `{w}` is not available here: {pointer}."))
}

/// Factr commands that write credentials or provider config: refused in private deployment.
const CREDENTIAL_COMMANDS: &[&str] = &[
    "login", "logout", "setup", "model", "secrets", "vault", "claw", "migrate", "import", "import-agent",
    "pairing", "slack", "whatsapp", "whatsapp-cloud", "uninstall",
];

/// Generic fallback: `factr <factr-subcommand> ...` and `factr factr ...` run the bundled Factr
/// CLI. Engine commands (`serve`, `gateway` with no subcommand, login/logout/auth, `memory audit`,
/// `-h/-V`) are not matched. `args` follow the binary name.
pub fn passthrough_plan(args: &[String], private: bool) -> Option<Plan> {
    // Leading `--profile P` / `--profile=P` belong to Factr too; the first word after them decides.
    let mut i = 0;
    while let Some(a) = args.get(i) {
        match a.as_str() {
            "--profile" => i += 2,
            s if s.starts_with("--profile=") => i += 1,
            _ => break,
        }
    }
    let first = args.get(i)?.as_str();
    if first.starts_with('-') {
        return None;
    }
    let next = args.get(i + 1).map(String::as_str);
    let engine = ENGINE_COMMANDS.contains(&first)
        || (first == "gateway" && next.is_none_or(|n| n.starts_with('-')))
        || (first == "memory" && next == Some("audit"));
    if engine {
        return None;
    }
    let mut v: Vec<String> = args.to_vec();
    if first == "factr" {
        v.remove(i);
        if v.get(i).is_none_or(|a| a.starts_with('-')) {
            // `factr factr` alone (or with only flags): Factr's own help/default.
            return Some(Plan::Factr(v));
        }
    }
    let word = v.get(i).map(String::as_str).unwrap_or_default();
    if let Some(msg) = deny_reason(&v[i..]) {
        return Some(Plan::Deny(msg));
    }
    let writes = CREDENTIAL_COMMANDS.contains(&word)
        || (word == "config" && matches!(v.get(i + 1).map(String::as_str), Some("set" | "edit" | "migrate")));
    Some(if private && writes { Plan::Refuse } else { Plan::Factr(v) })
}

/// Factr subprocess environment: what the engine gives its own Factr backend.
fn factr_command(mut cmd: Vec<String>, h: Vec<String>) -> std::process::Command {
    cmd.extend(h);
    let mut c = std::process::Command::new(&cmd[0]);
    c.args(&cmd[1..]);
    if let Some(home) = factr_home() {
        c.env("FACTR_CONFIG_HOME", home);
    }
    if let Ok(pythonpath) = std::env::var("FACTR_BACKEND_PYTHONPATH") {
        c.env("PYTHONPATH", pythonpath);
    }
    if let Some(tools) = std::env::var_os("FACTR_BACKEND_TOOLS_DIR") {
        let mut paths = vec![PathBuf::from(tools)];
        paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()));
        if let Ok(p) = std::env::join_paths(paths) {
            c.env("PATH", p);
        }
    }
    if let Ok(dir) = crate::storage::factr_dir() {
        c.env("FACTR_HOME", dir);
    }
    c
}

/// These commands run before the entrypoint exports the default `FACTR_CONFIG_HOME`, so they apply it here.
pub fn factr_home() -> Option<PathBuf> {
    factr_app_core::factr_config::home().or_else(|| dirs::home_dir().map(|h| h.join(".factr")))
}

/// Connected providers, names only. Public: `auth.json` entries and `*_API_KEY`/`*_TOKEN` names in
/// `.env`. Private: the same kind of names present in the environment.
pub fn status_lines(home: &Path, private: bool, env: &[(String, String)]) -> Vec<String> {
    let is_key = |k: &str| k.ends_with("_API_KEY") || k.ends_with("_TOKEN") || k.starts_with("AWS_BEARER_TOKEN");
    let mut out = Vec::new();
    if private {
        out.push("deployment: private (credentials come from the environment)".to_string());
        let mut names: Vec<_> = env.iter().filter(|(k, v)| is_key(k) && !v.is_empty()).map(|(k, _)| k.clone()).collect();
        names.sort();
        names.dedup();
        out.extend(names.iter().map(|n| format!("  env: {n}")));
        if names.is_empty() {
            out.push("  no provider key in the environment".into());
        }
        return out;
    }
    let mut providers = std::collections::BTreeSet::new();
    if let Ok(text) = std::fs::read_to_string(home.join("auth.json"))
        && let Ok(v) = serde_json::from_str::<serde_json::Value>(&text)
    {
        for (key, set) in [("credential_pool", true), ("providers", false)] {
            for (name, entry) in v.get(key).and_then(|x| x.as_object()).into_iter().flatten() {
                let live = if set { entry.as_array().is_some_and(|a| !a.is_empty()) } else { entry.as_object().is_some_and(|o| !o.is_empty()) };
                if live {
                    providers.insert(name.clone());
                }
            }
        }
    }
    let mut keys = std::collections::BTreeSet::new();
    for line in std::fs::read_to_string(home.join(".env")).unwrap_or_default().lines() {
        if let Some((k, v)) = line.trim().trim_start_matches("export ").split_once('=')
            && is_key(k.trim())
            && !v.trim().trim_matches(['"', '\'']).is_empty()
        {
            keys.insert(k.trim().to_string());
        }
    }
    out.push(format!("credential store: {}", home.display()));
    out.extend(providers.iter().map(|p| format!("  connected: {p}")));
    out.extend(keys.iter().map(|k| format!("  key set: {k}")));
    if providers.is_empty() && keys.is_empty() {
        out.push("  nothing connected; run `factr login openai-codex` or `factr auth add <provider>`".into());
    }
    out
}

/// Run an auth command; returns the process exit code.
pub fn run(args: &[String]) -> Option<i32> {
    let private = factr_gateway::deployment::is_private();
    let plan = plan(args, private).or_else(|| passthrough_plan(args, private))?;
    Some(match plan {
        Plan::Refuse => {
            eprintln!("{PRIVATE_MSG}");
            2
        }
        Plan::Deny(msg) => {
            eprintln!("{msg}");
            2
        }
        Plan::Status => {
            let env: Vec<_> = std::env::vars().collect();
            for l in status_lines(&factr_home().unwrap_or_default(), private, &env) {
                println!("{l}");
            }
            0
        }
        Plan::Factr(h) => {
            let Some(cmd) = crate::factr_runtime::factr_feature_command() else {
                eprintln!("factr: the bundled Factr backend was not found (set FACTR_BACKEND_CMD)");
                return Some(1);
            };
            let program = cmd[0].clone();
            let mut c = factr_command(cmd, h);
            match c.status() {
                Ok(s) => s.code().unwrap_or(1),
                Err(e) => {
                    eprintln!("factr: cannot run {program}: {e}");
                    1
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn a(s: &str) -> Vec<String> { s.split_whitespace().map(String::from).collect() }

    #[test]
    fn login_maps_to_auth_add_and_aliases() {
        assert_eq!(plan(&a("login openai"), false), Some(Plan::Factr(a("auth add openai-codex"))));
        assert_eq!(plan(&a("login chatgpt --browser"), false), Some(Plan::Factr(a("auth add openai-codex --browser"))));
        assert_eq!(plan(&a("login openrouter"), false), Some(Plan::Factr(a("auth add openrouter"))));
        assert_eq!(plan(&a("logout openai-api"), false), Some(Plan::Factr(a("logout --provider openai-codex"))));
        assert_eq!(plan(&a("auth remove openai 1"), false), Some(Plan::Factr(a("auth remove openai-codex 1"))));
        assert_eq!(plan(&a("auth"), false), Some(Plan::Status));
        assert_eq!(plan(&a("auth status"), false), Some(Plan::Status));
        assert_eq!(plan(&a("auth status openai"), false), Some(Plan::Factr(a("auth status openai-codex"))));
        assert_eq!(plan(&a("serve"), false), None);
    }

    #[test]
    fn passthrough_routes_everything_but_engine_commands() {
        for c in ["cron list", "mcp list", "config show", "doctor", "backup", "logs -f", "plugins list", "profile list", "update", "kanban list", "claw migrate", "--profile work mcp list"] {
            assert_eq!(passthrough_plan(&a(c), false), Some(Plan::Factr(a(c))), "{c}");
        }
        assert_eq!(passthrough_plan(&a("factr mcp list"), false), Some(Plan::Factr(a("mcp list"))));
        for c in ["chat", "chat -q hi", "sessions list", "memory status", "gateway run", "gateway start", "skills list", "cron run x", "cron tick", "kanban dispatch", "kanban daemon", "--profile w chat", "factr chat", "acp", "whatever"] {
            assert!(matches!(passthrough_plan(&a(c), false), Some(Plan::Deny(_))), "{c}");
        }
        assert_eq!(passthrough_plan(&a("factr"), false), Some(Plan::Factr(vec![])));
        for c in ["serve --port 1", "--profile w serve", "gateway", "gateway --port 1", "memory audit --apply", "--version", "-h", "login x", "auth", "__version"] {
            assert_eq!(passthrough_plan(&a(c), false), None, "{c}");
        }
        assert_eq!(passthrough_plan(&[], false), None);
    }

    #[test]
    fn private_passthrough_refuses_credential_writers_only() {
        for c in ["setup", "model", "secrets set", "claw migrate", "config set k v", "factr setup"] {
            assert_eq!(passthrough_plan(&a(c), true), Some(Plan::Refuse), "{c}");
        }
        for c in ["cron list", "config show", "doctor"] {
            assert!(matches!(passthrough_plan(&a(c), true), Some(Plan::Factr(_))), "{c}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn passthrough_runs_resolver_with_env_and_exit_code() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("factr-pt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("fake-factr");
        let out = dir.join("out.txt");
        std::fs::write(&script, format!("#!/bin/sh\necho \"$@|$FACTR_CONFIG_HOME\" > {}\nexit 7\n", out.display())).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut c = factr_command(vec![script.to_string_lossy().into_owned()], a("skills list"));
        c.env("FACTR_CONFIG_HOME", &dir);
        assert_eq!(c.status().unwrap().code(), Some(7));
        let text = std::fs::read_to_string(&out).unwrap();
        assert_eq!(text.trim(), format!("skills list|{}", dir.display()));
    }

    #[test]
    fn private_refuses_changes_and_never_delegates() {
        for c in ["login openai", "logout", "auth add openrouter", "auth remove x 1", "auth refresh openai"] {
            assert_eq!(plan(&a(c), true), Some(Plan::Refuse), "{c}");
        }
        assert_eq!(plan(&a("auth list"), true), Some(Plan::Status));
    }

    #[test]
    fn status_names_only_from_fake_stores() {
        let dir = std::env::temp_dir().join(format!("factr-auth-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("auth.json"), r#"{"credential_pool":{"openai-codex":[{"access_token":"SECRET-AT"}],"anthropic":[]},"providers":{"xai-oauth":{"tokens":{"access_token":"SECRET2"}}}}"#).unwrap();
        std::fs::write(dir.join(".env"), "OPENROUTER_API_KEY=SECRET3\nEMPTY_API_KEY=\n").unwrap();
        let before: Vec<_> = ["auth.json", ".env"].iter().map(|f| std::fs::read(dir.join(f)).unwrap()).collect();
        let text = status_lines(&dir, false, &[]).join("\n");
        assert!(text.contains("connected: openai-codex") && text.contains("connected: xai-oauth") && text.contains("OPENROUTER_API_KEY"));
        assert!(!text.contains("anthropic") && !text.contains("EMPTY_API_KEY") && !text.contains("SECRET"));
        let after: Vec<_> = ["auth.json", ".env"].iter().map(|f| std::fs::read(dir.join(f)).unwrap()).collect();
        assert_eq!(before, after);
        // Private: only the environment, never the store (the store here holds a different key).
        let env = vec![("OPENAI_API_KEY".to_string(), "SECRET4".to_string()), ("HOME".to_string(), "/x".to_string())];
        let text = status_lines(&dir, true, &env).join("\n");
        assert!(text.contains("OPENAI_API_KEY") && !text.contains("openai-codex") && !text.contains("SECRET") && !text.contains("OPENROUTER"));
    }
}
