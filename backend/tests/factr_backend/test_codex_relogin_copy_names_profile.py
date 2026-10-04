"""Codex credential-failure copy sends the user to THIS profile's own sign-in.

Profiles are islands (93889b770da): a bare ``factr auth`` / ``factr model`` from a named
profile's error text re-signs the ROOT store, which is exactly the loop #114012 measured.
Every relogin hint raised by ``factr_backend/auth_codex.py`` and appended by
``format_auth_error`` must carry the ``-p <profile>`` selector under a profile FACTR_CONFIG_HOME.
"""

from __future__ import annotations

from types import SimpleNamespace

import pytest

from factr_backend.auth import AuthError, _read_codex_tokens, format_auth_error
from factr_backend.auth_codex import _codex_refresh_failure_error


@pytest.fixture
def codex_profile_home(tmp_path, monkeypatch):
    profile_home = tmp_path / ".factr" / "profiles" / "codex"
    profile_home.mkdir(parents=True)
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setenv("FACTR_CONFIG_HOME", str(profile_home))
    return profile_home


def test_missing_and_reused_codex_credential_copy_names_the_profile(codex_profile_home):
    with pytest.raises(AuthError) as missing:
        _read_codex_tokens()
    reused = _codex_refresh_failure_error(SimpleNamespace(
        status_code=400,
        json=lambda: {"error": "refresh_token_reused", "error_description": "already used"},
    ))
    for err in (missing.value, reused):
        text = str(err)
        assert "`factr -p codex auth add openai-codex --type oauth`" in text, text
        assert "`factr auth`" not in text, text


def test_format_auth_error_relogin_suffix_names_the_profile(codex_profile_home):
    err = AuthError("Codex token refresh failed: invalid_grant", provider="openai-codex",
                    code="invalid_grant", relogin_required=True)
    rendered = format_auth_error(err)
    assert "`factr -p codex model`" in rendered, rendered
    assert "`factr model`" not in rendered, rendered
