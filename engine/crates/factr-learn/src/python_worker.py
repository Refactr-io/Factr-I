import sys

# First, before any other import: the working directory must not shadow the standard library or
# installed packages (and the sandbox cannot list it anyway).
sys.path[:] = [p for p in sys.path if p not in ("", ".")]
import ast
import asyncio
import contextlib
import inspect
import json
import os
from pathlib import Path
import traceback
import types

protocol_out = sys.stdout
sys.path.extend(sys.argv[1:])
skills_root = Path(sys.argv[2])
if skills_root.is_dir():
    for skill_dir in skills_root.iterdir():
        if skill_dir.is_dir():
            sys.path.extend([str(skill_dir), str(skill_dir / "src")])
namespace = {"__name__": "__main__"}
max_calls = 16
max_output = 65536


def write_frame(value):
    protocol_out.write(json.dumps(value, separators=(",", ":")) + "\n")
    protocol_out.flush()


async def host_call(name, *args):
    write_frame({"op": "call", "fn": name, "args": [str(x) for x in args]})
    line = sys.stdin.readline()
    if not line:
        raise RuntimeError("engine closed the REPL channel")
    reply = json.loads(line)
    if reply.get("error"):
        raise RuntimeError(reply["error"])
    return reply.get("value", "")


async def llm_query(prompt):
    return await host_call("llm_query", prompt)


async def llm_query_batch(prompts):
    """Run up to 64 sub-queries (8 at once) as ONE host call; replies in order, an 'Error: ...' string per failed item."""
    if isinstance(prompts, (str, bytes)):
        raise TypeError("llm_query_batch expects a list of strings")
    return json.loads(await host_call("llm_query_batch", json.dumps([str(p) for p in prompts])))


MAX_LOAD_BYTES = 64 * 1024 * 1024


async def load(path, start=0, length=None):
    """Read a workspace file (or the byte slice start:start+length) as text; only the slice is read from disk."""
    info = json.loads(await host_call("load_path", path))
    start = max(0, int(start))
    want = max(0, info["size"] - start)
    if length is not None:
        want = min(want, max(0, int(length)))
    if want > MAX_LOAD_BYTES:
        raise RuntimeError(f"{path}: {want} bytes requested, the limit is 64 MB per call; pass start/length")
    with open(info["path"], "rb") as handle:
        handle.seek(start)
        data = handle.read(want)
    return data.decode("utf-8", "replace")


async def refine(op="run", instructions=None, global_=False):
    return await host_call("refine", json.dumps({"op": op, "instructions": instructions, "global": global_}))


async def goal(op="get", objective=None):
    return await host_call("goal", json.dumps({"op": op, "text": objective}))


async def heartbeat(op="list", **options):
    return await host_call("heartbeat", json.dumps({"op": op, **options}))


async def spawn_subagent(prompt, name="worker"):
    return json.loads(await host_call("spawn_subagent", json.dumps({"prompt": prompt, "label": name})))


async def await_subagent(session_id, timeout=20):
    """Wait up to timeout seconds for a child and return its completion report."""
    return json.loads(await host_call("spawn_subagent", json.dumps({
        "action": "await", "target": session_id, "timeout": timeout,
    })))


async def agent_message(action, message=None, target=None):
    return await host_call("agent_message", json.dumps({"action": action, "message": message, "target": target}))


async def host_request(name, payload=None):
    """factr-learn skill bridge for host operations already implemented by Rust."""
    payload = payload or {}
    if name.startswith("agent_message."):
        if name not in {"agent_message.send"}:
            raise ValueError(f"unsupported factr-learn host request: {name}")
        return json.loads(await host_call("agent_message", json.dumps({"host_request": name, "payload": payload})))
    if name.startswith("agent_observe."):
        if name not in {"agent_observe.list", "agent_observe.get", "agent_observe.recent"}:
            raise ValueError(f"unsupported factr-learn host request: {name}")
        return json.loads(await host_call("agent_message", json.dumps({"host_request": name, "payload": payload})))
    if name.startswith("rlm_heartbeat."):
        op = name.removeprefix("rlm_heartbeat.")
        if op not in {"list", "create", "update", "delete"}:
            raise ValueError(f"unsupported factr-learn host request: {name}")
        request = {"op": "rlm_" + op, **payload}
        return json.loads(await host_call("heartbeat", json.dumps(request)))
    if name.startswith("compact."):
        op = name.removeprefix("compact.")
        if op not in {"status", "run"}:
            raise ValueError(f"unsupported factr-learn host request: {name}")
        return json.loads(await host_call("compact", json.dumps({"op": op, **payload})))
    if name.startswith("goal."):
        op = name.removeprefix("goal.")
        if op not in {"get", "create", "progress", "complete"}:
            raise ValueError(f"unsupported factr-learn host request: {name}")
        request = {"op": op}
        if op == "create":
            request["text"] = payload.get("objective", payload.get("text"))
            if "token_budget" in payload:
                request["token_budget"] = payload["token_budget"]
        for key in ("note", "verification", "error"):
            if key in payload:
                request[key] = payload[key]
        return json.loads(await host_call("goal", json.dumps(request)))
    if name.startswith("refine."):
        op = name.removeprefix("refine.")
        if op not in {"status", "run"}:
            raise ValueError(f"unsupported factr-learn host request: {name}")
        return json.loads(await host_call("refine", json.dumps({"op": op, **payload})))
    if name == "websearch.run":
        return {"results": await host_call("websearch", json.dumps(payload))}
    if name == "skill.create":
        return json.loads(await host_call("skill", json.dumps(payload)))
    raise ValueError(f"unsupported factr-learn host request: {name}")


_rlm_module = types.ModuleType("rlm")
_rlm_module.host_request = host_request
sys.modules["rlm"] = _rlm_module


namespace.update({
    "llm_query": llm_query,
    "llm_query_batch": llm_query_batch,
    "load": load,
    "refine": refine,
    "goal": goal,
    "heartbeat": heartbeat,
    "spawn_subagent": spawn_subagent,
    "await_subagent": await_subagent,
    "agent_message": agent_message,
})
write_frame({"op": "ready", "pid": os.getpid()})

# Every engine helper is async. A top-level statement that calls one without `await` (`s = load(p)`)
# would bind or drop a coroutine, and the next use fails; such a call is awaited as meant. Only a
# call that is the whole value of a top-level statement, to a helper the cell has not rebound, so a
# coroutine passed on deliberately (`asyncio.gather(llm_query(a), ...)`) is left alone.
_async_helpers = {name: fn for name, fn in namespace.items() if inspect.iscoroutinefunction(fn)}


def _bound_names(tree):
    """Every name the cell binds, so a helper it redefines is not mistaken for the engine's."""
    names = set()
    for node in ast.walk(tree):
        if isinstance(node, ast.Name) and isinstance(node.ctx, ast.Store):
            names.add(node.id)
        elif isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)):
            names.add(node.name)
        elif isinstance(node, (ast.Import, ast.ImportFrom)):
            names.update((alias.asname or alias.name).split(".")[0] for alias in node.names)
    return names


def _await_helper_calls(tree):
    rebound = _bound_names(tree)
    for statement in tree.body:
        value = getattr(statement, "value", None)
        if (
            isinstance(statement, (ast.Expr, ast.Assign, ast.AnnAssign))
            and isinstance(value, ast.Call)
            and isinstance(value.func, ast.Name)
            and value.func.id in _async_helpers
            and value.func.id not in rebound
            and namespace.get(value.func.id) is _async_helpers[value.func.id]
        ):
            statement.value = ast.copy_location(ast.Await(value), value)
    return tree


class _Output:
    def __init__(self, buffer):
        self.buffer = buffer
        self.size = 0

    def write(self, value):
        room = max_output - self.size
        if room > 0:
            piece = value[:room]
            self.buffer.append(piece)
            self.size += len(piece)
        return len(value)

    def flush(self):
        pass

def _traceback_text(error):
    """The traceback from the user's first `<repl>` frame on (the worker's own frames are noise), ending with the exception line."""
    tb = error.__traceback__
    while tb is not None and tb.tb_frame.f_code.co_filename != "<repl>":
        tb = tb.tb_next
    if tb is None:
        return "".join(traceback.format_exception_only(type(error), error)).strip()
    return "".join(traceback.format_exception(type(error), error, tb)).strip()


for line in sys.stdin:
    try:
        message = json.loads(line)
        if message.get("op") != "run":
            continue
        code = message.get("code", "")
        if len(code.encode("utf-8")) > 1_048_576:
            write_frame({"op": "done", "stdout": "", "value": None, "error": "cell exceeds 1 MiB"})
            continue
        output = []
        call_count = [0]
        original_host_call = namespace["llm_query"].__globals__["host_call"]

        async def counted_host_call(name, *args):
            call_count[0] += 1
            if call_count[0] > max_calls:
                raise RuntimeError("host call budget (16) exhausted")
            return await original_host_call(name, *args)

        namespace["llm_query"].__globals__["host_call"] = counted_host_call
        tree = _await_helper_calls(ast.parse(code, "<repl>", "exec"))
        last = tree.body.pop() if tree.body and isinstance(tree.body[-1], ast.Expr) else None
        with contextlib.redirect_stdout(_Output(output)), contextlib.redirect_stderr(_Output(output)):
            if tree.body:
                result = eval(compile(ast.Module(body=tree.body, type_ignores=[]), "<repl>", "exec", flags=ast.PyCF_ALLOW_TOP_LEVEL_AWAIT), namespace, namespace)
                if inspect.isawaitable(result):
                    asyncio.run(result)
            value = None
            if last is not None:
                result = eval(compile(ast.Expression(last.value), "<repl>", "eval", flags=ast.PyCF_ALLOW_TOP_LEVEL_AWAIT), namespace, namespace)
                value = asyncio.run(result) if inspect.isawaitable(result) else result
        value = repr(value)[:8192] if value is not None else None
        write_frame({"op": "done", "stdout": "".join(output)[:max_output], "value": value, "error": None, "host_calls": call_count[0]})
        namespace["llm_query"].__globals__["host_call"] = original_host_call
    except BaseException as error:
        namespace["llm_query"].__globals__["host_call"] = original_host_call
        write_frame({"op": "done", "stdout": "".join(output)[:max_output] if "output" in locals() else "", "value": None, "error": _traceback_text(error)[-8192:], "host_calls": call_count[0] if "call_count" in locals() else 0})
