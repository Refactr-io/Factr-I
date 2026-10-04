from __future__ import annotations

from typing import Any, List, Optional, Tuple

MCP_ACTIONS = ("install", "enable", "authorize")
ALL_ACTIONS = MCP_ACTIONS

_TARGET_FIELDS = frozenset({"name", "mcp"})


def normalize_targets(raw: Any) -> Tuple[List[str], Optional[str]]:
    """MCP server names from ``connectors``: bare names or ``{"name": ...}`` objects."""
    if raw is None:
        return [], None
    if isinstance(raw, (str, dict)):
        raw = [raw]
    if not isinstance(raw, list):
        return [], "'connectors' must be a list of names or {name} objects."
    names: List[str] = []
    for item in raw:
        if isinstance(item, dict):
            unknown = sorted(set(item) - _TARGET_FIELDS)
            if unknown:
                return [], (
                    f"unknown target field(s) {', '.join(unknown)}: a target is "
                    "{\"name\": \"<server>\"}. Transport, URLs and credentials come from the "
                    "catalog manifest, never from the call."
                )
            name = str(item.get("name") or "").strip().lower()
        else:
            name = str(item or "").strip().lower()
        if not name:
            return [], "every target needs a non-empty 'name'."
        if name not in names:
            names.append(name)
    return names, None


def validate_action(action: str, mcp: List[str]) -> Optional[str]:
    if action not in ALL_ACTIONS:
        return (
            f"action must be one of {', '.join(ALL_ACTIONS)}. Disconnecting an account is done "
            "by the user in the desktop app, not through this tool."
        )
    if not mcp:
        return (
            f"'{action}' requires 'connectors': the MCP server name(s), e.g. "
            "[{\"name\": \"linear\"}]."
        )
    return None
