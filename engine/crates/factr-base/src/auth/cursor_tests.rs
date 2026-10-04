use super::*;
use tempfile::TempDir;

#[test]
fn the_api_key_comes_from_the_environment_only() {
    let _lock = crate::storage::lock_test_env();
    let previous = std::env::var_os("CURSOR_API_KEY");
    crate::env::remove_var("CURSOR_API_KEY");
    assert!(load_api_key().is_err());
    crate::env::set_var("CURSOR_API_KEY", "env_test_key");
    assert_eq!(load_api_key().unwrap(), "env_test_key");
    match previous {
        Some(v) => crate::env::set_var("CURSOR_API_KEY", v),
        None => crate::env::remove_var("CURSOR_API_KEY"),
    }
}

#[test]
fn cursor_auth_file_path_respects_factr_home() {
    // Regression: on Linux the auth.json path previously used
    // `dirs::config_dir()` directly, ignoring FACTR_HOME. That leaked the real
    // `~/.config/cursor/auth.json` into the onboarding sandbox, so a
    // fresh-install sandbox showed only Cursor as importable while every other
    // provider correctly looked under `$FACTR_HOME/external/...`.
    let _guard = crate::storage::lock_test_env();
    let prev_home = std::env::var_os("FACTR_HOME");
    let temp = TempDir::new().unwrap();
    crate::env::set_var("FACTR_HOME", temp.path());

    let path = cursor_auth_file_path().expect("cursor auth path");
    assert!(
        path.starts_with(temp.path().join("external")),
        "cursor auth path should be under FACTR_HOME/external, got {}",
        path.display()
    );

    if let Some(prev_home) = prev_home {
        crate::env::set_var("FACTR_HOME", prev_home);
    } else {
        crate::env::remove_var("FACTR_HOME");
    }
}

#[cfg(target_os = "windows")]
#[test]
fn cursor_auth_file_path_does_not_escape_factr_home_on_windows() {
    let _guard = crate::storage::lock_test_env();
    let temp = tempfile::tempdir().unwrap();
    let old_home = std::env::var_os("FACTR_HOME");
    let old_appdata = std::env::var_os("APPDATA");
    crate::env::set_var("FACTR_HOME", temp.path());
    crate::env::set_var("APPDATA", r"C:\real-user-profile");

    let path = cursor_auth_file_path().unwrap();

    match old_home {
        Some(value) => crate::env::set_var("FACTR_HOME", value),
        None => crate::env::remove_var("FACTR_HOME"),
    }
    match old_appdata {
        Some(value) => crate::env::set_var("APPDATA", value),
        None => crate::env::remove_var("APPDATA"),
    }
    assert_eq!(
        path,
        temp.path()
            .join("external/AppData/Roaming/Cursor/auth.json")
    );
}

#[test]
fn cursor_vscdb_paths_respect_factr_home() {
    let _guard = crate::storage::lock_test_env();
    let prev_home = std::env::var_os("FACTR_HOME");
    let temp = TempDir::new().unwrap();
    crate::env::set_var("FACTR_HOME", temp.path());

    let paths = cursor_vscdb_paths();
    assert!(!paths.is_empty());
    for path in paths {
        assert!(path.starts_with(temp.path().join("external")));
    }

    if let Some(prev_home) = prev_home {
        crate::env::set_var("FACTR_HOME", prev_home);
    } else {
        crate::env::remove_var("FACTR_HOME");
    }
}

#[cfg(unix)]
#[test]
fn load_access_token_from_auth_file_does_not_change_external_permissions() {
    use std::os::unix::fs::PermissionsExt;

    let _guard = crate::storage::lock_test_env();
    let prev_home = std::env::var_os("FACTR_HOME");
    let temp = TempDir::new().unwrap();
    crate::env::set_var("FACTR_HOME", temp.path());

    let path = cursor_auth_file_path().expect("cursor auth path");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        r#"{"accessToken":"at-test","refreshToken":"rt-test"}"#,
    )
    .unwrap();
    std::fs::set_permissions(
        path.parent().unwrap(),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    crate::config::Config::allow_external_auth_source_for_path(CURSOR_AUTH_FILE_SOURCE_ID, &path)
        .expect("trust cursor auth path");

    let tokens = load_access_token_from_env_or_file().expect("load auth file token");
    assert_eq!(tokens.access_token, "at-test");
    assert_eq!(tokens.refresh_token.as_deref(), Some("rt-test"));
    assert!(has_cursor_auth_file_token());

    let dir_mode = std::fs::metadata(path.parent().unwrap())
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    let file_mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(dir_mode, 0o755);
    assert_eq!(file_mode, 0o644);

    if let Some(prev_home) = prev_home {
        crate::env::set_var("FACTR_HOME", prev_home);
    } else {
        crate::env::remove_var("FACTR_HOME");
    }
}

#[test]
fn reads_cursor_state_with_embedded_sqlite() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.vscdb");
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute(
            "CREATE TABLE ItemTable (key TEXT PRIMARY KEY, value TEXT)",
            [],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO ItemTable (key, value) VALUES (?1, ?2)",
            ("cursorAuth/accessToken", "native-token"),
        )
        .unwrap();
    drop(connection);

    assert_eq!(
        read_vscdb_key(&path, "cursorAuth/accessToken").unwrap(),
        "native-token"
    );
}

/// Helper: create a mock state.vscdb with the given key/value pairs.
fn create_mock_vscdb(dir: &std::path::Path, entries: &[(&str, &str)]) -> PathBuf {
    let db_path = dir.join("state.vscdb");
    let status = std::process::Command::new("sqlite3")
        .arg(&db_path)
        .arg("CREATE TABLE ItemTable (key TEXT UNIQUE ON CONFLICT REPLACE, value BLOB);")
        .status()
        .expect("sqlite3 must be installed for these tests");
    assert!(status.success(), "Failed to create mock vscdb");

    for (key, value) in entries {
        let sql = format!(
            "INSERT INTO ItemTable (key, value) VALUES ('{}', '{}');",
            key, value
        );
        let status = std::process::Command::new("sqlite3")
            .arg(&db_path)
            .arg(&sql)
            .status()
            .unwrap();
        assert!(status.success(), "Failed to insert into mock vscdb");
    }
    db_path
}

#[test]
fn vscdb_read_access_token() {
    let dir = TempDir::new().unwrap();
    let db = create_mock_vscdb(dir.path(), &[("cursorAuth/accessToken", "tok_abc123xyz")]);
    let result = read_vscdb_key(&db, "cursorAuth/accessToken").unwrap();
    assert_eq!(result, "tok_abc123xyz");
}

#[test]
fn vscdb_read_machine_id() {
    let dir = TempDir::new().unwrap();
    let db = create_mock_vscdb(
        dir.path(),
        &[(
            "storage.serviceMachineId",
            "550e8400-e29b-41d4-a716-446655440000",
        )],
    );
    let result = read_vscdb_key(&db, "storage.serviceMachineId").unwrap();
    assert_eq!(result, "550e8400-e29b-41d4-a716-446655440000");
}

#[test]
fn vscdb_missing_key_returns_error() {
    let dir = TempDir::new().unwrap();
    let db = create_mock_vscdb(dir.path(), &[("other/key", "value")]);
    let result = read_vscdb_key(&db, "cursorAuth/accessToken");
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("not found or empty")
    );
}

#[test]
fn vscdb_empty_value_returns_error() {
    let dir = TempDir::new().unwrap();
    let db = create_mock_vscdb(dir.path(), &[("cursorAuth/accessToken", "")]);
    let result = read_vscdb_key(&db, "cursorAuth/accessToken");
    assert!(result.is_err());
}

#[test]
fn vscdb_missing_file_returns_error() {
    let path = PathBuf::from("/tmp/nonexistent_vscdb_test_999.vscdb");
    let result = read_vscdb_key(&path, "cursorAuth/accessToken");
    assert!(result.is_err());
}

#[test]
fn vscdb_multiple_keys() {
    let dir = TempDir::new().unwrap();
    let db = create_mock_vscdb(
        dir.path(),
        &[
            ("cursorAuth/accessToken", "my_token"),
            ("storage.serviceMachineId", "machine_123"),
            ("cursorAuth/refreshToken", "refresh_456"),
            ("cursorAuth/cachedEmail", "user@example.com"),
        ],
    );
    assert_eq!(
        read_vscdb_key(&db, "cursorAuth/accessToken").unwrap(),
        "my_token"
    );
    assert_eq!(
        read_vscdb_key(&db, "storage.serviceMachineId").unwrap(),
        "machine_123"
    );
    assert_eq!(
        read_vscdb_key(&db, "cursorAuth/refreshToken").unwrap(),
        "refresh_456"
    );
    assert_eq!(
        read_vscdb_key(&db, "cursorAuth/cachedEmail").unwrap(),
        "user@example.com"
    );
}

#[test]
fn vscdb_wrong_table_name() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("state.vscdb");
    let status = std::process::Command::new("sqlite3")
        .arg(&db_path)
        .arg("CREATE TABLE WrongTable (key TEXT, value BLOB);")
        .status()
        .unwrap();
    assert!(status.success());
    let result = read_vscdb_key(&db_path, "cursorAuth/accessToken");
    assert!(result.is_err());
}

#[test]
fn vscdb_paths_not_empty() {
    let paths = cursor_vscdb_paths();
    assert!(!paths.is_empty(), "Should have at least one candidate path");
    for path in &paths {
        let s = path.to_string_lossy();
        assert!(
            s.contains("ursor"),
            "Path should contain 'Cursor' or 'cursor'"
        );
        assert!(s.ends_with("state.vscdb"));
    }
}

#[test]
fn find_vscdb_missing_returns_error() {
    let result = find_cursor_vscdb();
    // On this machine Cursor isn't installed, so it should fail
    // (if Cursor IS installed, this test still passes - it finds the file)
    if let Err(err) = result {
        assert!(err.to_string().contains("not found"));
    }
}
