"""Deliver cron output on the owner engine, preserving its canonical transcript and deduplication."""
import json
import os
import urllib.request


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
