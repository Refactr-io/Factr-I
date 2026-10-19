"""Deliver cron output on the owner engine, preserving its canonical transcript and deduplication."""
import json
import os
import urllib.request
import time
from pathlib import Path


def computer_target(profile: str, root: Path) -> dict | None:
    from tools.bot_relay import computer_connection_id, read_remote_roster
    return next((row for row in read_remote_roster(root)
                 if row["profile"] == profile and row["connection_id"] == computer_connection_id(profile)), None)


def deliver_other(message: str, delivery_id: str, profile: str, source: Path) -> dict:
    from tools.bot_mode_probe import _factr_root, _profile_name, _handle
    from tools.bot_relay import (enqueue_envelope, local_target, relay_root, relay_alive,
                                 REPLIES_DIR, REPLY_WAIT_SECONDS)
    root = _factr_root(source)
    if not relay_alive(root) and not os.environ.get("FACTR_BOT_COMPUTER"):
        raise RuntimeError("Desktop relay is offline; scheduled output was not delivered")
    target = computer_target(profile, root) or local_target(profile, profile)
    sender = _profile_name(source)
    key = delivery_id[:32]
    reply = relay_root(root) / REPLIES_DIR / f"{key}.json"
    if not reply.exists():
        enqueue_envelope(root, target=target, message=message, sender_profile=sender,
                         sender_handle=_handle(sender), envelope_id=key)
    deadline = time.monotonic() + REPLY_WAIT_SECONDS
    while time.monotonic() < deadline:
        if reply.exists():
            result = json.loads(reply.read_text(encoding="utf-8"))
            return {**result, "ok": not result.get("error")}
        time.sleep(0.25)
    raise TimeoutError("scheduled output completion is unverified; do not resend")


def deliver(message: str, delivery_id: str) -> dict:
    request = urllib.request.Request(
        os.environ["FACTR_ENGINE_URL"].rstrip("/") + "/api/bot/deliver",
        data=json.dumps({"text": message, "envelope_id": delivery_id}).encode("utf-8"),
        headers={"Content-Type": "application/json", "Authorization": "Bearer " + os.environ["FACTR_ENGINE_TOKEN"]},
        method="POST",
    )
    # Delivery retains the existing two attempts of at most ten minutes each.
    with urllib.request.urlopen(request, timeout=1230) as response:
        result = json.loads(response.read().decode("utf-8"))
    if not isinstance(result, dict) or "ok" not in result:
        raise ValueError("engine returned no delivery outcome; do not resend")
    return result
