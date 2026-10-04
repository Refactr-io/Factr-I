"""Bot turns run on the Factr engine when FACTR_ENGINE_URL/TOKEN are set (same helper as cron)."""

from unittest.mock import patch

import pytest

from gateway.config import Platform
from gateway.run import GatewayRunner
from gateway.session import SessionSource


def _runner():
    runner = object.__new__(GatewayRunner)
    runner._session_run_generation = {}
    runner._get_system_prompt_for_channel = lambda *a, **k: "Be terse."
    return runner


def _source():
    return SessionSource(platform=Platform.TELEGRAM, chat_id="42", chat_name="Ann", chat_type="dm",
                         user_id="7", user_name="ann", thread_id=None)


@pytest.mark.asyncio
async def test_turn_goes_to_engine_with_per_chat_key(monkeypatch):
    monkeypatch.setenv("FACTR_ENGINE_URL", "http://127.0.0.1:1")
    monkeypatch.setenv("FACTR_ENGINE_TOKEN", "test-token-not-real")
    calls = []

    def fake(url, token, title, prompt, workdir, session_key=None, instructions=None, surface="cron"):
        calls.append((url, token, title, prompt, session_key, instructions, surface))
        return {"ok": True, "text": "hi back"}

    with patch("cron.scheduler._run_job_via_factr_engine", fake):
        out = await _runner()._run_agent_inner(
            "hello", "## Current Session Context\n**User:** ann", [], _source(), "sid", session_key="tg:42",
            channel_prompt="Channel rule.")
    (url, token, title, prompt, key, instructions, surface), = calls
    assert (url, token, title, prompt, key, surface) == (
        "http://127.0.0.1:1", "test-token-not-real", "telegram Ann", "hello", "tg:42", "bot")
    # What an AIAgent turn puts in its system prompt: session context (user identity), channel prompts, and
    # the platform's formatting rules.
    assert "**User:** ann" in instructions and "Channel rule." in instructions and "Be terse." in instructions
    assert "You are on Telegram" in instructions
    assert out["final_response"] == "hi back"


@pytest.mark.asyncio
async def test_stopped_engine_turn_is_an_interrupted_result_not_an_error_reply(monkeypatch):
    monkeypatch.setenv("FACTR_ENGINE_URL", "http://127.0.0.1:1")
    monkeypatch.setenv("FACTR_ENGINE_TOKEN", "test-token-not-real")
    stopped = {"ok": True, "interrupted": True, "text": "", "error": None}
    with patch("cron.scheduler._run_job_via_factr_engine", lambda *a, **k: stopped):
        out = await _runner()._run_agent_inner("hello", "", [], _source(), "sid", session_key="tg:42")
    assert out["interrupted"] is True and out["final_response"] == "" and out["api_calls"] == 1
    assert "Engine error" not in str(out)


@pytest.mark.asyncio
async def test_engine_failure_is_reported_not_raised(monkeypatch):
    monkeypatch.setenv("FACTR_ENGINE_URL", "http://127.0.0.1:1")
    monkeypatch.setenv("FACTR_ENGINE_TOKEN", "test-token-not-real")

    def boom(*a, **k):
        raise RuntimeError("factr engine unreachable")

    with patch("cron.scheduler._run_job_via_factr_engine", boom):
        out = await _runner()._run_agent_inner("hello", "", [], _source(), "sid", session_key="tg:42")
    assert "Engine error" in out["final_response"]


def _engine_stub():
    """A local HTTP server standing in for the engine; returns (server, url, [(path, json body, auth)])."""
    import json
    import threading
    from http.server import BaseHTTPRequestHandler, HTTPServer

    seen = []

    class Handler(BaseHTTPRequestHandler):
        def do_POST(self):
            body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
            seen.append((self.path, body, self.headers["Authorization"]))
            out = json.dumps({"ok": True, "text": "done"}).encode()
            self.send_response(200)
            self.send_header("Content-Length", str(len(out)))
            self.end_headers()
            self.wfile.write(out)

        def log_message(self, *a):
            pass

    server = HTTPServer(("127.0.0.1", 0), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server, f"http://127.0.0.1:{server.server_port}", seen


def test_new_and_stop_reach_the_engine_and_cron_overrides_ride_along(monkeypatch):
    from cron.scheduler import _run_job_via_factr_engine, _factr_engine_agent_call

    server, url, seen = _engine_stub()
    try:
        monkeypatch.setenv("FACTR_ENGINE_URL", url)
        monkeypatch.setenv("FACTR_ENGINE_TOKEN", "test-token-not-real")
        _factr_engine_agent_call("reset", "tg:42")  # /new
        _factr_engine_agent_call("interrupt", "tg:42")  # /stop
        _run_job_via_factr_engine(url, "test-token-not-real", "nightly", "go", None,
                                      model="llama-3.3-70b", provider="groq")
        assert [(p, b.get("session_key")) for p, b, _ in seen[:2]] == [
            ("/api/agent/reset", "tg:42"), ("/api/agent/interrupt", "tg:42")]
        assert seen[0][2] == "Bearer test-token-not-real"
        run = seen[2][1]
        assert (seen[2][0], run["model"], run["provider"]) == ("/api/agent/run", "llama-3.3-70b", "groq")
        # Outside engine mode both calls do nothing and never raise.
        monkeypatch.delenv("FACTR_ENGINE_URL")
        _factr_engine_agent_call("reset", "tg:42")
        assert len(seen) == 3
    finally:
        server.shutdown()


def test_cron_toolset_policy_rides_along_to_the_engine(monkeypatch):
    from cron.scheduler import _run_job_via_factr_engine

    server, url, seen = _engine_stub()
    try:
        _run_job_via_factr_engine(url, "test-token-not-real", "nightly", "go", None,
                                      enabled_toolsets=["web"], disabled_toolsets=["cronjob", "messaging", "clarify"])
        _run_job_via_factr_engine(url, "test-token-not-real", "nightly", "go", None)
        with_policy, without = seen[0][1], seen[1][1]
        assert (with_policy["enabled_toolsets"], with_policy["disabled_toolsets"]) == (["web"], ["cronjob", "messaging", "clarify"])
        assert "enabled_toolsets" not in without and "disabled_toolsets" not in without
    finally:
        server.shutdown()


def test_run_job_sends_the_resolved_cron_denylist_to_the_engine(monkeypatch, tmp_path):
    import cron.scheduler as sched

    monkeypatch.setenv("FACTR_ENGINE_URL", "http://127.0.0.1:1")
    monkeypatch.setenv("FACTR_ENGINE_TOKEN", "test-token-not-real")
    sent = {}

    def fake(*a, **kw):
        sent.update(kw)
        return {"ok": True, "text": "done"}

    class _Cfg:
        cfg = {"agent": {"disabled_toolsets": ["browser"]}}

    monkeypatch.setattr(sched, "_run_job_via_factr_engine", fake)
    monkeypatch.setattr(sched, "_load_cron_job_config", lambda *a, **k: _Cfg())
    monkeypatch.setattr(sched, "_reload_dotenv_and_publish_delivery_target", lambda job: None)
    sched.run_job({"id": "j1", "name": "nightly", "prompt": "go", "schedule": {"kind": "interval", "minutes": 5}})
    assert sent["disabled_toolsets"] == ["cronjob", "messaging", "clarify", "browser"]
    assert sent["enabled_toolsets"] is None


def test_engine_cron_run_needs_no_model_in_factr_config(monkeypatch, tmp_path):
    """The engine picks the model, so a fresh FACTR_CONFIG_HOME (no config.yaml model) must still run the job."""
    import cron.scheduler as sched

    monkeypatch.setenv("FACTR_CONFIG_HOME", str(tmp_path))
    monkeypatch.delenv("FACTR_BACKEND_MODEL", raising=False)
    monkeypatch.setenv("FACTR_ENGINE_URL", "http://127.0.0.1:1")
    monkeypatch.setenv("FACTR_ENGINE_TOKEN", "test-token-not-real")
    monkeypatch.setattr(sched, "_run_job_via_factr_engine", lambda *a, **kw: {"ok": True, "text": "done"})
    monkeypatch.setattr(sched, "_reload_dotenv_and_publish_delivery_target", lambda job: None)
    ok, _doc, reply, err = sched.run_job({"id": "j2", "name": "n", "prompt": "go", "schedule": {"kind": "interval", "minutes": 5}})
    assert (ok, reply, err) == (True, "done", None)
    with pytest.raises(RuntimeError, match="no model configured"):  # the AIAgent path still fails fast
        sched._load_cron_job_config({"id": "j2"}, "j2", "n")


def test_engine_cron_run_is_not_booked_into_state_db_and_carries_its_run_id(monkeypatch, tmp_path):
    """The engine's hidden session is the run record (the engine serves the run list); no lossy copy in state.db."""
    import cron.scheduler as sched
    from factr_state import SessionDB, _default_db_path

    monkeypatch.setenv("FACTR_CONFIG_HOME", str(tmp_path))
    monkeypatch.setenv("FACTR_ENGINE_URL", "http://127.0.0.1:1")
    monkeypatch.setenv("FACTR_ENGINE_TOKEN", "test-token-not-real")
    sent = {}
    monkeypatch.setattr(sched, "_run_job_via_factr_engine", lambda *a, **kw: sent.update(kw) or {"ok": True, "text": "the reply"})
    monkeypatch.setattr(sched, "_reload_dotenv_and_publish_delivery_target", lambda job: None)
    sched.run_job({"id": "j3", "name": "nightly", "prompt": "go", "schedule": {"kind": "interval", "minutes": 5}})
    assert sent["run_id"].startswith("cron_j3_")
    db = SessionDB(_default_db_path())
    try:
        assert db.list_cron_job_runs("j3") == []
    finally:
        db.close()


def test_cron_timeout_is_sent_to_the_engine(monkeypatch):
    from cron.scheduler import _run_job_via_factr_engine

    server, url, seen = _engine_stub()
    try:
        monkeypatch.setenv("FACTR_CRON_TIMEOUT", "1800")
        _run_job_via_factr_engine(url, "test-token-not-real", "long", "go", None)
        monkeypatch.setenv("FACTR_CRON_TIMEOUT", "0")
        _run_job_via_factr_engine(url, "test-token-not-real", "unlimited", "go", None)
        _run_job_via_factr_engine(url, "test-token-not-real", "explicit", "go", None, timeout_s=90)
        assert [b["timeout_s"] for _, b, _ in seen] == [1800, 3600, 90]
    finally:
        server.shutdown()


def test_cron_run_id_rides_along_and_nothing_is_booked_into_state_db(monkeypatch):
    import cron.scheduler as sched

    assert not hasattr(sched, "_record_engine_cron_run"), "the engine session is the one run record"
    server, url, seen = _engine_stub()
    try:
        sched._run_job_via_factr_engine(url, "test-token-not-real", "nightly", "go", None, run_id="cron_j1_20261003_010203")
        sched._run_job_via_factr_engine(url, "test-token-not-real", "nightly", "go", None)
        assert seen[0][1]["run_id"] == "cron_j1_20261003_010203" and "run_id" not in seen[1][1]
    finally:
        server.shutdown()
