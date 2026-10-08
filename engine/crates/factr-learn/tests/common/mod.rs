//! Shared by the install-then-import tests.
#![allow(dead_code)]
use std::path::{Path, PathBuf};
use std::process::Command;

pub const PKG: &str = "dummyprobe_pkg";

/// A minimal pure-Python wheel, written without any build tooling.
pub fn build_wheel(python: &Path, dir: &Path) -> PathBuf {
    let script = r#"
import sys, zipfile, hashlib, base64
out, name = sys.argv[1], sys.argv[2]
dist = name + "-0.1.dist-info"
files = {
    name + "/__init__.py": "MARK = 'probe-ok'\n",
    dist + "/METADATA": "Metadata-Version: 2.1\nName: " + name + "\nVersion: 0.1\n",
    dist + "/WHEEL": "Wheel-Version: 1.0\nGenerator: probe\nRoot-Is-Purelib: true\nTag: py3-none-any\n",
}
rows = []
for path, text in files.items():
    digest = base64.urlsafe_b64encode(hashlib.sha256(text.encode()).digest()).rstrip(b"=").decode()
    rows.append(f"{path},sha256={digest},{len(text.encode())}")
rows.append(dist + "/RECORD,,")
files[dist + "/RECORD"] = "\n".join(rows) + "\n"
wheel = out + "/" + name + "-0.1-py3-none-any.whl"
with zipfile.ZipFile(wheel, "w") as z:
    for path, text in files.items():
        z.writestr(path, text)
print(wheel)
"#;
    std::fs::create_dir_all(dir).unwrap();
    let made = Command::new(python).args(["-c", script]).arg(dir).arg(PKG).output().unwrap();
    assert!(made.status.success());
    PathBuf::from(String::from_utf8(made.stdout).unwrap().trim())
}

