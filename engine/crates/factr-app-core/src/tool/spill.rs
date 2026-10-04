//! The one writer for tool output that does not fit the model's view (bash, webfetch, and the
//! generic history cap in `agent/tools.rs`): content-hash file names (same bytes, same file),
//! owner-only (0600) files, and a 7-day sweep of earlier spills, once per directory per process.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

const MAX_AGE_SECS: u64 = 7 * 24 * 3600;

static SWEPT: LazyLock<Mutex<HashSet<PathBuf>>> = LazyLock::new(Default::default);

fn content_hash(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)[..8].iter().map(|b| format!("{b:02x}")).collect()
}

/// A name this module wrote: `<stem>-<16 hex>.<ext>`. The sweep touches only these, so it never
/// deletes other files that share the scratch directory.
fn is_spill_name(name: &str) -> bool {
    let Some((stem, _ext)) = name.rsplit_once('.') else { return false };
    stem.rsplit_once('-').is_some_and(|(_, h)| h.len() == 16 && h.bytes().all(|b| b.is_ascii_hexdigit()))
}

fn sweep(dir: &Path, max_age: std::time::Duration) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let old = e.file_name().to_str().is_some_and(is_spill_name)
            && e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age > max_age);
        if old {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// Write `bytes` to `<dir>/<stem>-<hash>.<ext>` (0600) and return the path. An existing file of that
/// name is reused, so a repeated output neither writes again nor changes the text that points at it.
pub(crate) fn save(dir: &Path, stem: &str, ext: &str, bytes: &[u8]) -> Option<PathBuf> {
    std::fs::create_dir_all(dir).ok()?;
    if SWEPT.lock().unwrap_or_else(|e| e.into_inner()).insert(dir.to_path_buf()) {
        sweep(dir, std::time::Duration::from_secs(MAX_AGE_SECS));
    }
    let stem: String = stem.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    let path = dir.join(format!("{stem}-{}.{ext}", content_hash(bytes)));
    if !path.exists() {
        use std::io::Write;
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
        opts.open(&path).ok()?.write_all(bytes).ok()?;
    }
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_content_same_file_private_mode_and_old_spills_are_swept_once() {
        let dir = std::env::temp_dir().join(format!("spill-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // An old spill, an old file that is not a spill, and a fresh spill.
        let old = dir.join("bash-0123456789abcdef.txt");
        let foreign = dir.join("notes.txt");
        for p in [&old, &foreign] {
            std::fs::write(p, "x").unwrap();
            std::fs::File::options().write(true).open(p).unwrap().set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(8 * 24 * 3600)).unwrap();
        }
        let a = save(&dir, "ba sh", "txt", b"hello").unwrap();
        let b = save(&dir, "ba sh", "txt", b"hello").unwrap();
        let c = save(&dir, "ba sh", "txt", b"other").unwrap();
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert!(a.file_name().unwrap().to_str().unwrap().starts_with("ba_sh-"));
        assert!(is_spill_name(a.file_name().unwrap().to_str().unwrap()));
        assert!(!old.exists(), "old spill swept");
        assert!(foreign.exists(), "a foreign file is never swept");
        #[cfg(unix)]
        assert_eq!(std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(&a).unwrap().permissions()) & 0o777, 0o600);
        // the sweep ran once for this dir: a spill that ages later is left until the next process
        let later = dir.join("x-fedcba9876543210.txt");
        std::fs::write(&later, "x").unwrap();
        std::fs::File::options().write(true).open(&later).unwrap().set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(8 * 24 * 3600)).unwrap();
        save(&dir, "bash", "txt", b"again").unwrap();
        assert!(later.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
