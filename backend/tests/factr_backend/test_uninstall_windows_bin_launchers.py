"""Uninstall must not leave a dangling ``factr`` command on Windows.

Every uninstall mode deletes the code checkout, but the launchers install.ps1
staged in the managed binary dir (the default Factr root's ``bin``, shared
with the managed uv) live outside it. A surviving launcher makes ``factr``
in a new terminal resolve and then error on its missing venv target — worse
than command-not-found. The managed uv next to them must survive keep-data
uninstalls, so the PATH sweep takes the ``bin`` entry only on a full wipe.

Platform verdicts are injected parameters (input→output, not host fakes).
"""
from __future__ import annotations

from pathlib import Path

import pytest

from factr_backend import uninstall
from factr_backend._install_repair import _WINDOWS_BIN_LAUNCHERS


@pytest.fixture
def managed_bin(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> Path:
    """Default-root ``bin`` holding launchers of both forms plus managed uv."""
    home = tmp_path / "factr"
    bin_dir = home / "bin"
    bin_dir.mkdir(parents=True)
    (bin_dir / "factr.exe").write_bytes(b"MZ launcher")
    (bin_dir / "factr-acp.cmd").write_text("@echo off\r\n", encoding="ascii")
    (bin_dir / "uv.exe").write_bytes(b"MZ managed uv")
    (bin_dir / "uvx.exe").write_bytes(b"MZ managed uvx")
    monkeypatch.setenv("FACTR_CONFIG_HOME", str(home))
    return bin_dir


def test_removes_both_launcher_forms_and_keeps_managed_uv(managed_bin: Path):
    removed = uninstall.remove_windows_bin_launchers(windows=True)

    assert sorted(p.name for p in removed) == ["factr-acp.cmd", "factr.exe"]
    assert not (managed_bin / "factr.exe").exists()
    assert not (managed_bin / "factr-acp.cmd").exists()
    # The managed uv stays — keep-data reinstalls still need it.
    assert (managed_bin / "uv.exe").exists()
    assert (managed_bin / "uvx.exe").exists()


def test_anchors_on_default_root_not_profile_home(
    managed_bin: Path, monkeypatch: pytest.MonkeyPatch
):
    """The launcher dir is per-machine; a profile FACTR_CONFIG_HOME must not
    redirect the sweep into ``profiles/<name>/bin``."""
    home = managed_bin.parent
    monkeypatch.setenv("FACTR_CONFIG_HOME", str(home / "profiles" / "work"))

    removed = uninstall.remove_windows_bin_launchers(windows=True)

    assert sorted(p.name for p in removed) == ["factr-acp.cmd", "factr.exe"]
    assert not (managed_bin / "factr.exe").exists()


def test_noop_on_posix(managed_bin: Path):
    assert uninstall.remove_windows_bin_launchers(windows=False) == []
    assert (managed_bin / "factr.exe").exists()


def test_noop_when_no_launchers_staged(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    home = tmp_path / "factr"
    home.mkdir()
    monkeypatch.setenv("FACTR_CONFIG_HOME", str(home))

    assert uninstall.remove_windows_bin_launchers(windows=True) == []


class TestManagedBinPathMarker:
    """The managed ``bin`` PATH entry goes only when the dir itself goes.

    Markers match against Windows registry PATH entries, so the inputs here
    are Windows-shaped path strings regardless of the host — feeding
    ``tmp_path`` would make the test pass only on Windows hosts.
    """

    HOME = r"C:\Users\me\AppData\Local\factr"
    BIN_ENTRY = r"C:\Users\me\AppData\Local\factr\bin"

    def test_keep_data_markers_spare_the_managed_bin(self):
        markers = [m.lower() for m in uninstall._factr_path_markers(Path(self.HOME))]

        assert not any(self.BIN_ENTRY.lower().startswith(m) for m in markers)

    def test_full_wipe_markers_take_the_managed_bin(self):
        markers = [
            m.lower()
            for m in uninstall._factr_path_markers(
                Path(self.HOME), include_managed_bin=True
            )
        ]

        assert any(self.BIN_ENTRY.lower().startswith(m) for m in markers)
