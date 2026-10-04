"""Session key for a connection operation."""

from __future__ import annotations

from typing import Optional


def operation_session_key(session_id: Optional[str]) -> str:
    """The key an operation is registered under: the gateway session key the RPCs look up by
    (``FACTR_SESSION_KEY``), falling back to the agent's session id where no gateway bound one.
    The agent id alone is wrong on the desktop: compaction rotates it mid-turn while the gateway
    key stays, and a card keyed by the old id can no longer be driven."""
    from gateway.session_context import get_session_env

    return get_session_env("FACTR_SESSION_KEY", "") or str(session_id or "")
