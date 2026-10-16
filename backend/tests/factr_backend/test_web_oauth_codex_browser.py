"""Codex desktop sign-in: browser authorization with a loopback callback (no code to type)."""
import socket
import time
from urllib.parse import parse_qs, urlparse

import httpx

import factr_backend.auth_codex_browser as browser_mod
import factr_backend.web_routers.oauth as rt


def _start(monkeypatch, port=0):
    monkeypatch.setattr(browser_mod, "CODEX_BROWSER_CALLBACK_PORT", port)
    return rt._start_codex_browser(None)


def _wait(sid, want, secs=5.0):
    end = time.time() + secs
    while time.time() < end:
        if rt._oauth_sessions[sid]["status"] == want:
            return True
        time.sleep(0.05)
    return False


def test_browser_signin_saves_tokens_without_a_code(monkeypatch):
    saved = []
    import factr_backend.auth as auth_mod
    monkeypatch.setattr(auth_mod, "_save_codex_tokens", lambda t, *a, **k: saved.append(t))
    monkeypatch.setattr(browser_mod, "_codex_browser_exchange_code",
                        lambda code, **kw: {"access_token": "at-" + code, "refresh_token": "rt"})
    start = _start(monkeypatch)
    assert start["flow"] == "device_code" and start["user_code"] == ""
    q = parse_qs(urlparse(start["verification_url"]).query)
    redirect = q["redirect_uri"][0]
    assert q["code_challenge_method"] == ["S256"]
    httpx.get(redirect, params={"code": "abc", "state": q["state"][0]}, timeout=5)
    assert _wait(start["session_id"], "approved")
    assert saved == [{"access_token": "at-abc", "refresh_token": "rt"}]


def test_browser_signin_rejects_wrong_state(monkeypatch):
    saved = []
    import factr_backend.auth as auth_mod
    monkeypatch.setattr(auth_mod, "_save_codex_tokens", lambda t, *a, **k: saved.append(t))
    start = _start(monkeypatch)
    redirect = parse_qs(urlparse(start["verification_url"]).query)["redirect_uri"][0]
    httpx.get(redirect, params={"code": "abc", "state": "forged"}, timeout=5)
    assert _wait(start["session_id"], "error")
    assert saved == []


def test_busy_port_falls_back_to_device_code(monkeypatch):
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        s.listen(1)
        assert _start(monkeypatch, port=s.getsockname()[1]) is None


def test_cancel_releases_the_port(monkeypatch):
    start = _start(monkeypatch)
    redirect = parse_qs(urlparse(start["verification_url"]).query)["redirect_uri"][0]
    port = urlparse(redirect).port
    rt._oauth_sessions[start["session_id"]]["cancelled"] = True
    end = time.time() + 5
    while time.time() < end:
        with socket.socket() as s:
            if s.connect_ex(("127.0.0.1", port)) != 0:
                return
        time.sleep(0.1)
    raise AssertionError("loopback listener still bound after cancel")
