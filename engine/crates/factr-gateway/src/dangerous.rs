//! The ONE list of Factr' dangerous-command patterns (tools/approval_detection.py
//! `DANGEROUS_PATTERNS`), ported to regexes. The approval hook runs it on top of factr's risk
//! classifier: a match upgrades a Safe/Low rating to "ask the person" and never downgrades.
//! The pattern text is Factr' own; descriptions are the reason shown to the person.
//! `fancy_regex` is used because Factr' patterns rely on lookaround.
//!
//! Differences from Factr: its structural shell scanner (`_execution_flag_findings`) is
//! approximated by the two `-c`/`-e` rules at the end, and home-directory folding only covers the
//! `~`, `$HOME` spellings plus the current user's resolved home.

use fancy_regex::Regex;
use std::sync::OnceLock;

const SSH: &str = r"(?:~|\$home|\$\{home\})/\.ssh(?:/|$)";
const FACTR_ENV: &str = r"(?:~/\.factr/(?:profiles/[^/\s]+/)?|(?:\$home|\$\{home\})/\.factr/(?:profiles/[^/\s]+/)?|(?:\$factr_config_home|\$\{factr_config_home\})/)\.env\b";
const FACTR_CONFIG: &str = r"(?:~/\.factr/(?:profiles/[^/\s]+/)?|(?:\$home|\$\{home\})/\.factr/(?:profiles/[^/\s]+/)?|(?:\$factr_config_home|\$\{factr_config_home\})/)config\.yaml\b";
const PROJECT_ENV: &str = r#"(?:(?:/|\.{1,2}/)?(?:[^\s/"'`]+/)*\.env(?:\.[^/\s"'`]+)*)"#;
const PROJECT_CONFIG: &str = r#"(?:(?:/|\.{1,2}/)?(?:[^\s/"'`]+/)*config\.yaml)"#;
const SHELL_RC: &str = r"(?:~|\$home|\$\{home\})/\.(?:bashrc|zshrc|profile|bash_profile|zprofile)\b";
const CREDENTIALS: &str = r"(?:~|\$home|\$\{home\})/\.(?:netrc|pgpass|npmrc|pypirc)\b";
const SYSTEM_CONFIG: &str = r"(?:/etc/|/private/(?:etc|var|tmp|home)/)";
const COMMAND_TAIL: &str = r"(?:\s*(?:&&|\|\||;).*)?$";
const WRITE_BOUNDARY: &str = r#"(?=[\s;&|<>"']|$)"#;
const CMDPOS: &str = r"(?:^|[\n`]|\$\()\s*(?:sudo\s+(?:-[^\s]+\s+)*)?(?:env\s+(?:\w+=\S*\s+)*)?(?:(?:exec|nohup|setsid|time)\s+)*\s*";
const SHELLS: &str = "bash|sh|zsh|ksh|dash";
const PKG_OPTS: &str = r"(?:-[^\s]+(?:\s+[^-\s][^\s]*)?\s+)*";

fn patterns() -> Vec<(String, &'static str)> {
    let sensitive = format!("(?:{SSH}|{SHELL_RC}|{CREDENTIALS})");
    let user_sensitive = sensitive.clone();
    let project_sensitive = format!("(?:{PROJECT_ENV}|{PROJECT_CONFIG})");
    let mut p: Vec<(String, &'static str)> = Vec::new();
    let mut add = |re: String, d: &'static str| p.push((re, d));
    add(r"\brm\s+(-[^\s]*\s+)*/".into(), "delete in root path");
    add(r"\brm\s+-[^\s]*r".into(), "recursive delete");
    add(r"\brm\s+--recursive\b".into(), "recursive delete (long flag)");
    add(r#"\brm\s+(?!--(?:\s|$))(?:(?!\s--(?:\s|$))[^\n"';|&])*\s(?:-[a-z]*r[a-z]*\b|--recursive\b)"#.into(), "recursive delete (flags after operands)");
    add(r"\bcmd(?:\.exe)?\s+/(?:c|k)\s+.*\b(?:del|erase|rd|rmdir)\b".into(), "Windows cmd destructive delete");
    add(r#"\b(?:powershell|pwsh)(?:\.exe)?\b(?:\s+-\S+)*\s+(?:-(?:command|c)\s+)?["']?(?:remove-item|rmdir|erase|del|rd|ri|rm)\b"#.into(), "Windows PowerShell destructive delete");
    add(r"\b(?:powershell|pwsh)(?:\.exe)?\b.*\s-(?:encodedcommand|enc|e)\b".into(), "PowerShell encoded command execution");
    add(r"\bremove-item\b[^\n;|&]*\s-(?:recurse|force)\b".into(), "PowerShell destructive delete (Remove-Item)");
    add(r"\b(?:del|erase|rd|rmdir)\s+(?:/[a-z]\s+)*/[sq]\b".into(), "Windows destructive delete (recursive/quiet switch)");
    add(r"\b(?:iwr|invoke-webrequest|invoke-restmethod|irm|curl|wget)\b[^\n]*\|\s*(?:iex|invoke-expression)\b".into(), "pipe remote content to PowerShell (iwr | iex)");
    add(r"\b(?:iex|invoke-expression)\s*\(\s*(?:iwr|invoke-webrequest|invoke-restmethod|irm)\b".into(), "execute remote content via Invoke-Expression");
    add(r"\btaskkill\b[^\n]*\s/f\b".into(), "force kill processes (taskkill /F)");
    add(r"\bstop-process\b[^\n]*\s-force\b".into(), "force kill processes (Stop-Process -Force)");
    add(r"\bformat-volume\b".into(), "format filesystem (Format-Volume)");
    add(r"\bclear-disk\b".into(), "wipe disk (Clear-Disk)");
    add(r"\bdiskpart\b".into(), "disk partitioning (diskpart)");
    add(r"\bformat(?:\.com)?\s+[a-z]:".into(), "format drive (format.com)");
    add(r"\bcipher\s+/w\b".into(), "wipe free space (cipher /w)");
    add(r"\bicacls\b[^\n]*\s/grant\b[^\n]*\b(?:everyone|todos|jeder|tout\s+le\s+monde|\*s-1-1-0)\b".into(), "grant Everyone access (icacls)");
    add(r"\bicacls\b[^\n]*\s/reset\b".into(), "reset ACLs recursively (icacls /reset)");
    add(r"\bvssadmin\b[^\n]*\bdelete\s+shadows\b".into(), "delete volume shadow copies (vssadmin)");
    add(r"\bwbadmin\b[^\n]*\bdelete\b".into(), "delete backups (wbadmin)");
    add(r"\bbcdedit\b[^\n]*\s/set\b".into(), "modify boot configuration (bcdedit /set)");
    add(r"\breg(?:\.exe)?\s+delete\b".into(), "registry delete (reg delete)");
    add(r"\bremove-itemproperty\b[^\n]*\s-force\b".into(), "registry value delete (Remove-ItemProperty -Force)");
    add(r"\bstop-service\b[^\n]*\s-force\b".into(), "force stop service (Stop-Service -Force)");
    add(r"\bsc(?:\.exe)?\s+(?:stop|delete)\b".into(), "stop/delete service (sc)");
    add(r"\busers[\\/][^\\/\s]+[\\/]\.ssh\b".into(), "access to SSH keys (Windows path)");
    add(r"\bappdata[\\/](?:local|roaming)[\\/]factr[^\n]*\.env\b".into(), "access to Factr secrets (Windows path)");
    add(r"\bchmod\s+(-[^\s]*\s+)*(777|666|o\+[rwx]*w|a\+[rwx]*w)\b".into(), "world/other-writable permissions");
    add(r"\bchmod\s+--recursive\b.*(777|666|o\+[rwx]*w|a\+[rwx]*w)".into(), "recursive world/other-writable (long flag)");
    add(r"\bchown\s+(-[^\s]*)?R\s+root".into(), "recursive chown to root");
    add(r"\bchown\s+--recur[a-z]*\b.*root".into(), "recursive chown to root (long flag)");
    add(format!(r"{CMDPOS}mkfs\b"), "format filesystem");
    add(format!(r"{CMDPOS}dd\s+.*if="), "disk copy");
    add(r">\s*/dev/sd".into(), "write to block device");
    add(r"\bDROP\s+(TABLE|DATABASE)\b".into(), "SQL DROP");
    add(r"\bDELETE\s+FROM\b(?![^\n]*\bWHERE\b)".into(), "SQL DELETE without WHERE");
    add(r"\bTRUNCATE\s+(TABLE)?\s*\w".into(), "SQL TRUNCATE");
    add(format!(r">\s*{SYSTEM_CONFIG}"), "overwrite system config");
    add(r"\bsystemctl\s+(-[^\s]+\s+)*(stop|restart|disable|mask)\b".into(), "stop/restart system service");
    add(r"\bkill\s+-9\s+-1\b".into(), "kill all processes");
    add(r"\bpkill\s+-9\b".into(), "force kill processes");
    add(r"\bkillall\s+(-[^\s]*\s+)*-(9|KILL|SIGKILL)\b".into(), "force kill processes (killall -KILL)");
    add(r"\bkillall\s+(-[^\s]*\s+)*-s\s+(KILL|SIGKILL|9)\b".into(), "force kill processes (killall -s KILL)");
    add(r"\bkillall\s+(-[^\s]*\s+)*-r\b".into(), "kill processes by regex (killall -r)");
    add(r":\(\)\s*\{\s*:\s*\|\s*:\s*&\s*\}\s*;\s*:".into(), "fork bomb");
    add(format!(r"\b(curl|wget)\b.*\|\s*(?:[/\w]*/)?(?:{SHELLS})(?:\s|$|-c)"), "pipe remote content to shell");
    add(format!(r"\b(?:{SHELLS})\s+<\s*<?\s*\(\s*(curl|wget)\b"), "execute remote script via process substitution");
    add(r"(?:\beval\b|\bsource\b|\.)\s*(?:\$\(\s*|`\s*)(?:curl|wget)\b".into(), "execute remote content via command substitution");
    add(r"(?<![\d.])(?:169\.254\.169\.254|100\.100\.100\.200)(?![\d.])|(?<![\w.-])metadata\.google\.internal(?![\w.-])|fd00:ec2::254".into(), "cloud metadata endpoint access (instance credentials)");
    add(format!(r"\b(base64|base32|base16)\s+(?:-[dD]|--decode)\b.*\|\s*\b(?:{SHELLS})\b"), "pipe decoded content to shell (possible command obfuscation)");
    add(format!(r"\bxxd\s+-r\b.*\|\s*\b(?:{SHELLS})\b"), "pipe xxd-decoded content to shell (possible command obfuscation)");
    add(format!(r"\becho\b[^|]*\|\s*\btr\b[^|]*\|\s*\b(?:{SHELLS})\b"), "pipe tr-transformed output to shell (possible command obfuscation)");
    add(format!(r"\bopenssl\b.*\b(?:base64|enc)\b[^|]*\s+-[dD]\b[^|]*\|\s*\b(?:{SHELLS})\b"), "pipe openssl-decoded content to shell (possible command obfuscation)");
    add(format!(r#"\btee\b.*["']?{sensitive}"#), "overwrite system file via tee");
    add(format!(r#">>?\s*["']?{sensitive}"#), "overwrite system file via redirection");
    add(format!(r#"\btee\b.*["']?{project_sensitive}["']?{WRITE_BOUNDARY}"#), "overwrite project env/config via tee");
    add(format!(r#">>?\s*["']?{project_sensitive}["']?{WRITE_BOUNDARY}"#), "overwrite project env/config via redirection");
    add(r"\bxargs\s+.*\brm\b".into(), "xargs with rm");
    add(r"\bfind\b.*-exec(?:dir)?\s+(/\S*/)?rm\b".into(), "find -exec/-execdir rm");
    add(format!(r"{CMDPOS}find\s[^;|&\n]*(?<!\S)-(?:\{{[^}}\s]*(?:delete|exec(?:dir)?)[^}}\s]*\}}|(?:del(?:ete?)?|exec(?:dir)?)[*?\[])"), "find dynamic shell word may expand to destructive flag");
    add(r"\bfind\b.*-delete\b".into(), "find -delete");
    add(r"\b(?:rg|sort|ag|man)\b[^;|&\n]*(?<!\S)--(?:pre|hostname-bin|compress-program|pager|html)(?:\{|[*?\[])".into(), "dynamic shell word may expand to arbitrary program execution flag");
    add(r"\bfactr\s+(?:-{1,2}\S+(?:\s+\S+)?\s+)*gateway\s+(stop|restart)\b".into(), "stop/restart factr gateway (kills running agents)");
    add(r"\bfactr\s+update\b".into(), "factr update (restarts gateway, kills running agents)");
    add(r"\bdocker\s+(?:-{1,2}\S+(?:[=\s]\S+)?\s+)*(?:-h|--host)[=\s]+\S+".into(), "docker with remote daemon redirect (-H/--host)");
    add(r"\bdocker\s+(?:-{1,2}\S+(?:[=\s]\S+)?\s+)*(?:-c|--context)[=\s]+\S+".into(), "docker with daemon redirect (--context: alternate daemon)");
    add(r"\bdocker\s+context\s+use\b".into(), "docker context use (switches default daemon for future commands)");
    add(r"\bpodman\s+(?:-{1,2}\S+(?:[=\s]\S+)?\s+)*(?:--url|--connection|--identity)[=\s]+\S+".into(), "podman with remote daemon redirect (--url/--connection/--identity)");
    add(r"\bpodman\s+(?:-{1,2}\S+(?:[=\s]\S+)?\s+)*(?:-r\b|--remote\b)".into(), "podman remote mode (-r/--remote: remote daemon)");
    add(r"\b(?:docker_host|docker_context|container_host|container_connection)=\S+".into(), "docker/podman daemon redirect via environment (DOCKER_HOST/CONTAINER_HOST)");
    add(r"\bdocker(?:-compose|\s+compose)\s+(?:-{1,2}\S+(?:[=\s]\S+)?\s+)*(restart|stop|kill|down)\b".into(), "docker compose restart/stop/kill/down (container lifecycle)");
    add(r"\bdocker\s+(?:-{1,2}\S+(?:[=\s]\S+)?\s+)*(restart|stop|kill)\b".into(), "docker restart/stop/kill (container lifecycle)");
    add(r"gateway\s+run\b.*(&\s*$|&\s*;|\bdisown\b|\bsetsid\b)".into(), "start gateway outside systemd");
    add(r"\bnohup\b.*gateway\s+run\b".into(), "start gateway outside systemd");
    add(r"\b(pkill|killall)\b.*\b(factr|gateway|cli\.py)\b".into(), "kill factr/gateway process (self-termination)");
    add(r"\bkill\b.*\$\(\s*(pgrep|pidof)\b".into(), "kill process via pgrep/pidof expansion (self-termination)");
    add(r"\bkill\b.*`\s*(pgrep|pidof)\b".into(), "kill process via backtick pgrep/pidof expansion (self-termination)");
    add(r"\A(?=[\s\S]*\blaunchctl\s+(?:stop|kickstart|bootout|unload|kill|disable|remove)\b)(?=[\s\S]*\b(?:factr|ai\.factr)\b)".into(), "stop/restart factr launchd service (kills running agents)");
    add(format!(r"\b(cp|mv|install)\b.*\s{SYSTEM_CONFIG}"), "copy/move file into system config path");
    add(format!(r#"\b(cp|mv|install)\b.*\s["']?{project_sensitive}["']?{COMMAND_TAIL}"#), "overwrite project env/config file");
    add(format!(r#"\b(cp|mv|install)\b.*\s["']?{sensitive}[^\s"']*["']?{COMMAND_TAIL}"#), "copy/move file into sensitive credential/SSH/shell-rc path");
    add(format!(r#"\bsed\s+-[^\s]*i.*(?:{user_sensitive})[^\s"']*"#), "in-place edit of sensitive credential/SSH/shell-rc path");
    add(format!(r#"\bsed\s+--in-place\b.*(?:{user_sensitive})[^\s"']*"#), "in-place edit of sensitive credential/SSH/shell-rc path (long flag)");
    add(format!(r#"\b(?:perl|ruby)\b.*(?:^|\s)-[^\s]*i\b.*(?:{user_sensitive})[^\s"']*"#), "in-place edit of sensitive credential/SSH/shell-rc path (perl/ruby)");
    add(format!(r"\bsed\s+-[^\s]*i.*\s{SYSTEM_CONFIG}"), "in-place edit of system config");
    add(format!(r"\bsed\s+--in-place\b.*\s{SYSTEM_CONFIG}"), "in-place edit of system config (long flag)");
    add(format!(r"\bsed\s+-[^\s]*i.*(?:{FACTR_CONFIG}|{FACTR_ENV})"), "in-place edit of Factr config/env");
    add(format!(r"\bsed\s+--in-place\b.*(?:{FACTR_CONFIG}|{FACTR_ENV})"), "in-place edit of Factr config/env (long flag)");
    add(format!(r"\b(?:perl|ruby)\b.*(?:^|\s)-[^\s]*i\b.*(?:{FACTR_CONFIG}|{FACTR_ENV})"), "in-place edit of Factr config/env (perl/ruby)");
    add(format!(r"\b(?:{SHELLS})\s+<<"), "shell execution via heredoc");
    add(r"\bgit\s+reset\s+--h(?:a(?:r(?:d)?)?)?\b".into(), "git reset --hard (destroys uncommitted changes)");
    add(r"\bgit\s+push\b.*--forc[a-z]*\b".into(), "git force push (rewrites remote history)");
    add(r"\bgit\s+push\b.*-f\b".into(), "git force push short flag (rewrites remote history)");
    add(r"\bgit\s+clean\s+-[^\s]*f".into(), "git clean with force (deletes untracked files)");
    add(r"\bgit\s+branch\s+(?-i:-D)\b".into(), "git branch force delete");
    add(r"\bgit\s+branch\b[^;|&\n]*?(?:-d\b|--delete\b)[^;|&\n]*?(?:-f\b|--force\b)".into(), "git branch force delete (long flags)");
    add(r"\bgit\s+branch\b[^;|&\n]*?(?:-f\b|--force\b)[^;|&\n]*?(?:-d\b|--delete\b)".into(), "git branch force delete (long flags, force-first)");
    add(r"\bchmod\s+\+x\b.*[;&|]+\s*\./".into(), "chmod +x followed by immediate execution");
    add(r"\bsudo\b[^;|&\n]*?\s+(?:-s\b|--st[a-z]*\b|-a\b|--a[a-z]*\b)".into(), "sudo with privilege flag (stdin/askpass/shell/list)");
    add(r"\bsudo\b[^;|&\n]*?\s+-[a-z]*[sa][a-z]*\b".into(), "sudo with combined-flag privilege escalation");
    add(format!(r"{CMDPOS}npm\s+{PKG_OPTS}(?:uninstall|unlink|remove|rm|r|un)\b"), "package manager uninstall");
    add(format!(r"{CMDPOS}pnpm\s+{PKG_OPTS}(?:uninstall|remove|rm|un)\b"), "package manager uninstall");
    add(format!(r"{CMDPOS}yarn\s+{PKG_OPTS}(?:global\s+)?(?:uninstall|remove)\b"), "package manager uninstall");
    add(format!(r"{CMDPOS}pip(?:3)?\s+{PKG_OPTS}uninstall\b"), "package manager uninstall");
    add(format!(r"{CMDPOS}brew\s+{PKG_OPTS}(?:uninstall|remove|rm)\b"), "package manager uninstall");
    // Approximation of Factr' structural scan: an interpreter or shell handed code on the command line.
    add(format!(r"(?:^|[\n;&|`(]|\$\()\s*(?:sudo\s+(?:-\S+\s+)*)?(?:env\s+(?:\w+=\S*\s+)*)?(?:{SHELLS})\s+(?:-[^\s]*\s+)*-[a-z]*c\b"), "shell command via -c/-lc flag");
    add(r"(?:^|[\n;&|`(]|\$\()\s*(?:sudo\s+(?:-\S+\s+)*)?(?:env\s+(?:\w+=\S*\s+)*)?(?:[\w./-]*/)?(?:python[\d.]*|perl|ruby|node|nodejs|php|lua|deno)\s+(?:-[^\s]*\s+)*-(?:c|e|r|-eval|-print)\b".into(), "script execution via -e/-c flag");
    p
}

fn compiled() -> &'static Vec<(Regex, &'static str)> {
    static LIST: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
    LIST.get_or_init(|| {
        patterns()
            .into_iter()
            .map(|(re, d)| (Regex::new(&format!("(?is){re}")).unwrap_or_else(|e| panic!("bad pattern for {d}: {e}")), d))
            .collect()
    })
}

/// Fold the absolute spelling of `dir` (either separator, any mix) followed by a path tail into
/// `replacement` + the tail with `/` separators, like Factr's `_fold_home_prefixes`. A bare `dir`
/// (no tail) and degenerate dirs (fewer than two components: `/`, `C:\\`) are left alone.
fn fold_home(command: &str, dir: Option<&std::path::Path>, replacement: &str) -> String {
    let Some(dir) = dir else { return command.to_string() };
    let dir = dir.to_string_lossy();
    let parts: Vec<&str> = dir.split(['/', '\\']).filter(|c| !c.is_empty()).collect();
    if parts.len() < 2 {
        return command.to_string();
    }
    let joined = parts.iter().map(|c| fancy_regex::escape(c).into_owned()).collect::<Vec<_>>().join(r"[/\\]+");
    let Ok(re) = Regex::new(&format!(r#"[/\\]*{joined}(?P<tail>(?:[/\\][^/\\\s'"`;|&<>()]*)+)"#)) else { return command.to_string() };
    re.replace_all(command, |caps: &fancy_regex::Captures| format!("{replacement}{}", caps["tail"].replace('\\', "/"))).into_owned()
}

/// Factr' `_normalize_command_for_detection`, so escapes and splicing cannot dodge a pattern.
fn normalize(command: &str) -> String {
    let mut s = command.replace('\0', "").replace("\\\r\n", "").replace("\\\n", "");
    // The Factr config home first (it usually lies inside the user home), then the user home.
    s = fold_home(&s, factr_base::factr_config::home().as_deref(), "~/.factr");
    s = fold_home(&s, factr_base::platform::user_home_dir().as_deref(), "~");
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.peek() {
                Some('\n') | None => out.push(c),
                Some(_) => out.push(chars.next().unwrap()),
            }
        } else {
            out.push(c);
        }
    }
    let out = out.replace("''", "").replace("\"\"", "");
    let ifs = Regex::new(r"\$\{IFS\b[^}]*\}|\$IFS\b").expect("static pattern");
    ifs.replace_all(&out, " ").into_owned()
}

/// The first Factr dangerous-command description that matches, if any.
pub fn dangerous_reason(command: &str) -> Option<&'static str> {
    // Raw too: normalizing strips backslashes, which dissolves the Windows path forms.
    let variants = [command.to_string(), normalize(command)];
    compiled().iter().find(|(re, _)| variants.iter().any(|v| re.is_match(v).unwrap_or(true))).map(|(_, d)| *d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_config_home_variable_spellings_trip_the_env_and_config_rules() {
        for cmd in [
            "sed -i 's/a/b/' $FACTR_CONFIG_HOME/.env",
            "echo x >> ${FACTR_CONFIG_HOME}/config.yaml",
            "tee $factr_config_home/.env",
            "echo x > ~/.factr/profiles/work/config.yaml",
        ] {
            assert!(dangerous_reason(cmd).is_some(), "{cmd}");
        }
        assert!(dangerous_reason("cat $FACTR_CONFIG_HOME/notes.txt").is_none());
    }

    #[test]
    fn an_absolute_config_home_folds_to_the_tilde_factr_form() {
        let home = std::path::Path::new("/srv/data/factr");
        let fold = |c: &str| fold_home(c, Some(home), "~/.factr");
        assert_eq!(fold("tee /srv/data/factr/.env"), "tee ~/.factr/.env");
        assert_eq!(fold("tee /srv//data/factr/profiles/a/config.yaml"), "tee ~/.factr/profiles/a/config.yaml");
        assert_eq!(fold("ls /srv/data/factr"), "ls /srv/data/factr", "a bare dir is not folded");
        assert_eq!(fold("ls /srv/data/factr2/x"), "ls /srv/data/factr2/x");
        // The Windows spelling folds too, backslashes in the tail included.
        let win = std::path::Path::new(r"C:\Users\me\.factr");
        assert_eq!(fold_home(r"type C:\Users\me\.factr\config.yaml", Some(win), "~/.factr"), "type ~/.factr/config.yaml");
        assert_eq!(fold_home("x /", Some(std::path::Path::new("/")), "~"), "x /");
    }

    #[test]
    fn the_resolved_config_home_path_is_caught_through_normalize() {
        let _g = crate::factr_env::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("danger-home-{}", std::process::id()));
        let before = std::env::var_os("FACTR_CONFIG_HOME");
        // SAFETY: env is only touched under ENV_LOCK.
        unsafe { std::env::set_var("FACTR_CONFIG_HOME", &dir) };
        let cmd = format!("echo k=v >> {}/.env", dir.display());
        let reason = dangerous_reason(&cmd);
        match before { Some(v) => unsafe { std::env::set_var("FACTR_CONFIG_HOME", v) }, None => unsafe { std::env::remove_var("FACTR_CONFIG_HOME") } }
        assert!(reason.is_some(), "{cmd}");
    }
}
