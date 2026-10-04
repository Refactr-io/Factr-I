"""Update failure classification: only Windows git breakage points users at the releases page."""

from __future__ import annotations

import subprocess
from factr_backend import main as factr_main
import factr_backend.main_install_repair as main_install_repair
from factr_backend import update_cmd


def _cpe(cmd, returncode=2, stderr="", stdout="") -> subprocess.CalledProcessError:
    exc = subprocess.CalledProcessError(returncode, cmd)
    exc.stderr = stderr
    exc.stdout = stdout
    return exc


# ---------------------------------------------------------------------------
# Stage classification
# ---------------------------------------------------------------------------


def test_uv_pip_install_is_a_dependency_failure_not_git():
    exc = _cpe([r"C:\venv\Scripts\uv.exe", "pip", "install", "-e", "."])
    assert update_cmd._called_process_error_is_git(exc) is False
    assert update_cmd._called_process_error_is_python_dep_install(exc) is True
    assert update_cmd._format_update_failure_stage(exc) == (
        "Python dependency install failed"
    )


def test_venv_pip_install_is_a_dependency_failure():
    exc = _cpe([r"C:\venv\Scripts\python.exe", "-m", "pip", "install", "-e", "."])
    assert update_cmd._called_process_error_is_python_dep_install(exc) is True
    assert update_cmd._called_process_error_is_git(exc) is False


def test_ensurepip_is_a_dependency_failure():
    exc = _cpe([r"C:\venv\Scripts\python.exe", "-m", "ensurepip", "--upgrade"])
    assert update_cmd._called_process_error_is_python_dep_install(exc) is True
    assert update_cmd._format_update_failure_stage(exc) == (
        "Python dependency install failed"
    )


def test_git_pull_is_classified_as_git():
    exc = _cpe(["git", "-c", "windows.appendAtomically=false", "pull"], returncode=1)
    assert update_cmd._called_process_error_is_git(exc) is True
    assert update_cmd._called_process_error_is_python_dep_install(exc) is False
    assert update_cmd._format_update_failure_stage(exc) == "Git update failed"


def test_git_exe_path_is_still_git():
    exc = _cpe([r"C:\Program Files\Git\cmd\git.exe", "fetch", "origin", "main"])
    assert update_cmd._called_process_error_is_git(exc) is True


def test_unknown_command_gets_generic_stage():
    exc = _cpe(["npm", "install"], returncode=1)
    assert update_cmd._format_update_failure_stage(exc) == "Update step failed"






def test_posix_git_failure_is_not_windows_git_breakage(monkeypatch):
    monkeypatch.setattr(factr_main, "_is_windows", lambda: False)
    monkeypatch.setattr(main_install_repair, "_is_windows", lambda: False)
    exc = _cpe(["git", "pull"], returncode=1)
    assert update_cmd._is_windows_git_breakage(exc) is False
