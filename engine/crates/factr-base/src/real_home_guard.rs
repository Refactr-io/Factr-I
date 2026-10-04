//! A test must never write the developer's real `~/.factr` or `~/.factr/engine`. Writers of `config.yaml` and
//! `factr.db` call [`refuse_real_home`] with `cfg!(test)`; the check is also on when the
//! `FACTR_TEST_GUARD` environment variable is set or the process is a cargo test binary (dependent
//! crates' tests see this crate compiled without `cfg(test)`).

use std::path::Path;

pub use factr_storage::{in_test_binary, is_real_home_path, passwd_home};

/// Panic when `enforce`, `FACTR_TEST_GUARD`, or running inside a cargo test binary is on and `path` is inside a real user home dir.
pub fn refuse_real_home(path: &Path, enforce: bool) {
    if (enforce || std::env::var_os("FACTR_TEST_GUARD").is_some() || in_test_binary()) && is_real_home_path(path) {
        panic!(
            "test guard: {} is inside the real ~/.factr or ~/.factr/engine; point FACTR_CONFIG_HOME/FACTR_HOME at a temp dir under ENV_LOCK",
            path.display()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_home_dirs_are_refused_and_temp_dirs_are_not() {
        let home = passwd_home().expect("passwd home");
        assert!(is_real_home_path(&home.join(".factr").join("config.yaml")));
        assert!(is_real_home_path(&home.join(".factr/engine/sessions/x")));
        assert!(!is_real_home_path(&std::env::temp_dir().join("anything").join("config.yaml")));
        assert!(!is_real_home_path(&home.join(".factr-other").join("config.yaml")));
    }

    #[test]
    #[should_panic(expected = "test guard")]
    fn refuse_panics_on_the_real_home() {
        let home = passwd_home().expect("passwd home");
        refuse_real_home(&home.join(".factr").join("config.yaml"), true);
    }

    #[test]
    fn refuse_is_a_no_op_off_the_real_home() {
        refuse_real_home(&std::env::temp_dir().join("config.yaml"), true);
    }
}
