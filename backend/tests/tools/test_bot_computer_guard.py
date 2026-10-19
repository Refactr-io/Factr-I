"""A bot with its own computer (container) is never a target of host-side CLI delivery: the container owns
that profile folder's SQLite, and a host CLI turn would be a second writer across the VM boundary. The desktop
app writes ``<profile>/.computer`` while the container exists (electron/bot-computers.ts). Messages reach such a bot
through the Desktop relay instead (its computer is its own relay connection), with a hop cap against loops."""

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


def test_inside_a_computer_the_marker_never_refuses(tmp_path, monkeypatch):
    (tmp_path / bot_relay.COMPUTER_MARKER).write_text("{}")
    monkeypatch.setenv(bot_relay.IN_COMPUTER_ENV, "1")
    bot_relay.assert_no_computer("scout", tmp_path)  # the profile IS this computer's own


def _roster_row(profile):
    return {"profile": profile, "handle": profile, "connection_id": bot_relay.computer_connection_id(profile),
            "connection_label": f"{profile} computer"}


def test_message_agent_without_a_relay_route_refuses_and_spawns_no_cli_turn(tmp_path, monkeypatch):
    calls = _capture_spawn(monkeypatch)
    home = _managed_home(tmp_path, teammates=("researcher", "scout"))
    (home / "profiles" / "scout" / bot_relay.COMPUTER_MARKER).write_text("{}")
    agent = _FakeAgent(home, title="Bot Chat")

    refused = json.loads(bot_mode_dm.message_agent_tool(target="@scout", message="hi", agent=agent))
    assert refused["reason"] == "has_computer"
    assert "Desktop relay" in refused["error"]
    assert calls == []

    ok = json.loads(bot_mode_dm.message_agent_tool(target="@researcher", message="hi", agent=agent))
    assert ok["status"] == "queued"
    assert len(calls) == 1


def test_message_agent_to_a_computer_bot_goes_through_the_relay_outbox(tmp_path, monkeypatch):
    calls = _capture_spawn(monkeypatch)
    home = _managed_home(tmp_path, teammates=("scout",))
    (home / "profiles" / "scout" / bot_relay.COMPUTER_MARKER).write_text("{}")
    bot_relay.write_remote_roster(home, [_roster_row("scout")])
    agent = _FakeAgent(home, title="Bot Chat")

    result = json.loads(bot_mode_dm.message_agent_tool(target="scout", message="what is the secret?", agent=agent))
    assert result["status"] == "queued"

    [envelope] = [json.loads(p.read_text()) for p in (home / "bot_relay" / "outbox").glob("*.json")]
    assert (envelope["target_connection"], envelope["target_profile"]) == ("computer-scout", "scout")
    assert envelope["hops"] == 1
    # The only spawned process is the reply waiter; nothing runs `-p scout chat` on the host.
    assert len(calls) == 1 and "--wait-reply" in calls[0]["command"] and "chat" not in calls[0]["command"].split()


def test_mcp_deliver_to_a_computer_bot_uses_the_relay(tmp_path, monkeypatch):
    from tools import message_agent_mcp

    home = _managed_home(tmp_path, teammates=("me", "scout"))
    (home / "profiles" / "scout" / bot_relay.COMPUTER_MARKER).write_text("{}")
    bot_relay.write_remote_roster(home, [_roster_row("scout")])
    ran = []
    monkeypatch.setattr(message_agent_mcp, "_run", lambda command, *, label: ran.append((command, label)) or ("pong", True))
    monkeypatch.setattr(bot_mode_probe, "_profile_name", lambda h: "me")

    assert message_agent_mcp.deliver("scout", "ping", home) == ("pong", True)
    [(command, label)] = ran
    assert "--wait-reply" in command and "chat" not in command.split()
    assert [e["target_connection"] for e in map(json.loads, (p.read_text() for p in (home / "bot_relay" / "outbox").glob("*.json")))] == ["computer-scout"]

    # No route in the relay roster: a clear refusal, never a host CLI turn.
    bot_relay.write_remote_roster(home, [])
    text, ok = message_agent_mcp.deliver("scout", "ping", home)
    assert ok is False and "Desktop relay" in text and len(ran) == 1


def test_hop_cap_refuses_a_chain_longer_than_the_limit(tmp_path, monkeypatch):
    root = tmp_path
    target = _roster_row("scout")
    for hops_so_far, allowed in ((0, True), (1, True), (2, False), (5, False)):
        monkeypatch.setenv(bot_relay.RELAY_HOPS_ENV, str(hops_so_far))
        if allowed:
            envelope = bot_relay.enqueue_envelope(root, target=target, message="m", sender_profile="a", sender_handle="a")
            assert envelope["hops"] == hops_so_far + 1
        else:
            with pytest.raises(bot_relay.EnvelopeRefusedError) as exc:
                bot_relay.enqueue_envelope(root, target=target, message="m", sender_profile="a", sender_handle="a")
            assert exc.value.reason == "hop_limit"
    assert bot_failure_reasons.HOP_LIMIT in bot_failure_reasons.ALL_REASONS
    monkeypatch.delenv(bot_relay.RELAY_HOPS_ENV)
    assert bot_relay.enqueue_envelope(root, target=target, message="m", sender_profile="a", sender_handle="a")["hops"] == 1


def test_delivery_env_carries_the_hop_count_to_the_turn(monkeypatch):
    monkeypatch.delenv(bot_relay.RELAY_HOPS_ENV, raising=False)
    assert bot_relay.delivery_env(None)[bot_relay.RELAY_HOPS_ENV] == "0"
    assert bot_relay.delivery_env(None, hops=1)[bot_relay.RELAY_HOPS_ENV] == "1"
    monkeypatch.setenv(bot_relay.RELAY_HOPS_ENV, "1")
    assert bot_relay.current_relay_hops() == 1
    monkeypatch.setenv(bot_relay.RELAY_HOPS_ENV, "junk")
    assert bot_relay.current_relay_hops() == 0


def test_host_ensure_teammate_mcp_leaves_a_computer_bots_config_to_its_container(tmp_path, monkeypatch):
    from tools import message_agent_mcp

    monkeypatch.delenv("FACTR_BOT_COMPUTER", raising=False)
    home = _managed_home(tmp_path, teammates=("plain", "boxed"))
    for name in ("plain", "boxed"):
        (home / "profiles" / name / "config.yaml").write_text("model: {default: x}\n")
    (home / "profiles" / "boxed" / bot_relay.COMPUTER_MARKER).write_text("{}")

    assert message_agent_mcp.ensure_teammate_mcp(home) == 2  # default plus the host bot
    assert "factr_teammates" in (home / "profiles" / "plain" / "config.yaml").read_text()
    assert "factr_teammates" not in (home / "profiles" / "boxed" / "config.yaml").read_text()

    # Inside the computer the entrypoint runs the same function: it does write there.
    monkeypatch.setenv("FACTR_BOT_COMPUTER", "1")
    assert message_agent_mcp.ensure_teammate_mcp(home) == 1
    assert "factr_teammates" in (home / "profiles" / "boxed" / "config.yaml").read_text()


def test_hops_marker_stands_in_for_the_env_in_an_engine_run_turn(tmp_path, monkeypatch):
    monkeypatch.delenv(bot_relay.RELAY_HOPS_ENV, raising=False)
    assert bot_relay.current_relay_hops(tmp_path, "scout") == 0
    with bot_relay.relay_hops_marker(tmp_path, "scout", 1):
        assert bot_relay.current_relay_hops(tmp_path, "scout") == 1
        assert bot_relay.current_relay_hops(tmp_path, "other") == 0  # one marker per profile
        with pytest.raises(bot_relay.EnvelopeRefusedError):  # hop 2 is the last allowed; a 3rd is refused
            with bot_relay.relay_hops_marker(tmp_path, "scout", 2):
                bot_relay.enqueue_envelope(tmp_path, target=_roster_row("b"), message="m", sender_profile="scout",
                                           sender_handle="scout")
    assert bot_relay.current_relay_hops(tmp_path, "scout") == 0  # removed after the turn
    # A stale marker (a crashed turn) is ignored.
    with bot_relay.relay_hops_marker(tmp_path, "scout", 2):
        import os
        marker = bot_relay._hops_marker(tmp_path, "scout")
        old = bot_relay.time.time() - 10 * bot_relay.TURN_ATTEMPT_TIMEOUT_SECONDS
        marker.write_text(json.dumps({"hops": 2, "at": int(old)}))
        assert bot_relay.current_relay_hops(tmp_path, "scout") == 0
        del os


def test_engine_delivery_runs_the_turn_on_the_computers_own_engine(tmp_path, monkeypatch):
    from tui_gateway import methods_bot_relay as mbr

    msg = tmp_path / "m.txt"
    msg.write_text("hello from a", encoding="utf-8")
    monkeypatch.setattr(mbr, "_relay_root", lambda: tmp_path)
    for var in ("FACTR_BOT_COMPUTER", "FACTR_ENGINE_URL", "FACTR_ENGINE_TOKEN"):
        monkeypatch.delenv(var, raising=False)
    assert mbr._engine_delivery("scout", str(msg), {}, 5) is None  # not a computer: the CLI transport runs

    monkeypatch.setenv("FACTR_BOT_COMPUTER", "1")
    monkeypatch.setenv("FACTR_ENGINE_URL", "http://127.0.0.1:9")
    monkeypatch.setenv("FACTR_ENGINE_TOKEN", "t" * 40)
    seen = {}

    def fake(url, token, name, prompt, workdir, **kw):
        seen.update(url=url, name=name, prompt=prompt, kw=kw, hops=bot_relay.current_relay_hops(tmp_path, "scout"))
        return {"ok": True, "final_text": "the answer"}

    monkeypatch.setattr("cron.scheduler._run_job_via_factr_engine", fake)
    done = mbr._engine_delivery("scout", str(msg), {bot_relay.RELAY_HOPS_ENV: "1"}, 5)
    assert (done.returncode, done.stdout) == (0, "the answer")
    assert seen["prompt"] == "hello from a" and seen["kw"]["session_key"] == "bot-chat:scout"
    assert seen["kw"]["surface"] == "bot" and seen["hops"] == 1  # the marker is live during the turn
    assert bot_relay.current_relay_hops(tmp_path, "scout") == 0  # and gone after

    monkeypatch.setattr("cron.scheduler._run_job_via_factr_engine", lambda *a, **k: {"ok": False, "error": "boom"})
    failed = mbr._engine_delivery("scout", str(msg), {}, 5)
    assert failed.returncode == 1 and "boom" in failed.stderr
    monkeypatch.setattr("cron.scheduler._run_job_via_factr_engine",
                        lambda *a, **k: (_ for _ in ()).throw(RuntimeError("engine unreachable")))
    assert mbr._engine_delivery("scout", str(msg), {}, 5).returncode == 1


def test_deliver_job_runs_once_per_envelope_and_answers_pending_until_done():
    import threading

    gate, runs = threading.Event(), []

    def job():
        runs.append(1)
        gate.wait(5)
        return {"result": {"reply": "late"}}

    assert bot_relay.deliver_job("e" * 32, job, 0.05) == (False, None)
    assert bot_relay.deliver_job("e" * 32, job, 0.05) == (False, None)  # joins the same turn, never a second one
    gate.set()
    assert bot_relay.deliver_job("e" * 32, job, 5) == (True, {"result": {"reply": "late"}})
    assert runs == [1]
    # Forgotten once fetched: the same id would start a fresh turn.
    assert bot_relay.deliver_job("e" * 32, lambda: "again", 5) == (True, "again")


def test_deliver_job_surfaces_the_jobs_failure():
    def boom():
        raise RuntimeError("turn exploded")

    with pytest.raises(RuntimeError, match="turn exploded"):
        bot_relay.deliver_job("f" * 32, boom, 5)
