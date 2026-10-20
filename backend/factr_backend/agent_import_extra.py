"""Imports beyond the Claude Code / Codex core: Hermes homes, OpenCode, subagent definitions (bots),
scheduled jobs, system prompts and chats.

Same rules as :mod:`factr_backend.agent_import`: secrets are never copied, every step is recorded as an
item on the importer (imported / skipped / conflict / error), and a dry run touches nothing. Anything that
could act on its own is imported switched off: scheduled jobs arrive paused so nothing fires until the
user has looked at it.
"""

from __future__ import annotations

import json
import re
import sqlite3
import time
import uuid
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Dict, Iterator, List, Optional, Tuple

import yaml

from utils import atomic_write_text

# The persona Hermes writes on first run: importing it would only replace yours with a stock prompt.
HERMES_DEFAULT_SOUL_PREFIX = "You are Hermes Agent"
MAX_CHAT_SESSIONS = 500
MAX_CHAT_TEXT = 20_000
CHAT_SOURCE = "cli"


def _read(path: Path) -> str:
    return path.read_text(encoding="utf-8", errors="replace")


def parse_frontmatter(text: str) -> Tuple[Dict[str, Any], str]:
    """Split ``---`` YAML frontmatter from the body; no frontmatter → ``({}, text)``."""
    match = re.match(r"^---\s*\n(.*?)\n---\s*\n?(.*)$", text or "", re.DOTALL)
    if not match:
        return {}, text or ""
    try:
        meta = yaml.safe_load(match.group(1))
    except yaml.YAMLError:
        return {}, match.group(2)
    return (meta if isinstance(meta, dict) else {}), match.group(2)


def slugify(name: str) -> str:
    return re.sub(r"[^a-z0-9]+", "-", (name or "").lower()).strip("-")[:48]


# --- system prompt ------------------------------------------------------------------------------------------

def import_soul(imp, source: Path, default_prefix: str = "") -> None:
    """A persona file (SOUL.md) → the profile's SOUL.md. Never replaces a different one without ``overwrite``."""
    from factr_backend.agent_import import backup_memory_file

    destination = imp.target_root / "SOUL.md"
    if not source.is_file():
        imp.record("system-prompt", None, destination, "skipped", f"No {source.name} found")
        return
    text = _read(source).strip()
    if not text:
        imp.record("system-prompt", source, destination, "skipped", f"{source.name} is empty")
        return
    if default_prefix and text.startswith(default_prefix):
        imp.record("system-prompt", source, destination, "skipped", "Stock persona, nothing of yours to bring over")
        return
    if destination.exists():
        current = _read(destination).strip()
        if current == text:
            imp.record("system-prompt", source, destination, "skipped", "Already the same")
            return
        if not imp.overwrite:
            imp.record("system-prompt", source, destination, "conflict",
                       "This profile already has its own SOUL.md; kept it")
            return

    def write() -> Optional[str]:
        try:
            backup_memory_file(destination)
            atomic_write_text(destination, text + "\n")
        except OSError as exc:
            return f"Could not write SOUL.md: {exc}"
        return None

    imp.apply("system-prompt", source, destination, "Would write SOUL.md", write)


# --- memories -----------------------------------------------------------------------------------------------

def import_delimited_memory(imp, source: Path, destination_name: str) -> None:
    """A Hermes memory file (entries joined by §) → the matching file in memories/."""
    destination = imp.target_root / "memories" / destination_name
    if not source.is_file():
        imp.record("memories", None, destination, "skipped", f"No {source.name} found")
        return
    incoming = [e.strip() for e in _read(source).split("§") if e.strip()]
    if not incoming:
        imp.record("memories", source, destination, "skipped", "No entries found")
        return
    imp.merge_memory_entries("memories", source, destination, incoming)


# --- scheduled jobs -----------------------------------------------------------------------------------------

def _now_iso() -> str:
    return datetime.now(timezone.utc).astimezone().isoformat()


def import_cron_jobs(imp, jobs_file: Path) -> None:
    """Hermes cron/jobs.json → cron/jobs.json. Jobs arrive paused with no next run, so nothing fires
    until the user reviews and resumes it; delivery to chat platforms is reset to local."""
    destination = imp.target_root / "cron" / "jobs.json"
    if not jobs_file.is_file():
        imp.record("cron-jobs", None, destination, "skipped", "No scheduled jobs found")
        return
    try:
        data = json.loads(_read(jobs_file))
    except json.JSONDecodeError as exc:
        imp.record("cron-jobs", jobs_file, destination, "error", f"Could not parse {jobs_file.name}: {exc}")
        return
    source_jobs = data.get("jobs", []) if isinstance(data, dict) else data
    source_jobs = [j for j in source_jobs
                   if isinstance(j, dict) and (j.get("prompt") is not None or j.get("script"))]
    if not source_jobs:
        imp.record("cron-jobs", jobs_file, destination, "skipped", "No scheduled jobs found")
        return

    existing_doc: Dict[str, Any] = {"jobs": []}
    if destination.exists():
        try:
            loaded = json.loads(_read(destination))
            existing_doc = loaded if isinstance(loaded, dict) else {"jobs": loaded}
        except json.JSONDecodeError as exc:
            imp.record("cron-jobs", jobs_file, destination, "error",
                       f"Refusing to overwrite an unreadable jobs.json ({exc})")
            return
    existing = [j for j in existing_doc.get("jobs", []) if isinstance(j, dict)]
    seen = {(j.get("name"), j.get("prompt")) for j in existing}

    fresh: List[Dict[str, Any]] = []
    for job in source_jobs:
        name = job.get("name") or "imported job"
        if (job.get("name"), job.get("prompt")) in seen:
            imp.record("cron-job", name, destination, "skipped", "A job with this name and prompt already exists")
            continue
        schedule = job.get("schedule")
        if not isinstance(schedule, dict) or not schedule.get("kind"):
            imp.record("cron-job", name, None, "skipped", "Job has no usable schedule")
            continue
        record = {k: v for k, v in job.items()
                  if k not in {"provider_snapshot", "model_snapshot", "provider", "base_url", "origin"}}
        record.update({
            "id": uuid.uuid4().hex[:12], "enabled": False, "state": "paused", "paused_at": _now_iso(),
            "paused_reason": f"Imported from {imp.agent}: review, then resume",
            "next_run_at": None, "last_run_at": None, "last_status": None, "last_error": None,
            "last_delivery_error": None, "created_at": _now_iso(), "origin": None,
            "provider": None, "provider_snapshot": None, "model_snapshot": None, "base_url": None,
            "deliver": "local",
        })
        if isinstance(record.get("repeat"), dict):
            record["repeat"] = {**record["repeat"], "completed": 0}
        details = {"schedule": job.get("schedule_display") or schedule.get("display") or schedule.get("kind")}
        if job.get("deliver") not in (None, "local"):
            details["delivery_reset_from"] = job.get("deliver")
        fresh.append(record)
        imp.record("cron-job", name, f"cron/jobs.json ({name})", "imported",
                   "" if imp.execute else "Would add, paused", **details)
    if fresh and imp.execute:
        existing_doc["jobs"] = existing + fresh
        existing_doc["updated_at"] = _now_iso()
        destination.parent.mkdir(parents=True, exist_ok=True)
        atomic_write_text(destination, json.dumps(existing_doc, indent=2) + "\n")


# --- bots (subagent definitions, Hermes profiles) -----------------------------------------------------------

def collect_agent_definitions(imp, agents_dir: Path) -> None:
    """``agents/*.md`` (Claude Code / OpenCode subagents) → bots. The caller materialises each as a profile."""
    if not agents_dir.is_dir():
        imp.record("bots", None, None, "skipped", "No agent definitions found")
        return
    files = sorted(agents_dir.glob("*.md"))
    if not files:
        imp.record("bots", agents_dir, None, "skipped", "No agent definitions found")
        return
    for md in files:
        try:
            meta, body = parse_frontmatter(_read(md))
        except OSError as exc:
            imp.record("bot", md, None, "error", f"Could not read file: {exc}")
            continue
        name = slugify(str(meta.get("name") or md.stem))
        prompt = body.strip()
        if not name or not prompt:
            imp.record("bot", md, None, "skipped", "No name or prompt")
            continue
        imp.bots.append({"name": name, "description": str(meta.get("description") or "")[:300],
                         "prompt": prompt, "source": str(md)})
        imp.record("bot", md, f"profile {name}", "imported", "" if imp.execute else "Would create a profile")


def collect_hermes_profiles(imp, profiles_dir: Path) -> None:
    """Hermes ``profiles/<name>/`` homes → bots (each becomes a profile carrying its skills, MCP, SOUL, jobs)."""
    if not profiles_dir.is_dir():
        return
    for home in sorted(p for p in profiles_dir.iterdir() if p.is_dir()):
        name = slugify(home.name)
        if not name:
            continue
        imp.bots.append({"name": name, "description": f"Imported from the Hermes profile {home.name}",
                         "home": str(home), "source": str(home)})
        imp.record("bot", home, f"profile {name}", "imported", "" if imp.execute else "Would create a profile")


def materialize_bots(imp, profiles_mod) -> List[Dict[str, Any]]:
    """Create one profile per collected bot. Returns a result row per bot (name, ok, detail)."""
    from factr_backend.agent_import import AgentImporter

    results: List[Dict[str, Any]] = []
    for bot in imp.bots:
        name = bot["name"]
        try:
            path = profiles_mod.create_profile(name=name, description=bot.get("description") or None)
        except FileExistsError:
            results.append({"name": name, "ok": False, "detail": "A profile with this name already exists"})
            continue
        except Exception as exc:  # noqa: BLE001 - one bad definition must not stop the rest
            results.append({"name": name, "ok": False, "detail": str(exc)})
            continue
        path = Path(path)
        if bot.get("prompt"):
            atomic_write_text(path / "SOUL.md", bot["prompt"].rstrip() + "\n")
        if bot.get("home"):
            nested = AgentImporter("hermes", Path(bot["home"]), path, execute=True, overwrite=True,
                                   groups=("skills", "mcp", "memory", "prompt", "cron"))
            nested.run()
        results.append({"name": name, "ok": True, "detail": str(path)})
    return results


# --- Hermes -------------------------------------------------------------------------------------------------

def run_hermes(imp) -> None:
    root = imp.source_root
    if imp.wants("prompt"):
        import_soul(imp, root / "SOUL.md", HERMES_DEFAULT_SOUL_PREFIX)
    if imp.wants("memory"):
        import_delimited_memory(imp, root / "memories" / "MEMORY.md", "MEMORY.md")
        import_delimited_memory(imp, root / "memories" / "USER.md", "USER.md")
    if imp.wants("mcp"):
        config = imp._load_source_mapping("config", root / "config.yaml", yaml.safe_load, yaml.YAMLError)
        servers = config.get("mcp_servers")
        imp.import_mcp_servers(servers if isinstance(servers, dict) else {}, kind="mcp-servers")
    if imp.wants("skills"):
        imp.import_skills(root / "skills", nested=True)
    if imp.wants("cron"):
        import_cron_jobs(imp, root / "cron" / "jobs.json")
    if imp.wants("bots"):
        collect_hermes_profiles(imp, root / "profiles")
    if imp.wants("chats"):
        import_hermes_chats(imp)


# --- OpenCode -----------------------------------------------------------------------------------------------

def _opencode_config(root: Path) -> Dict[str, Any]:
    for name in ("opencode.jsonc", "opencode.json"):
        path = root / name
        if path.is_file():
            text = re.sub(r"^\s*//.*$", "", _read(path), flags=re.MULTILINE)
            text = re.sub(r",(\s*[}\]])", r"\1", text)
            try:
                data = json.loads(text)
            except json.JSONDecodeError:
                continue
            if isinstance(data, dict):
                return data
    return {}


def opencode_servers(config: Dict[str, Any]) -> Dict[str, Any]:
    """OpenCode ``mcp`` entries (``local`` command arrays / ``remote`` urls) → the Claude-style shape."""
    servers: Dict[str, Any] = {}
    mcp = config.get("mcp")
    for name, entry in (mcp.items() if isinstance(mcp, dict) else []):
        if not isinstance(entry, dict) or entry.get("enabled") is False:
            continue
        if entry.get("type") == "remote" or entry.get("url"):
            servers[name] = {"url": entry.get("url"), "headers": entry.get("headers")}
        elif isinstance(entry.get("command"), list) and entry["command"]:
            command, *args = [str(c) for c in entry["command"]]
            servers[name] = {"command": command, "args": args, "env": entry.get("environment")}
    return servers


def run_opencode(imp) -> None:
    root = imp.source_root
    if imp.wants("mcp"):
        imp.import_mcp_servers(opencode_servers(_opencode_config(root)), kind="mcp-servers")
    if imp.wants("memory"):
        imp.import_context_file(root / "AGENTS.md", "agents-md")
    if imp.wants("bots"):
        collect_agent_definitions(imp, root / "agents")
    if imp.wants("skills"):
        commands = root / "command"
        if commands.is_dir() and any(commands.glob("*.md")):
            imp.record("slash-commands", commands, None, "skipped",
                       "OpenCode commands have no direct Factr equivalent; consider converting them into skills")


# --- chats --------------------------------------------------------------------------------------------------

def _text_of(content: Any) -> str:
    """Plain text of a message body: a string, or a list of blocks (only text blocks count)."""
    if isinstance(content, str):
        return content
    parts: List[str] = []
    for block in content if isinstance(content, list) else []:
        if isinstance(block, dict) and block.get("type") in {"text", "input_text", "output_text"}:
            if isinstance(block.get("text"), str):
                parts.append(block["text"])
    return "\n".join(parts)


def _ts(value: Any) -> float:
    if isinstance(value, (int, float)):
        return float(value) / (1000.0 if value > 1e11 else 1.0)
    if isinstance(value, str):
        try:
            return datetime.fromisoformat(value.replace("Z", "+00:00")).timestamp()
        except ValueError:
            pass
    return time.time()


def iter_claude_sessions(root: Path) -> Iterator[Dict[str, Any]]:
    """``projects/*/*.jsonl``: user / assistant text turns, tool traffic and sidechains left out."""
    for path in sorted((root / "projects").glob("*/*.jsonl")):
        messages: List[Dict[str, Any]] = []
        title = ""
        try:
            for line in path.open(encoding="utf-8", errors="replace"):
                try:
                    row = json.loads(line)
                except json.JSONDecodeError:
                    continue
                if row.get("type") == "custom-title" and row.get("customTitle"):
                    title = str(row["customTitle"])
                if row.get("type") not in {"user", "assistant"} or row.get("isSidechain"):
                    continue
                message = row.get("message")
                text = _text_of(message.get("content")) if isinstance(message, dict) else ""
                if text.strip() and not text.lstrip().startswith("<"):
                    messages.append({"role": row["type"], "text": text, "ts": _ts(row.get("timestamp"))})
        except OSError:
            continue
        yield {"id": path.stem, "title": title, "messages": messages, "cwd": None}


def iter_codex_sessions(root: Path) -> Iterator[Dict[str, Any]]:
    """``sessions/**/rollout-*.jsonl``: user / assistant messages; developer text and reasoning left out."""
    for path in sorted((root / "sessions").rglob("rollout-*.jsonl")):
        messages: List[Dict[str, Any]] = []
        session_id = path.stem
        try:
            for line in path.open(encoding="utf-8", errors="replace"):
                try:
                    row = json.loads(line)
                except json.JSONDecodeError:
                    continue
                payload = row.get("payload") if isinstance(row.get("payload"), dict) else {}
                if row.get("type") == "session_meta" and payload.get("id"):
                    session_id = str(payload["id"])
                if row.get("type") != "response_item" or payload.get("type") != "message":
                    continue
                if payload.get("role") not in {"user", "assistant"}:
                    continue
                text = _text_of(payload.get("content"))
                if text.strip() and not text.lstrip().startswith("<"):
                    messages.append({"role": payload["role"], "text": text, "ts": _ts(row.get("timestamp"))})
        except OSError:
            continue
        yield {"id": session_id, "title": "", "messages": messages, "cwd": None}


def iter_hermes_sessions(root: Path) -> Iterator[Dict[str, Any]]:
    """Hermes ``state.db`` (read-only): sessions with their user / assistant messages."""
    db = root / "state.db"
    if not db.is_file():
        return
    try:
        conn = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
        conn.row_factory = sqlite3.Row
        sessions = conn.execute("SELECT * FROM sessions ORDER BY started_at").fetchall()
    except sqlite3.Error:
        return
    try:
        for row in sessions:
            try:
                rows = conn.execute(
                    "SELECT role, content, timestamp FROM messages WHERE session_id = ? "
                    "AND role IN ('user', 'assistant') ORDER BY timestamp, id", (row["id"],)).fetchall()
            except sqlite3.Error:
                continue
            keys = row.keys()
            yield {"id": str(row["id"]), "title": (row["title"] if "title" in keys else "") or "",
                   "cwd": row["cwd"] if "cwd" in keys else None,
                   "messages": [{"role": m["role"], "text": m["content"], "ts": _ts(m["timestamp"])}
                                for m in rows if isinstance(m["content"], str) and m["content"].strip()]}
    finally:
        conn.close()


def _count_session_files(imp) -> int:
    root = imp.source_root
    if imp.agent == "claude-code":
        return sum(1 for _ in (root / "projects").glob("*/*.jsonl"))
    if imp.agent == "codex":
        return sum(1 for _ in (root / "sessions").rglob("rollout-*.jsonl"))
    db = root / "state.db"
    if not db.is_file():
        return 0
    try:
        conn = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
        try:
            return int(conn.execute("SELECT COUNT(*) FROM sessions").fetchone()[0])
        finally:
            conn.close()
    except sqlite3.Error:
        return 0


def _import_chats(imp, sessions: Iterator[Dict[str, Any]]) -> None:
    destination = imp.target_root / "state.db"
    total = _count_session_files(imp)
    if total == 0:
        imp.record("chats", None, destination, "skipped", "No chats found")
        return
    if not imp.execute:
        imp.record("chats", imp.source_root, destination, "imported", "Would import chats",
                   sessions=min(total, MAX_CHAT_SESSIONS), found=total)
        return
    from factr_state import SessionDB

    db = SessionDB(db_path=destination)
    imported = skipped = messages = 0
    try:
        for session in sessions:
            if imported >= MAX_CHAT_SESSIONS:
                break
            turns = [m for m in session["messages"] if m["text"].strip()]
            if len(turns) < 2:
                skipped += 1
                continue
            session_id = f"imp-{imp.agent}-{session['id']}"
            if db.get_session(session_id):
                skipped += 1
                continue
            db.create_session(session_id, CHAT_SOURCE, cwd=session.get("cwd") or None)
            # The row is stamped "now"; keep the chat's real date so it sorts where it belongs.
            db._write_sql("UPDATE sessions SET started_at = ? WHERE id = ?", (turns[0]["ts"], session_id))
            for turn in turns:
                db.append_message(session_id, turn["role"], turn["text"][:MAX_CHAT_TEXT], timestamp=turn["ts"])
            title = (session.get("title") or next((t["text"] for t in turns if t["role"] == "user"), ""))
            title = re.sub(r"\s+", " ", title).strip()[:80]
            if title:
                try:
                    db.set_session_title(session_id, f"{title} (from {imp.agent})")
                except Exception:  # noqa: BLE001 - a taken title never blocks the chat itself
                    pass
            imported += 1
            messages += len(turns)
    finally:
        close = getattr(db, "close", None)
        if close:
            close()
    imp.record("chats", imp.source_root, destination, "imported", "", sessions=imported,
               messages=messages, skipped=skipped)


def import_claude_chats(imp) -> None:
    _import_chats(imp, iter_claude_sessions(imp.source_root))


def import_codex_chats(imp) -> None:
    _import_chats(imp, iter_codex_sessions(imp.source_root))


def import_hermes_chats(imp) -> None:
    _import_chats(imp, iter_hermes_sessions(imp.source_root))
