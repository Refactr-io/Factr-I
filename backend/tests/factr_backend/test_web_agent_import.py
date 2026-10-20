"""``/api/agent-import``: detect, preview and run an import of another agent's setup into a profile."""

from __future__ import annotations

import json
from pathlib import Path

import pytest

pytest.importorskip("fastapi")
from fastapi.testclient import TestClient  # noqa: E402


@pytest.fixture()
def env(tmp_path, monkeypatch):
    monkeypatch.setattr(Path, "home", lambda: tmp_path)
    home = tmp_path / ".factr"
    home.mkdir(exist_ok=True)
    monkeypatch.setenv("FACTR_CONFIG_HOME", str(home))
    claude = tmp_path / ".claude"
    (claude / "skills" / "tidy").mkdir(parents=True)
    (claude / "skills" / "tidy" / "SKILL.md").write_text("---\nname: tidy\n---\nbody")
    (claude / "CLAUDE.md").write_text("# Rules\n- Always run tests\n")
    (claude / "agents").mkdir()
    (claude / "agents" / "scout.md").write_text("---\nname: scout\ndescription: Finds things\n---\nYou scout.")
    return tmp_path


@pytest.fixture()
def client(env):
    from factr_backend import web_server

    with TestClient(web_server.app, raise_server_exceptions=False) as c:
        c.headers["Authorization"] = f"Bearer {web_server._SESSION_TOKEN}"
        yield c


def test_detect_lists_found_agents_with_counts(client):
    body = client.get("/api/agent-import/detect").json()
    rows = {r["agent"]: r for r in body["agents"]}
    assert rows["claude-code"]["found"] and rows["claude-code"]["counts"]["skill"] == 1
    assert rows["claude-code"]["counts"]["bot"] == 1
    assert not rows["hermes"]["found"] and rows["hermes"]["counts"] == {}
    assert "chats" in body["groups"]


def test_preview_then_run_writes_into_the_profile(client, env):
    preview = client.post("/api/agent-import", json={"agent": "claude-code", "groups": ["skills", "memory"]}).json()
    assert preview["dry_run"] is True and not (env / ".factr/skills/claude-code-imports").exists()
    run = client.post("/api/agent-import", json={"agent": "claude-code", "groups": ["skills", "memory"],
                                                 "execute": True}).json()
    assert run["summary"]["error"] == 0
    assert (env / ".factr/skills/claude-code-imports/tidy/SKILL.md").exists()
    assert "Always run tests" in (env / ".factr/memories/MEMORY.md").read_text()


def test_bots_become_real_profiles(client, env):
    run = client.post("/api/agent-import", json={"agent": "claude-code", "groups": ["bots"], "execute": True}).json()
    assert [b["name"] for b in run["bots"]] == ["scout"] and run["bots"][0]["ok"]
    profiles = client.get("/api/profiles").json()
    names = [p["name"] if isinstance(p, dict) else p for p in (profiles.get("profiles", profiles))]
    assert "scout" in names


def test_auto_guesses_a_picked_folder_and_rejects_unknown_ones(client, env):
    picked = env / "somewhere"
    (picked / "cron").mkdir(parents=True)
    (picked / "cron" / "jobs.json").write_text(json.dumps({"jobs": [
        {"name": "j", "prompt": "p", "schedule": {"kind": "interval", "minutes": 5}}]}))
    ok = client.post("/api/agent-import", json={"agent": "auto", "source": str(picked), "groups": ["cron"],
                                                "execute": True}).json()
    assert ok["agent"] == "hermes" and ok["summary"]["imported"] == 1
    empty = env / "empty"
    empty.mkdir()
    assert client.post("/api/agent-import", json={"agent": "auto", "source": str(empty)}).status_code == 400
    assert client.post("/api/agent-import", json={"agent": "nope"}).status_code == 400
    assert client.post("/api/agent-import", json={"agent": "codex", "source": str(env / "missing")}).status_code == 404
