"""``request_takeover``: the agent asks the human to take over its Bot Desktop (login, 2FA, CAPTCHA).

Records the ask on the lease (``Lease.request``) so the Screen pane shows it and the human can take over with
the reason already on screen, makes sure the desktop is up, and tells the model to wait for the hand-back
instead of asking for credentials in chat. Offered only where a Bot Desktop can run (Linux with the packages,
which is every bot computer)."""

from __future__ import annotations

import json
import time

from tools.registry import no_cache_check_fn, registry, tool_error

_MAX_WAIT = 600

SCHEMA = {
    "name": "request_takeover",
    "description": (
        "Ask the user to take over your Screen (the bot desktop) for something only a person should do: log in, "
        "enter a 2FA code, solve a CAPTCHA, enter card details. The user is notified in their Screen tab. "
        "NEVER ask for passwords, codes or card numbers in chat. After calling, tell the user in one line what "
        "you need, then wait for the hand-back (set wait_seconds to block until they hand control back) "
        "before continuing; screen actions are refused while they hold control."),
    "parameters": {
        "type": "object",
        "properties": {
            "site": {"type": "string", "description": "The site or app that needs the person, e.g. 'github.com'."},
            "why": {"type": "string", "description": "What they need to do, in one short sentence."},
            "wait_seconds": {"type": "integer", "minimum": 0, "maximum": _MAX_WAIT,
                             "description": "Block up to this long for the hand-back (default 0: return at once)."},
        },
        "required": ["site", "why"],
    },
}


def _available() -> bool:
    from tools.bot_desktop import runtime
    return runtime.is_supported_host() and not runtime.missing_binaries()


def _wait_for_handback(lease, seconds: int) -> str:
    """'handed_back' once a human held and returned control, 'timeout' otherwise."""
    deadline, seen_human = time.monotonic() + seconds, False
    while time.monotonic() < deadline:
        holder = lease.get().holder
        seen_human = seen_human or holder == lease.HUMAN
        if seen_human and holder == lease.AGENT:
            return "handed_back"
        time.sleep(1)
    return "timeout"


def handle_request_takeover(args: dict, **_kw) -> str:
    site, why = str(args.get("site") or "").strip()[:80], str(args.get("why") or "").strip()[:200]
    if not site or not why:
        return tool_error("site and why are both required")
    from tools.bot_desktop import lease, runtime
    try:
        runtime.start()
    except Exception as exc:
        return tool_error(f"The desktop could not start ({exc}). Tell the user in your reply what you need them to do.")
    if lease.get().holder == lease.HUMAN:
        status = "human_already_in_control"
    else:
        lease.request_takeover(f"{site}: {why}")
        status = "requested"
        wait = max(0, min(int(args.get("wait_seconds") or 0), _MAX_WAIT))
        if wait:
            status = _wait_for_handback(lease, wait)
    note = {
        "requested": "The user was asked in their Screen tab. Do not ask for credentials in chat. End your turn with one "
                     "line saying what you need; continue when they tell you they handed control back.",
        "human_already_in_control": "The user is already in control of the screen. Do not touch it or ask for credentials in "
                                    "chat; wait for them to hand control back.",
        "handed_back": "The user finished and handed control back. Take a fresh screenshot or snapshot and continue.",
        "timeout": "No hand-back yet. Do not ask for credentials in chat; end your turn and tell the user you are waiting.",
    }[status]
    return json.dumps({"status": status, "site": site, "note": note}, ensure_ascii=False)


registry.register(
    name="request_takeover",
    toolset="bot_desktop",
    schema=SCHEMA,
    handler=handle_request_takeover,
    check_fn=no_cache_check_fn(_available),
    emoji="🙋",
)
