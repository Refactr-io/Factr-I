use std::sync::{LazyLock, Mutex, RwLock};

use factr_provider_metadata::is_safe_env_key_name;

/// Override resolvers consulted by [`load_api_key`] BEFORE the environment: a key an override
/// owner defines (Factr `.env`) always wins, and the process environment only fills the gaps it
/// leaves. Higher-level crates register resolvers at startup so this leaf crate does not need to
/// depend on auth.
type ApiKeyOverrideResolver = fn(&str) -> Option<String>;

static API_KEY_OVERRIDE_RESOLVERS: LazyLock<RwLock<Vec<ApiKeyOverrideResolver>>> =
    LazyLock::new(|| RwLock::new(Vec::new()));

/// Register an API-key resolver that takes precedence over the environment.
pub fn register_api_key_override_resolver(resolver: ApiKeyOverrideResolver) {
    API_KEY_OVERRIDE_RESOLVERS
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .push(resolver);
}

static ENV_ONLY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Private deployment (`FACTR_DEPLOYMENT=private`): keys come from the process environment ONLY,
/// never from an override resolver (Factr `.env`).
pub fn set_env_only(on: bool) {
    ENV_ONLY.store(on, std::sync::atomic::Ordering::Relaxed);
}

pub fn env_only() -> bool {
    ENV_ONLY.load(std::sync::atomic::Ordering::Relaxed)
}

fn resolve_api_key_override(env_key: &str) -> Option<String> {
    if env_only() {
        return None;
    }
    let resolvers = API_KEY_OVERRIDE_RESOLVERS
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    for resolver in resolvers.iter() {
        if let Some(key) = resolver(env_key) {
            return Some(key);
        }
    }
    None
}

/// Characters that editors, terminals, and `cat` render invisibly but that
/// corrupt a credential when embedded in it. Rust's [`str::trim`] only removes
/// ASCII whitespace, so these survive a plain trim and silently break auth
/// (see GitHub issue #376). [`char::is_whitespace`] covers Unicode White_Space
/// (NBSP U+00A0, the en/em spaces U+2002-U+200A, line/paragraph separators,
/// etc.); the explicit cases below are zero-width characters and the BOM, which
/// are not classified as whitespace.
fn is_invisible_boundary_char(c: char) -> bool {
    c.is_whitespace()
        || matches!(
            c,
            '\u{200B}' // zero-width space
                | '\u{200C}' // zero-width non-joiner
                | '\u{200D}' // zero-width joiner
                | '\u{2060}' // word joiner
                | '\u{FEFF}' // BOM / zero-width no-break space
        )
}

/// Strip leading/trailing invisible (Unicode whitespace and zero-width)
/// characters and one optional layer of surrounding quotes from a loaded
/// secret or config value.
///
/// Exposed so other credential loaders (e.g. the Cursor key reader) can apply
/// the same sanitizing as [`load_api_key`].
pub fn sanitize_secret_value(raw: &str) -> &str {
    raw.trim_matches(is_invisible_boundary_char)
        .trim_matches('"')
        .trim_matches('\'')
        .trim_matches(is_invisible_boundary_char)
}

/// Sanitize a loaded value and surface a warning when Unicode invisible
/// characters were present, so the failure mode in issue #376 is no longer
/// silent. Returns `None` for values that are empty after sanitizing.
fn clean_loaded_value(raw: &str, env_key: &str) -> Option<String> {
    let cleaned = sanitize_secret_value(raw);
    if cleaned.is_empty() {
        return None;
    }
    // A plain ASCII trim is what we previously did; if it leaves a different
    // result than the Unicode-aware sanitize, hidden characters were stripped.
    let ascii_only = raw.trim().trim_matches('"').trim_matches('\'').trim();
    if ascii_only != cleaned {
        factr_logging::warn(&format!(
            "Stripped Unicode invisible or non-ASCII whitespace characters from '{}' while loading credentials; verify the value contains no hidden characters",
            env_key
        ));
    }
    Some(cleaned.to_string())
}

/// `(variable, value)` pairs that were once served from the override owner or the process environment,
/// hashed (no secrets kept). Such a key can be re-read later, so its absence means it was removed.
static LIVE_VALUES: LazyLock<Mutex<std::collections::HashSet<u64>>> = LazyLock::new(Default::default);

fn live_hash(env_key: &str, value: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (env_key, value).hash(&mut hasher);
    hasher.finish()
}

fn note_live(env_key: &str, value: &str) {
    LIVE_VALUES.lock().unwrap_or_else(|e| e.into_inner()).insert(live_hash(env_key, value));
}

/// Whether `value` was ever read for `env_key` from the override owner / process env (so it can be re-read).
pub fn was_live(env_key: &str, value: &str) -> bool {
    LIVE_VALUES.lock().unwrap_or_else(|e| e.into_inner()).contains(&live_hash(env_key, value))
}

/// The key to use now for a runtime that was built with `built`: the live value, else `built` -
/// unless `built` itself came from a live source that has since been removed (a disconnected key
/// must stop authenticating), which is `None`.
pub fn current_secret(env_key: &str, built: &str) -> Option<String> {
    env_secret(env_key).or_else(|| (!was_live(env_key, built)).then(|| built.to_string()))
}

/// A credential variable as the engine sees it: the override owner's value (Factr `.env`), else the
/// process environment. For direct reads and auth-status probes that have no env file of their own;
/// everything else goes through [`load_api_key`].
pub fn env_secret(env_key: &str) -> Option<String> {
    if !is_safe_env_key_name(env_key) {
        return None;
    }
    let value = resolve_api_key_override(env_key)
        .or_else(|| std::env::var(env_key).ok().and_then(|value| clean_loaded_value(&value, env_key)))?;
    note_live(env_key, &value);
    Some(value)
}

/// The credential `env_key` names: the override owner's value (Factr `.env`), else the process
/// environment. `ZHIPU_API_KEY` also accepts the legacy `ZAI_API_KEY`.
pub fn load_api_key(env_key: &str) -> Option<String> {
    if !is_safe_env_key_name(env_key) {
        factr_logging::warn(&format!(
            "Ignoring invalid API key variable name '{}' while loading credentials",
            env_key
        ));
        return None;
    }

    if let Some(key) = resolve_api_key_override(env_key) {
        note_live(env_key, &key);
        return Some(key);
    }

    if let Ok(key) = std::env::var(env_key)
        && let Some(key) = clean_loaded_value(&key, env_key)
    {
        note_live(env_key, &key);
        return Some(key);
    }

    if env_key == "ZHIPU_API_KEY"
        && let Ok(key) = std::env::var("ZAI_API_KEY")
        && let Some(key) = clean_loaded_value(&key, "ZAI_API_KEY")
    {
        return Some(key);
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::sync::{Mutex, MutexGuard};

    pub(super) static ENV_LOCK: Mutex<()> = Mutex::new(());

    struct EnvGuard {
        _lock: MutexGuard<'static, ()>,
        saved: Vec<(&'static str, Option<OsString>)>,
    }

    impl EnvGuard {
        fn new(keys: &[&'static str]) -> Self {
            let lock = ENV_LOCK
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let saved = keys
                .iter()
                .map(|key| (*key, std::env::var_os(key)))
                .collect::<Vec<_>>();
            for key in keys {
                factr_core::env::remove_var(key);
            }
            Self { _lock: lock, saved }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (key, value) in self.saved.drain(..) {
                match value {
                    Some(value) => factr_core::env::set_var(key, value),
                    None => factr_core::env::remove_var(key),
                }
            }
        }
    }

    #[test]
    fn loads_api_key_and_values_from_the_environment() {
        let _guard = EnvGuard::new(&["FACTR_PROVIDER_ENV_TEST_KEY", "FACTR_PROVIDER_ENV_TEST_VALUE"]);
        assert_eq!(load_api_key("FACTR_PROVIDER_ENV_TEST_KEY"), None);
        factr_core::env::set_var("FACTR_PROVIDER_ENV_TEST_KEY", "env-key");
        factr_core::env::set_var("FACTR_PROVIDER_ENV_TEST_VALUE", "  env-value ");
        assert_eq!(load_api_key("FACTR_PROVIDER_ENV_TEST_KEY").as_deref(), Some("env-key"));
        assert_eq!(env_secret("FACTR_PROVIDER_ENV_TEST_VALUE").as_deref(), Some("env-value"));
    }

    /// The old per-provider `<config dir>/*.env` files are no longer a credential source.
    #[test]
    fn a_key_only_in_a_legacy_provider_env_file_is_not_found() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _guard = EnvGuard::new(&["FACTR_HOME", "FACTR_PROVIDER_LEGACY_API_KEY"]);
        factr_core::env::set_var("FACTR_HOME", temp.path());
        let config_dir = factr_storage::app_config_dir().expect("config dir");
        std::fs::create_dir_all(&config_dir).expect("create config dir");
        std::fs::write(config_dir.join("legacy.env"), "FACTR_PROVIDER_LEGACY_API_KEY=file-key\n").expect("write env file");
        assert_eq!(load_api_key("FACTR_PROVIDER_LEGACY_API_KEY"), None);
        assert_eq!(env_secret("FACTR_PROVIDER_LEGACY_API_KEY"), None);
    }

    #[test]
    fn accepts_legacy_zai_key_for_zhipu() {
        let _guard = EnvGuard::new(&["ZHIPU_API_KEY", "ZAI_API_KEY"]);
        factr_core::env::set_var("ZAI_API_KEY", "legacy-zai-key");
        assert_eq!(load_api_key("ZHIPU_API_KEY").as_deref(), Some("legacy-zai-key"));
        factr_core::env::set_var("ZHIPU_API_KEY", "zhipu-key");
        assert_eq!(load_api_key("ZHIPU_API_KEY").as_deref(), Some("zhipu-key"));
    }

    #[test]
    fn sanitize_strips_unicode_invisible_characters() {
        // Zero-width space, BOM, NBSP, en space around the value.
        assert_eq!(
            sanitize_secret_value("\u{200B}sk-key123\u{FEFF}"),
            "sk-key123"
        );
        assert_eq!(sanitize_secret_value("\u{00A0}sk-key\u{2002}"), "sk-key");
        // Quotes plus invisible padding both stripped.
        assert_eq!(
            sanitize_secret_value("\u{FEFF}\"sk-quoted\"\u{200B}"),
            "sk-quoted"
        );
        // Interior characters are preserved.
        assert_eq!(
            sanitize_secret_value("sk-mid\u{200B}dle"),
            "sk-mid\u{200B}dle"
        );
        // Empty after sanitize.
        assert_eq!(sanitize_secret_value("\u{200B}\u{FEFF}"), "");
    }

    #[test]
    fn loads_api_key_with_invisible_chars_from_env_var() {
        let _guard = EnvGuard::new(&["FACTR_PROVIDER_BAR_API_KEY"]);
        // NBSP + BOM padding around the env-provided key.
        factr_core::env::set_var("FACTR_PROVIDER_BAR_API_KEY", "\u{00A0}sk-env-key\u{FEFF}");

        assert_eq!(
            load_api_key("FACTR_PROVIDER_BAR_API_KEY").as_deref(),
            Some("sk-env-key")
        );
    }
}

#[cfg(test)]
mod private_mode_tests {
    use super::*;

    fn fake_owner(key: &str) -> Option<String> {
        (key == "FAKE_PRIVATE_KEY").then(|| "from-owner".to_string())
    }

    #[test]
    fn env_only_reads_the_environment_and_never_the_override_owner() {
        let _g = super::tests::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        register_api_key_override_resolver(fake_owner);
        assert_eq!(load_api_key("FAKE_PRIVATE_KEY").as_deref(), Some("from-owner"));
        set_env_only(true);
        assert_eq!(load_api_key("FAKE_PRIVATE_KEY"), None);
        // SAFETY: serialised by the lock above.
        unsafe { std::env::set_var("FAKE_PRIVATE_KEY", "from-env") };
        assert_eq!(load_api_key("FAKE_PRIVATE_KEY").as_deref(), Some("from-env"));
        set_env_only(false);
        unsafe { std::env::remove_var("FAKE_PRIVATE_KEY") };
    }
}
