//! OpenCode on this machine: stat-only detection, a cached keyless Zen model list, and the one-click
//! "use my OpenCode sign-in" consent. The key itself is never read here: the engine reads OpenCode's
//! `auth.json` in place (`factr_base::auth::external`) once the user has trusted that source.
//!
//! Inference needs a key (the free tier answers `FreeTierError` outside OpenCode), but `/models` is public.

use factr_base::auth::external::{ExternalAuthSource, load_api_key_for_env, preferred_unconsented_api_key_source_for_env, trust_external_auth_source};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

/// The variable the engine reads a Zen key from (its `.env` in the config home, then the process).
const KEY_ENV: &str = "OPENCODE_API_KEY";
const ZEN_BASE: &str = "https://opencode.ai/zen/v1";
const CACHE_TTL: Duration = Duration::from_secs(6 * 3600);
/// Ids the relay still lists but no longer serves (same set the Python catalog drops).
const DELISTED: [&str; 3] = ["ox-alpha-free", "deepseek-v4-flash-free", "x-preview-f-free"];
static REFRESHING: AtomicBool = AtomicBool::new(false);

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").filter(|h| !h.is_empty()).map(PathBuf::from)
}

fn on_path(name: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(name).is_file()))
}

/// Installed means: the binary on PATH or in the usual install spots, the macOS app, or its config/data dir.
pub(super) fn installed() -> bool {
    if matches!(std::env::var("FACTR_OPENCODE_DETECT").as_deref(), Ok("0" | "off" | "false")) {
        return false;
    }
    let home = home();
    let under = |rel: &str| home.as_ref().map(|h| h.join(rel));
    let spots = [
        under(".opencode/bin/opencode"),
        under(".npm-global/bin/opencode"),
        under(".bun/bin/opencode"),
        Some("/opt/homebrew/bin/opencode".into()),
        Some("/usr/local/bin/opencode".into()),
        Some("/home/linuxbrew/.linuxbrew/bin/opencode".into()),
    ];
    let dirs = [
        Some(PathBuf::from("/Applications/OpenCode.app")),
        under("Applications/OpenCode.app"),
        under(".config/opencode"),
        under(".local/share/opencode"),
    ];
    on_path("opencode")
        || on_path("opencode.exe")
        || spots.iter().flatten().any(|p| p.is_file())
        || dirs.iter().flatten().any(|p| p.is_dir())
}

fn cache_file() -> Option<PathBuf> {
    factr_base::factr_config::home().map(|h| h.join("cache").join("opencode-zen-public-models.json"))
}

/// Ids from an OpenAI-style `{"data":[{"id":..}]}` body: deduped, order kept, delisted ids dropped, junk ignored.
pub(super) fn parse_models(body: &Value) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for row in body["data"].as_array().into_iter().flatten() {
        let id = row["id"].as_str().map(str::trim).unwrap_or_default();
        if !id.is_empty() && !out.iter().any(|m| m == id) && !DELISTED.contains(&id.to_ascii_lowercase().as_str()) {
            out.push(id.to_string());
        }
    }
    out
}

fn refresh_cache(path: PathBuf) {
    let base = std::env::var("OPENCODE_ZEN_BASE_URL").ok().filter(|b| !b.is_empty()).unwrap_or_else(|| ZEN_BASE.into());
    let url = format!("{}/models", base.trim_end_matches('/'));
    let fetched = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(6))
        .build()
        .and_then(|c| c.get(url).header("Accept", "application/json").header("User-Agent", "Factr-I").send())
        .and_then(|r| r.error_for_status())
        .and_then(|r| r.json::<Value>());
    if let Ok(body) = fetched {
        let ids = parse_models(&body);
        if !ids.is_empty() {
            let at = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs());
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(&path, json!({ "at": at, "ids": ids }).to_string());
        }
    }
    REFRESHING.store(false, Ordering::SeqCst);
}

/// The cached public Zen list. Never blocks: a missing or stale cache warms on a background thread.
pub(super) fn public_models() -> Vec<String> {
    let Some(path) = cache_file() else { return Vec::new() };
    let cached = std::fs::read_to_string(&path).ok().and_then(|raw| serde_json::from_str::<Value>(&raw).ok());
    let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let fresh = cached.as_ref().and_then(|c| c["at"].as_u64()).is_some_and(|at| now.saturating_sub(at) < CACHE_TTL.as_secs());
    if !fresh && !REFRESHING.swap(true, Ordering::SeqCst) {
        let target: PathBuf = path.clone();
        if std::thread::Builder::new().name("opencode-models".into()).spawn(move || refresh_cache(target)).is_err() {
            REFRESHING.store(false, Ordering::SeqCst);
        }
    }
    cached.map(|c| c["ids"].as_array().into_iter().flatten().filter_map(|v| v.as_str().map(str::to_owned)).collect()).unwrap_or_default()
}

pub(super) fn is_free(id: &str) -> bool {
    id.ends_with("-free") || id == "big-pickle"
}

/// The model-less row for an installed OpenCode that is not connected yet. `adoptable`: its sign-in
/// holds a key the user has not approved.
pub(super) fn detected_row() -> Option<Value> {
    if !installed() {
        return None;
    }
    let adoptable = preferred_unconsented_api_key_source_for_env(KEY_ENV) == Some(ExternalAuthSource::OpenCode) || load_api_key_for_env(KEY_ENV).is_some();
    let ids = public_models();
    let free: Vec<&String> = ids.iter().filter(|m| is_free(m)).collect();
    Some(json!({
        "slug": "opencode", "name": "OpenCode Zen", "models": [], "total_models": ids.len(), "is_current": false,
        "authenticated": false, "settable": true, "source": "opencode-detected", "detected": true,
        "adoptable": adoptable, "free_models": free, "auth_type": "api_key", "key_env": KEY_ENV,
        "warning": "OpenCode is installed here. Connect it to use Zen models."
    }))
}

/// `model.save_key {slug:"opencode", source:"opencode"}`: the user approves OpenCode's own sign-in. Consent is
/// recorded the engine's way (trusted external source), then the Zen key moves from OpenCode's `auth.json`
/// into this app's `.env` at runtime, where the engine reads credentials. The value is never logged or returned.
pub(super) fn adopt() -> Result<(), String> {
    const NONE: &str = "No OpenCode sign-in with a key was found. Paste a Zen key instead.";
    let _ = trust_external_auth_source(ExternalAuthSource::OpenCode);
    let key = load_api_key_for_env(KEY_ENV).ok_or(NONE)?;
    let home = factr_base::factr_config::home().ok_or("No Factr config home.")?;
    // The engine reads OPENCODE_API_KEY; the Python runtime that saves the default model reads OPENCODE_ZEN_API_KEY.
    for name in [KEY_ENV, "OPENCODE_ZEN_API_KEY"] {
        write_env_key(&home.join(".env"), name, &key).map_err(|e| e.to_string())?;
    }
    factr_base::auth::AuthStatus::invalidate_cache();
    Ok(())
}

/// Upsert `NAME=value` in a dotenv file (0600), keeping every other line.
fn write_env_key(path: &std::path::Path, name: &str, value: &str) -> std::io::Result<()> {
    let old = std::fs::read_to_string(path).unwrap_or_default();
    let mut out: String = old.lines().filter(|l| !l.trim_start().starts_with(&format!("{name}="))).map(|l| format!("{l}\n")).collect();
    out.push_str(&format!("{name}={value}\n"));
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, out)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ids_deduped_and_without_delisted_or_junk() {
        let body = json!({ "data": [{ "id": "big-pickle" }, { "id": "big-pickle" }, { "id": "ox-alpha-free" }, { "id": "" }, "x", { "id": " gpt-6 " }] });
        assert_eq!(parse_models(&body), vec!["big-pickle", "gpt-6"]);
        assert!(parse_models(&json!(null)).is_empty());
        assert!(parse_models(&json!({ "data": "no" })).is_empty());
    }

    #[test]
    fn env_upsert_replaces_only_its_own_line() {
        let dir = std::env::temp_dir().join(format!("oc-env-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join(".env");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, "A=1\nOPENCODE_API_KEY=old\nB=2\n").unwrap();
        write_env_key(&path, "OPENCODE_API_KEY", "new").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "A=1\nB=2\nOPENCODE_API_KEY=new\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn free_ids() {
        assert!(is_free("beam-free") && is_free("big-pickle") && !is_free("gpt-6"));
    }

    #[test]
    fn detection_follows_path_and_the_off_switch() {
        let _lock = crate::factr_env::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("oc-detect-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        let (path, off) = (std::env::var_os("PATH"), std::env::var_os("FACTR_OPENCODE_DETECT"));
        // SAFETY: env is only touched under ENV_LOCK.
        unsafe {
            std::env::set_var("PATH", dir.join("bin"));
            std::env::remove_var("FACTR_OPENCODE_DETECT");
        }
        std::fs::write(dir.join("bin/opencode"), "#!/bin/sh\n").unwrap();
        assert!(installed());
        unsafe { std::env::set_var("FACTR_OPENCODE_DETECT", "off") };
        assert!(!installed());
        unsafe {
            match path { Some(p) => std::env::set_var("PATH", p), None => std::env::remove_var("PATH") }
            match off { Some(o) => std::env::set_var("FACTR_OPENCODE_DETECT", o), None => std::env::remove_var("FACTR_OPENCODE_DETECT") }
        }
        let _ = std::fs::remove_dir_all(dir);
    }
}
