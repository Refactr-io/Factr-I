"""The engine's tool bridge: list/describe/invoke read the live registry."""

import asyncio
import json

import pytest
from fastapi import HTTPException

from factr_backend.web_routers import agent_tools


def _run(coro):
    return asyncio.run(coro)


def test_lists_registry_tools_without_loop_tools():
    names = {t["name"] for t in _run(agent_tools.list_agent_tools())["tools"]}
    assert "text_to_speech" in names
    assert not names & agent_tools._LOOP_TOOLS


def test_describe_returns_the_registry_schema_and_unknown_is_404():
    body = _run(agent_tools.describe_agent_tool("text_to_speech"))
    assert body["schema"]["name"] == "text_to_speech" and body["toolset"] == "tts"
    with pytest.raises(HTTPException) as err:
        _run(agent_tools.describe_agent_tool("no_such_tool"))
    assert err.value.status_code == 404


def test_invoke_runs_the_handler_and_refuses_loop_tools():
    out = _run(agent_tools.invoke_agent_tool(
        agent_tools.AgentToolInvoke(name="cronjob_manage", args={"action": "list"}, session_id="t")))
    assert json.loads(out["result"])["success"] is True
    with pytest.raises(HTTPException) as err:
        _run(agent_tools.invoke_agent_tool(agent_tools.AgentToolInvoke(name="memory")))
    assert err.value.status_code == 400
