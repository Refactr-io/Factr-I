import sys

# First, before any other import: the working directory must not shadow the standard library or
# installed packages (and the sandbox cannot list it anyway).
sys.path[:] = [p for p in sys.path if p not in ("", ".")]
import ast
import asyncio
import contextlib
import contextvars
import functools
import hashlib
import importlib
import inspect
import json
import os
import random
import re
import site
import statistics
import time
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


def _utf8(text):
    """`text` as valid UTF-8: a lone surrogate (from a lenient decode) becomes "?" instead of breaking the
    engine's JSON protocol. Valid text is unchanged."""
    text = str(text)
    try:
        text.encode("utf-8")
        return text
    except UnicodeEncodeError:
        return text.encode("utf-8", "replace").decode("utf-8")


async def host_call(name, *args):
    write_frame({"op": "call", "fn": name, "args": [_utf8(x) for x in args]})
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
    """Run up to 64 sub-queries (up to FACTR_BATCH_CONCURRENCY at once, default 8) as ONE host call; replies in order, an 'Error: ...' string per failed item."""
    if isinstance(prompts, (str, bytes)):
        raise TypeError("llm_query_batch expects a list of strings")
    return json.loads(await host_call("llm_query_batch", json.dumps([_utf8(p) for p in prompts])))


CLASSIFY_CHUNK_ITEMS = 40
CLASSIFY_CHUNK_CHARS = 24000
CLASSIFY_ITEM_CHARS = 6000
CLASSIFY_RETRIES = 2
CLASSIFY_TRANSPORT_TRIES = 3
CLASSIFY_BATCH_BYTES = 1_800_000
# Settings the engine sends with every cell (this process has no environment of its own). The defaults
# are the 0.0.4 behaviour; "legacy" selects the 0.0.3 one (FACTR_COST_LEGACY=1 on the engine side).
# chunk_items / chunk_chars are None when the user has not set them (the data-shaped default applies, see _chunk_plan).
CFG = {"legacy": False, "format": "json", "chunk_items": None, "chunk_chars": None,
       "dedupe": False, "log": False, "effort": "", "model": "", "backoff": 1.0, "concurrency": 8}
_CLASSIFY_CACHE = {}
_CHUNK = contextvars.ContextVar("classify_chunk", default=None)   # (items, chars, source) of the running classify
# Data-shaped chunk default: short records and few labels take big chunks, anything else the 0.0.3 size.
WIDE_LABELS, WIDE_MEDIAN_CHARS, WIDE_ITEMS, WIDE_CHARS = 6, 600, 80, 48000
_CLASSIFY_CACHE_MAX = 50000


def _calls_left():
    counter = globals().get("call_count")
    return max_calls - counter[0] if counter else max_calls


def _label_key(text):
    return " ".join("".join(c if c.isalnum() else " " for c in str(text).casefold()).split())


def _lkey(text):
    """The 0.0.4 label key: case and punctuation aside, but a label made only of symbols (an emoji, "+") keys on
    its own whitespace-normalised text instead of becoming empty."""
    return _label_key(text) or " ".join(str(text).split())


def _match_label(value, keyed):
    """The 0.0.3 matching: exact (case and punctuation aside), else a unique whole-word fragment either way."""
    key = _label_key(value)
    if not key:
        return None
    if key in keyed:
        return keyed[key]
    found = {label for k, label in keyed.items() if f" {key} " in f" {k} " or f" {k} " in f" {key} "}
    return found.pop() if len(found) == 1 else None


def _legacy_check_reply(reply, ids, keyed):
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


def _legacy_chunks(indices, texts, size):
    chunk, chars = [], 0
    for i in indices:
        if chunk and (len(chunk) >= size or chars + len(texts[i]) > CLASSIFY_CHUNK_CHARS):
            yield chunk
            chunk, chars = [], 0
        chunk.append(i)
        chars += len(texts[i])
    if chunk:
        yield chunk


def _chunks(indices, texts, size, chars_cap):
    """Consecutive runs of at most `size` records and `chars_cap` characters. A record is never split: one
    longer than the cap rides alone, so no chunk ever carries a partial record."""
    chunk, chars = [], 0
    for i in indices:
        if chunk and (len(chunk) >= size or chars + len(texts[i]) > chars_cap):
            yield chunk
            chunk, chars = [], 0
        chunk.append(i)
        chars += len(texts[i])
    if chunk:
        yield chunk


def _groups(prompts):
    """Prompts split into host batches of at most 64 prompts and CLASSIFY_BATCH_BYTES UTF-8 bytes."""
    out, group, size = [], [], 0
    for prompt in prompts:
        n = len(prompt.encode("utf-8"))
        if group and (len(group) >= 64 or size + n > CLASSIFY_BATCH_BYTES):
            out.append(group)
            group, size = [], 0
        group.append(prompt)
        size += n
    if group:
        out.append(group)
    return out


async def _classify_batches(prompts):
    """The 0.0.3 batching (FACTR_COST_LEGACY=1): a budget or host failure raises and ends the job."""
    out, group, size = [], [], 0
    for prompt in prompts + [None]:
        if group and (prompt is None or len(group) >= 64 or size + len(prompt) > CLASSIFY_BATCH_BYTES):
            if _calls_left() < 1:
                raise RuntimeError("classify: the cell's host call budget (16) is used up; call classify in a new cell")
            out.extend(await _ask_group(group))
            group, size = [], 0
        if prompt is not None:
            group.append(prompt)
            size += len(prompt)
    return out


async def _ask_all(prompts):
    """Every prompt asked, one host call per byte/prompt group, each reply stamped with its own batch's start
    ("t0"). A host-level failure (budget, the cell's wait, a refused batch) comes back as that group's replies with
    "host" set, so replies already received are kept; after a host failure or an error asking again cannot help
    (authentication, an unsupported setting), the remaining groups of the wave are not sent."""
    out, halt = [], None
    for group in _groups(prompts):
        if halt:
            out.extend({"text": "", "error": halt, "host": True, "t0": _now_ms()} for _ in group)
            continue
        t0 = _now_ms()
        try:
            if _calls_left() < 1:
                raise RuntimeError("the cell's host call budget (16) is used up")
            replies = await _ask_group(group)
        except RuntimeError as error:
            replies = [{"text": "", "error": str(error), "host": True} for _ in group]
            halt = "not sent: an earlier batch of this wave failed (host call)"
        for r in replies:
            r["t0"] = t0
            if r.get("error") and not r.get("host") and _error_class(r["error"]) == "fatal":
                halt = halt or "not sent: an earlier batch of this wave failed (" + _error_tag(r["error"], "fatal") + ")"
        out.extend(replies)
    return out


async def _ask_group(group):
    """One host call for these prompts: a list of {"text", "error" or None, "in", "out", "cached", "reasoning",
    "ms", "at" (ms after the batch started that the call got its slot), "effort" (the effort it ran at),
    "requested" (the effort it asked for), "refused_ms" (a first request refused for its effort)}, in the order
    of `group`. The longest prompts are sent first, so a wave is not left waiting on a long one started last.
    Under FACTR_COST_LEGACY=1 the order is unchanged and an error is also the reply text ("Error: ..."), exactly
    what 0.0.3's `llm_query_batch` returned."""
    order = list(range(len(group)))
    if not CFG["legacy"]:
        order.sort(key=lambda k: -len(group[k]))
    raw = json.loads(await host_call("llm_query_batch_meta", json.dumps([group[k] for k in order])))
    out = [None] * len(group)
    for k, r in zip(order, raw):
        error = r.get("e")
        text = (error if CFG["legacy"] else None) if error else (r.get("t") or "")
        out[k] = {"text": text or "", "error": error, "in": r.get("i"), "out": r.get("o"), "cached": r.get("c"),
                  "reasoning": r.get("r"), "ms": r.get("ms"), "at": r.get("s"), "effort": r.get("f"),
                  "requested": r.get("q"), "refused_ms": r.get("x")}
    return out


def _json_prompt(order, guidance, note, lines):
    return (
        "Classify every numbered item with exactly one label from the allowed list. Judge each item by its own text only.\n"
        f"Allowed labels (use these exact strings): {json.dumps(order)}\n"
        + (f"{guidance}\n" if guidance else "")
        + "Reply with ONLY a JSON object that maps every id below to one allowed label, each id exactly once, like "
        + '{"<id>": "<label>"}. No other text.\n'
        + (f"Your previous reply was rejected: {note}\n" if note else "")
        + f"Items:\n{lines}"
    )


LETTERS = "ABCDEFGHIJKLMNOPQRSTUVWXYZ"


def _codes_prompt(order_codes, guidance, note, lines):
    # Letters, not numbers: ids are numbers, and a number code next to a number id invites off-by-one answers.
    table = " | ".join(f"{LETTERS[code]}={json.dumps(label, ensure_ascii=False)}" for code, label in order_codes)
    return (
        "Label every numbered item with one letter code. Judge each item by its own text only.\n"
        f"Codes: {table}\n"
        + (f"{guidance}\n" if guidance else "")
        + 'Reply with one line per id: "<id>:<letter>", each id exactly once, nothing else.\n'
        + (f"Your previous reply was rejected: {note}\n" if note else "")
        + f"Items:\n{lines}"
    )


def _format_for(labels):
    """'codes' (letters) unless JSON is asked for, there are more labels than letters, a label has a character
    that cannot ride safely in the one-line code table (control or separator characters), or a label is itself a
    single letter other than its own code (`A="B"` would invite the wrong letter)."""
    if CFG["format"] == "json" or len(labels) > len(LETTERS) or not all(x.isprintable() for x in labels):
        return "json"
    for code, label in enumerate(labels):
        key = label.strip().upper()
        if len(key) == 1 and key in LETTERS and key != LETTERS[code]:
            return "json"
    return "codes"


# One `id:letter` line. "1. A" and "1) A" (one letter) are read too: a model mirroring the numbered item list is
# not wrong, while an echoed item line ("1. apple") stays prose.
_LINE = re.compile(r"^\s*(\d+)\s*(?:[:=]\s*([A-Za-z]+)|[.)]\s*([A-Za-z]))\s*$")
_DECLINED = re.compile(r"^\W*(i'?m sorry|sorry|i can(?:no|')t|i cannot|i am unable|i'm unable|i won't|i will not|unable to)", re.I)


def _line_pairs(text):
    """([(id, letters)], other_lines): the `id:letter` lines anywhere in the reply, and how many other non-blank lines."""
    pairs, other = [], 0
    for line in text.splitlines():
        if not line.strip() or line.strip().startswith("```"):
            continue
        m = _LINE.match(line)
        if m:
            pairs.append((int(m.group(1)), m.group(2) or m.group(3)))
        else:
            other += 1
    return pairs, other


def _json_pairs(text):
    start, end = text.find("{"), text.rfind("}")
    if start < 0 or end <= start:
        return None
    try:
        raw = json.loads(text[start:end + 1], object_pairs_hook=lambda p: p)
    except ValueError:
        return None
    if not isinstance(raw, list):
        return None
    out = []
    for key, value in raw:
        key = str(key).strip()
        out.append((int(key) if key.isdigit() else key, value))
    return out


def _check_reply(reply, ids, labels, keyed, fmt):
    """(accepted {index: label}, reason or None, whole). Strict: every id once with a letter in range (codes) or
    exactly an allowed label, case and punctuation aside (no fragment matching, never a bare number unless that
    number is itself an allowed label in the JSON format). A reply that is unparseable, repeats an id, invents one,
    or wraps the lines in prose without covering every id is rejected whole; otherwise only the failing ids are."""
    text = str(reply).strip()
    pairs, strict = None, False
    if fmt == "codes":
        lines, other = _line_pairs(text)
        if lines:
            pairs, strict = [(n, v, True) for n, v in lines], other > 0
    if pairs is None:
        found = _json_pairs(text)
        if found is None:
            return {}, ("the reply was not `id:letter` lines (one per id, like 3:B)" if fmt == "codes" else "the reply was not a JSON object"), True
        pairs = [(n, v, False) for n, v in found]
    # Reasons carry engine categories and id numbers only, never the reply's own keys or text.
    seen = {}
    for n, value, is_line in pairs:
        if n in seen:
            return {}, (f"id {n} appeared more than once" if isinstance(n, int) else "a key appeared more than once"), True
        seen[n] = (value, is_line)
    wanted = {i + 1: i for i in ids}
    if any(not isinstance(n, int) for n in seen):
        return {}, "the reply had keys that are not ids", True
    extra = [str(n) for n in seen if n not in wanted]
    if extra:
        return {}, f"ids not in the list: {', '.join(extra[:5])}", True
    good, bad = {}, []
    for n, i in wanted.items():
        label = None
        if n in seen:
            value, is_line = seen[n]
            if is_line:
                label = labels[LETTERS.index(value.upper())] if len(value) == 1 and value.upper() in LETTERS[:len(labels)] else None
            elif isinstance(value, str):
                label = keyed.get(_lkey(value))
                if label is None and fmt == "codes" and len(value.strip()) == 1 and value.strip().upper() in LETTERS[:len(labels)]:
                    label = labels[LETTERS.index(value.strip().upper())]
            elif fmt == "json" and isinstance(value, (int, float)) and not isinstance(value, bool):
                # Only a label that IS this number ("0", "1"): the model was shown those strings.
                label = keyed.get(_lkey(json.dumps(value)))
        if label is None:
            bad.append(str(n))
        else:
            good[i] = label
    if bad:
        reason = f"missing, or not an allowed {'letter' if fmt == 'codes' else 'label'}, for ids: {', '.join(bad[:8])}"
        return ({}, reason, True) if strict else (good, reason, False)
    return good, None, False


def _digest(text):
    return hashlib.sha256(text.encode("utf-8", "replace")).hexdigest()


def _now_ms():
    return int(time.time() * 1000)


def _write_log(rows):
    if not (CFG["log"] and rows):
        return
    try:
        # Not through the counted host call: bookkeeping must not spend the cell's host-call budget.
        for at in range(0, len(rows), 2000):
            write_frame({"op": "call", "fn": "classify_log", "args": ["\n".join(json.dumps(r, separators=(",", ":")) for r in rows[at:at + 2000])]})
            sys.stdin.readline()
    except Exception:
        pass


def _nap(seconds):
    """Wait on the engine's clock: a sleep here would be spent from the cell's 20 s compute allowance."""
    try:
        write_frame({"op": "call", "fn": "sleep_ms", "args": [str(int(seconds * 1000))]})
        sys.stdin.readline()
    except Exception:
        pass


_FILTER = re.compile(r"content.?filter|content.?policy|safety system|flagged", re.I)
_FATAL = re.compile(
    r"\b(401|403|404)\b|unauthori|forbidden|authenticat|api.?key|permission|unsupported|not supported|does not support"
    r"|model.?not.?found|no such model|no active model|insufficient.?quota", re.I)
_SIZE = re.compile(
    r"\b413\b|context.?length|context.?window|maximum context|too long|too large|too many tokens|token limit"
    r"|prompt is \d+ char|payload", re.I)
_NON_TRANSIENT = re.compile(
    r"\b(400|401|403|404|413|422)\b|content.?filter|content.?policy|prompt is \d+ char|limit is \d+|too large|context.?length"
    r"|maximum context|unsupported|not supported|invalid.?request|unauthori|forbidden|authenticat|api key|permission|bad request", re.I)


def _error_class(error):
    """transport (429, 5xx, timeout, connection, overload, or unknown: ask again unchanged once), filter (a record
    the provider will not take: halve until it is isolated), fatal (authentication, permission, an unsupported
    setting, a missing model: asking again cannot help, stop), size (a prompt over a context or size limit: halve
    until it fits), or invalid (any other 4xx: halve, but a wave refused whole stops)."""
    text = str(error)
    if re.search(r"\b(429|5\d\d)\b", text):
        return "transport"
    if _FILTER.search(text):
        return "filter"
    if _FATAL.search(text):
        return "fatal"
    if _SIZE.search(text):
        return "size"
    if _NON_TRANSIENT.search(text):
        return "invalid"
    return "transport"


def _error_tag(error, kind):
    """The log's `error` field: an engine-made category and status code, never provider text."""
    text = str(error)
    code = re.search(r"\b([45]\d\d)\b", text)
    if kind == "host":
        return "host: " + ("wait" if "time budget" in text else "call budget" if "budget" in text else "not sent" if text.startswith("not sent") else "batch failed")
    if kind == "transport":
        if code:
            return f"transport: {code.group(1)}"
        low = text.lower()
        return "transport: " + next((w for w in ("timeout", "rate limit", "overloaded", "connection") if w in low or (w == "timeout" and "timed out" in low)), "error")
    if kind == "filter":
        return "refusal: content filter"
    if kind == "fatal":
        return "request failed: " + (code.group(1) if code else "unsupported setting" if re.search(r"support", text, re.I) else "not retried")
    if kind == "size":
        return "request error: " + (code.group(1) + " " if code else "") + "over a size limit"
    return "request error: " + (code.group(1) if code else "invalid request")


def _scaffold_prompt(chunk, texts, labels, guidance, fmt):
    ids, p = chunk["ids"], chunk["p"]
    shift = p % len(labels)
    order = labels[shift:] + labels[:shift]
    lines = "\n".join(f"{i + 1}. {texts[i]}" for i in ids)
    if fmt == "json":
        return _json_prompt(order, guidance, chunk["note"], lines)
    return _codes_prompt([(labels.index(x), x) for x in order], guidance, chunk["note"], lines)


def _balanced(n, cap):
    """Chunk size for n records at most `cap` per chunk, spread evenly (100 at 40: 34+33+33, never 40+40+20)."""
    return max(1, -(-n // -(-n // cap))) if n else cap


def _chunk_plan(texts, labels):
    """(items, chars, source) for this job. A user setting (FACTR_CLASSIFY_CHUNK_ITEMS / _CHARS) always wins, each
    value on its own, and the source is then "user". Otherwise: at most WIDE_LABELS labels and a median record
    length (of the records to label) of at most WIDE_MEDIAN_CHARS take 80 items / 48000 chars ("default-80"), all
    else 40 / 24000 ("default-40", the 0.0.3 size)."""
    items, chars = CFG.get("chunk_items"), CFG.get("chunk_chars")
    wide = len(labels) <= WIDE_LABELS and bool(texts) and statistics.median(len(t) for t in texts) <= WIDE_MEDIAN_CHARS
    source = "user" if items or chars else ("default-80" if wide else "default-40")
    return (items or (WIDE_ITEMS if wide else CLASSIFY_CHUNK_ITEMS), chars or (WIDE_CHARS if wide else CLASSIFY_CHUNK_CHARS), source)


def _chunk_now():
    return _CHUNK.get() or (CFG.get("chunk_items") or CLASSIFY_CHUNK_ITEMS, CFG.get("chunk_chars") or CLASSIFY_CHUNK_CHARS,
                              "user" if CFG.get("chunk_items") or CFG.get("chunk_chars") else "default-40")


def _first_chunks(texts, jobs):
    n, chars, _ = _chunk_now()
    return [{"p": p, "ids": c, "asks": 0, "replies": 0, "note": None, "splits": 0, "tre": 0, "size": n}
            for p, ix in jobs.items() for c in _chunks(ix, texts, _balanced(len(ix), n), chars)]


def _max_splits():
    """Halvings that take the largest chunk down to single records: ceil(log2(chunk_items)) + 1."""
    return max(1, int(_chunk_now()[0]) - 1).bit_length() + 1


def _preflight(texts, labels, guidance, votes):
    """Refuse a job that cannot fit before anything is sent (a mid-way failure would throw the labels away). With
    votes > 1 the worst case counts: every record disagrees, so the tie-break passes ask every record again."""
    fmt = _format_for(labels)
    everything = list(range(len(texts)))
    first = _first_chunks(texts, {p: everything for p in range(min(votes, 2))})
    more = _first_chunks(texts, {2 + k: everything for k in range(max(1, votes - 2))}) if votes > 1 else []
    waves = [[_scaffold_prompt(c, texts, labels, guidance, fmt) for c in chunks] for chunks in (first, more) if chunks]
    for prompt in (p for wave in waves for p in wave):
        if len(prompt) > 200000:
            raise _Refused(f"classify: a chunk's prompt is {len(prompt)} characters, the limit is 200000; lower FACTR_CLASSIFY_CHUNK_CHARS")
    need = sum(len(_groups(wave)) for wave in waves)
    if need > _calls_left():
        worst = f", counting votes={votes} at worst (every record disagreeing)" if votes > 1 else ""
        raise _Refused(
            f"classify: {len(texts)} records need up to {need} host calls ({len(first) + len(more)} chunks of at most {_chunk_now()[0]} records / "
            f"{_chunk_now()[1]} characters, 64 per batch call{worst}) and this cell has {_calls_left()} of {max_calls} left; "
            "classify fewer records per cell" + (" or use fewer votes" if votes > 1 else ""))


class _Refused(RuntimeError):
    """A job refused before anything was sent."""


MIN_SYSTEMIC = 4     # a wave of at least this many requests, every one refused (not for size), stops the job


class _Incomplete(Exception):
    """Some work items have no label: `missing` (work indexes), `details` (why, at most three distinct reasons,
    engine categories only), `category` (the first error category, for the log)."""

    def __init__(self, missing, details, category=None, labelled=None):
        super().__init__("; ".join(details))
        self.missing, self.details, self.category, self.labelled = missing, details, category, labelled or {}


_SEQ = [0]


def _cfg_fields():
    """The settings every call and job row carries."""
    n, chars, source = _chunk_now()
    return {"chunk_items": n, "chunk_chars": chars, "chunk_source": source, "concurrency": CFG.get("concurrency")}


def _call_rows(call, ch, cid, wave, reply, prompt, fmt, votes, legacy, good, ids, tag, declined, hashes, texts, started):
    """The `type=call` log rows of one sub-call attempt (the field names a record-level comparison reads): one
    for the call, preceded by one for a request the API refused for its effort, if there was one."""
    t_batch = reply.get("t0") or started
    at, ms = reply.get("at"), reply.get("ms")
    t0 = t_batch + int(at) if isinstance(at, (int, float)) else t_batch
    t1 = t0 + int(ms) if isinstance(ms, (int, float)) else _now_ms()
    error = reply.get("error")
    effort = reply.get("effort") or CFG.get("effort") or None
    requested = reply.get("requested") or CFG.get("requested_effort") or effort
    fell_back = bool(requested and effort and requested != effort)
    results = [[(hashes[i] if hashes else _digest(texts[i])[:16]), good[i]] for i in ids if i in good]
    if sum(len(label) for _, label in results) > 100_000:
        results = None   # very long labels: the row stays under the engine's size limit, the occ rows carry them
    base = {"type": "call", "classify_call": call, "pass": ch["p"], "wave": wave, "attempt": ch["asks"], "chunk_size": len(ids),
            "format": fmt, "votes": votes, "model": CFG.get("model") or None, "legacy": legacy,
            "dedupe": bool(CFG["dedupe"]) and not legacy, **_cfg_fields(), "prompt_chars": len(prompt)}
    rows = []
    refused_ms = reply.get("refused_ms")
    if isinstance(refused_ms, (int, float)):
        rows.append(dict(base, call_id=cid + "r", records_count=0, input_tokens=None, output_tokens=None, reasoning_tokens=None,
                         cached_tokens=None, latency_ms=int(refused_ms), ts_start_ms=t0 - int(refused_ms), ts_end_ms=t0,
                         effort=requested, requested_effort=requested, effort_fallback=True, reply_chars=0, error="request failed: effort refused",
                         refusal=False, transport_error=False, validation_failure=False, results=[]))
    rows.append(dict(base, call_id=cid, records_count=len(good), input_tokens=reply.get("in"), output_tokens=reply.get("out"),
                     reasoning_tokens=reply.get("reasoning"), cached_tokens=reply.get("cached"), latency_ms=ms,
                     ts_start_ms=t0, ts_end_ms=t1, effort=effort, requested_effort=requested, effort_fallback=fell_back,
                     reply_chars=len(reply.get("text") or "") if not error else 0, error=tag,
                     refusal=bool(declined) or (bool(error) and _error_class(error) == "filter"),
                     transport_error=bool(error) and not reply.get("host") and _error_class(error) == "transport",
                     validation_failure=not error and len(good) < len(ids), results=results))
    return rows


async def _classify_jobs(texts, labels, guidance, jobs, hashes=None, call=0, votes=1, on_label=None, where=None):
    """Label the given {pass number: [item indexes]}; each pass sees the labels in a different order. Work is a
    queue of chunks, each with its own counts and rejection note: a record gets at most 3 answered asks (2 re-asks
    for invalid replies), one re-ask after a transport error, and halvings down to single records; every bound is
    separate, so a halving or a transport error never spends the budget for invalid replies, and the loop ends.
    - A reply with some invalid ids re-asks only those ids (ids of one pass and reply count that failed in
      different chunks are regrouped; a reply rejected whole is re-asked at half its size).
    - A transport error (429, 5xx, timeout, connection) re-asks the SAME chunk unchanged once after a jittered
      backoff (the host already retried the request itself); it is never regrouped with validation failures.
    - A size, content-filter or other 4xx error is never repeated at the same size: the chunk is halved (at most
      ceil(log2(chunk_items)) + 1 times, so down to single records), so an over-long prompt fits and a filtered
      record is isolated; a single record that is refused is given up. A whole wave of at least MIN_SYSTEMIC
      requests refused for something other than size stops the job, as do an authentication, permission or
      unsupported-setting error and a failed host call (budget, wait): asking again cannot help.
    Labelled items are never asked again; `on_label(i, label, effort)` sees every pass-0 label as it arrives with
    the effort it was given at, and `where[i]` gets (call_id, position in the chunk, effort). Raises _Incomplete
    naming the unlabelled items."""
    fmt = _format_for(labels)
    keyed = {_lkey(label): label for label in labels}
    results = {p: {} for p in jobs}
    queue = _first_chunks(texts, jobs)
    max_splits = _max_splits()
    dead, wave, tlevel = [], 0, 0
    while queue:
        prompts = [_scaffold_prompt(c, texts, labels, guidance, fmt) for c in queue]
        started = _now_ms()
        replies = await _ask_all(prompts)
        wave += 1
        again, failed, rows, re_ask_transport, stop = [], {}, [], False, None
        answered, refused, refusal = 0, 0, None
        for ch, reply, prompt in zip(queue, replies, prompts):
            ch["asks"] += 1
            _SEQ[0] += 1
            cid = f"{call}.{_SEQ[0]}"
            ids, p = ch["ids"], ch["p"]
            error = reply.get("error")
            good, reason, whole, tag, declined = {}, None, False, None, False
            if error:
                kind = "host" if reply.get("host") else _error_class(error)
                tag = _error_tag(error, kind)
                if kind == "transport":
                    if ch["tre"] < 1:
                        again.append(dict(ch, tre=ch["tre"] + 1))
                        re_ask_transport = True
                    else:
                        dead.append((p, ids, f"a transport error persisted after one re-ask ({tag})", tag))
                elif kind in ("size", "filter", "invalid"):
                    if kind != "size":
                        refused, refusal = refused + 1, refusal or tag
                    if len(ids) > 1 and ch["splits"] < max_splits:
                        # Halved, down to single records: a prompt over a limit fits again, a filtered record is
                        # isolated, and the rest of the chunk is labelled.
                        half = (len(ids) + 1) // 2
                        again.extend(dict(ch, ids=part, splits=ch["splits"] + 1) for part in (ids[:half], ids[half:]))
                    else:
                        dead.append((p, ids, f"the sub-model refused a request and it cannot be retried at a smaller size ({tag})", tag))
                else:
                    detail = (f"the host call failed ({tag}); continue in a new cell" if kind == "host"
                              else f"a sub-model request failed and asking again cannot help ({tag})")
                    dead.append((p, ids, detail, tag))
                    stop = stop or (detail, tag)
            else:
                answered += 1
                # The effort that keys the cache: the session's effective one, or what the call fell back to when
                # the API refused it (the next cell's effective effort is then that one too).
                effort = (reply.get("effort") or "") if reply.get("refused_ms") is not None else (CFG.get("effort") or "")
                good, reason, whole = _check_reply(reply["text"], ids, labels, keyed, fmt)
                results[p].update(good)
                if p == 0:
                    for pos, i in enumerate(ids):
                        if i in good:
                            if on_label:
                                on_label(i, good[i], effort)
                            if where is not None:
                                where[i] = (cid, pos, effort)
                declined = bool(reason) and not good and bool(_DECLINED.search(str(reply["text"])))
                tag = None if not reason else ("refusal: the reply declined" if declined else "reject: " + reason)
                left = [i for i in ids if i not in good]
                ch["replies"] += 1
                if left and ch["replies"] >= 3:
                    dead.append((p, left, f"after {CLASSIFY_RETRIES} retries the reply was still invalid ({reason})", tag))
                elif left:
                    failed.setdefault((p, ch["replies"]), []).append({"ids": left, "note": reason, "asks": ch["asks"],
                                                                   "size": max(1, len(ids) // 2) if whole else ch["size"]})
            if CFG["log"]:
                rows.extend(_call_rows(call, ch, cid, wave, reply, prompt, fmt, votes, False, good, ids, tag, declined, hashes, texts, started))
        _write_log(rows)
        for (p, replies_n), groups in failed.items():
            ids = [i for g in groups for i in g["ids"]]
            size = min(g["size"] for g in groups)
            for c in _chunks(ids, texts, size, _chunk_now()[1]):
                again.append({"p": p, "ids": c, "asks": max(g["asks"] for g in groups), "replies": replies_n, "note": groups[0]["note"],
                              "splits": 0, "tre": 0, "size": size})
        if not stop and refused >= MIN_SYSTEMIC and refused == len(queue):
            # Every request of the wave was refused, not for its size: a setting or policy, not a record.
            stop = (f"every request of a wave was refused ({refusal}); smaller requests would not help", refusal)
        if stop:
            dead.extend((ch["p"], ch["ids"], stop[0], stop[1]) for ch in again)
            break
        if re_ask_transport:
            tlevel += 1
            _nap(min(8.0, CFG["backoff"] * 2 ** (tlevel - 1)) * (1 + random.random() * 0.5))
        else:
            tlevel = 0
        queue = again
    if dead:
        missing = sorted({i for p, ix in jobs.items() for i in ix if i not in results[p]})
        details = list(dict.fromkeys(d[2] for d in dead))[:3]
        raise _Incomplete(missing, details, dead[0][3], dict(results.get(0, {})) if len(jobs) == 1 else {})
    return results


async def _legacy_classify_jobs(texts, labels, guidance, jobs, call=0, votes=1, keys=None, where=None):
    """The 0.0.3 loop, kept verbatim for FACTR_COST_LEGACY=1 (the log rows are bookkeeping only)."""
    keyed = {_label_key(label): label for label in labels}
    results = {p: {} for p in jobs}
    pending = {p: list(ix) for p, ix in jobs.items()}
    notes = {}
    size = CLASSIFY_CHUNK_ITEMS
    for wave in range(CLASSIFY_RETRIES + 1):
        chunks = [(p, c) for p, ix in pending.items() for c in _legacy_chunks(ix, texts, size)]
        if not chunks:
            break
        prompts = []
        for p, ids in chunks:
            shift = p % len(labels)
            order = labels[shift:] + labels[:shift]
            lines = "\n".join(f"{i + 1}. {texts[i]}" for i in ids)
            prompts.append(_json_prompt(order, guidance, notes.get(p), lines))
        started = _now_ms()
        answers = await _classify_batches(prompts)
        replies = [r["text"] for r in answers]
        pending, rows = {}, []
        for (p, ids), reply, answer, prompt in zip(chunks, replies, answers, prompts):
            good, reason = _legacy_check_reply(reply, ids, keyed)
            results[p].update(good)
            left = [i for i in ids if i not in good]
            if left:
                pending.setdefault(p, []).extend(left)
                notes[p] = reason
            if CFG["log"]:
                _SEQ[0] += 1
                cid = f"{call}.{_SEQ[0]}"
                effort = answer.get("effort") or CFG.get("effort") or ""
                if p == 0:
                    where.update({i: (cid, pos, effort) for pos, i in enumerate(ids) if i in good})
                error = answer.get("error")
                tag = _error_tag(error, _error_class(error)) if error else (("reject: " + _legacy_reason_tag(reason)) if reason else None)
                answer = dict(answer, text=answer["text"] if not error else "")
                rows.extend(_call_rows(call, {"p": p, "asks": wave + 1}, cid, wave + 1, answer, prompt, "json", votes, True,
                                       good, ids, tag, False, keys, texts, started))
        _write_log(rows)
        size = max(5, size // 2)
    left = sum(len(v) for v in pending.values())
    if left:
        raise RuntimeError(f"classify: {left} items still without a valid label after {CLASSIFY_RETRIES} retries ({next(iter(notes.values()), '')})")
    return results


def _legacy_reason_tag(reason):
    """The log category of a 0.0.3 rejection reason (its ids and duplicates can carry the reply's own keys)."""
    for start in ("the reply was not", "missing or not an allowed label"):
        if reason.startswith(start):
            return reason
    return "duplicate id" if "more than once" in reason else "ids not in the list" if reason.startswith("ids not") else "invalid reply"


async def _vote(texts, labels, guidance, votes, run):
    """Labels for `texts` (all distinct work items): votes>1 asks again with the labels reordered and re-asks
    only the items where passes disagree; the majority wins (a tie goes to the earliest pass)."""
    everything = list(range(len(texts)))
    first = await run(texts, labels, guidance, {p: everything for p in range(min(votes, 2))})
    tally = [[first[p][i] for p in first] for i in everything]
    if votes > 1:
        split = [i for i in everything if len(set(tally[i])) > 1]
        if split:
            more = await run(texts, labels, guidance, {2 + k: split for k in range(max(1, votes - 2))})
            for i in split:
                tally[i].extend(more[p][i] for p in more)
    out = []
    for cast in tally:
        counts = {label: cast.count(label) for label in cast}
        best = max(counts.values())
        out.append(next(label for label in cast if counts[label] == best))
    return out


_CALL_SEQ = [0]


def _occ_rows(call, keys, fulls, out, cached, firsts, spots, cached_effort):
    """One `type=occ` log row per input occurrence, in order, with the FINAL label (null: the job failed before
    labelling it) and the effort it was given at. `chunk`/`pos`: the sub-call (call_id) that labelled this
    occurrence (or, for a duplicate, the copy that was asked) in the first pass and its position there; null for a
    cached record."""
    rows = []
    for i in range(len(out)):
        cid, pos, effort = spots[i] or (None, None, cached_effort if keys[i] in cached else None)
        rows.append({"type": "occ", "classify_call": call, "occ": i, "h": keys[i][:16], "label": out[i], "effort": effort or None,
                     "truncated": len(fulls[i]) > CLASSIFY_ITEM_CHARS, "cached": keys[i] in cached,
                     "deduped": firsts.get(keys[i], i) != i, "chunk": cid, "pos": pos})
    return rows


def _job_row(call, status, error, records, distinct, cached, to_label, unlabelled, votes, fmt, effort, legacy):
    requested = CFG.get("requested_effort") or ""
    return {"type": "job", "classify_call": call, "status": status, "error": error, "records": records, "distinct": distinct,
            "cached": cached, "to_label": to_label, "unlabelled": unlabelled, "votes": votes, "format": fmt,
            "effort": effort or None, "requested_effort": requested or effort or None,
            "effort_fallback": bool(requested and effort and requested != effort), "model": CFG.get("model") or None, "legacy": legacy,
            "dedupe": bool(CFG["dedupe"]) and not legacy, **_cfg_fields()}


async def classify(items, labels, guidance=None, votes=1):
    """One label per item from the FULL label list, as a list aligned with items. The sub-model gets the items numbered in chunks and answers a JSON object id -> label per chunk (opt-in FACTR_CLASSIFY_FORMAT=codes: one `id:letter` line per item); every id must appear once with an allowed label or the chunk's failing ids are re-asked. Every item is asked unless FACTR_CLASSIFY_DEDUPE=1. votes=1 (default) asks once; votes>1 asks again with the labels reordered and re-asks only the items where passes disagree; the majority wins. If some items cannot be labelled, the error names them; with votes=1 the labels already given are kept, so calling classify again on the same items asks only for the missing ones."""
    if isinstance(items, (str, bytes)) or isinstance(labels, (str, bytes)):
        raise TypeError("classify expects a list of items and a list of labels")
    items, labels = list(items), [str(x) for x in labels]
    legacy = bool(CFG["legacy"])
    key = _label_key if legacy else _lkey
    if not labels or len({key(x) for x in labels}) != len(labels) or not all(key(x) for x in labels):
        raise ValueError("classify: labels must be a non-empty list of distinct, non-empty strings")
    if not items:
        return []
    # Identity is the whole normalised text; only the prompt gets the first CLASSIFY_ITEM_CHARS of it.
    fulls = [" ".join(str(x).split()) for x in items]
    if not legacy:
        fulls = [_utf8(f) for f in fulls]
    texts = [f[:CLASSIFY_ITEM_CHARS] for f in fulls]
    keys = [_digest(f) for f in fulls]
    votes = max(1, int(votes))
    _CALL_SEQ[0] += 1
    call = _CALL_SEQ[0]
    effort = CFG.get("effort") or ""
    if legacy:
        where = {}
        run = functools.partial(_legacy_classify_jobs, call=call, votes=votes, keys=[k[:16] for k in keys], where=where)
        try:
            out = await _vote(texts, labels, guidance, votes, run)
        except Exception:
            _write_log([_job_row(call, "failed", "reject: still invalid after retries", len(items), len(set(keys)), 0, len(items), len(items), votes, "json", effort, True)])
            raise
        if CFG["log"]:
            _write_log(_occ_rows(call, keys, fulls, out, set(), {}, [where.get(i) for i in range(len(out))], effort)
                       + [_job_row(call, "ok", None, len(items), len(set(keys)), 0, len(items), 0, votes, "json", effort, True)])
        return out
    fmt = _format_for(labels)
    stamp = lambda eff: (fmt, CFG.get("model") or "", eff, tuple(labels), str(guidance) if guidance else "", votes)
    known, cached, firsts, work = {}, set(), {}, []
    if len(labels) == 1:
        # One allowed label: it is the answer for every item, nothing to ask.
        known = {k: labels[0] for k in keys}
    elif CFG["dedupe"]:
        known = {k: _CLASSIFY_CACHE[(k,) + stamp(effort)] for k in set(keys) if (k,) + stamp(effort) in _CLASSIFY_CACHE}
        cached = set(known)
        for i, k in enumerate(keys):
            if k not in known:
                firsts.setdefault(k, i)
        work = [(k, i) for k, i in firsts.items()]
    else:
        work = [(keys[i], i) for i in range(len(keys))]
    _CHUNK.set(_chunk_plan([texts[i] for _, i in work], labels))
    # The work item that stands for each occurrence: its first copy with dedupe, itself without.
    j_of = {k: j for j, (k, _) in enumerate(work)} if CFG["dedupe"] else None
    occ_work = [(j_of.get(k) if j_of is not None else i) for i, k in enumerate(keys)] if work else [None] * len(keys)

    def log_end(out, status, error, unlabelled, spots):
        if CFG["log"]:
            _write_log(_occ_rows(call, keys, fulls, out, cached, firsts, [spots.get(j) if j is not None else None for j in occ_work], effort)
                       + [_job_row(call, status, error, len(items), len(set(keys)), len(cached), len(work), unlabelled, votes, fmt, effort, False)])

    spots = {}
    if work:
        work_texts = [texts[i] for _, i in work]
        try:
            _preflight(work_texts, labels, guidance, votes)
        except _Refused:
            log_end([known.get(k) for k in keys], "failed", "refused before sending: too large for the cell", sum(k not in known for k in keys), spots)
            raise

        def keep(j, label, eff):
            if len(_CLASSIFY_CACHE) >= _CLASSIFY_CACHE_MAX:
                _CLASSIFY_CACHE.clear()
            # Keyed by the effort the label was really given at (an API refusal may have changed it).
            _CLASSIFY_CACHE[(work[j][0],) + stamp(eff or effort)] = label

        progressive = CFG["dedupe"] and votes == 1
        run = functools.partial(_classify_jobs, hashes=[k[:16] for k, _ in work], call=call, votes=votes,
                                on_label=keep if progressive else None, where=spots)
        try:
            got = await _vote(work_texts, labels, guidance, votes, run)
        except _Incomplete as gap:
            labelled = gap.labelled if votes == 1 else {}
            out = [known[k] if k in known else labelled.get(occ_work[i]) for i, k in enumerate(keys)]
            log_end(out, "failed", gap.category, sum(v is None for v in out), spots)
            raise RuntimeError(_incomplete_message(gap, work, keys, len(items), progressive)) from None
        if CFG["dedupe"]:
            efforts = {w[2] for w in spots.values()}
            for j, ((k, _), label) in enumerate(zip(work, got)):
                known[k] = label
                if votes == 1 or len(efforts) <= 1:
                    keep(j, label, spots[j][2] if j in spots else next(iter(efforts), effort))
            out = [known[k] for k in keys]
        else:
            out = got
    else:
        out = [known[k] for k in keys]
    log_end(out, "ok", None, 0, spots)
    return out


def _incomplete_message(gap, work, keys, total, progressive):
    lost = {work[j][0] for j in gap.missing}
    positions = [i for i, k in enumerate(keys) if k in lost] if CFG["dedupe"] else [work[j][1] for j in gap.missing]
    shown = ", ".join(str(i) for i in positions[:20]) + (", ..." if len(positions) > 20 else "")
    kept = (f" The {total - len(positions)} labels already given are kept: calling classify again with the same items, labels "
            "and guidance asks only for the missing ones." if progressive and len(positions) < total else "")
    return (f"classify: {len(positions)} of {total} items still without a valid label: {'; '.join(gap.details)}. "
            f"Items without a label (positions in your list): [{shown}].{kept}")


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


def _refresh_imports():
    """Before each cell: a package installed since the last one must be importable. Drops the finders'
    directory caches, and adds a user-site or site-packages directory that did not exist when `site`
    ran at startup (it only adds existing ones), processing its .pth files. Stat calls only."""
    importlib.invalidate_caches()
    try:
        dirs = list(site.getsitepackages()) if hasattr(site, "getsitepackages") else []
        if site.ENABLE_USER_SITE:
            dirs.append(site.getusersitepackages())
        for directory in dirs:
            if directory and directory not in sys.path and os.path.isdir(directory):
                site.addsitedir(directory)
    except Exception:
        pass


for line in sys.stdin:
    try:
        message = json.loads(line)
        if message.get("op") != "run":
            continue
        code = message.get("code", "")
        CFG.update(message.get("cfg") or {})
        if len(code.encode("utf-8")) > 1_048_576:
            write_frame({"op": "done", "stdout": "", "value": None, "error": "cell exceeds 1 MiB"})
            continue
        _refresh_imports()
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
