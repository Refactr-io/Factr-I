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


CLASSIFY_CHUNK_ITEMS = 40
CLASSIFY_CHUNK_CHARS = 24000
CLASSIFY_ITEM_CHARS = 6000
CLASSIFY_RETRIES = 2
CLASSIFY_BATCH_BYTES = 1_800_000


def _calls_left():
    counter = globals().get("call_count")
    return max_calls - counter[0] if counter else max_calls


def _label_key(text):
    return " ".join("".join(c if c.isalnum() else " " for c in str(text).casefold()).split())


def _match_label(value, keyed):
    """The allowed label a reply value means: exact (case and punctuation aside), else a unique whole-word fragment either way."""
    key = _label_key(value)
    if not key:
        return None
    if key in keyed:
        return keyed[key]
    found = {label for k, label in keyed.items() if f" {key} " in f" {k} " or f" {k} " in f" {key} "}
    return found.pop() if len(found) == 1 else None


def _check_reply(reply, ids, keyed):
    """(accepted {index: label}, reason or None). A reply that is not one JSON object, repeats or invents an id is rejected whole."""
    text = str(reply).strip()
    if text.startswith("Error:"):
        return {}, text[:160]
    start, end = text.find("{"), text.rfind("}")
    if start < 0 or end < start:
        return {}, "the reply was not a JSON object"
    try:
        pairs = json.loads(text[start:end + 1], object_pairs_hook=lambda p: p)
    except ValueError:
        return {}, "the reply was not valid JSON"
    if not isinstance(pairs, list):
        return {}, "the reply was not a JSON object"
    seen = {}
    for key, value in pairs:
        key = str(key).strip()
        if key in seen:
            return {}, f"id {key} appeared more than once"
        seen[key] = value
    wanted = {str(i + 1) for i in ids}
    extra = [k for k in seen if k not in wanted]
    if extra:
        return {}, f"ids not in the list: {', '.join(extra[:5])}"
    good, bad = {}, []
    for i in ids:
        value = seen.get(str(i + 1))
        label = _match_label(value, keyed) if isinstance(value, str) else None
        if label is None:
            bad.append(str(i + 1))
        else:
            good[i] = label
    return good, (f"missing or not an allowed label for ids: {', '.join(bad[:8])}" if bad else None)


def _chunks(indices, texts, size):
    chunk, chars = [], 0
    for i in indices:
        if chunk and (len(chunk) >= size or chars + len(texts[i]) > CLASSIFY_CHUNK_CHARS):
            yield chunk
            chunk, chars = [], 0
        chunk.append(i)
        chars += len(texts[i])
    if chunk:
        yield chunk


async def _classify_batches(prompts):
    out, group, size = [], [], 0
    for prompt in prompts + [None]:
        if group and (prompt is None or len(group) >= 64 or size + len(prompt) > CLASSIFY_BATCH_BYTES):
            if _calls_left() < 1:
                raise RuntimeError("classify: the cell's host call budget (16) is used up; call classify in a new cell")
            out.extend(await llm_query_batch(group))
            group, size = [], 0
        if prompt is not None:
            group.append(prompt)
            size += len(prompt)
    return out


async def _classify_jobs(texts, labels, guidance, jobs):
    """Label the given {pass number: [item indexes]}; each pass sees the labels in a different order. Chunks that fail validation are re-asked (smaller) up to CLASSIFY_RETRIES times, one batch call per wave."""
    keyed = {_label_key(label): label for label in labels}
    results = {p: {} for p in jobs}
    pending = {p: list(ix) for p, ix in jobs.items()}
    notes = {}
    size = CLASSIFY_CHUNK_ITEMS
    for _ in range(CLASSIFY_RETRIES + 1):
        chunks = [(p, c) for p, ix in pending.items() for c in _chunks(ix, texts, size)]
        if not chunks:
            break
        prompts = []
        for p, ids in chunks:
            shift = p % len(labels)
            order = labels[shift:] + labels[:shift]
            lines = "\n".join(f"{i + 1}. {texts[i]}" for i in ids)
            prompts.append(
                "Classify every numbered item with exactly one label from the allowed list. Judge each item by its own text only.\n"
                f"Allowed labels (use these exact strings): {json.dumps(order)}\n"
                + (f"{guidance}\n" if guidance else "")
                + "Reply with ONLY a JSON object that maps every id below to one allowed label, each id exactly once, like "
                + '{"<id>": "<label>"}. No other text.\n'
                + (f"Your previous reply was rejected: {notes[p]}\n" if p in notes else "")
                + f"Items:\n{lines}"
            )
        replies = await _classify_batches(prompts)
        pending = {}
        for (p, ids), reply in zip(chunks, replies):
            good, reason = _check_reply(reply, ids, keyed)
            results[p].update(good)
            left = [i for i in ids if i not in good]
            if left:
                pending.setdefault(p, []).extend(left)
                notes[p] = reason
        size = max(5, size // 2)
    left = sum(len(v) for v in pending.values())
    if left:
        raise RuntimeError(f"classify: {left} items still without a valid label after {CLASSIFY_RETRIES} retries ({next(iter(notes.values()), '')})")
    return results


async def classify(items, labels, guidance=None, votes=1):
    """One label per item from the FULL label list, as a list aligned with items. The sub-model gets the items numbered in chunks and must return JSON; every id must appear once with an allowed label (case and fragments are normalised) or the chunk is re-asked. votes>1 asks again with the labels reordered and re-asks only the items where passes disagree; the majority wins."""
    if isinstance(items, (str, bytes)) or isinstance(labels, (str, bytes)):
        raise TypeError("classify expects a list of items and a list of labels")
    items, labels = list(items), [str(x) for x in labels]
    if len({_label_key(x) for x in labels}) != len(labels) or not all(_label_key(x) for x in labels):
        raise ValueError("classify: labels must be distinct, non-empty strings")
    if not items:
        return []
    texts = [" ".join(str(x).split())[:CLASSIFY_ITEM_CHARS] for x in items]
    votes = max(1, int(votes))
    everything = list(range(len(items)))
    first = await _classify_jobs(texts, labels, guidance, {p: everything for p in range(min(votes, 2))})
    tally = [[first[p][i] for p in first] for i in everything]
    if votes > 1:
        split = [i for i in everything if len(set(tally[i])) > 1]
        if split:
            more = await _classify_jobs(texts, labels, guidance, {2 + k: split for k in range(max(1, votes - 2))})
            for i in split:
                tally[i].extend(more[p][i] for p in more)
    out = []
    for cast in tally:
        counts = {label: cast.count(label) for label in cast}
        best = max(counts.values())
        out.append(next(label for label in cast if counts[label] == best))
    return out


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
    "classify": classify,
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
