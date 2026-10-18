"""Teammate messaging for bots whose chat runs in the Rust engine (the packaged desktop app).

The engine has no ``message_agent`` tool, so each Bot-Mode bot profile gets one stdio MCP server in its OWN
``config.yaml`` (``mcp_servers.factr_teammates``). The default profile's config is never touched, so default
chats (and the benchmark harness) load exactly the tool list they always did.

This module is the seam for a later Rust port: ``ensure_teammate_mcp`` (config), the stdio server below
(tool surface), and ``deliver`` (transport). ``deliver`` runs the same cleanup-owning runner, per-profile turn
lock, live-owner admission and author attribution as the Python ``message_agent`` tool, but waits for the reply and
returns it as the tool result instead of relying on a completion notification the engine cannot consume.
A Rust port replaces ``deliver`` (and ``bot_relay.bot_cli_prefix``) and leaves the rest alone.

Run: ``python -m tools.message_agent_mcp`` (stdio, newline-delimited JSON-RPC, MCP).
"""

from __future__ import annotations

import json
import os
import shlex
import subprocess
import sys
from pathlib import Path
from typing import Any, Optional

SERVER_NAME = "factr_teammates"  # keep in step with tools.bot_relay.TEAMMATES_MCP_SERVER
TOOL_NAME = "message_agent"
# A teammate's turn can run for minutes; the engine's default MCP reply timeout is 30s.
REPLY_TIMEOUT_SECONDS = 900
_PROTOCOL_FALLBACK = "2024-11-05"


# ── config: which profiles get the server ───────────────────────────────────────────────────────

def desired_server_entry() -> dict:
    """The ``mcp_servers`` entry. Packaged Desktop exports FACTR_BACKEND_PYTHON / FACTR_BACKEND_PYTHONPATH to the
    engine, which expands ``${VAR}`` in MCP commands, so the entry survives the app moving or updating."""
    if os.environ.get("FACTR_BACKEND_PYTHON"):
        command, pythonpath = "${FACTR_BACKEND_PYTHON}", "${FACTR_BACKEND_PYTHONPATH}"
    else:
        command = sys.executable or "python3"
        pythonpath = str(Path(__file__).resolve().parent.parent)
    return {"command": command, "args": ["-m", "tools.message_agent_mcp"], "env": {"PYTHONPATH": pythonpath},
            "timeout_secs": REPLY_TIMEOUT_SECONDS}


def ensure_teammate_mcp(root: "Path | str") -> int:
    """Add (or refresh) the teammate server in every Bot-Mode-managed NAMED profile's config.yaml.
    Returns how many files were written. Never raises; the default profile is never written."""
    written = 0
    try:
        from tools.bot_mode_probe import _is_bot_managed, _roster

        desired = desired_server_entry()
        for name, profile_dir in _roster(Path(root)):
            try:
                path = Path(profile_dir) / "config.yaml"
                if name == "default" or not path.is_file() or not _is_bot_managed(Path(profile_dir)):
                    continue
                import yaml

                data = yaml.safe_load(path.read_text(encoding="utf-8")) or {}
                if not isinstance(data, dict):
                    continue
                servers = data.get("mcp_servers")
                if servers is not None and not isinstance(servers, dict):
                    continue
                if (servers or {}).get(SERVER_NAME) == desired:
                    continue
                data["mcp_servers"] = {**(servers or {}), SERVER_NAME: desired}
                from utils import atomic_yaml_write

                atomic_yaml_write(path, data)
                written += 1
            except Exception:
                continue
    except Exception:
        pass
    return written


# ── transport: deliver one message and wait for the reply ──────────────────────────────────────────

def _home() -> Path:
    from tools.bot_mode_dm import _default_home

    return Path(_default_home())


def _run(command: str, *, label: str) -> tuple[str, bool]:
    """Run a runner command to completion; (text, ok). The runner's stdout is the reply or a JSON failure."""
    try:
        proc = subprocess.run(shlex.split(command), capture_output=True, text=True, stdin=subprocess.DEVNULL,
                              timeout=REPLY_TIMEOUT_SECONDS - 30, check=False, encoding="utf-8", errors="replace")
    except subprocess.TimeoutExpired:
        return (f"No reply from {label} yet. The message may still be delivered; do not resend.", False)
    text = (proc.stdout or "").strip()
    if proc.returncode != 0:
        return (text or (proc.stderr or "").strip()[-800:] or f"Delivery to {label} failed.", False)
    return (text or "(empty reply)", True)


def deliver(target: str, message: str, home: Optional[Path] = None) -> tuple[str, bool]:
    """Deliver ``message`` from this bot to ``target`` and return ``(text, ok)``: the teammate's reply, or why not."""
    from tools import bot_mode_dm as dm
    from tools.bot_mode_probe import (
        _display_name, _factr_root, _handle, _peers, _profile_name, _roster, is_bot_mode_managed,
    )
    from tools.bot_relay import BOT_CHAT_TURN_ARGS, bot_cli_prefix

    home = Path(home or _home())
    if not is_bot_mode_managed(home):
        return ("This install has no bot roster, so message_agent is unavailable. Do not retry.", False)
    root, me = _factr_root(home), _profile_name(home)
    homes = dict(_roster(root))
    body = str(message or "").strip()
    if not body:
        return ("message is required: write what you want to say to that agent.", False)
    if len(body) > dm.MESSAGE_MAX_CHARS:
        return (f"message too long ({len(body)} > {dm.MESSAGE_MAX_CHARS} chars). Send the essentials.", False)
    raw = str(target or "").strip().lstrip("@")
    teammates = [_handle(n) for n in homes if n != me]
    if not raw:
        return (f"target is required. Teammates: {', '.join(teammates) or 'none'}.", False)
    content = f"Message from 🤖 {_display_name(me, homes.get(me, home))} (@{_handle(me)}): " + body
    author = {"id": f"bot:{me}", "name": _handle(me), "is_bot": True}

    peers = _peers(root)
    peer = dm._PEER_TARGET_RE.match(raw)
    if peer or raw.lower() in peers:
        name, profile = peer.groups() if peer else (raw.lower(), None)
        if name not in peers:
            return (f"No registered peer named '{name}'.", False)
        from agent.turn_author import bot_author_id, local_origin

        dm_target = f"{name}/{profile}" if profile else name
        file = dm._write_dm_file(content)
        cmd = dm._delivery_command([*bot_cli_prefix(), "-p", me, "peer", "dm", dm_target], file, stdin_file=True,
                                   author={**author, "id": bot_author_id(me, local_origin())})
        return _run(cmd, label=f"@{profile or name} on peer '{name}'")

    resolved = dm._resolve_local_name(raw, list(homes), root)
    if resolved is not None and resolved != me:
        from tools.bot_relay import HAS_COMPUTER_MESSAGE, has_computer, local_target, relay_alive
        if has_computer(homes[resolved]):
            return (f"@{_handle(resolved)}: {HAS_COMPUTER_MESSAGE}. Do not retry.", False)
        if relay_alive(root):
            # The Desktop relay is running: the message becomes a real turn in the teammate's own Bot Chat, on its own
            # engine (`bot.chat.deliver`), not a Python CLI turn in a session its Bot Chat never shows.
            text = _relay_local(root, me, content, local_target(resolved, _handle(resolved)))
            if text is not None:
                return text
        file = dm._write_dm_file(content)
        cmd = dm._delivery_command([*bot_cli_prefix(), "-p", resolved, *BOT_CHAT_TURN_ARGS], file,
                                   stdin_file=False, profile_home=homes[resolved], author=author)
        return _run(cmd, label=f"@{_handle(resolved)}")

    # Another connected machine, through the Desktop relay: queue the envelope, wait on its reply file.
    try:
        from tools.bot_mode_probe import local_taken_forms
        from tools.bot_relay import (
            EnvelopeRefusedError, _target_aliases, enqueue_envelope, read_remote_roster, remote_target_forms,
            resolve_remote_target, waiter_command,
        )

        roster = read_remote_roster(root)
        match = resolve_remote_target(raw, roster) if roster else None
        if match == "ambiguous":
            want = raw.partition("@")[0].lower()
            forms = ", ".join(f for r, f in zip(roster, remote_target_forms(roster, local_taken_forms(root)))
                              if want in _target_aliases(r))
            return (f"'{raw}' exists on several connected machines; use one of: {forms}.", False)
        if match is not None:
            try:
                envelope = enqueue_envelope(root, target=match, message=content, sender_profile=me,
                                            sender_handle=_handle(me))
            except EnvelopeRefusedError as exc:
                return (json.dumps({"error": str(exc), "reason": exc.reason}), False)
            return _run(waiter_command(root, envelope), label=f"@{match['handle']}")
    except Exception as exc:
        return (f"Relay delivery failed: {exc}", False)
    if resolved == me:
        return ("You can't message yourself. Pick a teammate from the roster.", False)
    return (f"No teammate named '{raw}'. Teammates: {', '.join(teammates) or 'none'}.", False)


def _relay_local(root: Path, me: str, content: str, match: dict) -> "tuple[str, bool] | None":
    """Queue ``content`` for another bot on this install through the Desktop relay and wait for its reply.
    None when the relay could not be used (the caller then runs the local CLI turn instead)."""
    try:
        from tools.bot_mode_probe import _handle
        from tools.bot_relay import EnvelopeRefusedError, enqueue_envelope, waiter_command

        try:
            envelope = enqueue_envelope(root, target=match, message=content, sender_profile=me,
                                        sender_handle=_handle(me))
        except EnvelopeRefusedError as exc:
            return (json.dumps({"error": str(exc), "reason": exc.reason}), False)
        return _run(waiter_command(root, envelope), label=f"@{match['handle']}")
    except Exception:
        return None


# ── MCP stdio server ──────────────────────────────────────────────────────────────────────────

def _tool_description(home: Path) -> str:
    base = ("Send a message to another bot (teammate) on this install, an agent on another connected machine, or a "
            "registered peer, and WAIT for their reply, which is returned as the result. Write the message yourself "
            "(lead with the point and the concrete ask); never paste the user's words verbatim. Message one clearly "
            "relevant teammate only when it helps the user's goal.")
    try:
        from tools.bot_mode_probe import _factr_root, _handle, _profile_name, _profile_role, _roster

        root, me = _factr_root(home), _profile_name(home)
        lines = [f"- {_handle(n)}" + (f": {_profile_role(d)}" if _profile_role(d) else "")
                 for n, d in _roster(root) if n != me]
        if lines:
            base += "\nTeammates (use the name as target):\n" + "\n".join(lines)
    except Exception:
        pass
    return base


def _tool_schema(home: Path) -> dict:
    return {"name": TOOL_NAME, "description": _tool_description(home), "inputSchema": {
        "type": "object", "additionalProperties": False, "required": ["target", "message"],
        "properties": {"target": {"type": "string", "description": "Teammate name, '<handle>@<connection>' or '<peer>/<agent>'."},
                       "message": {"type": "string", "description": "The message you composed for that agent."}}}}


def handle(request: dict, home: Optional[Path] = None) -> Optional[dict]:
    """One JSON-RPC request -> response dict (None for notifications)."""
    rid, method, params = request.get("id"), request.get("method"), request.get("params") or {}
    home = Path(home or _home())
    if rid is None:
        return None

    def ok(result: Any) -> dict:
        return {"jsonrpc": "2.0", "id": rid, "result": result}

    if method == "initialize":
        return ok({"protocolVersion": params.get("protocolVersion") or _PROTOCOL_FALLBACK,
                   "capabilities": {"tools": {}}, "serverInfo": {"name": SERVER_NAME, "version": "1"}})
    if method == "ping":
        return ok({})
    if method == "tools/list":
        return ok({"tools": [_tool_schema(home)]})
    if method == "tools/call":
        if params.get("name") != TOOL_NAME:
            return {"jsonrpc": "2.0", "id": rid, "error": {"code": -32602, "message": f"unknown tool {params.get('name')}"}}
        args = params.get("arguments") or {}
        try:
            text, good = deliver(str(args.get("target") or ""), str(args.get("message") or ""), home)
        except Exception as exc:
            text, good = f"Delivery failed: {exc}", False
        return ok({"content": [{"type": "text", "text": text}], "isError": not good})
    return {"jsonrpc": "2.0", "id": rid, "error": {"code": -32601, "message": f"method not found: {method}"}}


def main() -> int:
    sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            response = handle(json.loads(line))
        except Exception as exc:
            response = {"jsonrpc": "2.0", "id": None, "error": {"code": -32700, "message": str(exc)}}
        if response is not None:
            sys.stdout.write(json.dumps(response) + "\n")
            sys.stdout.flush()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
