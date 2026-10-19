"""A bot's message to a teammate on the SAME install goes through the Desktop relay (so it becomes a real turn in the
teammate's own Bot Chat, on its own engine: ``bot.chat.deliver``) while the relay is alive, and falls back to the local
CLI turn when no Desktop is draining. A message chain is capped at two relay hops."""

import json
import os
import time

import pytest

from tests.tools.test_bot_mode_dm import _FakeAgent, _capture_spawn, _managed_home  # noqa: F401
from tools import bot_mode_dm, bot_mode_probe, bot_relay, message_agent_mcp


@pytest.fixture(autouse=True)
def _fresh(monkeypatch):
    bot_mode_probe._reset_cache_for_tests()
    monkeypatch.delenv(bot_relay.RELAY_HOPS_ENV, raising=False)
    monkeypatch.delenv("FACTR_BOT_COMPUTER", raising=False)
    yield
    bot_mode_probe._reset_cache_for_tests()


def _beat(home, age=0.0):
    base = bot_relay.relay_root(home)
    base.mkdir(parents=True, exist_ok=True)
    path = base / bot_relay.RELAY_BEAT_FILE
    path.write_text("{}")
    stamp = time.time() - age
    os.utime(path, (stamp, stamp))


def _outbox(home):
    return [json.loads(p.read_text()) for p in sorted((bot_relay.relay_root(home) / "outbox").glob("*.json"))]


def test_relay_alive_means_a_fresh_heartbeat(tmp_path):
    assert bot_relay.relay_alive(tmp_path) is False
    _beat(tmp_path)
    assert bot_relay.relay_alive(tmp_path) is True
    _beat(tmp_path, age=bot_relay.RELAY_BEAT_FRESH_SECONDS + 5)
    assert bot_relay.relay_alive(tmp_path) is False


def test_message_agent_to_a_local_teammate_queues_an_envelope_for_the_local_connection(tmp_path, monkeypatch):
    calls = _capture_spawn(monkeypatch)
    home = _managed_home(tmp_path, teammates=("scout",))
    _beat(home)
    agent = _FakeAgent(home, title="Bot Chat")

    result = json.loads(bot_mode_dm.message_agent_tool(target="@scout", message="what is the secret?", agent=agent))

    assert result["status"] == "queued"
    [envelope] = _outbox(home)
    assert (envelope["target_connection"], envelope["target_profile"], envelope["hops"]) == ("local", "scout", 1)
    assert envelope["message"].startswith("Message from 🤖 ") and envelope["message"].endswith("what is the secret?")
    # Only the reply waiter ran: no Python CLI turn for the teammate, and the waiter is labelled by name alone.
    assert len(calls) == 1 and "--wait-reply" in calls[0]["command"] and " chat" not in calls[0]["command"]
    assert "@scout on local" not in calls[0]["command"]


def test_without_a_running_relay_the_local_cli_turn_still_delivers(tmp_path, monkeypatch):
    calls = _capture_spawn(monkeypatch)
    home = _managed_home(tmp_path, teammates=("scout",))
    _beat(home, age=bot_relay.RELAY_BEAT_FRESH_SECONDS + 60)  # a Desktop that went away
    agent = _FakeAgent(home, title="Bot Chat")

    result = json.loads(bot_mode_dm.message_agent_tool(target="@scout", message="hi", agent=agent))

    assert result["status"] == "queued"
    assert _outbox(home) == []
    assert "--run-delivery" in calls[0]["command"]


def test_a_local_target_is_never_refused_as_offline_by_the_remote_roster(tmp_path):
    home = tmp_path
    # A fresh roster that lists only OTHER machines' agents: the local connection is absent from it by design.
    bot_relay.write_remote_roster(home, [{"profile": "far", "handle": "far", "connection_id": "cloud-1"}])
    envelope = bot_relay.enqueue_envelope(home, target=bot_relay.local_target("scout", "scout"), message="m",
                                          sender_profile="alpha", sender_handle="alpha")
    assert envelope["target_connection"] == "local"
    with pytest.raises(bot_relay.EnvelopeRefusedError) as exc:  # a remote target that left IS refused, as before
        bot_relay.enqueue_envelope(home, target={"profile": "gone", "handle": "gone", "connection_id": "cloud-1"},
                                   message="m", sender_profile="alpha", sender_handle="alpha")
    assert exc.value.reason == "runtime_offline"


def test_mcp_deliver_to_a_local_teammate_uses_the_relay_when_it_is_alive(tmp_path, monkeypatch):
    home = _managed_home(tmp_path, teammates=("me", "scout"))
    _beat(home)
    ran = []
    monkeypatch.setattr(message_agent_mcp, "_run", lambda command, *, label: ran.append((command, label)) or ("pong", True))
    monkeypatch.setattr(bot_mode_probe, "_profile_name", lambda h: "me")

    assert message_agent_mcp.deliver("scout", "ping", home) == ("pong", True)
    [(command, label)] = ran
    assert "--wait-reply" in command and label == "@scout"
    assert [e["target_profile"] for e in _outbox(home)] == ["scout"]

    # No heartbeat: the same call falls back to the local CLI turn.
    (bot_relay.relay_root(home) / bot_relay.RELAY_BEAT_FILE).unlink()
    ran.clear()
    message_agent_mcp.deliver("scout", "ping", home)
    assert "--wait-reply" not in ran[0][0] and len(_outbox(home)) == 1


def test_the_hop_cap_stops_a_chain_at_two_relays(tmp_path, monkeypatch):
    target = bot_relay.local_target("scout", "scout")
    send = lambda: bot_relay.enqueue_envelope(tmp_path, target=target, message="m", sender_profile="b", sender_handle="b")

    assert send()["hops"] == 1  # a turn nobody relayed to: hop 1
    marker = bot_relay.relay_root(tmp_path) / bot_relay.HOPS_DIR / "b.json"
    marker.parent.mkdir(parents=True, exist_ok=True)
    marker.write_text(json.dumps({"hops": 1, "at": int(time.time())}))  # B's turn was itself relayed once
    assert send()["hops"] == 2
    marker.write_text(json.dumps({"hops": 2, "at": int(time.time())}))  # ... twice: it may not relay on
    with pytest.raises(bot_relay.EnvelopeRefusedError) as exc:
        send()
    assert exc.value.reason == "hop_limit"
    from tools import bot_failure_reasons
    assert bot_failure_reasons.HOP_LIMIT in bot_failure_reasons.ALL_REASONS
    # Only that profile's turn carries it, a stale marker (a crashed turn) is ignored, and the env wins.
    assert bot_relay.current_relay_hops(tmp_path, "other") == 0
    old = time.time() - 10 * bot_relay.TURN_ATTEMPT_TIMEOUT_SECONDS
    marker.write_text(json.dumps({"hops": 2, "at": int(old)}))
    assert bot_relay.current_relay_hops(tmp_path, "b") == 0
    monkeypatch.setenv(bot_relay.RELAY_HOPS_ENV, "2")
    assert bot_relay.current_relay_hops(tmp_path, "b") == 2


def test_the_waiter_names_a_local_target_by_handle_only(tmp_path):
    local = bot_relay.waiter_command(tmp_path, {"id": "a" * 32, "target_handle": "scout", "target_connection": "local"})
    remote = bot_relay.waiter_command(tmp_path, {"id": "b" * 32, "target_handle": "far", "target_connection": "cloud-1"})
    assert "@scout" in local
    assert "@scout on local" not in local and "@far on cloud-1" in remote
