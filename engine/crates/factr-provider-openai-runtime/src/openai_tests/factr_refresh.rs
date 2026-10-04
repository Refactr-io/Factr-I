// A Codex grant that came from Factr's auth.json has no refresh token in the engine: Factr is the
// only process that spends it. Near expiry and on a 401 the engine runs Factr's refresh command
// and re-reads. Fake tokens and a fake `factr` script only; the OAuth endpoint is never reachable
// from these tests, and a refresh token cannot be spent because the credential carries none.
#[cfg(unix)]
mod factr_owned_credentials {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    struct FakeFactr {
        _factr_home: tempfile::TempDir,
        bin: tempfile::TempDir,
        _env: [EnvVarGuard; 2],
    }

    fn pool(access: &str) -> String {
        serde_json::json!({"credential_pool": {"openai-codex": [{
            "auth_type": "oauth", "access_token": access, "refresh_token": "fake-refresh",
            "expires_at": "2999-01-01T00:00:00Z"}]}})
        .to_string()
    }

    /// `succeed`: the fake command writes a rotated grant, as `factr auth refresh` does.
    fn fake_factr(stored_access: &str, succeed: bool) -> FakeFactr {
        let factr_home = tempfile::TempDir::new().unwrap();
        let bin = tempfile::TempDir::new().unwrap();
        let env = [
            EnvVarGuard::set_path("FACTR_HOME", factr_home.path()),
            EnvVarGuard::set_path("FACTR_CONFIG_HOME", factr_home.path()),
        ];
        std::fs::write(factr_home.path().join("auth.json"), pool(stored_access)).unwrap();
        factr_base::auth::external::trust_external_auth_source(
            factr_base::auth::external::ExternalAuthSource::Factr,
        )
        .unwrap();
        let script = bin.path().join("fake-factr");
        let write = if succeed {
            format!("printf '%s' '{}' > \"$FACTR_CONFIG_HOME/auth.json\"", pool("fake-access-rotated"))
        } else {
            "exit 1".to_string()
        };
        std::fs::write(
            &script,
            format!("#!/bin/sh\necho \"$@\" >> \"{}/calls\"\nsleep 0.2\n{write}\n", bin.path().display()),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        factr_base::auth::external::set_factr_refresh_command(vec![script.to_string_lossy().into_owned()]);
        FakeFactr { _factr_home: factr_home, bin, _env: env }
    }

    impl FakeFactr {
        fn calls(&self) -> usize {
            std::fs::read_to_string(self.bin.path().join("calls")).map(|t| t.lines().count()).unwrap_or(0)
        }
    }

    fn factr_credentials(access: &str, expires_in_ms: i64) -> Arc<RwLock<CodexCredentials>> {
        Arc::new(RwLock::new(CodexCredentials {
            access_token: access.into(),
            refresh_token: String::new(),
            id_token: None,
            account_id: None,
            expires_at: Some(chrono::Utc::now().timestamp_millis() + expires_in_ms),
        }))
    }

    #[tokio::test]
    async fn a_factr_credential_is_chatgpt_mode_without_a_refresh_token() {
        let creds = factr_credentials("fake-access", 3_600_000);
        let creds = creds.read().await;
        assert!(OpenAIProvider::is_chatgpt_mode(&creds));
        assert!(creds.is_refreshed_by_factr());
    }

    #[tokio::test]
    async fn near_expiry_asks_factr_instead_of_the_oauth_endpoint() {
        let _guard = factr_base::storage::lock_test_env();
        let factr = fake_factr("fake-access", true);
        let creds = factr_credentials("fake-access", 60_000);
        let token = openai_access_token(&creds).await.unwrap();
        assert_eq!(token, "fake-access-rotated");
        assert_eq!(creds.read().await.access_token, "fake-access-rotated");
        assert!(creds.read().await.refresh_token.is_empty());
        assert_eq!(factr.calls(), 1);
    }

    #[tokio::test]
    async fn a_valid_token_does_not_touch_factr() {
        let _guard = factr_base::storage::lock_test_env();
        let factr = fake_factr("fake-access", true);
        let creds = factr_credentials("fake-access", 3_600_000);
        assert_eq!(openai_access_token(&creds).await.unwrap(), "fake-access");
        assert_eq!(factr.calls(), 0);
    }

    #[tokio::test]
    async fn near_expiry_with_factr_failing_keeps_the_still_valid_token() {
        let _guard = factr_base::storage::lock_test_env();
        let factr = fake_factr("fake-access", false);
        let creds = factr_credentials("fake-access", 60_000);
        assert_eq!(openai_access_token(&creds).await.unwrap(), "fake-access");
        assert_eq!(factr.calls(), 1);
        let expired = factr_credentials("fake-access", -1_000);
        assert!(openai_access_token(&expired).await.is_err());
    }

    #[tokio::test]
    async fn a_401_rereads_through_factr_and_never_the_oauth_endpoint() {
        let _guard = factr_base::storage::lock_test_env();
        let factr = fake_factr("fake-access", true);
        // Not near expiry by the local clock, but the server rejected it.
        let creds = factr_credentials("fake-access", 3_600_000);
        let fresh = crate::openai_stream_runtime::reload_factr_openai_token(&creds, "fake-access").await.unwrap();
        assert_eq!(fresh, "fake-access-rotated");
        assert_eq!(creds.read().await.access_token, "fake-access-rotated");
        assert_eq!(factr.calls(), 1);
    }

    #[tokio::test]
    async fn parallel_requests_cause_one_factr_refresh_and_no_token_endpoint_call() {
        let _guard = factr_base::storage::lock_test_env();
        let factr = fake_factr("fake-access", true);
        let creds = factr_credentials("fake-access", 60_000);
        let tasks: Vec<_> = (0..8)
            .map(|_| {
                let creds = Arc::clone(&creds);
                tokio::spawn(async move { openai_access_token(&creds).await.unwrap() })
            })
            .collect();
        for task in tasks {
            assert_eq!(task.await.unwrap(), "fake-access-rotated");
        }
        assert_eq!(factr.calls(), 1);
        assert!(creds.read().await.refresh_token.is_empty());
    }
}
