"""The packaged desktop app runs the uninstaller from its bundled backend
(``<Resources>/backend-python/source``). That tree is part of the signed bundle and is removed with
the app by the desktop's cleanup script, so the uninstaller must not delete it piecemeal."""
import warnings
from pathlib import Path

from factr_backend import uninstall


def test_is_bundled_source_matches_only_the_app_bundle_layout(tmp_path):
    assert uninstall.is_bundled_source(Path("/Applications/Factr-I.app/Contents/Resources/backend-python/source"))
    assert not uninstall.is_bundled_source(tmp_path / ".factr" / "factr-backend")
    assert not uninstall.is_bundled_source(tmp_path / "source")


def _run(monkeypatch, project_root, factr_home):
    for name in ("uninstall_gateway_service", "remove_path_from_shell_configs", "remove_wrapper_script"):
        monkeypatch.setattr(uninstall, name, lambda *a, **k: [])
    monkeypatch.setattr(uninstall, "remove_node_symlinks", lambda *a, **k: [])
    monkeypatch.setattr(uninstall, "_is_windows", lambda: False)
    monkeypatch.setattr("factr_backend.gui_uninstall.uninstall_gui", lambda *a, **k: [])
    uninstall._perform_uninstall(
        project_root=project_root, factr_home=factr_home, full_uninstall=False,
        remove_profiles=False, named_profiles=[])


def test_keep_data_uninstall_leaves_bundled_source_to_the_app_removal(monkeypatch, tmp_path, capsys):
    source = tmp_path / "Factr-I.app" / "Contents" / "Resources" / "backend-python" / "source"
    source.mkdir(parents=True)
    (tmp_path / "home").mkdir()
    _run(monkeypatch, source, tmp_path / "home")
    assert source.exists()
    assert "part of the app bundle" in capsys.readouterr().out


def test_keep_data_uninstall_still_removes_a_source_install(monkeypatch, tmp_path):
    checkout = tmp_path / "factr-backend"
    checkout.mkdir()
    (tmp_path / "home").mkdir()
    _run(monkeypatch, checkout, tmp_path / "home")
    assert not checkout.exists()


def test_uninstall_module_compiles_without_escape_warnings():
    path = Path(uninstall.__file__)
    with warnings.catch_warnings():
        warnings.simplefilter("error", SyntaxWarning)
        compile(path.read_text(encoding="utf-8"), str(path), "exec")


def _desktop_run(monkeypatch, tmp_path, targets, *, full):
    import base64
    import json

    def forbidden(*_a, **_k):
        raise AssertionError("a desktop run must not touch anything outside its confirmed targets")

    for name in ("uninstall_gateway_service", "remove_path_from_shell_configs", "remove_wrapper_script",
                 "remove_node_symlinks"):
        monkeypatch.setattr(uninstall, name, forbidden)
    monkeypatch.setattr(uninstall, "_is_windows", lambda: False)
    monkeypatch.setattr("factr_backend.gui_uninstall.uninstall_gui", lambda *a, **k: [])
    monkeypatch.setattr(Path, "home", lambda: tmp_path / "home")
    monkeypatch.setenv("FACTR_UNINSTALL_TARGETS_B64",
                       base64.b64encode(json.dumps({"targets": targets}).encode()).decode())
    checkout = tmp_path / "factr-backend"
    config = tmp_path / "factr home"
    for d in (checkout, config, tmp_path / "home"):
        d.mkdir(exist_ok=True)
    uninstall._perform_uninstall(project_root=checkout, factr_home=config, full_uninstall=full,
                                 remove_profiles=False, named_profiles=[])
    return checkout, config


def test_desktop_full_run_removes_only_the_listed_config_home(monkeypatch, tmp_path):
    config = tmp_path / "factr home"
    checkout, config = _desktop_run(
        monkeypatch, tmp_path, [{"kind": "config", "path": str(config)}], full=True)
    assert not config.exists()
    # The checkout was not listed: it stays.
    assert checkout.exists()


def test_desktop_full_run_keeps_an_unlisted_config_home(monkeypatch, tmp_path):
    checkout = tmp_path / "factr-backend"
    checkout, config = _desktop_run(
        monkeypatch, tmp_path, [{"kind": "agent", "path": str(checkout)}], full=True)
    assert config.exists()
    assert not checkout.exists()
