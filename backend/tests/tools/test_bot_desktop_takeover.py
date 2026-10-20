"""request_takeover: records the agent's ask on the Bot Desktop lease, starts the screen, tells the model to wait
for the hand-back, and is offered only where a Bot Desktop can run."""

from __future__ import annotations

import json

import pytest

from tools import bot_desktop_takeover_tool as tk
from tools.bot_desktop import lease, runtime


@pytest.fixture(autouse=True)
def _fresh_lease(monkeypatch):
    lease._reset_for_tests()
    monkeypatch.setattr(runtime, "start", lambda **_: None)
    yield
    lease._reset_for_tests()


def test_request_is_recorded_for_the_screen_pane_and_becomes_the_takeover_reason():
    out = json.loads(tk.handle_request_takeover({"site": "github.com", "why": "log in with 2FA"}))
    assert out["status"] == "requested" and "credentials in chat" in out["note"]
    pending = lease.get()
    assert pending.holder == lease.AGENT and pending.request == "github.com: log in with 2FA"
    assert lease.public_view(pending)["request"] == "github.com: log in with 2FA"

    taken = lease.acquire("viewer")  # the human clicks Take over: the ask stays on screen as the reason
    assert taken.reason == "github.com: log in with 2FA" and taken.request == ""
    lease.release("viewer")
    assert lease.get().reason == "" and lease.get().request == ""


def test_a_repeat_ask_is_idempotent_and_a_human_in_control_is_not_re_requested():
    tk.handle_request_takeover({"site": "a.com", "why": "x"})
    epoch = lease.get().epoch
    tk.handle_request_takeover({"site": "a.com", "why": "x"})
    assert lease.get().epoch == epoch
    lease.acquire("viewer")
    out = json.loads(tk.handle_request_takeover({"site": "b.com", "why": "y"}))
    assert out["status"] == "human_already_in_control" and lease.get().request == ""


def test_both_fields_are_required_and_a_start_failure_is_reported_not_raised(monkeypatch):
    assert "required" in json.loads(tk.handle_request_takeover({"site": "a.com"}))["error"]

    def boom(**_):
        raise RuntimeError("needs 1536 MB free")
    monkeypatch.setattr(runtime, "start", boom)
    err = json.loads(tk.handle_request_takeover({"site": "a.com", "why": "x"}))["error"]
    assert "could not start" in err and lease.get().request == ""


def test_wait_returns_when_the_human_hands_control_back(monkeypatch):
    monkeypatch.setattr(tk.time, "sleep", lambda _s: None)
    steps = iter([lambda: lease.acquire("viewer"), lambda: lease.release("viewer")])
    real_get = lease.get

    def get_and_advance(*a, **k):
        got = real_get(*a, **k)
        nxt = next(steps, None)
        if nxt:
            nxt()
        return got
    monkeypatch.setattr(lease, "get", get_and_advance)
    out = json.loads(tk.handle_request_takeover({"site": "a.com", "why": "x", "wait_seconds": 5}))
    assert out["status"] == "handed_back"


def test_wait_times_out_without_a_human():
    assert tk._wait_for_handback(lease, 0) == "timeout"


def test_tool_is_gated_to_hosts_with_a_bot_desktop(monkeypatch):
    monkeypatch.setattr(runtime, "is_supported_host", lambda: True)
    monkeypatch.setattr(runtime, "missing_binaries", lambda: [])
    assert tk._available() is True
    monkeypatch.setattr(runtime, "missing_binaries", lambda: ["Xvnc"])
    assert tk._available() is False
    monkeypatch.setattr(runtime, "missing_binaries", lambda: [])
    monkeypatch.setattr(runtime, "is_supported_host", lambda: False)
    assert tk._available() is False


def test_tool_is_registered_in_its_own_toolset():
    from tools.registry import registry
    assert registry.get_toolset_for_tool("request_takeover") == "bot_desktop"


def test_a_bot_computer_always_auto_starts_its_screen_without_touching_config(monkeypatch):
    monkeypatch.setattr(runtime, "is_supported_host", lambda: True)
    monkeypatch.setattr(runtime, "missing_binaries", lambda: [])
    assert runtime._should_auto_start({"FACTR_BOT_COMPUTER": "1"}) is True
    assert runtime._should_auto_start({"FACTR_BOT_COMPUTER": "1", "DISPLAY": ":0"}) is False, "a real seat wins"
    monkeypatch.setattr("factr_backend.config.load_config_readonly", lambda: {"bot_desktop": {"auto_start": False}})
    assert runtime._should_auto_start({}) is False
