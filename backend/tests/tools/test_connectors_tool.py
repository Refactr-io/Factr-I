"""Behavior tests for manage_connections.

DI-callable idiom: a fake client injected through manage_connections'
seams; no module mocks, no network.
"""

import json

from tools.connectors.tool import MANAGE_CONNECTIONS_SCHEMA, manage_connections


def test_install_without_connectors_is_a_usage_error():
    out = json.loads(manage_connections({"action": "install"}))
    assert "requires 'connectors'" in out["error"]


def test_disconnect_is_refused():
    # De-authentication is user-only: the tool rejects it up front.
    out = json.loads(manage_connections({"action": "disconnect", "connectors": ["gmail"]}))
    assert "action must be one of" in out["error"]


def test_mcp_actions_are_the_only_actions():
    enum = MANAGE_CONNECTIONS_SCHEMA["parameters"]["properties"]["action"]["enum"]
    assert set(enum) == {"install", "enable", "authorize"}

    out = json.loads(manage_connections({"action": "connect", "connectors": ["linear"]}))
    assert "action must be one of" in out["error"]

    out = json.loads(manage_connections({"action": "uninstall", "connectors": ["gmail"]}))
    assert "action must be one of" in out["error"]


# ---------------------------------------------------------------------------
# reachability: a registered tool nobody enables is a tool nobody can call
# ---------------------------------------------------------------------------


def _session_tool_names(enabled_toolsets, *, connectors=True, disabled_toolsets=None):
    """Tool names a session would actually receive, through the real assembly.

    Skips the tool_search step so the assertion is about NAME resolution and
    check_fn, not about how many MCP servers the developer running the suite
    happens to have configured.
    """
    from model_tools import _compute_tool_definitions
    from tools.registry import invalidate_check_fn_cache

    invalidate_check_fn_cache()
    try:
        defs = _compute_tool_definitions(
            enabled_toolsets=enabled_toolsets,
            disabled_toolsets=disabled_toolsets,
            quiet_mode=True,
            skip_tool_search_assembly=True,
        )
    finally:
        invalidate_check_fn_cache()
    return {d["function"]["name"] for d in defs}



def test_cli_session_gets_the_tool_outside_a_code_workspace(tmp_path, monkeypatch):
    """The path a plain `factr` run takes: _get_platform_tools, no git cwd."""
    from factr_backend.tools_config import _get_platform_tools

    monkeypatch.chdir(tmp_path)
    enabled = sorted(_get_platform_tools({}, "cli", include_default_mcp_servers=True))

    assert "connections" in enabled
    assert "manage_connections" in _session_tool_names(enabled, connectors=True)




def test_tui_and_desktop_sessions_get_the_tool(monkeypatch):
    """The path the TUI/desktop gateway takes to build its selection."""
    from tui_gateway.server import _load_enabled_toolsets

    monkeypatch.delenv("FACTR_TUI_TOOLSETS", raising=False)
    for platform in ("tui", "desktop"):
        selection = _load_enabled_toolsets(platform)
        names = _session_tool_names(selection, connectors=True)
        assert "manage_connections" in names, platform


def test_focus_mode_coding_posture_gets_the_tool(monkeypatch):
    """An engineer pinned to the coding posture still sees their accounts."""
    from pathlib import Path

    from agent.coding_context import coding_selection

    repo = Path(__file__).resolve().parents[2]
    monkeypatch.chdir(repo)
    selection = coding_selection(
        platform="cli", cwd=str(repo), config={"agent": {"coding_context": "focus"}}
    )
    assert selection == ["coding"]  # posture collapse still collapses
    assert "manage_connections" in _session_tool_names(selection, connectors=True)


def test_operator_can_still_turn_it_off(tmp_path, monkeypatch):
    """`agent.disabled_toolsets: [connections]` wins; a bundle name does not.

    The name is added before the disabled subtraction, so the toolset behaves
    like any other. Naming a platform composite instead must NOT strip it —
    that branch preserves core tools on purpose (#33924).
    """
    from factr_backend.tools_config import _get_platform_tools

    monkeypatch.chdir(tmp_path)
    enabled = sorted(_get_platform_tools({}, "cli", include_default_mcp_servers=True))

    assert "manage_connections" not in _session_tool_names(
        enabled, connectors=True, disabled_toolsets=["connections"]
    )
    assert "manage_connections" in _session_tool_names(
        enabled, connectors=True, disabled_toolsets=["factr-cli"]
    )


def test_tool_is_never_deferrable():
    from tools.tool_search import is_deferrable_tool_name

    # Core names short-circuit before the toolset check, so listing
    # "connections" in _DIRECT_SURFACE_TOOLSETS would be redundant.
    assert is_deferrable_tool_name("manage_connections") is False
