import json

import pytest

from factr_backend import opencode_detect as oc


@pytest.fixture
def home(tmp_path, monkeypatch):
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path / "data"))
    monkeypatch.setenv("XDG_CONFIG_HOME", str(tmp_path / "cfg"))
    monkeypatch.setenv("PATH", str(tmp_path / "empty"))
    monkeypatch.setattr(oc, "_binary_candidates", lambda: [tmp_path / ".opencode" / "bin" / "opencode"])
    monkeypatch.delenv("FACTR_OPENCODE_DETECT", raising=False)
    return tmp_path


def test_not_installed(home):
    if oc.Path("/Applications/OpenCode.app").exists():
        pytest.skip("real OpenCode.app on this machine")
    assert oc.detect() == {"installed": False, "via": "", "auth_present": False}


def test_detects_binary_and_auth_file_without_reading_it(home):
    (home / ".opencode" / "bin").mkdir(parents=True)
    (home / ".opencode" / "bin" / "opencode").write_text("#!/bin/sh\n")
    (home / "data" / "opencode").mkdir(parents=True)
    (home / "data" / "opencode" / "auth.json").write_text("not json at all")
    assert oc.detect() == {"installed": True, "via": "binary", "auth_present": True}


def test_detects_config_dir_only(home):
    (home / "cfg" / "opencode").mkdir(parents=True)
    d = oc.detect()
    assert d["installed"] and d["via"] in ("config", "app")


def test_parse_models_dedupes_and_drops_delisted_and_junk():
    body = {"data": [{"id": "big-pickle"}, {"id": "big-pickle"}, {"id": "ox-alpha-free"}, {"id": ""}, "x", {"id": "gpt-6"}]}
    assert oc.parse_models(body) == ["big-pickle", "gpt-6"]
    assert oc.parse_models(None) == [] and oc.parse_models({"data": "no"}) == []


def test_is_free():
    assert oc.is_free("beam-free") and oc.is_free("big-pickle") and not oc.is_free("gpt-6")


def test_adopt_saves_only_api_keys_and_never_returns_values(home, monkeypatch):
    (home / "data" / "opencode").mkdir(parents=True)
    (home / "data" / "opencode" / "auth.json").write_text(json.dumps({
        "opencode": {"type": "api", "key": "zen-secret"},
        "opencode-go": {"type": "oauth", "access": "x"},
        "anthropic": {"type": "api", "key": "other"}}))
    saved = {}
    import factr_backend.credential_lifecycle as cl
    monkeypatch.setattr(cl, "save_provider_env_credential", lambda k, v: saved.__setitem__(k, v))
    monkeypatch.delenv("OPENCODE_ZEN_API_KEY", raising=False)
    res = oc.adopt()
    assert res == {"adopted": ["opencode-zen"], "reason": ""}
    assert saved == {"OPENCODE_ZEN_API_KEY": "zen-secret"}
    assert "zen-secret" not in json.dumps(res)
    monkeypatch.delenv("OPENCODE_ZEN_API_KEY", raising=False)


def test_adopt_without_sign_in(home):
    assert oc.adopt() == {"adopted": [], "reason": "not_signed_in"}


def _st(**kw):
    base = {"installed": True, "via": "path", "auth_present": True, "zen_connected": False, "go_connected": False,
            "models": ["big-pickle", "gpt-6"], "free_models": ["big-pickle"]}
    return {**base, **kw}


def test_picker_row_only_when_installed_and_unconnected(monkeypatch):
    from factr_backend import inventory
    monkeypatch.setattr(oc, "status", lambda: _st())
    rows = inventory._with_opencode_detected_row([])
    assert len(rows) == 1 and rows[0]["slug"] == "opencode-zen" and rows[0]["models"] == []
    assert rows[0]["detected"] and rows[0]["adoptable"] and rows[0]["authenticated"] is False
    monkeypatch.setattr(oc, "status", lambda: _st(installed=False))
    assert inventory._with_opencode_detected_row([]) == []
    monkeypatch.setattr(oc, "status", lambda: _st(zen_connected=True))
    assert inventory._with_opencode_detected_row([]) == []
    real = [{"slug": "opencode-zen", "name": "OpenCode Zen", "models": ["a"]}]
    monkeypatch.setattr(oc, "status", lambda: _st())
    assert inventory._with_opencode_detected_row(real) == real
