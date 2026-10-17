"""A bot with its own computer (container) is never a target of host-side local delivery: the container owns
that profile folder's SQLite, and a host CLI turn would be a second writer across the VM boundary. The desktop
app writes ``<profile>/.computer`` while the container exists (electron/bot-computers.ts)."""

import json

import pytest

from tests.tools.test_bot_mode_dm import _FakeAgent, _capture_spawn, _managed_home  # noqa: F401
from tools import bot_failure_reasons, bot_mode_dm, bot_mode_probe, bot_relay


@pytest.fixture(autouse=True)
def _fresh_probe_cache():
    bot_mode_probe._reset_cache_for_tests()
    yield
    bot_mode_probe._reset_cache_for_tests()


def test_marker_name_and_reason_are_stable():
    assert bot_relay.COMPUTER_MARKER == ".computer"
    assert bot_failure_reasons.HAS_COMPUTER in bot_failure_reasons.ALL_REASONS


def test_assert_no_computer_refuses_only_with_marker(tmp_path):
    bot_relay.assert_no_computer("scout", tmp_path)  # no marker: fine
    (tmp_path / bot_relay.COMPUTER_MARKER).write_text("{}")
    with pytest.raises(bot_relay.ComputerHandoffRefused) as exc:
        bot_relay.assert_no_computer("scout", tmp_path)
    assert exc.value.reason == "has_computer"
    assert "own computer" in str(exc.value)
    assert bot_failure_reasons.delivery_failure_reason(exc.value) == "has_computer"


def test_local_delivery_command_refuses_a_bot_with_a_computer(tmp_path, monkeypatch):
    monkeypatch.setenv("FACTR_CONFIG_HOME", str(tmp_path))
    (tmp_path / "profiles" / "scout").mkdir(parents=True)
    assert bot_relay.local_delivery_command("scout", "q.txt")[-2:] == ["--query-file", "q.txt"]
    (tmp_path / "profiles" / "scout" / ".computer").write_text("{}")
    with pytest.raises(bot_relay.ComputerHandoffRefused):
        bot_relay.local_delivery_command("scout", "q.txt")


def test_message_agent_refuses_a_teammate_with_a_computer_and_spawns_nothing(tmp_path, monkeypatch):
    calls = _capture_spawn(monkeypatch)
    home = _managed_home(tmp_path, teammates=("researcher", "scout"))
    (home / "profiles" / "scout" / bot_relay.COMPUTER_MARKER).write_text("{}")
    agent = _FakeAgent(home, title="Bot Chat")

    refused = json.loads(bot_mode_dm.message_agent_tool(target="@scout", message="hi", agent=agent))
    assert refused["reason"] == "has_computer"
    assert "own computer" in refused["error"]
    assert calls == []

    ok = json.loads(bot_mode_dm.message_agent_tool(target="@researcher", message="hi", agent=agent))
    assert ok["status"] == "queued"
    assert len(calls) == 1
