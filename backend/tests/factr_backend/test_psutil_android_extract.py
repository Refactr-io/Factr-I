"""Regression tests for the Android psutil compatibility installer."""

from __future__ import annotations

import io
import shutil
import tarfile
from pathlib import Path
from unittest.mock import patch

import pytest

from factr_backend.psutil_android import (
    MARKER,
    REPLACEMENT,
    PSUTIL_URL,
    PsutilAndroidInstallError,
    prepare_patched_psutil_sdist,
)
from factr_backend import update_cmd


def _add_dir(tf: tarfile.TarFile, name: str) -> None:
    info = tarfile.TarInfo(name)
    info.type = tarfile.DIRTYPE
    info.mode = 0o755
    tf.addfile(info)


def _add_file(tf: tarfile.TarFile, name: str, content: str) -> None:
    payload = content.encode("utf-8")
    info = tarfile.TarInfo(name)
    info.size = len(payload)
    info.mode = 0o644
    tf.addfile(info, io.BytesIO(payload))


def _build_psutil_archive(archive: Path, *, malicious_symlink: bool) -> None:
    with tarfile.open(archive, "w:gz") as tf:
        _add_dir(tf, "psutil-7.2.2")
        if malicious_symlink:
            link = tarfile.TarInfo("psutil-7.2.2/psutil")
            link.type = tarfile.SYMTYPE
            link.linkname = "../../outside"
            tf.addfile(link)
        else:
            _add_dir(tf, "psutil-7.2.2/psutil")
        _add_file(
            tf,
            "psutil-7.2.2/psutil/_common.py",
            f"{MARKER}\n",
        )


def test_prepare_patched_psutil_sdist_rejects_symlink_member(tmp_path):
    """A symlink member must be rejected before any file payload is written."""
    archive = tmp_path / "evil.tar.gz"
    _build_psutil_archive(archive, malicious_symlink=True)

    destination = tmp_path / "extract"
    with pytest.raises(PsutilAndroidInstallError, match="Unsupported archive member type"):
        prepare_patched_psutil_sdist(archive, destination)

    assert not (tmp_path / "outside" / "_common.py").exists()


def test_prepare_patched_psutil_sdist_rejects_traversal_member(tmp_path):
    """A ``..`` member must be refused the same way the shared archive guard refuses it
    (one traversal check for every tar.gz we extract), surfaced as the installer's own error."""
    archive = tmp_path / "evil.tar.gz"
    with tarfile.open(archive, "w:gz") as tf:
        _add_dir(tf, "psutil-7.2.2")
        _add_file(tf, "psutil-7.2.2/../escaped.py", "x")

    with pytest.raises(PsutilAndroidInstallError, match="Unsafe archive member path"):
        prepare_patched_psutil_sdist(archive, tmp_path / "extract")

    assert not (tmp_path / "escaped.py").exists()


def test_install_psutil_android_compat_uses_patched_tree(tmp_path):
    """Updater path should install from the patched temporary sdist tree."""
    archive = tmp_path / "psutil.tar.gz"
    _build_psutil_archive(archive, malicious_symlink=False)

    from factr_backend import main as factr_main

    captured: dict[str, object] = {}

    def fake_urlretrieve(url: str, dest: Path):
        assert url == PSUTIL_URL
        shutil.copyfile(archive, dest)
        return str(dest), None

    def fake_run_install(cmd: list[str], *, env=None):
        src_root = Path(cmd[-1])
        captured["cmd"] = cmd
        captured["env"] = env
        captured["common_py"] = (src_root / "psutil" / "_common.py").read_text(
            encoding="utf-8"
        )

    with patch("urllib.request.urlretrieve", side_effect=fake_urlretrieve), \
         patch.object(factr_main, "_run_install_with_heartbeat", side_effect=fake_run_install):
        update_cmd._install_psutil_android_compat(
            ["uv", "pip"],
            env={"FACTR_TEST": "1"},
        )

    assert captured["cmd"][:4] == ["uv", "pip", "install", "--no-build-isolation"]
    assert captured["env"] == {"FACTR_TEST": "1"}
    assert REPLACEMENT in str(captured["common_py"])
