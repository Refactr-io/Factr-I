"""The packaged desktop app ships ``auth.adopt_external_logins: false`` in its bundled defaults
(``FACTR_BACKEND_DEFAULTS``) so it never borrows, and by refreshing logs out, the Claude Code / Codex
CLI logins. The CLI/dev default (no bundled defaults) stays True, and the user's config.yaml wins."""
from __future__ import annotations

import json

import pytest

from agent import credential_sources


@pytest.fixture
def home(tmp_path, monkeypatch):
    factr_home = tmp_path / "factr"
    factr_home.mkdir()
    (factr_home / ".env").write_text("")
    monkeypatch.setenv("FACTR_CONFIG_HOME", str(factr_home))
    monkeypatch.delenv("FACTR_BACKEND_DEFAULTS", raising=False)
    monkeypatch.setattr(credential_sources, "_notice_logged", False, raising=False)
    return factr_home


def _bundle_defaults(tmp_path, monkeypatch, value):
    path = tmp_path / "defaults.json"
    path.write_text(json.dumps({"auth": {"adopt_external_logins": value}, "model": {}}))
    monkeypatch.setenv("FACTR_BACKEND_DEFAULTS", str(path))


def test_cli_default_still_adopts(home):
    assert credential_sources.packaged_adopt_default() is None
    assert credential_sources.adopt_external_logins_enabled() is True


def test_packaged_default_turns_adoption_off(home, tmp_path, monkeypatch):
    _bundle_defaults(tmp_path, monkeypatch, False)
    assert credential_sources.packaged_adopt_default() is False
    assert credential_sources.adopt_external_logins_enabled() is False


def test_user_config_overrides_the_packaged_default(home, tmp_path, monkeypatch):
    _bundle_defaults(tmp_path, monkeypatch, False)
    (home / "config.yaml").write_text("auth:\n  adopt_external_logins: true\n")
    assert credential_sources.adopt_external_logins_enabled() is True


def test_unreadable_or_odd_bundle_defaults_are_ignored(home, tmp_path, monkeypatch):
    bad = tmp_path / "defaults.json"
    bad.write_text("{not json")
    monkeypatch.setenv("FACTR_BACKEND_DEFAULTS", str(bad))
    assert credential_sources.packaged_adopt_default() is None
    bad.write_text(json.dumps({"auth": {"adopt_external_logins": "no"}}))
    assert credential_sources.packaged_adopt_default() is None
    assert credential_sources.adopt_external_logins_enabled() is True
