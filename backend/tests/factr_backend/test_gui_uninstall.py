"""Tests for factr_backend.gui_uninstall — GUI-only uninstall + install discovery.

Covers the cross-platform artifact discovery, the agent/GUI detection the
desktop UI gates options on, and that ``uninstall_gui`` removes only GUI
artifacts (built renderer/release/node_modules, packaged bundle, Electron
userData) while leaving the Python agent + config/sessions/.env intact.
"""

import sys
from pathlib import Path

import pytest

import factr_backend.gui_uninstall as gu


def _make_agent(factr_home: Path) -> Path:
    """Create a fake agent install: source package + venv."""
    agent_root = factr_home / "factr-backend"
    (agent_root / "factr_backend").mkdir(parents=True)
    (agent_root / "factr_backend" / "__init__.py").write_text("")
    (agent_root / "venv" / "bin").mkdir(parents=True)
    return agent_root


def _make_gui_build(factr_home: Path) -> None:
    """Create the source-built GUI artifacts a `factr desktop` run produces."""
    desktop = gu.desktop_app_dir(factr_home / "factr-backend")
    (desktop / "dist").mkdir(parents=True)
    (desktop / "dist" / "index.html").write_text("<html>")
    (desktop / "release" / "linux-unpacked").mkdir(parents=True)
    (desktop / "node_modules").mkdir(parents=True)
    (factr_home / "factr-backend" / "node_modules").mkdir(parents=True)
    (factr_home / "desktop-build-stamp.json").write_text("{}")


def test_gui_install_summary_shape(tmp_path, monkeypatch):
    factr_home = tmp_path / ".factr"
    _make_agent(factr_home)
    _make_gui_build(factr_home)
    monkeypatch.setattr(gu, "packaged_gui_app_paths", lambda: [])
    monkeypatch.setattr(gu, "desktop_userdata_dir", lambda: tmp_path / "none")

    summary = gu.gui_install_summary(factr_home)
    # JSON-serializable primitives the desktop UI gates on.
    assert summary["agent_installed"] is True
    assert summary["gui_installed"] is True
    assert isinstance(summary["source_built_artifacts"], list)
    assert all(isinstance(p, str) for p in summary["source_built_artifacts"])
    assert summary["factr_home"] == str(factr_home)
    assert summary["platform"] == sys.platform


@pytest.mark.linux_only
def test_uninstall_removes_launcher_entry_and_refreshes_cache(tmp_path, monkeypatch):
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path / "xdg"))

    from factr_backend import linux_desktop_entry as lde

    entry = lde.desktop_entry_path()
    entry.parent.mkdir(parents=True, exist_ok=True)
    entry.write_text("x", encoding="utf-8")

    refreshed: list[Path] = []
    monkeypatch.setattr(
        lde, "refresh_desktop_databases", lambda d: refreshed.append(d) or ["kbuildsycoca6"]
    )

    factr_home = tmp_path / ".factr"
    _make_agent(factr_home)
    icon = lde.icon_path(factr_home / "factr-backend")
    icon.parent.mkdir(parents=True, exist_ok=True)
    icon.write_bytes(b"\x89PNG")
    monkeypatch.setattr(gu, "desktop_userdata_dir", lambda: tmp_path / "none")

    removed = gu.uninstall_gui(factr_home)

    assert entry in removed and not entry.exists()
    assert refreshed == [entry.parent]
    # The icon lives in the checkout. A GUI uninstall must not delete it.
    assert lde.icon_path(factr_home / "factr-backend").exists()
    # The agent itself survives a GUI uninstall.
    assert (factr_home / "factr-backend" / "factr_backend").is_dir()


@pytest.mark.skipif(sys.platform == "win32", reason="POSIX symlink semantics")
def test_remove_path_handles_symlink(tmp_path):
    target = tmp_path / "real"
    target.mkdir()
    link = tmp_path / "link"
    link.symlink_to(target)
    assert gu._remove_path(link) is True
    assert not link.exists()
    # The symlink is gone but its target is untouched.
    assert target.exists()


def test_uninstall_args_namespace_mode_mapping():
    """_UninstallArgs maps mode → the gui/full flags run_uninstall reads."""
    import factr_backend.uninstall as uninstall

    gui = uninstall._UninstallArgs(mode="gui")
    assert gui.gui is True and gui.full is False and gui.yes is True

    lite = uninstall._UninstallArgs(mode="lite")
    assert lite.gui is False and lite.full is False and lite.yes is True

    full = uninstall._UninstallArgs(mode="full")
    assert full.gui is False and full.full is True and full.yes is True



def test_userdata_dir_is_factr_i_and_never_the_engine_config_folder(monkeypatch, tmp_path):
    """userData is "Factr-I"; the engine's own folder is "factr" and must never be targeted."""
    monkeypatch.setattr(Path, "home", lambda: tmp_path)
    monkeypatch.setenv("APPDATA", str(tmp_path / "Roaming"))
    monkeypatch.setenv("XDG_CONFIG_HOME", str(tmp_path / "cfg"))
    assert gu.desktop_userdata_dir().name == "Factr-I"
    assert gu.desktop_userdata_dir().name.lower() != "factr"


def test_packaged_app_names_are_factr_i(monkeypatch, tmp_path):
    monkeypatch.setattr(Path, "home", lambda: tmp_path)
    monkeypatch.setenv("LOCALAPPDATA", str(tmp_path / "Local"))
    monkeypatch.setenv("ProgramFiles", str(tmp_path / "PF"))
    names = {p.name for p in gu.packaged_gui_app_paths()}
    assert not ({"Factr.app", "Factr", "Factr.desktop"} & names)


# --- D-2: a desktop-launched run deletes exactly what its confirmation listed ---------------------------

def _encode(targets):
    import base64
    import json
    return base64.b64encode(json.dumps({"targets": targets}).encode()).decode()


@pytest.fixture
def fake_home(tmp_path, monkeypatch):
    """Point every home lookup at a temp dir, so nothing here can reach a real install."""
    home = tmp_path / "home"
    home.mkdir()
    monkeypatch.setattr(Path, "home", lambda: home)
    monkeypatch.delenv("FACTR_UNINSTALL_TARGETS_B64", raising=False)
    monkeypatch.delenv("FACTR_UNINSTALL_USERDATA", raising=False)
    return home


def test_desktop_run_never_targets_standard_app_locations(fake_home, monkeypatch):
    monkeypatch.setenv("FACTR_UNINSTALL_TARGETS_B64", _encode([]))
    assert gu.packaged_gui_app_paths() == []


def test_desktop_run_removes_the_listed_userdata_only(fake_home, tmp_path, monkeypatch):
    # The running copy's userData is overridden (FACTR_DESKTOP_USER_DATA_DIR) and has spaces in its path.
    listed = tmp_path / "QA copies" / "run 1" / "user data"
    listed.mkdir(parents=True)
    (listed / "Local Storage").mkdir()
    # The default location belongs to another copy (the founder's own app): it must survive.
    default = fake_home / "Library" / "Application Support" / "Factr-I"
    default.mkdir(parents=True)
    factr_home = tmp_path / "factr home"
    factr_home.mkdir()
    (factr_home / "config.yaml").write_text("model: {}\n")
    monkeypatch.setattr(gu, "packaged_gui_app_paths", lambda: [])
    monkeypatch.setenv("FACTR_UNINSTALL_TARGETS_B64", _encode([{"kind": "userdata", "path": str(listed)}]))

    removed = gu.uninstall_gui(factr_home)

    assert removed == [listed]
    assert not listed.exists()
    assert default.exists()
    assert (factr_home / "config.yaml").exists()


def test_unreadable_desktop_targets_delete_nothing(fake_home, tmp_path, monkeypatch):
    default = fake_home / "Library" / "Application Support" / "Factr-I"
    default.mkdir(parents=True)
    monkeypatch.setattr(gu, "packaged_gui_app_paths", lambda: [])
    monkeypatch.setenv("FACTR_UNINSTALL_TARGETS_B64", "not base64 json!")

    assert gu.uninstall_gui(tmp_path / "factr") == []
    assert default.exists()


def test_summary_probe_reports_the_running_copys_userdata(fake_home, tmp_path, monkeypatch):
    override = tmp_path / "custom user data"
    monkeypatch.setenv("FACTR_UNINSTALL_USERDATA", str(override))
    assert gu.desktop_userdata_dir() == override


@pytest.mark.skipif(sys.platform == "win32", reason="POSIX paths")
def test_refusal_reason_protects_system_and_home_folders(fake_home, tmp_path):
    for path in (Path("/"), Path("/Applications"), Path("/Users"), fake_home, fake_home.parent,
                 fake_home / "Library" / "Application Support"):
        assert gu.refusal_reason(path), path
    assert gu.refusal_reason(Path("relative/dir"))
    assert gu.refusal_reason(fake_home / "Library" / "Application Support" / "Factr-I") is None
    assert gu.refusal_reason(tmp_path / "QA copy.app") is None


@pytest.mark.skipif(sys.platform == "win32", reason="POSIX symlink semantics")
def test_remove_path_refuses_home_and_a_link_into_it(fake_home, tmp_path):
    assert gu._remove_path(fake_home) is False
    assert fake_home.exists()
    link = tmp_path / "userdata-link"
    link.symlink_to(fake_home)
    assert gu._remove_path(link) is False
    assert fake_home.exists()
