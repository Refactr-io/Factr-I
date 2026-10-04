"""Tests for the Command Installation check in factr doctor."""

import sys
import types
from argparse import Namespace
from pathlib import Path

import pytest

import factr_backend.doctor as doctor_mod


def _setup_doctor_env(monkeypatch, tmp_path, venv_name="venv"):
    """Create a minimal FACTR_CONFIG_HOME + PROJECT_ROOT for doctor tests."""
    home = tmp_path / ".factr"
    home.mkdir(parents=True, exist_ok=True)
    (home / "config.yaml").write_text("memory: {}\n", encoding="utf-8")

    project = tmp_path / "project"
    project.mkdir(exist_ok=True)

    # Create a fake venv entry point
    venv_bin_dir = project / venv_name / "bin"
    venv_bin_dir.mkdir(parents=True, exist_ok=True)
    factr_bin = venv_bin_dir / "factr"
    factr_bin.write_text("#!/usr/bin/env python\n# entry point\n")
    factr_bin.chmod(0o755)

    monkeypatch.setattr(doctor_mod, "FACTR_CONFIG_HOME", home)
    monkeypatch.setattr(doctor_mod, "PROJECT_ROOT", project)
    monkeypatch.setattr(doctor_mod, "_DHH", str(home))

    # Stub model_tools so doctor doesn't fail on import
    fake_model_tools = types.SimpleNamespace(
        check_tool_availability=lambda *a, **kw: ([], []),
        TOOLSET_REQUIREMENTS={},
    )
    monkeypatch.setitem(sys.modules, "model_tools", fake_model_tools)

    # Stub auth checks
    try:
        from factr_backend import auth as _auth_mod
        monkeypatch.setattr(_auth_mod, "get_codex_auth_status", lambda: {})
    except Exception:
        pass

    # Stub httpx.get to avoid network calls
    try:
        import httpx
        monkeypatch.setattr(httpx, "get", lambda *a, **kw: types.SimpleNamespace(status_code=200))
    except Exception:
        pass

    return home, project, factr_bin


def _run_doctor(fix=False):
    """Run doctor and capture stdout."""
    import io
    import contextlib

    buf = io.StringIO()
    with contextlib.redirect_stdout(buf):
        doctor_mod.run_doctor(Namespace(fix=fix))
    return buf.getvalue()


class TestDoctorCommandInstallation:
    """Tests for the ◆ Command Installation section."""





    @pytest.mark.skipif(sys.platform == "win32", reason="Symlink check is Unix-only")
    def test_fix_repairs_wrong_symlink(self, monkeypatch, tmp_path):
        home, project, factr_bin = _setup_doctor_env(monkeypatch, tmp_path)

        # Create a symlink pointing to wrong target
        cmd_link_dir = tmp_path / ".local" / "bin"
        cmd_link_dir.mkdir(parents=True)
        cmd_link = cmd_link_dir / "factr"
        wrong_target = tmp_path / "wrong_factr"
        wrong_target.write_text("#!/usr/bin/env python\n")
        cmd_link.symlink_to(wrong_target)

        monkeypatch.setattr(Path, "home", lambda: tmp_path)

        out = _run_doctor(fix=True)
        assert "Fixed symlink" in out

        # Verify the symlink now points to the correct target
        assert cmd_link.is_symlink()
        assert cmd_link.resolve() == factr_bin.resolve()





