"""Agent tool bridge for the Rust engine: list, describe and invoke Factr registry tools.

The engine (``factr-engine``) runs the agent loop; the tools only Factr implements (image
generation, vision, TTS, cron jobs, messaging, Home Assistant, computer use, kanban, ...) stay here.
Schemas are read from the live registry at request time, so nothing is copied into the engine.
Which tools the engine may call (its own native toolsets, per-run denylists, approvals) is decided
on the engine side; this router only exposes the registry.
"""

import contextlib
import json
import os
import threading
from typing import Any, Dict, Optional

from fastapi import APIRouter, HTTPException
from pydantic import BaseModel
from starlette.concurrency import run_in_threadpool

router = APIRouter()

# Tools the agent loop itself owns; the registry entry is a stub that errors when dispatched.
_LOOP_TOOLS = {"todo_list", "memory", "session_search", "delegate_task"}


_SURFACE_LOCK = threading.RLock()


@contextlib.contextmanager
def _agent_surface():
    """Mark the process interactive while the engine's agent turn is served.

    Tools such as ``cronjob_manage`` only advertise themselves (``check_fn``) when a person is driving
    the session, and the engine's turns are exactly that. The flag is restored afterwards, and it is the
    only one set: ``FACTR_EXEC_ASK`` would route terminal approvals to callbacks this process lacks.
    """
    with _SURFACE_LOCK:
        was = os.environ.get("FACTR_INTERACTIVE")
        os.environ["FACTR_INTERACTIVE"] = "1"
        try:
            yield
        finally:
            if was is None:
                os.environ.pop("FACTR_INTERACTIVE", None)
            else:
                os.environ["FACTR_INTERACTIVE"] = was


class AgentToolInvoke(BaseModel):
    name: str
    args: Dict[str, Any] = {}
    session_id: Optional[str] = None


def _definitions() -> Dict[str, Dict[str, Any]]:
    """name -> {"toolset", "schema"} for every tool whose availability check passes right now."""
    import model_tools
    from tools.registry import registry

    out: Dict[str, Dict[str, Any]] = {}
    with _agent_surface():
        definitions = model_tools.get_tool_definitions(
            enabled_toolsets=list(registry.get_registered_toolset_names()), quiet_mode=True, skip_tool_search_assembly=True)
    for definition in definitions:
        schema = definition.get("function") or {}
        name = schema.get("name")
        if name and name not in _LOOP_TOOLS:
            out[name] = {"toolset": registry.get_toolset_for_tool(name) or "", "schema": schema}
    return out


def _summary(description: str) -> str:
    first = (description or "").strip().split("\n", 1)[0]
    return first if len(first) <= 200 else first[:197] + "..."


@router.get("/api/agent-tools")
async def list_agent_tools():
    defs = await run_in_threadpool(_definitions)
    return {"tools": [
        {"name": name, "toolset": info["toolset"], "description": _summary(info["schema"].get("description", ""))}
        for name, info in sorted(defs.items())]}


@router.get("/api/agent-tools/{name}")
async def describe_agent_tool(name: str):
    info = (await run_in_threadpool(_definitions)).get(name)
    if info is None:
        raise HTTPException(status_code=404, detail=f"tool '{name}' is unknown or unavailable (missing credentials?)")
    return {"name": name, "toolset": info["toolset"], "schema": info["schema"]}


@router.post("/api/agent-tools/invoke")
async def invoke_agent_tool(body: AgentToolInvoke):
    if body.name in _LOOP_TOOLS:
        raise HTTPException(status_code=400, detail=f"{body.name} is handled by the agent loop")
    if body.name not in await run_in_threadpool(_definitions):
        raise HTTPException(status_code=404, detail=f"tool '{body.name}' is unknown or unavailable (missing credentials?)")

    def run() -> str:
        import model_tools

        with _agent_surface():
            return model_tools.handle_function_call(
                body.name, dict(body.args), task_id=body.session_id, session_id=body.session_id)

    raw = await run_in_threadpool(run)
    if not isinstance(raw, str):
        raw = json.dumps(raw, ensure_ascii=False, default=str)
    return {"result": raw}
