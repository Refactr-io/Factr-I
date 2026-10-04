"""Tests for get_factr_home() profile-mode fallback warning.

Regression test for https://github.com/Refactr-io/Factr-I

When FACTR_CONFIG_HOME is unset but an active_profile file indicates a non-default
profile is active, get_factr_home() should:
  1. STILL return ~/.factr (raising would brick 30+ module-level callers)
  2. Emit a loud one-shot warning to stderr so operators can diagnose
     cross-profile data contamination after the fact.

The warning goes to stderr directly (not through logging) because this
function is called at module-import time from 30+ sites, often before the
logging subsystem has been configured.
"""

from pathlib import Path

import pytest


@pytest.fixture
def fresh_constants(monkeypatch, tmp_path):
    """Import factr_constants fresh and reset the one-shot warn flag."""
    import importlib
    import factr_constants
    importlib.reload(factr_constants)
    monkeypatch.setattr(Path, "home", lambda: tmp_path)
    monkeypatch.delenv("FACTR_CONFIG_HOME", raising=False)
    return factr_constants


class TestGetFactrHomeProfileWarning:
    def test_classic_mode_no_active_profile_no_warning(
        self, fresh_constants, tmp_path, capsys
    ):
        """Classic mode: no active_profile file → silent, returns ~/.factr."""
        result = fresh_constants.get_factr_home()
        assert result == tmp_path / ".factr"
        assert "FACTR_CONFIG_HOME fallback" not in capsys.readouterr().err


    def test_named_profile_unset_home_warns_once(
        self, fresh_constants, tmp_path, capsys
    ):
        """active_profile=coder + FACTR_CONFIG_HOME unset → warn loudly, still return fallback."""
        factr_dir = tmp_path / ".factr"
        factr_dir.mkdir()
        (factr_dir / "active_profile").write_text("coder\n")

        result = fresh_constants.get_factr_home()

        # 1. Still returns the fallback — no import-time crash
        assert result == tmp_path / ".factr"
        # 2. Stderr got the warning exactly once
        err = capsys.readouterr().err
        assert err.count("FACTR_CONFIG_HOME fallback") == 1
        assert "'coder'" in err

        # 3. One-shot: second and third calls don't re-warn
        fresh_constants.get_factr_home()
        fresh_constants.get_factr_home()
        err2 = capsys.readouterr().err
        assert "FACTR_CONFIG_HOME fallback" not in err2

    def test_factr_home_set_suppresses_warning(
        self, fresh_constants, tmp_path, capsys, monkeypatch
    ):
        """Even if active_profile is 'coder', setting FACTR_CONFIG_HOME suppresses warning."""
        profile_dir = tmp_path / ".factr" / "profiles" / "coder"
        profile_dir.mkdir(parents=True)
        (tmp_path / ".factr" / "active_profile").write_text("coder\n")
        monkeypatch.setenv("FACTR_CONFIG_HOME", str(profile_dir))

        result = fresh_constants.get_factr_home()

        assert result == profile_dir
        assert "FACTR_CONFIG_HOME fallback" not in capsys.readouterr().err

    def test_unreadable_active_profile_no_crash(
        self, fresh_constants, tmp_path, capsys
    ):
        """active_profile that can't be decoded → fall through silently."""
        factr_dir = tmp_path / ".factr"
        factr_dir.mkdir()
        # Write bytes that aren't valid utf-8
        (factr_dir / "active_profile").write_bytes(b"\xff\xfe\x00\x00")

        result = fresh_constants.get_factr_home()

        assert result == tmp_path / ".factr"
        # Shouldn't crash; shouldn't warn either (can't tell what profile was intended)
        assert "FACTR_CONFIG_HOME fallback" not in capsys.readouterr().err


class TestBootReadersBeforeProfileOverride:
    """Readers that run before the CLI applies the sticky ``active_profile`` must not warn.

    ``factr_bootstrap`` points ``TMPDIR`` at the scratch dir of the *process* home during
    import, and ``main._apply_profile_override`` re-homes the process a few lines later. A
    caller that already resolved its home must not send the policy back through
    ``get_factr_home()``: for a sticky-profile user with ``FACTR_CONFIG_HOME`` unset in a plain
    shell that lookup falls back to the default profile and warns, on every ``factr``
    command, while nothing lands in the wrong place. Same fix the parser's ``_cfg_path()``
    carries for the ``--no-config`` help string.
    """

    def test_scratch_export_uses_the_process_home_silently(
        self, fresh_constants, tmp_path, capsys
    ):
        """Boot scratch setup: silent, and pointed at the home the bootstrap resolved."""
        factr_dir = tmp_path / ".factr"
        (factr_dir / "profiles" / "coder").mkdir(parents=True)
        (factr_dir / "active_profile").write_text("coder\n")
        capsys.readouterr()  # drop anything the setup above printed

        env = {"PATH": "/usr/bin:/bin"}
        assert fresh_constants.apply_scratch_tmp_env(env) is True

        assert env["TMPDIR"] == str(factr_dir / "cache" / "scratch")
        assert "FACTR_CONFIG_HOME fallback" not in capsys.readouterr().err

    def test_explicit_home_scratch_dir_never_reads_the_effective_home(
        self, fresh_constants, tmp_path, capsys
    ):
        """A caller that passes a home (`factr doctor` for another profile) stays silent."""
        factr_dir = tmp_path / ".factr"
        profile_dir = factr_dir / "profiles" / "coder"
        profile_dir.mkdir(parents=True)
        (factr_dir / "active_profile").write_text("coder\n")
        capsys.readouterr()  # drop anything the setup above printed

        scratch = fresh_constants.get_scratch_dir(profile_dir)

        assert scratch == profile_dir / "cache" / "scratch"
        assert scratch.is_dir()
        assert "FACTR_CONFIG_HOME fallback" not in capsys.readouterr().err

