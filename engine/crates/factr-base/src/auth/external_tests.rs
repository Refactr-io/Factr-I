use super::*;
use tempfile::TempDir;

fn write_auth_file(path: &std::path::Path, value: serde_json::Value) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, serde_json::to_string(&value).unwrap()).unwrap();
}

#[test]
fn novita_api_key_import_requires_trust() {
    let _guard = crate::storage::lock_test_env();
    let dir = TempDir::new().unwrap();
    let prev = std::env::var_os("FACTR_HOME");
    crate::env::set_var("FACTR_HOME", dir.path());

    let path = ExternalAuthSource::OpenCode.path().unwrap();
    write_auth_file(
        &path,
        serde_json::json!({ "novita": { "type": "api", "key": "novita_test_secret" } }),
    );
    assert!(load_api_key_for_env("NOVITA_API_KEY").is_none());
    trust_external_auth_source(ExternalAuthSource::OpenCode).unwrap();
    assert_eq!(
        load_api_key_for_env("NOVITA_API_KEY").as_deref(),
        Some("novita_test_secret")
    );

    if let Some(prev) = prev {
        crate::env::set_var("FACTR_HOME", prev);
    } else {
        crate::env::remove_var("FACTR_HOME");
    }
}

#[test]
fn opencode_api_key_imports_from_trusted_file() {
    let _guard = crate::storage::lock_test_env();
    let dir = TempDir::new().unwrap();
    let prev = std::env::var_os("FACTR_HOME");
    crate::env::set_var("FACTR_HOME", dir.path());

    let path = ExternalAuthSource::OpenCode.path().unwrap();
    write_auth_file(
        &path,
        serde_json::json!({
            "opencode": { "type": "api", "key": "oc_test_secret" }
        }),
    );

    assert!(load_api_key_for_env("OPENCODE_API_KEY").is_none());
    trust_external_auth_source(ExternalAuthSource::OpenCode).unwrap();
    assert_eq!(
        load_api_key_for_env("OPENCODE_API_KEY").as_deref(),
        Some("oc_test_secret")
    );

    if let Some(prev) = prev {
        crate::env::set_var("FACTR_HOME", prev);
    } else {
        crate::env::remove_var("FACTR_HOME");
    }
}

#[test]
fn pi_api_key_env_reference_uses_named_env_var() {
    let _guard = crate::storage::lock_test_env();
    let dir = TempDir::new().unwrap();
    let prev_home = std::env::var_os("FACTR_HOME");
    let prev_key = std::env::var_os("PI_OPENAI_KEY");
    crate::env::set_var("FACTR_HOME", dir.path());
    crate::env::set_var("PI_OPENAI_KEY", "sk-from-env-ref");

    let path = ExternalAuthSource::Pi.path().unwrap();
    write_auth_file(
        &path,
        serde_json::json!({
            "openai": { "type": "api_key", "key": "PI_OPENAI_KEY" }
        }),
    );

    trust_external_auth_source(ExternalAuthSource::Pi).unwrap();
    assert_eq!(
        load_api_key_for_env("OPENAI_API_KEY").as_deref(),
        Some("sk-from-env-ref")
    );

    if let Some(prev_home) = prev_home {
        crate::env::set_var("FACTR_HOME", prev_home);
    } else {
        crate::env::remove_var("FACTR_HOME");
    }
    if let Some(prev_key) = prev_key {
        crate::env::set_var("PI_OPENAI_KEY", prev_key);
    } else {
        crate::env::remove_var("PI_OPENAI_KEY");
    }
}

#[test]
fn pi_shell_command_api_keys_are_not_executed() {
    let _guard = crate::storage::lock_test_env();
    let dir = TempDir::new().unwrap();
    let prev = std::env::var_os("FACTR_HOME");
    crate::env::set_var("FACTR_HOME", dir.path());

    let path = ExternalAuthSource::Pi.path().unwrap();
    write_auth_file(
        &path,
        serde_json::json!({
            "openai": { "type": "api_key", "key": "!security find-generic-password -ws openai" }
        }),
    );

    trust_external_auth_source(ExternalAuthSource::Pi).unwrap();
    assert!(load_api_key_for_env("OPENAI_API_KEY").is_none());

    if let Some(prev) = prev {
        crate::env::set_var("FACTR_HOME", prev);
    } else {
        crate::env::remove_var("FACTR_HOME");
    }
}

#[test]
fn load_copilot_oauth_token_from_pi_auth() {
    let _guard = crate::storage::lock_test_env();
    let dir = TempDir::new().unwrap();
    let prev = std::env::var_os("FACTR_HOME");
    crate::env::set_var("FACTR_HOME", dir.path());

    let path = ExternalAuthSource::Pi.path().unwrap();
    write_auth_file(
        &path,
        serde_json::json!({
            "github-copilot": {
                "type": "oauth",
                "access": "ghu_pi_token",
                "refresh": "refresh",
                "expires": chrono::Utc::now().timestamp_millis() + 60_000
            }
        }),
    );

    trust_external_auth_source(ExternalAuthSource::Pi).unwrap();
    assert_eq!(load_copilot_oauth_token().as_deref(), Some("ghu_pi_token"));

    if let Some(prev) = prev {
        crate::env::set_var("FACTR_HOME", prev);
    } else {
        crate::env::remove_var("FACTR_HOME");
    }
}

#[test]
fn unconsented_source_detects_supported_api_key_files() {
    let _guard = crate::storage::lock_test_env();
    let dir = TempDir::new().unwrap();
    let prev = std::env::var_os("FACTR_HOME");
    crate::env::set_var("FACTR_HOME", dir.path());

    let path = ExternalAuthSource::OpenCode.path().unwrap();
    write_auth_file(
        &path,
        serde_json::json!({
            "opencode": { "type": "api", "key": "oc_test_secret" }
        }),
    );

    assert_eq!(
        preferred_unconsented_api_key_source_for_env("OPENCODE_API_KEY"),
        Some(ExternalAuthSource::OpenCode)
    );

    if let Some(prev) = prev {
        crate::env::set_var("FACTR_HOME", prev);
    } else {
        crate::env::remove_var("FACTR_HOME");
    }
}

#[test]
fn source_provider_labels_reports_supported_oauth_and_api_key_imports() {
    let _guard = crate::storage::lock_test_env();
    let dir = TempDir::new().unwrap();
    let prev = std::env::var_os("FACTR_HOME");
    crate::env::set_var("FACTR_HOME", dir.path());

    let path = ExternalAuthSource::OpenCode.path().unwrap();
    write_auth_file(
        &path,
        serde_json::json!({
            "openai": {
                "type": "oauth",
                "access": "sk-access",
                "refresh": "refresh",
                "expires": chrono::Utc::now().timestamp_millis() + 60_000
            },
            "anthropic": {
                "type": "oauth",
                "access": "claude-access",
                "refresh": "refresh",
                "expires": chrono::Utc::now().timestamp_millis() + 60_000
            },
            "openrouter": { "type": "api", "key": "sk-or-test" }
        }),
    );

    let labels = source_provider_labels(ExternalAuthSource::OpenCode);
    assert!(labels.contains(&"OpenAI/Codex"));
    assert!(labels.contains(&"Claude"));
    assert!(labels.contains(&"OpenRouter/API-key providers"));

    if let Some(prev) = prev {
        crate::env::set_var("FACTR_HOME", prev);
    } else {
        crate::env::remove_var("FACTR_HOME");
    }
}

#[test]
fn openclaw_api_key_imports_from_trusted_file() {
    let _guard = crate::storage::lock_test_env();
    let dir = TempDir::new().unwrap();
    let prev = std::env::var_os("FACTR_HOME");
    crate::env::set_var("FACTR_HOME", dir.path());

    let path = ExternalAuthSource::OpenClaw.path().unwrap();
    write_auth_file(
        &path,
        serde_json::json!({
            "anthropic": { "type": "api_key", "key": "sk-ant-openclaw" }
        }),
    );

    assert!(load_api_key_for_env("ANTHROPIC_API_KEY").is_none());
    trust_external_auth_source(ExternalAuthSource::OpenClaw).unwrap();
    assert_eq!(
        load_api_key_for_env("ANTHROPIC_API_KEY").as_deref(),
        Some("sk-ant-openclaw")
    );

    if let Some(prev) = prev {
        crate::env::set_var("FACTR_HOME", prev);
    } else {
        crate::env::remove_var("FACTR_HOME");
    }
}

#[test]
fn openclaw_oauth_tokens_import_like_pi() {
    let _guard = crate::storage::lock_test_env();
    let dir = TempDir::new().unwrap();
    let prev = std::env::var_os("FACTR_HOME");
    crate::env::set_var("FACTR_HOME", dir.path());

    let path = ExternalAuthSource::OpenClaw.path().unwrap();
    write_auth_file(
        &path,
        serde_json::json!({
            "anthropic": {
                "type": "oauth",
                "access": "claude-access",
                "refresh": "claude-refresh",
                "expires": chrono::Utc::now().timestamp_millis() + 60_000
            }
        }),
    );

    assert!(load_anthropic_oauth_tokens().is_none());
    trust_external_auth_source(ExternalAuthSource::OpenClaw).unwrap();
    let tokens = load_anthropic_oauth_tokens().expect("oauth tokens imported");
    assert_eq!(tokens.access_token, "claude-access");
    assert_eq!(tokens.refresh_token, "claude-refresh");

    if let Some(prev) = prev {
        crate::env::set_var("FACTR_HOME", prev);
    } else {
        crate::env::remove_var("FACTR_HOME");
    }
}

#[test]
fn factr_api_key_imports_from_credential_pool() {
    let _guard = crate::storage::lock_test_env();
    let dir = TempDir::new().unwrap();
    let prev = std::env::var_os("FACTR_HOME");
    crate::env::set_var("FACTR_HOME", dir.path());

    let path = ExternalAuthSource::Factr.path().unwrap();
    write_auth_file(
        &path,
        serde_json::json!({
            "version": 1,
            "active_provider": "anthropic",
            "credential_pool": {
                "anthropic": [
                    {
                        "id": "abc123",
                        "label": "manual",
                        "auth_type": "api_key",
                        "priority": 0,
                        "source": "manual:1",
                        "access_token": "sk-ant-factr"
                    }
                ]
            }
        }),
    );

    assert!(load_api_key_for_env("ANTHROPIC_API_KEY").is_none());
    trust_external_auth_source(ExternalAuthSource::Factr).unwrap();
    assert_eq!(
        load_api_key_for_env("ANTHROPIC_API_KEY").as_deref(),
        Some("sk-ant-factr")
    );

    if let Some(prev) = prev {
        crate::env::set_var("FACTR_HOME", prev);
    } else {
        crate::env::remove_var("FACTR_HOME");
    }
}

#[test]
fn factr_oauth_tokens_import_from_credential_pool() {
    let _guard = crate::storage::lock_test_env();
    let dir = TempDir::new().unwrap();
    let prev = std::env::var_os("FACTR_HOME");
    crate::env::set_var("FACTR_HOME", dir.path());

    let path = ExternalAuthSource::Factr.path().unwrap();
    write_auth_file(
        &path,
        serde_json::json!({
            "version": 1,
            "credential_pool": {
                "openai-codex": [
                    {
                        "id": "def456",
                        "label": "loopback_pkce",
                        "auth_type": "oauth_external",
                        "priority": 0,
                        "source": "loopback_pkce",
                        "access_token": "codex-access",
                        "refresh_token": "codex-refresh",
                        "expires_at_ms": chrono::Utc::now().timestamp_millis() + 60_000
                    }
                ]
            }
        }),
    );

    assert!(load_openai_oauth_tokens().is_none());
    trust_external_auth_source(ExternalAuthSource::Factr).unwrap();
    let tokens = load_openai_oauth_tokens().expect("oauth tokens imported");
    assert_eq!(tokens.access_token, "codex-access");
    assert!(tokens.refresh_token.is_empty(), "the single-use Codex refresh token stays with Factr");

    if let Some(prev) = prev {
        crate::env::set_var("FACTR_HOME", prev);
    } else {
        crate::env::remove_var("FACTR_HOME");
    }
}

#[test]
fn factr_oauth_tokens_parse_rfc3339_expiry() {
    let _guard = crate::storage::lock_test_env();
    let dir = TempDir::new().unwrap();
    let prev = std::env::var_os("FACTR_HOME");
    crate::env::set_var("FACTR_HOME", dir.path());

    let future = chrono::Utc::now() + chrono::Duration::minutes(5);
    let path = ExternalAuthSource::Factr.path().unwrap();
    write_auth_file(
        &path,
        serde_json::json!({
            "version": 1,
            "credential_pool": {
                "anthropic": [
                    {
                        "auth_type": "oauth_external",
                        "access_token": "claude-access",
                        "refresh_token": "claude-refresh",
                        "expires_at": future.to_rfc3339()
                    }
                ]
            }
        }),
    );

    trust_external_auth_source(ExternalAuthSource::Factr).unwrap();
    let tokens = load_anthropic_oauth_tokens().expect("oauth tokens imported");
    assert_eq!(tokens.access_token, "claude-access");
    assert!(tokens.expires_at > chrono::Utc::now().timestamp_millis());

    if let Some(prev) = prev {
        crate::env::set_var("FACTR_HOME", prev);
    } else {
        crate::env::remove_var("FACTR_HOME");
    }
}

#[test]
fn openclaw_auth_profiles_store_resolves_and_flattens() {
    let _guard = crate::storage::lock_test_env();
    let dir = TempDir::new().unwrap();
    let prev = std::env::var_os("FACTR_HOME");
    crate::env::set_var("FACTR_HOME", dir.path());

    // No legacy ~/.openclaw/agent/auth.json: the current per-agent
    // auth-profiles.json store must be discovered instead.
    let profiles_path =
        crate::storage::user_home_path(".openclaw/agents/main/agent/auth-profiles.json").unwrap();
    write_auth_file(
        &profiles_path,
        serde_json::json!({
            "version": 1,
            "profiles": {
                "openai:work": {
                    "type": "oauth",
                    "provider": "openai",
                    "access": "work-access",
                    "refresh": "work-refresh",
                    "expires": chrono::Utc::now().timestamp_millis() + 60_000
                },
                "openai:default": {
                    "type": "oauth",
                    "provider": "openai",
                    "access": "openclaw-access",
                    "refresh": "openclaw-refresh",
                    "expires": chrono::Utc::now().timestamp_millis() + 60_000
                },
                "openrouter:default": {
                    "type": "api_key",
                    "provider": "openrouter",
                    "key": "sk-or-openclaw"
                }
            }
        }),
    );

    assert_eq!(ExternalAuthSource::OpenClaw.path().unwrap(), profiles_path);
    trust_external_auth_source(ExternalAuthSource::OpenClaw).unwrap();

    // The `:default` profile wins over the sibling `openai:work` profile.
    let tokens = load_openai_oauth_tokens().expect("oauth tokens imported");
    assert_eq!(tokens.access_token, "openclaw-access");
    assert_eq!(
        load_api_key_for_env("OPENROUTER_API_KEY").as_deref(),
        Some("sk-or-openclaw")
    );

    if let Some(prev) = prev {
        crate::env::set_var("FACTR_HOME", prev);
    } else {
        crate::env::remove_var("FACTR_HOME");
    }
}

#[test]
fn openclaw_legacy_flat_auth_json_still_wins_when_present() {
    let _guard = crate::storage::lock_test_env();
    let dir = TempDir::new().unwrap();
    let prev = std::env::var_os("FACTR_HOME");
    crate::env::set_var("FACTR_HOME", dir.path());

    // Both layouts exist: the original pi-fork path must take precedence so
    // previously-recorded trust decisions stay bound to the same file.
    let legacy_path = crate::storage::user_home_path(".openclaw/agent/auth.json").unwrap();
    write_auth_file(
        &legacy_path,
        serde_json::json!({
            "anthropic": { "type": "api_key", "key": "sk-ant-legacy" }
        }),
    );
    let profiles_path =
        crate::storage::user_home_path(".openclaw/agents/main/agent/auth-profiles.json").unwrap();
    write_auth_file(
        &profiles_path,
        serde_json::json!({ "version": 1, "profiles": {} }),
    );

    assert_eq!(ExternalAuthSource::OpenClaw.path().unwrap(), legacy_path);

    if let Some(prev) = prev {
        crate::env::set_var("FACTR_HOME", prev);
    } else {
        crate::env::remove_var("FACTR_HOME");
    }
}

#[test]
fn factr_store_follows_factr_home_and_never_hands_out_an_expired_codex_grant() {
    let _guard = crate::storage::lock_test_env();
    let dir = TempDir::new().unwrap();
    let factr = TempDir::new().unwrap();
    let prev = (std::env::var_os("FACTR_HOME"), std::env::var_os("FACTR_CONFIG_HOME"));
    crate::env::set_var("FACTR_HOME", dir.path());
    crate::env::set_var("FACTR_CONFIG_HOME", factr.path());
    assert_eq!(ExternalAuthSource::Factr.path().unwrap(), factr.path().join("auth.json"));

    let entry = |expires_at: &str| {
        serde_json::json!({"credential_pool": {"openai-codex": [{
            "auth_type": "oauth", "access_token": "fake-access", "refresh_token": "fake-refresh",
            "expires_at": expires_at}]}})
    };
    let path = ExternalAuthSource::Factr.path().unwrap();
    write_auth_file(&path, entry("2999-01-01T00:00:00Z"));
    trust_external_auth_source(ExternalAuthSource::Factr).unwrap();
    assert_eq!(load_openai_oauth_tokens().expect("UI-written grant").access_token, "fake-access");
    // Expired: only Factr's Python backend may spend the single-use refresh token.
    write_auth_file(&path, entry("2001-01-01T00:00:00Z"));
    assert!(load_openai_oauth_tokens().is_none());
    // Private deployment: the store is never read.
    write_auth_file(&path, entry("2999-01-01T00:00:00Z"));
    factr_provider_env::set_env_only(true);
    assert!(load_openai_oauth_tokens().is_none());
    factr_provider_env::set_env_only(false);

    for (k, v) in [("FACTR_HOME", prev.0), ("FACTR_CONFIG_HOME", prev.1)] {
        match v {
            Some(v) => crate::env::set_var(k, v),
            None => crate::env::remove_var(k),
        }
    }
}

#[test]
fn private_deployment_reads_no_credential_file_for_any_provider() {
    let _guard = crate::storage::lock_test_env();
    let dir = TempDir::new().unwrap();
    let prev = std::env::var_os("FACTR_HOME");
    crate::env::set_var("FACTR_HOME", dir.path());
    let far = chrono::Utc::now().timestamp_millis() + 3_600_000;

    // Fake credentials on disk for each provider store.
    crate::auth::codex::upsert_account_from_tokens("fake", "fake-c", "fake-cr", None, Some(far)).unwrap();
    let factr = ExternalAuthSource::Factr.path().unwrap();
    write_auth_file(&factr, serde_json::json!({"credential_pool": {"openai-codex": [{
        "auth_type": "oauth", "access_token": "fake-h", "refresh_token": "fake-hr", "expires_at": "2999-01-01T00:00:00Z"}]}}));
    trust_external_auth_source(ExternalAuthSource::Factr).unwrap();

    assert!(crate::auth::codex::load_oauth_credentials().is_ok());

    factr_provider_env::set_env_only(true);
    assert!(crate::auth::google::load_tokens().is_err());
    assert!(crate::auth::google::load_credentials().is_err());
    assert!(crate::auth::codex::load_oauth_credentials().is_err());
    assert!(crate::auth::codex::load_credentials_for_account("fake").is_err());
    assert!(crate::auth::claude::load_opencode_credentials().is_err());
    assert!(crate::auth::claude::load_auth_file().unwrap().anthropic_accounts.is_empty());
    assert!(crate::auth::grok_build::load_credential().is_none());
    assert!(load_openai_oauth_tokens().is_none());
    factr_provider_env::set_env_only(false);

    match prev {
        Some(p) => crate::env::set_var("FACTR_HOME", p),
        None => crate::env::remove_var("FACTR_HOME"),
    }
}

#[cfg(unix)]
mod factr_refresh {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn entry(expires_at: &str) -> serde_json::Value {
        serde_json::json!({"credential_pool": {"openai-codex": [{
            "auth_type": "oauth", "access_token": "fake-access", "refresh_token": "fake-refresh", "expires_at": expires_at}]}})
    }

    /// A fake `factr`: counts calls; on success rewrites auth.json with a fresh expiry.
    fn fake_factr(dir: &std::path::Path, succeed: bool) -> Vec<String> {
        let script = dir.join("fake-factr");
        let fresh = serde_json::to_string(&entry("2999-01-01T00:00:00Z")).unwrap();
        let write = if succeed { format!("printf '%s' '{fresh}' > \"$FACTR_CONFIG_HOME/auth.json\"") } else { "exit 1".into() };
        std::fs::write(&script, format!("#!/bin/sh\necho \"$@\" >> \"{}/calls\"\nsleep 0.2\n{write}\n", dir.display())).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        vec![script.to_string_lossy().into_owned()]
    }

    fn calls(dir: &std::path::Path) -> usize {
        std::fs::read_to_string(dir.join("calls")).map(|t| t.lines().count()).unwrap_or(0)
    }

    fn setup(expires_at: &str) -> (TempDir, TempDir, [Option<std::ffi::OsString>; 2]) {
        let factr = TempDir::new().unwrap();
        let bin = TempDir::new().unwrap();
        let prev = [std::env::var_os("FACTR_HOME"), std::env::var_os("FACTR_CONFIG_HOME")];
        crate::env::set_var("FACTR_HOME", factr.path());
        crate::env::set_var("FACTR_CONFIG_HOME", factr.path());
        write_auth_file(&factr.path().join("auth.json"), entry(expires_at));
        trust_external_auth_source(ExternalAuthSource::Factr).unwrap();
        (factr, bin, prev)
    }

    fn restore(prev: [Option<std::ffi::OsString>; 2]) {
        for (k, v) in ["FACTR_HOME", "FACTR_CONFIG_HOME"].into_iter().zip(prev) {
            match v { Some(v) => crate::env::set_var(k, v), None => crate::env::remove_var(k) }
        }
    }

    #[test]
    fn valid_grant_never_spawns() {
        let _g = crate::storage::lock_test_env();
        let (_home, bin, prev) = setup("2999-01-01T00:00:00Z");
        set_factr_refresh_command(fake_factr(bin.path(), true));
        assert_eq!(load_openai_oauth_tokens().unwrap().access_token, "fake-access");
        assert_eq!(calls(bin.path()), 0);
        restore(prev);
    }

    #[test]
    fn expired_grant_is_refreshed_by_factr_then_reread() {
        let _g = crate::storage::lock_test_env();
        let (_home, bin, prev) = setup("2001-01-01T00:00:00Z");
        set_factr_refresh_command(fake_factr(bin.path(), true));
        assert!(load_openai_oauth_tokens().is_some());
        assert_eq!(calls(bin.path()), 1);
        assert!(std::fs::read_to_string(bin.path().join("calls")).unwrap().contains("auth refresh openai-codex"));
        restore(prev);
    }

    #[test]
    fn failed_refresh_is_one_clear_error() {
        let _g = crate::storage::lock_test_env();
        let (_home, bin, prev) = setup("2001-01-01T00:00:00Z");
        set_factr_refresh_command(fake_factr(bin.path(), false));
        assert!(load_openai_oauth_tokens().is_none());
        let err = crate::auth::codex::load_credentials().unwrap_err().to_string();
        assert_eq!(err, CODEX_EXPIRED_MSG);
        restore(prev);
    }

    /// A fake `factr` that rotates the access token: the grant it writes differs from the one it replaces.
    fn fake_factr_rotating(dir: &std::path::Path) -> Vec<String> {
        let script = dir.join("fake-factr-rotating");
        let fresh = serde_json::to_string(&serde_json::json!({"credential_pool": {"openai-codex": [{
            "auth_type": "oauth", "access_token": "fake-access-rotated", "refresh_token": "fake-refresh-rotated",
            "expires_at": "2999-01-01T00:00:00Z"}]}})).unwrap();
        std::fs::write(&script, format!(
            "#!/bin/sh\necho \"$@\" >> \"{}/calls\"\nsleep 0.2\nprintf '%s' '{fresh}' > \"$FACTR_CONFIG_HOME/auth.json\"\n", dir.display())).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        vec![script.to_string_lossy().into_owned()]
    }

    #[test]
    fn the_engine_never_holds_the_factr_refresh_token() {
        let _g = crate::storage::lock_test_env();
        let (_home, bin, prev) = setup("2999-01-01T00:00:00Z");
        set_factr_refresh_command(fake_factr(bin.path(), true));
        let creds = crate::auth::codex::load_credentials().unwrap();
        assert_eq!(creds.access_token, "fake-access");
        assert!(creds.refresh_token.is_empty(), "the single-use refresh token stays with Factr");
        assert!(creds.is_chatgpt_oauth() && creds.is_refreshed_by_factr());
        assert_eq!(calls(bin.path()), 0);
        restore(prev);
    }

    #[test]
    fn a_rejected_token_makes_factr_refresh_and_the_engine_re_read() {
        let _g = crate::storage::lock_test_env();
        let (_home, bin, prev) = setup("2999-01-01T00:00:00Z");
        set_factr_refresh_command(fake_factr_rotating(bin.path()));
        let creds = crate::auth::codex::reload_after_rejection("fake-access").unwrap();
        assert_eq!(creds.access_token, "fake-access-rotated");
        assert!(creds.refresh_token.is_empty());
        assert_eq!(calls(bin.path()), 1);
        assert!(std::fs::read_to_string(bin.path().join("calls")).unwrap().contains("auth refresh openai-codex"));
        restore(prev);
    }

    #[test]
    fn parallel_rejections_cause_one_factr_refresh() {
        let _g = crate::storage::lock_test_env();
        let (_home, bin, prev) = setup("2999-01-01T00:00:00Z");
        set_factr_refresh_command(fake_factr_rotating(bin.path()));
        let threads: Vec<_> = (0..8).map(|_| std::thread::spawn(|| {
            crate::auth::codex::reload_after_rejection("fake-access").unwrap().access_token
        })).collect();
        for t in threads { assert_eq!(t.join().unwrap(), "fake-access-rotated"); }
        assert_eq!(calls(bin.path()), 1);
        restore(prev);
    }

    #[test]
    fn concurrent_callers_cause_one_spawn() {
        let _g = crate::storage::lock_test_env();
        let (_home, bin, prev) = setup("2001-01-01T00:00:00Z");
        set_factr_refresh_command(fake_factr(bin.path(), true));
        let threads: Vec<_> = (0..4).map(|_| std::thread::spawn(|| load_openai_oauth_tokens().is_some())).collect();
        assert!(threads.into_iter().all(|t| t.join().unwrap()));
        assert_eq!(calls(bin.path()), 1);
        restore(prev);
    }
}

#[test]
fn a_factr_codex_entry_without_an_expiry_field_uses_the_jwt_exp_claim() {
    use base64::Engine;
    let enc = |v: &serde_json::Value| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(v.to_string());
    let exp = chrono::Utc::now().timestamp() + 3600;
    let jwt = format!("{}.{}.fakesig", enc(&serde_json::json!({"alg": "none"})), enc(&serde_json::json!({"exp": exp})));
    let entry = serde_json::json!({"auth_type": "oauth", "access_token": jwt, "refresh_token": "fake-r", "last_refresh": "2026-10-02T11:12:21Z"});
    let tokens = extract_oauth_tokens_factr_style(&entry).expect("a JWT exp is an expiry");
    assert_eq!(tokens.expires_at, exp * 1000);
    let opaque = serde_json::json!({"auth_type": "oauth", "access_token": "not-a-jwt", "refresh_token": "fake-r"});
    assert!(extract_oauth_tokens_factr_style(&opaque).is_none());
}
