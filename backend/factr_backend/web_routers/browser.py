"""REST bridge exposing Factr's own browser tool to the Rust (factr) engine.

M8a: the engine's chat turns run with factr's own tools (crates/factr-app-core);
browsing is not reimplemented there. One route lets the engine's `browser` tool
forward a single action to the functions in ``tools.browser_tool`` — the same
backends (local Chromium, Browser Use / Browserbase / Firecrawl cloud, a CDP
endpoint, or Camofox) the Python agent itself uses. One owner for the browser
implementation; the engine only calls it.

Auth is the standard dashboard session-token middleware (``X-Factr-Session-
Token`` / ``FACTR_DASHBOARD_SESSION_TOKEN``, see web_server.py); no extra
check is needed here, and this route is never reachable unless a feature
route wakes this backend (the engine's Rust proxy does that on demand).
"""

import json
from typing import Any, Dict, Optional

from fastapi import APIRouter
from pydantic import BaseModel

router = APIRouter()


class BrowserActRequest(BaseModel):
    action: str
    task_id: Optional[str] = None
    params: Dict[str, Any] = {}


def _dispatch(action: str, params: Dict[str, Any], task_id: str):
    """Call the matching ``tools.browser_tool`` function. Imported lazily so this
    router module stays free of the heavy browser-tool import graph until the
    engine actually asks for a browser action."""
    from tools import browser_tool as bt

    table = {
        "navigate": lambda: bt.browser_navigate(url=params["url"], task_id=task_id),
        "snapshot": lambda: bt.browser_snapshot(full=params.get("full", False), task_id=task_id),
        "click": lambda: bt.browser_click(ref=params["ref"], task_id=task_id),
        "type": lambda: bt.browser_type(ref=params["ref"], text=params["text"], task_id=task_id),
        "scroll": lambda: bt.browser_scroll(direction=params["direction"], task_id=task_id),
        "back": lambda: bt.browser_back(task_id=task_id),
        "press": lambda: bt.browser_press(key=params["key"], task_id=task_id),
        "get_images": lambda: bt.browser_get_images(task_id=task_id),
        "vision": lambda: bt.browser_vision(
            question=params["question"], annotate=params.get("annotate", False), task_id=task_id
        ),
        "console": lambda: bt.browser_console(
            clear=params.get("clear", False), expression=params.get("expression"), task_id=task_id
        ),
    }
    handler = table.get(action)
    if handler is None:
        return {"success": False, "error": f"Unknown browser action '{action}'. Valid: {sorted(table)}"}
    return handler()


@router.post("/api/browser/act")
async def browser_act(req: BrowserActRequest) -> Dict[str, Any]:
    """Run one browser action for the engine and return its JSON result."""
    # One task_id per engine chat session keeps concurrent engine sessions on
    # separate browser sessions, same as the Python agent's own ``task_id`` scoping.
    task_id = req.task_id or "engine-default"
    try:
        result = _dispatch(req.action, req.params, task_id)
    except KeyError as exc:
        return {"success": False, "error": f"Missing required parameter {exc} for action '{req.action}'"}
    except Exception as exc:  # tools.browser_tool already handles its own errors; this is a last resort
        return {"success": False, "error": f"browser action '{req.action}' failed: {exc}"}
    if isinstance(result, str):
        try:
            return json.loads(result)
        except ValueError:
            return {"success": True, "result": result}
    return result
