#!/usr/bin/env python3
"""Connection lifecycle tool for local MCP servers from the bundled catalog."""

from typing import Any, Callable, Dict, Optional

from tools.connectors.catalog_tool import MANAGE_CATALOG_SCHEMA, manage_catalog
from tools.connectors.mcp import run_mcp_operation
from tools.connectors.targets import ALL_ACTIONS, normalize_targets, validate_action
from tools.registry import registry, tool_error


def manage_connections(
    args: Dict[str, Any],
    *,
    mcp_backend: Optional[Any] = None,
    session_id: Optional[str] = None,
    tool_call_id: Optional[str] = None,
    connection_callback: Optional[Callable[[Dict[str, Any]], Optional[str]]] = None,
) -> str:
    action = str(args.get("action") or "").strip().lower()
    mcp_targets, target_error = normalize_targets(args.get("connectors"))
    if target_error:
        return tool_error(target_error)
    action_error = validate_action(action, mcp_targets)
    if action_error:
        return tool_error(action_error)
    return run_mcp_operation(
        mcp_targets, action, backend=mcp_backend,
        connection_callback=connection_callback, session_id=session_id, tool_call_id=tool_call_id,
    )


MANAGE_CONNECTIONS_SCHEMA = {
    "name": "manage_connections",
    "description": (
        "Connect the user to apps through local MCP servers from the bundled catalog. "
        "Targets go in 'connectors' as {\"name\": \"linear\"}. 'install' adds a catalog entry, "
        "'enable' re-enables a disabled configured server, 'authorize' runs its OAuth. "
        "Pass SEVERAL names in one call. In the desktop app, the terminal UI and the interactive CLI "
        "the call shows the user an approval card and blocks until every app is connected, skipped, "
        "or the deadline passes; the result lists each target as connected / skipped / not_connected. "
        "Never hand-edit mcp_servers config: always use this tool. After a skip or a timeout, do not "
        "re-ask on your own: continue without the app or ask in chat. A later request from the USER "
        "for that same app is not a re-ask: run it. A connected server's tools are named in the result "
        "and are callable at once through tool_describe/tool_call. Where no card exists a target runs "
        "at once and the result says what happened, with a link for the user to open when one is "
        "needed. This tool can NOT disconnect, delete, or revoke an account: that is deliberately "
        "user-only. When asked, say so and direct the user to the desktop app."
    ),
    "parameters": {
        "type": "object",
        "properties": {
            "action": {
                "type": "string",
                "enum": list(ALL_ACTIONS),
                "description": "install, enable or authorize.",
            },
            "connectors": {
                "type": "array",
                "items": {
                    "anyOf": [
                        {"type": "string"},
                        {
                            "type": "object",
                            "properties": {"name": {"type": "string"}},
                            "required": ["name"],
                            "additionalProperties": False,
                        },
                    ]
                },
                "description": "MCP server names, e.g. [\"linear\", {\"name\": \"notion\"}].",
            },
        },
        "required": ["action", "connectors"],
    },
}


registry.register(
    name="manage_connections",
    toolset="connections",
    schema=MANAGE_CONNECTIONS_SCHEMA,
    handler=lambda args, **kw: manage_connections(args, session_id=kw.get("session_id")),
    emoji="🔗",
)

# The setup profile's catalog install. Reachable only through the ``setup`` toolset, which the
# profile's role grants; registry dispatch has no card callback, so it answers with the CLI pointer.
registry.register(
    name="manage_catalog",
    toolset="setup",
    schema=MANAGE_CATALOG_SCHEMA,
    handler=lambda args, **kw: manage_catalog(args, session_id=kw.get("session_id")),
    emoji="🧩",
)
