"""Post-install console-script verification on Windows (issue #52931).

``uv pip install -e .`` can record ``factr.exe`` in the wheel RECORD while the
file never lands, so ``factr`` drops off PATH after a "successful" install.
``_verify_console_scripts_installed`` must notice the missing shim and repair it
with ``--reinstall -e .`` under quarantine. The check is gated on the real host,
so these tests run on the Windows lane.
"""

from __future__ import annotations

import textwrap

import pytest

from factr_backend import main_install_repair

pytestmark = pytest.mark.windows_only

_SCRIPTS = ("factr", "factr-backend", "factr-acp")


@pytest.fixture
def scripts_dir(tmp_path, monkeypatch):
    """A project with three declared console scripts and an empty venv Scripts dir."""
    (tmp_path / "pyproject.toml").write_text(
        textwrap.dedent(
            """\
            [project]
            name = "fake"
            version = "0.0.0"

            [project.scripts]
            factr = "factr_backend.main:main"
            factr-backend = "run_agent:main"
            factr-acp = "acp_adapter.entry:main"
            """
        ),
        encoding="utf-8",
    )
    import factr_backend.main as main_mod

    monkeypatch.setattr(main_mod, "PROJECT_ROOT", tmp_path)
    scripts = tmp_path / "venv" / "Scripts"
    scripts.mkdir(parents=True)
    monkeypatch.setattr(main_install_repair, "_venv_scripts_dir", lambda: scripts)
    return scripts


def _record_installs(monkeypatch, *, lands: tuple[str, ...] = ()):
    """Replace the quarantined uv install; it writes the *lands* shims like a real repair."""
    calls = []

    def fake_install(cmd, *, env=None, scripts_dir=None, **_kwargs):
        calls.append((list(cmd), scripts_dir))
        assert scripts_dir is not None
        for name in lands:
            (scripts_dir / f"{name}.exe").write_bytes(b"shim")

    monkeypatch.setattr(main_install_repair, "_run_quarantined_install", fake_install)
    return calls


def test_missing_factr_exe_is_reinstalled(scripts_dir, monkeypatch, capsys):
    """The #52931 shape: factr-backend/factr-acp landed, factr.exe did not."""
    for name in ("factr-backend", "factr-acp"):
        (scripts_dir / f"{name}.exe").write_bytes(b"shim")
    calls = _record_installs(monkeypatch, lands=("factr",))

    main_install_repair._verify_console_scripts_installed(["uv", "pip"], env={})

    assert calls == [(["uv", "pip", "install", "--reinstall", "-e", "."], scripts_dir)]
    assert (scripts_dir / "factr.exe").is_file()
    out = capsys.readouterr().out
    assert "factr" in out and "missing" in out
    assert "Still missing" not in out


def test_repair_that_still_leaves_a_shim_missing_is_reported(scripts_dir, monkeypatch, capsys):
    """A reinstall that again fails to land the shim must not claim success."""
    for name in ("factr-backend", "factr-acp"):
        (scripts_dir / f"{name}.exe").write_bytes(b"shim")
    _record_installs(monkeypatch)

    main_install_repair._verify_console_scripts_installed(["uv", "pip"], env={})

    out = capsys.readouterr().out
    assert "Still missing after repair: factr" in out
    assert "python -m factr_backend.main" in out


def test_no_reinstall_when_every_shim_is_present(scripts_dir, monkeypatch):
    for name in _SCRIPTS:
        (scripts_dir / f"{name}.exe").write_bytes(b"shim")
    calls = _record_installs(monkeypatch)

    main_install_repair._verify_console_scripts_installed(["uv", "pip"], env={})

    assert calls == []
