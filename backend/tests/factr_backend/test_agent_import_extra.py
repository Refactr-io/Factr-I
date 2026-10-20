"""Hermes / OpenCode imports, scheduled jobs, system prompts, bots and chats (agent_import_extra)."""

import json
import sqlite3
from pathlib import Path

import pytest

from factr_backend.agent_import import AgentImporter, guess_agent, run_import


def _write(path: Path, text: str) -> Path:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")
    return path


def _hermes_home(root: Path) -> Path:
    home = root / "hermes"
    _write(home / "SOUL.md", "You are a terse release engineer.")
    _write(home / "memories" / "MEMORY.md", "Atlas deploys on Fridays.\n§\nPrefer short answers.\n")
    _write(home / "memories" / "USER.md", "Timezone: Asia/Karachi\n")
    _write(home / "config.yaml", "mcp_servers:\n  fs:\n    command: npx\n    args: [-y, server]\n"
           "    env: {API_KEY: sk-1, LEVEL: info}\n  docs:\n    url: https://example.com/mcp\n"
           "    headers: {Authorization: Bearer x, X-Team: docs}\n")
    _write(home / "skills" / "github" / "SKILL.md", "---\nname: github\n---\nbody")
    _write(home / "skills" / "devops" / "deploy" / "SKILL.md", "---\nname: deploy\n---\nbody")
    _write(home / "skills" / ".hub" / "hidden" / "SKILL.md", "---\nname: hidden\n---\n")
    jobs = {"jobs": [{"id": "abc", "name": "digest", "prompt": "Summarise", "skills": ["github"],
                      "schedule": {"kind": "interval", "minutes": 120, "display": "every 120m"},
                      "enabled": True, "state": "scheduled", "deliver": "telegram:42",
                      "next_run_at": "2026-01-01T00:00:00+00:00", "repeat": {"times": None, "completed": 3}}]}
    _write(home / "cron" / "jobs.json", json.dumps(jobs))
    return home


def _conn(path: Path):
    return sqlite3.connect(path)


def test_guess_agent(tmp_path):
    assert guess_agent(_hermes_home(tmp_path)) == "hermes"
    (tmp_path / "cx").mkdir()
    _write(tmp_path / "cx" / "config.toml", "")
    assert guess_agent(tmp_path / "cx") == "codex"
    _write(tmp_path / "oc" / "opencode.json", "{}")
    assert guess_agent(tmp_path / "oc") == "opencode"
    assert guess_agent(tmp_path / "nothing") is None


def test_hermes_dry_run_changes_nothing_and_counts(tmp_path):
    home, target = _hermes_home(tmp_path), tmp_path / "factr"
    target.mkdir()
    report = run_import("hermes", home, target, execute=False)
    assert list(target.iterdir()) == []
    assert report["counts"] == {"system-prompt": 1, "memories": 2, "mcp-servers": 2, "skill": 2, "cron-job": 1}
    assert report["dry_run"] is True


def test_hermes_execute_writes_everything_and_strips_secrets(tmp_path):
    home, target = _hermes_home(tmp_path), tmp_path / "factr"
    report = run_import("hermes", home, target, execute=True)
    assert report["summary"]["error"] == 0
    assert (target / "SOUL.md").read_text().startswith("You are a terse release engineer")
    assert (target / "skills/hermes-imports/github/SKILL.md").exists()
    assert (target / "skills/hermes-imports/deploy/SKILL.md").exists()  # the registry reads <category>/<name>
    assert not (target / "skills/hermes-imports/.hub").exists()
    assert "Atlas deploys on Fridays." in (target / "memories/MEMORY.md").read_text()
    assert "Asia/Karachi" in (target / "memories/USER.md").read_text()
    config = (target / "config.yaml").read_text()
    assert "sk-1" not in config and "Bearer" not in config and "LEVEL: info" in config and "X-Team" in config
    assert set(report["stripped_secrets"]) == {"mcp_servers.fs.env.API_KEY", "mcp_servers.docs.headers.Authorization"}


def test_jobs_arrive_paused_and_local_and_are_not_duplicated(tmp_path):
    home, target = _hermes_home(tmp_path), tmp_path / "factr"
    run_import("hermes", home, target, execute=True, groups=["cron"])
    jobs = json.loads((target / "cron/jobs.json").read_text())["jobs"]
    assert len(jobs) == 1
    job = jobs[0]
    assert job["id"] != "abc" and job["enabled"] is False and job["state"] == "paused"
    assert job["next_run_at"] is None and job["deliver"] == "local" and job["repeat"]["completed"] == 0
    assert job["skills"] == ["github"] and job["schedule"]["minutes"] == 120
    again = run_import("hermes", home, target, execute=True, groups=["cron"])
    assert len(json.loads((target / "cron/jobs.json").read_text())["jobs"]) == 1
    assert again["summary"]["skipped"] >= 1


def test_default_hermes_persona_is_not_imported(tmp_path):
    home = _hermes_home(tmp_path)
    _write(home / "SOUL.md", "You are Hermes Agent, an intelligent AI assistant created by Nous Research.")
    report = run_import("hermes", home, tmp_path / "factr", execute=True, groups=["prompt"])
    assert not (tmp_path / "factr" / "SOUL.md").exists()
    assert report["summary"]["skipped"] == 1


def test_existing_soul_is_kept_unless_overwrite(tmp_path):
    home, target = _hermes_home(tmp_path), tmp_path / "factr"
    _write(target / "SOUL.md", "Mine.")
    kept = run_import("hermes", home, target, execute=True, groups=["prompt"])
    assert (target / "SOUL.md").read_text() == "Mine." and kept["summary"]["conflict"] == 1
    run_import("hermes", home, target, execute=True, groups=["prompt"], overwrite=True)
    assert "terse release engineer" in (target / "SOUL.md").read_text()
    assert list(target.glob("SOUL.md.bak.*"))


def test_groups_limit_what_runs(tmp_path):
    home, target = _hermes_home(tmp_path), tmp_path / "factr"
    run_import("hermes", home, target, execute=True, groups=["skills"])
    assert (target / "skills").is_dir() and not (target / "config.yaml").exists()
    assert not (target / "cron").exists() and not (target / "SOUL.md").exists()


def test_opencode_mcp_agents_and_context(tmp_path):
    root = tmp_path / "oc"
    _write(root / "opencode.jsonc", '{\n // comment\n "mcp": {"local1": {"type": "local", "command": ["npx", "-y", "srv"],'
           ' "environment": {"TOKEN": "t", "MODE": "x"}}, "remote1": {"type": "remote", "url": "https://x/mcp"},'
           ' "off": {"type": "remote", "url": "https://y", "enabled": false}},}\n')
    _write(root / "AGENTS.md", "# Rules\n- Always run tests\n")
    _write(root / "agents" / "reviewer.md", "---\nname: Code Reviewer\ndescription: Reviews diffs\n---\nYou review code.")
    importer = AgentImporter("opencode", root, tmp_path / "factr", execute=True)
    report = importer.run()
    config = (tmp_path / "factr" / "config.yaml").read_text()
    assert "local1" in config and "remote1" in config and "off" not in config and "MODE: x" in config
    assert "TOKEN" not in config
    assert "Always run tests" in (tmp_path / "factr/memories/MEMORY.md").read_text()
    assert [b["name"] for b in importer.bots] == ["code-reviewer"]
    assert report["summary"]["error"] == 0


def test_bots_become_profiles(tmp_path):
    root = tmp_path / "claude"
    _write(root / "agents" / "scout.md", "---\nname: scout\ndescription: Finds things\n---\nYou scout code.")
    created = {}

    class Profiles:
        @staticmethod
        def create_profile(name, description=None, **_):
            path = tmp_path / "profiles" / name
            path.mkdir(parents=True)
            created[name] = description
            return path

    report = run_import("claude-code", root, tmp_path / "factr", execute=True, groups=["bots"], profiles_mod=Profiles)
    assert created == {"scout": "Finds things"}
    assert report["bots"] == [{"name": "scout", "ok": True, "detail": str(tmp_path / "profiles" / "scout")}]
    assert (tmp_path / "profiles/scout/SOUL.md").read_text().strip() == "You scout code."


def test_hermes_profiles_carry_their_own_skills_and_soul(tmp_path):
    home = _hermes_home(tmp_path)
    _write(home / "profiles" / "research" / "SOUL.md", "You research.")
    _write(home / "profiles" / "research" / "skills" / "notes" / "SKILL.md", "---\nname: notes\n---\nx")

    class Profiles:
        @staticmethod
        def create_profile(name, description=None, **_):
            path = tmp_path / "profiles" / name
            path.mkdir(parents=True)
            return path

    run_import("hermes", home, tmp_path / "factr", execute=True, groups=["bots"], profiles_mod=Profiles)
    assert (tmp_path / "profiles/research/SOUL.md").read_text().strip() == "You research."
    assert (tmp_path / "profiles/research/skills/hermes-imports/notes/SKILL.md").exists()


def _claude_session(root: Path, name: str, rows):
    _write(root / "projects" / "-p" / f"{name}.jsonl", "\n".join(json.dumps(r) for r in rows))


def test_claude_chats_keep_text_turns_only(tmp_path):
    root = tmp_path / "claude"
    _claude_session(root, "s1", [
        {"type": "custom-title", "customTitle": "Atlas deploy", "sessionId": "s1"},
        {"type": "user", "timestamp": "2026-01-01T10:00:00Z", "message": {"role": "user", "content": "Plan it"}},
        {"type": "assistant", "timestamp": "2026-01-01T10:00:05Z",
         "message": {"role": "assistant", "content": [{"type": "text", "text": "Sure."},
                                                      {"type": "tool_use", "name": "Bash", "input": {}}]}},
        {"type": "user", "isSidechain": True, "message": {"role": "user", "content": "side"}},
        {"type": "user", "message": {"role": "user", "content": "<system-reminder>x</system-reminder>"}},
    ])
    _claude_session(root, "tiny", [{"type": "user", "message": {"role": "user", "content": "hi"}}])
    target = tmp_path / "factr"
    preview = run_import("claude-code", root, target, execute=False, groups=["chats"])
    assert preview["counts"] == {"chats": 2} and not target.exists()
    report = run_import("claude-code", root, target, execute=True, groups=["chats"])
    from factr_state import SessionDB
    db = SessionDB(db_path=target / "state.db")
    session = db.get_session("imp-claude-code-s1")
    assert session and session["source"] == "cli"
    messages = db.get_messages("imp-claude-code-s1")
    assert [(m["role"], m["content"]) for m in messages] == [("user", "Plan it"), ("assistant", "Sure.")]
    assert db.get_session("imp-claude-code-tiny") is None
    chats = [i for i in report["items"] if i["kind"] == "chats"][0]
    assert chats["sessions"] == 1 and chats["skipped"] == 1
    # a second run adds nothing
    again = run_import("claude-code", root, target, execute=True, groups=["chats"])
    assert [i for i in again["items"] if i["kind"] == "chats"][0]["sessions"] == 0


def test_codex_chats(tmp_path):
    root = tmp_path / "codex"
    rows = [{"type": "session_meta", "payload": {"id": "cx-1"}},
            {"type": "response_item", "timestamp": "2026-01-01T10:00:00Z",
             "payload": {"type": "message", "role": "developer", "content": [{"type": "input_text", "text": "rules"}]}},
            {"type": "response_item", "timestamp": "2026-01-01T10:00:01Z",
             "payload": {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "Fix it"}]}},
            {"type": "response_item", "timestamp": "2026-01-01T10:00:09Z",
             "payload": {"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "Done"}]}}]
    _write(root / "sessions/2026/01/01/rollout-1.jsonl", "\n".join(json.dumps(r) for r in rows))
    run_import("codex", root, tmp_path / "factr", execute=True, groups=["chats"])
    from factr_state import SessionDB
    db = SessionDB(db_path=tmp_path / "factr" / "state.db")
    assert [(m["role"], m["content"]) for m in db.get_messages("imp-codex-cx-1")] == [("user", "Fix it"), ("assistant", "Done")]


def test_hermes_chats_from_its_state_db(tmp_path):
    home = _hermes_home(tmp_path)
    conn = _conn(home / "state.db")
    conn.executescript("CREATE TABLE sessions (id TEXT, title TEXT, started_at REAL, cwd TEXT);"
                       "CREATE TABLE messages (id INTEGER PRIMARY KEY, session_id TEXT, role TEXT, content TEXT, timestamp REAL);")
    conn.execute("INSERT INTO sessions VALUES ('h1', 'Release', 1700000000, '/work')")
    conn.executemany("INSERT INTO messages (session_id, role, content, timestamp) VALUES ('h1', ?, ?, ?)",
                     [("user", "Ship it?", 1700000001), ("tool", "ignored", 1700000002), ("assistant", "Yes", 1700000003)])
    conn.commit()
    conn.close()
    run_import("hermes", home, tmp_path / "factr", execute=True, groups=["chats"])
    from factr_state import SessionDB
    db = SessionDB(db_path=tmp_path / "factr" / "state.db")
    assert [(m["role"], m["content"]) for m in db.get_messages("imp-hermes-h1")] == [("user", "Ship it?"), ("assistant", "Yes")]


REAL_HERMES = Path("/private/tmp/hermes-test/home")


@pytest.mark.skipif(not (REAL_HERMES / "cron" / "jobs.json").exists(), reason="no real Hermes fixture on this machine")
def test_real_hermes_home_round_trip(tmp_path):
    """A home written by the real hermes-agent CLI (skills installed from the hub, jobs from `hermes cron create`)."""
    report = run_import("hermes", REAL_HERMES, tmp_path / "factr", execute=True)
    assert report["summary"]["error"] == 0
    jobs = json.loads((tmp_path / "factr/cron/jobs.json").read_text())["jobs"]
    assert {j["name"] for j in jobs} >= {"inbox-digest", "standup-prep"} and not any(j["enabled"] for j in jobs)
    assert (tmp_path / "factr/skills/hermes-imports/skill-creator/SKILL.md").exists()


def test_with_an_engine_home_skills_and_chats_go_where_the_app_reads_them(tmp_path):
    """The desktop lists skills and chats from <profile>/engine, not from the Python config home."""
    target = tmp_path / "factr"
    (target / "engine").mkdir(parents=True)
    root = tmp_path / "claude"
    _write(root / "skills" / "tidy" / "SKILL.md", "---\nname: tidy\n---\nbody")
    _claude_session(root, "s1", [
        {"type": "user", "timestamp": "2026-01-01T10:00:00Z", "message": {"role": "user", "content": "Plan it"}},
        {"type": "assistant", "timestamp": "2026-01-01T10:00:05Z", "message": {"role": "assistant", "content": [{"type": "text", "text": "Sure."}]}}])
    report = run_import("claude-code", root, target, execute=True, groups=["skills", "chats"])
    assert report["summary"]["error"] == 0
    assert (target / "engine/skills/claude-code-imports/tidy/SKILL.md").exists()
    assert not (target / "skills").exists() and not (target / "state.db").exists()
    files = list((target / "engine/sessions").glob("session_*.json"))
    assert len(files) == 1
    doc = json.loads(files[0].read_text())
    assert doc["id"] == files[0].stem and doc["custom_title"] == "Plan it (from claude-code)"
    assert [(m["role"], m["content"][0]["text"]) for m in doc["messages"]] == [("user", "Plan it"), ("assistant", "Sure.")]
    assert doc["created_at"].endswith("Z") and len(doc["created_at"]) == 27 and doc["status"] == "Closed"
    row = _conn(target / "engine/session-metadata-v1.sqlite3").execute(
        "SELECT session_id, custom_title FROM recent_sessions").fetchall()
    assert row == [(doc["id"], "Plan it (from claude-code)")]
    again = run_import("claude-code", root, target, execute=True, groups=["chats"])
    assert [i for i in again["items"] if i["kind"] == "chats"][0]["sessions"] == 0
    assert len(list((target / "engine/sessions").glob("session_*.json"))) == 1


def test_detection_follows_each_os_and_environment_overrides(tmp_path):
    from factr_backend.agent_import import candidate_dirs, detect_sources, find_agent_dir

    home = tmp_path / "home"
    linux = candidate_dirs("opencode", env={"XDG_CONFIG_HOME": "/x/cfg"}, home=home, platform="linux")
    assert linux == [Path("/x/cfg/opencode"), home / ".config/opencode"]
    win = candidate_dirs("hermes", env={"LOCALAPPDATA": "C:/L", "HERMES_HOME": "D:/h"}, home=home, platform="win32")
    assert win == [Path("D:/h"), home / ".hermes", Path("C:/L/hermes")]
    assert candidate_dirs("hermes", env={"LOCALAPPDATA": "C:/L"}, home=home, platform="linux") == [home / ".hermes"]
    assert candidate_dirs("claude-code", env={"CLAUDE_CONFIG_DIR": "/c"}, home=home, platform="darwin")[0] == Path("/c")
    assert candidate_dirs("codex", env={"CODEX_HOME": "/cx"}, home=home, platform="darwin")[0] == Path("/cx")

    (home / ".hermes" / "skills").mkdir(parents=True)
    (home / ".hermes" / "SOUL.md").write_text("x")
    (home / ".claude").mkdir()
    rows = {r["agent"]: r for r in detect_sources(home=home, env={}, platform="linux")}
    assert rows["hermes"]["found"] and rows["claude-code"]["found"]
    assert not rows["codex"]["found"] and rows["codex"]["path"] == str(home / ".codex")
    assert find_agent_dir("opencode", home=home, env={}) is None


def test_prime_is_read_as_whichever_layout_its_folder_has(tmp_path):
    from factr_backend.agent_import import detect_sources

    home = tmp_path / "home"
    (home / ".prime" / "agent" / "kernel-venv").mkdir(parents=True)
    assert not {r["agent"]: r for r in detect_sources(home=home, env={})}["prime"]["found"]  # only a venv: nothing to read
    _write(home / ".prime" / "agent" / "SOUL.md", "You are a careful assistant.")
    _write(home / ".prime" / "agent" / "skills" / "tidy" / "SKILL.md", "---\nname: tidy\n---\nbody")
    assert {r["agent"]: r for r in detect_sources(home=home, env={})}["prime"]["found"]
    target = tmp_path / "factr"
    report = run_import("prime", home / ".prime" / "agent", target, execute=True)
    assert report["summary"]["error"] == 0
    assert (target / "SOUL.md").read_text().startswith("You are a careful assistant")
    assert (target / "skills/prime-imports/tidy/SKILL.md").exists()
    assert run_import("prime", home, target)["summary"]["skipped"] >= 1  # a folder with nothing readable
