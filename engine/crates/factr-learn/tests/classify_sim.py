#!/usr/bin/env python3
"""Free end-to-end simulator of `classify`: the REAL REPL worker (python_worker.py) driven by a fake host that
mirrors host.rs (16 host calls per cell, 64 prompts and 2,000,000 bytes per batch, 200,000 characters per prompt,
120 s of model-call wait per cell plus 30 s per wave of the batch concurrency, 8 s per backoff sleep) and a
scripted fake provider on a VIRTUAL clock. No model is called; nothing here is benchmark data (invented labels
and records).

THIS IS A MODEL OF THE HARNESS, NOT A MEASUREMENT. The latency model (5.5 s fixed + 0.0164 s per output token)
and the 0.364 input tokens per prompt character come from the 0.0.3 profile; the token costs of a reply and of
hidden reasoning per record are assumptions stated in TOKENS below.

Run: python3 classify_sim.py            (asserts, then prints the comparison table)
"""
import hashlib
import heapq
import json
import math
import os
import random
import re
import subprocess
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
WORKER = os.path.join(HERE, "..", "src", "python_worker.py")
PYTHON = os.environ.get("FACTR_SIM_PYTHON") or sys.executable

# ---- the model of the provider -------------------------------------------------------------------------------
FIXED_S, PER_OUT_TOKEN_S = 5.5, 0.0164          # profile: sub-call latency = 5.5 s + 0.0164 s per output token
IN_TOKENS_PER_CHAR = 0.364                       # profile: prompt tokens per character
TOKENS = {                                        # ASSUMED, per record in a reply
    "codes_line": 3.0,                            # "123:B\n"
    "json_pair": 4.2,                             # '"123": "amber", ' (the profile's JSON floor)
    "prose_char": 0.25,                           # any other reply text
    "reasoning": {"medium": 4.7, "low": 1.0, "none": 0.0},   # hidden reasoning; medium = profile excess, low ASSUMED
}
ERROR_LATENCY_S = {"429": 1.0, "5xx": 2.0, "400": 0.3, "refused_effort": 0.4}

# ---- the host limits (host.rs) --------------------------------------------------------------------------------
MAX_HOST_CALLS, MAX_BATCH_PROMPTS, MAX_BATCH_BYTES, MAX_QUERY_CHARS = 16, 64, 2_000_000, 200_000
HOST_WAIT_S, WAVE_ALLOWANCE_S, MAX_SLEEP_S, COMPUTE_S = 120.0, 30.0, 8.0, 20.0

LABELS = ["amber", "cobalt", "jade", "umber"]
KEYWORDS = {"amber": ["honey", "resin", "topaz", "marigold"], "cobalt": ["glacier", "sapphire", "harbor", "indigo"],
            "jade": ["fern", "moss", "pine", "lichen"], "umber": ["clay", "walnut", "bark", "soil"]}
WORD_LABEL = {w: lab for lab, ws in KEYWORDS.items() for w in ws}
FILLER = "quiet ledger window parcel morning signal copper ribbon lantern meadow orbit canvas thimble".split()
LOG_KEYS = {   # what the worker may write (the engine adds session, cell, run_id)
    "call": {"type", "call_id", "classify_call", "pass", "wave", "attempt", "chunk_size", "records_count", "input_tokens",
             "output_tokens", "reasoning_tokens", "cached_tokens", "latency_ms", "ts_start_ms", "ts_end_ms", "effort",
             "requested_effort", "effort_fallback", "format", "votes", "model", "legacy", "dedupe", "chunk_items", "chunk_chars", "chunk_source", "concurrency",
             "prompt_chars", "reply_chars", "error", "refusal", "transport_error", "validation_failure", "results"},
    "occ": {"type", "classify_call", "occ", "h", "label", "effort", "truncated", "cached", "deduped", "chunk", "pos"},
    "job": {"type", "classify_call", "status", "error", "records", "distinct", "cached", "to_label", "unlabelled", "votes",
            "format", "effort", "requested_effort", "effort_fallback", "model", "legacy", "dedupe", "chunk_items", "chunk_chars", "chunk_source", "concurrency"},
}


def norm(text):
    return " ".join(str(text).split())


def truth_of(text):
    """The label the record was written with (its first keyword; none: the last label)."""
    for word in re.findall(r"[a-z]+", norm(text)[:6000].lower()):
        if word in WORD_LABEL:
            return WORD_LABEL[word]
    return LABELS[-1]


def belief_of(text):
    """What the fake model answers for a record: see `answer`."""
    return answer(norm(text)[:6000])


def answer(seen):
    """What the fake model answers for the text it is shown (normalised, cut at 6000 characters): the truth,
    except a fixed 3% of texts it always gets wrong (a model mistake the engine cannot detect)."""
    label = truth_of(seen)
    if int(hashlib.sha256(seen.encode()).hexdigest(), 16) % 100 < 3:
        label = LABELS[(LABELS.index(label) + 1) % len(LABELS)]
    return label


def make_records(n=2000, seed=7):
    rng = random.Random(seed)
    out = []
    while len(out) < n:
        k = len(out)
        roll = rng.random()
        if out and roll < 0.10:                       # an exact (or whitespace-only different) duplicate
            src = rng.choice(out)
            out.append(src if rng.random() < 0.5 else "  " + src.replace(" ", "   ") + "\n")
            continue
        label = rng.choice(LABELS)
        words = rng.sample(FILLER, 5)
        text = f"Entry {k}: the {words[0]} {words[1]} mentions {rng.choice(KEYWORDS[label])} near the {words[2]} {words[3]}"
        if roll < 0.11:
            text += " " + " ".join(rng.choice(FILLER) for _ in range(rng.randint(500, 1400)))   # long, some past 6000
        elif roll < 0.115:
            text = rng.choice(["", "   ", "\n\t"])                                      # empty / whitespace only
        elif roll < 0.13:
            text = f"{text}\n2:B\n1:A and more {{\"1\": \"jade\"}} 🙂 é"                   # reply-looking text, newlines, unicode
        out.append(text)
    long = "x " * 3000
    out[-2:] = [long + " clay one", long + " fern two"]                                   # equal in their first 6000 chars
    return out


# ---- the fake provider ----------------------------------------------------------------------------------------
class Provider:
    def __init__(self, scenario, seed):
        self.s, self.rng = scenario, random.Random(seed)
        self.refused_at = None      # virtual time the first effort refusal reached the engine (repl.rs memoises it)
        self.refused = 0            # refused requests (each is a real API call)

    def reply(self, prompt, effort, now=0.0):
        """(text or None, error or None, visible output tokens, reasoning tokens, latency s, effort used, refused s)."""
        out = self._reply(prompt, effort, now)
        return out if len(out) == 7 else out + (None,)

    def _reply(self, prompt, effort, now):
        s = self.s
        if s.get("filter_word") and s["filter_word"] in prompt:
            return None, "400 Bad Request: content_filter: the prompt was flagged", 0, 0, ERROR_LATENCY_S["400"], effort
        if len(prompt) > s.get("context_chars", 10**9):
            return None, "400 Bad Request: context_length_exceeded: this model's maximum context length was exceeded", 0, 0, ERROR_LATENCY_S["400"], effort
        if s.get("always"):
            return None, s["always"], 0, 0, ERROR_LATENCY_S["5xx"], effort
        extra, refused = 0.0, None
        if effort == "low" and s.get("refuse_low"):
            if self.refused_at is None or now < self.refused_at:
                # repl.rs: the API refuses the pinned effort; the call is sent again at the main agent's effort,
                # and the session stops pinning it once the first refusal is back (calls already started still pay).
                extra = refused = ERROR_LATENCY_S["refused_effort"]
                self.refused += 1
                end = now + refused
                self.refused_at = end if self.refused_at is None else min(self.refused_at, end)
            effort = "medium"
        if self.rng.random() < s.get("p5xx", 0):
            return None, "503 Service Unavailable", 0, 0, ERROR_LATENCY_S["5xx"] + extra, effort
        body = prompt.split("\nItems:\n", 1)[1]
        rows = [(int(line.split(". ", 1)[0]), line.split(". ", 1)[1] if ". " in line else "") for line in body.split("\n")]
        codes = "\nCodes: " in prompt
        if codes:
            table = prompt.split("\nCodes: ", 1)[1].split("\n", 1)[0]
            code_of = {json.loads(v): k for k, v in re.findall(r'([A-Z])=("(?:[^"\\]|\\.)*")', table)}
            lines = [f"{i}:{code_of[answer(t)]}" for i, t in rows]
        else:
            lines = [(str(i), answer(t)) for i, t in rows]
        r, prose = self.rng.random(), ""
        p = s.get("p_reply_fault", 0)
        if r < p * 0.25 and len(lines) > 1:                                    # dropped ids
            for _ in range(min(len(lines) - 1, self.rng.randint(1, 3))):
                lines.pop(self.rng.randrange(len(lines)))
        elif r < p * 0.35:                                                     # a wrong letter / unknown label
            j = self.rng.randrange(len(lines))
            lines[j] = (lines[j].split(":")[0] + ":Q") if codes else (lines[j][0], "magenta")
        elif r < p * 0.45:                                                     # a duplicate id
            lines.insert(0, lines[-1])
        elif r < p * 0.55:                                                     # cut off half way
            lines = lines[: max(1, len(lines) // 2)]
        elif r < p * 0.65:                                                     # empty
            lines = []
        elif r < p * 0.70:                                                     # declined
            lines, prose = [], "I'm sorry, I can't help with that."
        elif r < p:                                                            # leading prose (harmless if complete)
            prose = "Sure, here are the labels:"
        if codes:
            text = "\n".join(([prose] if prose else []) + lines)
            visible = len(lines) * TOKENS["codes_line"] + len(prose) * TOKENS["prose_char"]
        else:
            body = "{" + ", ".join(f"{json.dumps(i)}: {json.dumps(v)}" for i, v in lines) + "}" if lines else ""
            if r < p * 0.55 and r >= p * 0.45 and body:
                body = body[: len(body) // 2]                                  # truncated JSON is invalid JSON
            text = (prose + "\n" if prose else "") + body
            visible = len(lines) * TOKENS["json_pair"] + len(prose) * TOKENS["prose_char"]
        reasoning = len(rows) * TOKENS["reasoning"].get(effort, 0.0)
        latency = (FIXED_S + PER_OUT_TOKEN_S * (visible + reasoning)) * s.get("slow", 1.0) + extra
        return text, None, visible, reasoning, latency, effort, refused


# ---- the fake host --------------------------------------------------------------------------------------------
class Host:
    def __init__(self, scenario, cfg, seed):
        self.provider = Provider(scenario, seed)
        self.s, self.cfg = scenario, cfg
        self.conc = cfg.get("concurrency", 8)
        self.effort = "medium" if cfg.get("legacy") else (cfg.get("effort") or "medium")
        self.clock = 0.0
        self.stats = {"calls": 0, "failed_calls": 0, "in": 0.0, "out": 0.0, "reasoning": 0.0, "batches": 0, "naps": 0.0, "cells": 0}
        self.rows = []

    def batch(self, prompts, host_left):
        """One `llm_query_batch_meta` host call: (reply objects, virtual seconds it took, error or None)."""
        if not prompts:
            return None, 0.0, "llm_query_batch: the list is empty"
        if len(prompts) > MAX_BATCH_PROMPTS:
            return None, 0.0, f"llm_query_batch: {len(prompts)} prompts, the limit is {MAX_BATCH_PROMPTS}"
        total = sum(len(p.encode()) for p in prompts)
        if total > MAX_BATCH_BYTES:
            return None, 0.0, f"llm_query_batch: {total} bytes of input, the limit is {MAX_BATCH_BYTES}"
        slots = [0.0] * min(self.conc, self.s.get("queue_limit", self.conc))   # a provider that serialises calls
        heapq.heapify(slots)
        active, out, makespan = [], [], 0.0
        for prompt in prompts:
            start = heapq.heappop(slots)
            active = [e for e in active if e > start]
            self.stats["calls"] += 1
            now = self.clock + start
            limit = self.s.get("provider_concurrency")
            burst = self.s.get("burst_429")
            if len(prompt) > MAX_QUERY_CHARS:
                text, err, vis, rea, lat, eff, ref = None, f"llm_query: prompt is {len(prompt)} characters, the limit is {MAX_QUERY_CHARS}; split the input into smaller slices", 0, 0, 0.0, None, None
            elif (limit and len(active) >= limit) or (burst and burst[0] <= now < burst[1]):
                text, err, vis, rea, lat, eff, ref = None, "429 Too Many Requests: rate limit reached", 0, 0, ERROR_LATENCY_S["429"], None, None
            else:
                text, err, vis, rea, lat, eff, ref = self.provider.reply(prompt, self.effort, now)
                if ref is not None:
                    self.stats["calls"] += 1
            end = start + lat
            heapq.heappush(slots, end)
            active.append(end)
            makespan = max(makespan, end)
            tin = math.ceil(len(prompt) * IN_TOKENS_PER_CHAR) if err is None else None
            if err is None:
                self.stats["in"] += tin
                self.stats["out"] += vis + rea
                self.stats["reasoning"] += rea
                out.append({"t": text, "e": None, "i": tin, "o": round(vis + rea), "c": 0, "r": None, "ms": int((lat - (ref or 0)) * 1000), "s": int(start * 1000), "f": eff,
                            "q": self.cfg.get("requested_effort") or self.effort, "x": int(ref * 1000) if ref is not None else None})
            else:
                self.stats["failed_calls"] += 1
                out.append({"t": "", "e": "Error: " + err, "i": None, "o": None, "c": None, "r": None, "ms": int(lat * 1000), "s": int(start * 1000), "f": None})
        if makespan > host_left:
            return None, host_left, ("host call time budget exhausted: the cell may wait 120s in total on model calls "
                                     "(more for a large batch); split the work across cells")
        return out, makespan, None

    def cell(self, proc, code, max_frames=200_000, max_real_s=300.0):
        """Run one REPL cell on `proc`: returns the done frame. Mirrors host.rs `drive`."""
        self.stats["cells"] += 1
        proc.stdin.write(json.dumps({"op": "run", "code": code, "cfg": self.cfg}) + "\n")
        proc.stdin.flush()
        host_calls, host_left, compute, frames = 0, HOST_WAIT_S, 0.0, 0
        real_start = time.monotonic()
        while True:
            waited = time.monotonic()
            line = proc.stdout.readline()
            compute += time.monotonic() - waited
            frames += 1
            assert line, "the worker exited"
            assert frames < max_frames and time.monotonic() - real_start < max_real_s, "no end in sight: an endless loop?"
            msg = json.loads(line)
            if msg["op"] == "done":
                assert compute < COMPUTE_S, f"the cell spent {compute:.1f} s of worker compute (limit {COMPUTE_S})"
                msg["compute_s"] = compute
                return msg
            name, args = msg["fn"], msg["args"]
            if name == "classify_log":
                for row in args[0].split("\n"):
                    obj = json.loads(row)
                    assert obj["type"] in LOG_KEYS and set(obj) <= LOG_KEYS[obj["type"]], f"unexpected log row {obj}"
                    self.rows.append(obj)
                reply = {"op": "reply", "value": ""}
            elif name == "sleep_ms":
                wait = min(int(args[0]) / 1000.0, MAX_SLEEP_S, host_left)
                self.clock += wait
                self.stats["naps"] += wait
                host_left -= wait
                reply = {"op": "reply", "value": ""}
            else:
                host_calls += 1
                if host_calls > MAX_HOST_CALLS:
                    reply = {"op": "reply", "error": "host call budget (16) exhausted"}
                elif name != "llm_query_batch_meta":
                    reply = {"op": "reply", "error": f"the simulator does not serve {name}"}
                else:
                    prompts = json.loads(args[0])
                    host_left += WAVE_ALLOWANCE_S * math.ceil(min(len(prompts), MAX_BATCH_PROMPTS) / min(self.conc, 8))
                    self.stats["batches"] += 1
                    value, took, error = self.batch(prompts, host_left)
                    self.clock += took
                    host_left = max(0.0, host_left - took)
                    reply = {"op": "reply", "error": error} if error else {"op": "reply", "value": json.dumps(value)}
            proc.stdin.write(json.dumps(reply) + "\n")
            proc.stdin.flush()


def worker():
    tmp = tempfile.mkdtemp(prefix="classify-sim-")
    proc = subprocess.Popen([PYTHON, "-I", "-S", "-u", "-c", open(WORKER).read(), tmp, tmp], stdin=subprocess.PIPE,
                            stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True, encoding="utf-8")
    assert json.loads(proc.stdout.readline())["op"] == "ready"
    return proc, tmp


def run(records, scenario, cfg, votes=1, seed=1, max_cells=3):
    """Like an agent: classify in one cell; if it names unlabelled items, call it again in a new cell (the error says
    the labels already given are kept). Returns (labels or None, host, errors)."""
    proc, tmp = worker()
    data, result = os.path.join(tmp, "records.json"), os.path.join(tmp, "labels.json")
    with open(data, "w") as f:
        json.dump(records, f)
    # The seed makes the worker's backoff jitter, and so the virtual clock, repeatable.
    code = (f"import json, random\nrandom.seed(5)\nitems = json.load(open({data!r}))\nr = await classify(items, {LABELS!r}, votes={votes})\n"
            f"json.dump(r, open({result!r}, 'w'))")
    host, errors = Host(scenario, cfg, seed), []
    try:
        for _ in range(max_cells):
            done = host.cell(proc, code)
            if done["error"] is None:
                return json.load(open(result)), host, errors
            errors.append(done["error"])
        return None, host, errors
    finally:
        proc.kill()


# ---- scenarios, assertions and the table -----------------------------------------------------------------------
CLEAN = {"name": "clean"}
FAULTY = {"name": "faulty", "p5xx": 0.02, "p_reply_fault": 0.10, "context_chars": 12000, "provider_concurrency": 12,
          "burst_429": (30.0, 34.0)}


def n_chunks(texts, size, cap=24000, balanced=True):
    """How many chunks the engine cuts: consecutive runs of at most `size` records (spread evenly in 0.0.4) and
    `cap` characters."""
    if balanced and texts:
        size = -(-len(texts) // -(-len(texts) // size))
    n, count, chars = 0, 0, 0
    for t in texts:
        if count and (count >= size or chars + len(t) > cap):
            n, count, chars = n + 1, 0, 0
        count, chars = count + 1, chars + len(t)
    return n + (1 if count else 0)


def expected_clean(texts, chunk, conc, fmt, effort, balanced=True, cap=24000):
    """The model's own prediction for a clean run (one wave): calls, output tokens, seconds."""
    calls = n_chunks(texts, chunk, cap=cap, balanced=balanced)
    per_record = (TOKENS["codes_line"] if fmt == "codes" else TOKENS["json_pair"]) + TOKENS["reasoning"][effort]
    out = len(texts) * per_record
    call_s = FIXED_S + PER_OUT_TOKEN_S * per_record * len(texts) / calls
    return calls, out, math.ceil(calls / conc) * call_s


def main():
    records = make_records()
    normed = [norm(r) for r in records]
    distinct = len(set(normed))
    all_texts = [t[:6000] for t in normed]
    work_texts = [t[:6000] for t in dict.fromkeys(normed)]
    truth = [truth_of(r) for r in records]
    belief = [belief_of(r) for r in records]
    configs = [
        ("LEGACY 0.0.3 (json, medium)", {"legacy": True, "log": True}, 40, 8, "json", "medium"),
        ("NEW c40 k8 (medium)", {"format": "codes", "dedupe": True, "chunk_items": 40, "concurrency": 8, "log": True, "effort": "medium", "model": "sim"}, 40, 8, "codes", "medium"),
        ("NEW c40 k8 (opt-in low)", {"format": "codes", "dedupe": True, "chunk_items": 40, "concurrency": 8, "log": True, "effort": "low", "requested_effort": "low", "model": "sim"}, 40, 8, "codes", "low"),
        ("NEW c40 k16 (opt-in low)", {"format": "codes", "dedupe": True, "chunk_items": 40, "concurrency": 16, "log": True, "effort": "low", "requested_effort": "low", "model": "sim"}, 40, 16, "codes", "low"),
        ("NEW c80 k8", {"format": "codes", "dedupe": True, "chunk_items": 80, "concurrency": 8, "log": True, "effort": "medium", "model": "sim"}, 80, 8, "codes", "medium"),
        # The 0.0.4 candidate defaults: json, no dedupe, 4 labels and short records pick 80 items / 48000 chars.
        ("NEW default (json, no dedupe, 80)", {"dedupe": False, "concurrency": 8, "log": True, "effort": "medium", "model": "sim"}, 80, 8, "json", "medium"),
        ("NEW c80 k16", {"format": "codes", "dedupe": True, "chunk_items": 80, "concurrency": 16, "log": True, "effort": "medium", "model": "sim"}, 80, 16, "codes", "medium"),
    ]
    table = []
    for scenario in (CLEAN, FAULTY):
        for name, cfg, chunk, conc, fmt, effort in configs:
            got, host, errors = run(records, scenario, cfg)
            st = host.stats
            ok = got is not None
            if cfg.get("legacy") and not ok:
                pass   # 0.0.3 may give up under faults (it is reported, not asserted)
            else:
                assert ok, f"{name}/{scenario['name']}: {errors}"
                bad = [i for i in range(len(records)) if got[i] != belief[i]]
                assert not bad, f"{name}/{scenario['name']}: {len(bad)} labels differ from what the model answered, first {bad[:5]}"
            rows = host.rows
            occs = [r for r in rows if r["type"] == "occ"]
            if ok:
                assert len(occs) >= len(records) and [r["occ"] for r in occs[-len(records):]] == list(range(len(records)))
                assert [r["label"] for r in occs[-len(records):]] == got, "the log's occurrence rows carry the final labels"
                assert all(re.fullmatch(r"[0-9a-f]{16}", r["h"]) for r in occs)
            calls = [r for r in rows if r["type"] == "call"]
            want_src = "default-80" if name.startswith("NEW default") else "default-40" if cfg.get("legacy") else "user"
            assert {r["chunk_source"] for r in rows if r["type"] in ("call", "job")} == {want_src}, f"{name}: chunk_source"
            assert len(calls) == st["calls"], f"{name}: one log row per sub-call ({len(calls)} vs {st['calls']})"
            for e in errors:
                assert "items still without a valid label" in e and "positions in your list" in e or "classify:" in e, e
            texts = all_texts if (cfg.get("legacy") or cfg.get("dedupe") is False) else work_texts
            if scenario is CLEAN:
                want_calls, want_out, want_s = expected_clean(texts, chunk, conc, fmt, effort, balanced=not cfg.get("legacy"),
                                                          cap=cfg.get("chunk_chars") or (24000 if cfg.get("legacy") else 48000))
                assert st["calls"] == want_calls and st["cells"] == 1, f"{name}: {st}"
                assert abs(st["out"] - want_out) <= 0.05 * want_out + 50, f"{name}: output {st['out']} vs {want_out}"
                assert want_s * 0.6 <= host.clock <= want_s * 1.25, f"{name}: {host.clock:.1f} s vs {want_s:.1f} s"
            elif not cfg.get("legacy"):
                clean = next(r for r in table if r[0] == name and r[1] == "clean")
                assert st["calls"] <= 3 * clean[3] + 32, f"{name}: {st['calls']} calls under faults"
                assert host.clock <= 4 * clean[7] + 60, f"{name}: {host.clock:.0f} s under faults"
            acc = sum(g == t for g, t in zip(got, truth)) / len(truth) if ok else float("nan")
            table.append((name, scenario["name"], st["cells"], st["calls"], st["failed_calls"], st["in"], st["out"], host.clock, acc, ok))

    # Extra scenarios: effort refused (falls back per call), votes=2 under faults, and the ways a job must end loudly.
    got, host, errors = run(records, {"name": "refuse-low", "refuse_low": True}, configs[2][1])
    refused_rows = [r for r in host.rows if r["type"] == "call" and r["error"] == "request failed: effort refused"]
    assert got == belief and all(r["effort"] == "medium" and r["requested_effort"] in ("low", "medium") for r in host.rows
                                 if r["type"] == "call" and r not in refused_rows), "a refused effort falls back and the log says so"
    assert all(r["effort_fallback"] is True for r in refused_rows), "the refused request is flagged as a fallback"
    assert len(refused_rows) == host.provider.refused <= 8, f"memoised: only the calls started before the first refusal came back pay it ({host.provider.refused})"
    assert host.stats["calls"] == n_chunks(work_texts, 40) + host.provider.refused, "every later call is sent once"
    table.append(("NEW c40 k8, API refuses `low`", "clean", host.stats["cells"], host.stats["calls"], host.stats["failed_calls"],
                  host.stats["in"], host.stats["out"], host.clock, sum(g == t for g, t in zip(got, truth)) / len(truth), True))
    got, host, errors = run(records, FAULTY, configs[2][1], votes=2)
    assert got == belief, "votes=2 under faults: the majority of identical answers"
    table.append(("NEW c40 k8, votes=2", "faulty", host.stats["cells"], host.stats["calls"], host.stats["failed_calls"],
                  host.stats["in"], host.stats["out"], host.clock, sum(g == t for g, t in zip(got, truth)) / len(truth), True))
    got, host, errors = run(records, {"name": "down", "always": "503 Service Unavailable"}, configs[2][1], max_cells=1)
    assert got is None and "2000 of 2000 items still without a valid label" in errors[0] and "transport error persisted" in errors[0], errors
    assert host.stats["calls"] == 2 * n_chunks(work_texts, 40), "every chunk asked twice (one transport re-ask), then a clear error"
    got, host, errors = run(records, {"name": "auth", "always": "401 Unauthorized: invalid api key"}, configs[2][1], max_cells=1)
    assert got is None and "asking again cannot help" in errors[0] and host.stats["calls"] == n_chunks(work_texts, 40), errors
    # A record the provider filters, in chunks of 80: halving isolates it alone; the other 1,999 are labelled.
    marked = list(records)
    marked[1234] = "Entry flagged-marker mentions fern"
    got, host, errors = run(marked, {"name": "filter", "filter_word": "flagged-marker"}, dict(configs[4][1]), max_cells=1)
    assert got is None and "1 of 2000 items" in errors[0] and "[1234]" in errors[0] and "1999 labels already given" in errors[0], errors
    jobs = [r for r in host.rows if r["type"] == "job"]
    assert jobs[-1]["status"] == "failed" and jobs[-1]["unlabelled"] == 1 and jobs[-1]["error"] == "refusal: content filter", jobs
    assert sum(r["label"] is None for r in host.rows if r["type"] == "occ") == 1, "the failed job still logs every occurrence"
    # 16-wide against a provider that serialises at 8 and answers slowly: the wave allowance must not shrink with
    # the concurrency (at 0.0.4-rc it did, and the whole batch was dropped at the host timeout).
    got, host, errors = run(records, {"name": "slow", "slow": 5.0, "queue_limit": 8}, configs[3][1], max_cells=1)
    assert got == belief and host.stats["cells"] == 1, errors
    table.append(("NEW c40 k16, slow provider at 8", "slow", host.stats["cells"], host.stats["calls"], host.stats["failed_calls"],
                  host.stats["in"], host.stats["out"], host.clock, sum(g == t for g, t in zip(got, truth)) / len(truth), True))
    got, host, errors = run(records, CLEAN, dict(configs[2][1], chunk_items=1), max_cells=1)
    assert got is None and "host calls" in errors[0] and host.stats["calls"] == 0, "a job that cannot fit is refused before anything is sent"

    print("\nclassify over 2,000 synthetic records (4 invented labels, %d distinct texts) on a VIRTUAL clock." % distinct)
    print("MODEL OF THE HARNESS, NOT A MEASUREMENT: latency 5.5 s + 0.0164 s/output token, 0.364 input tokens/char;")
    print("ASSUMED reply tokens per record: codes 3.0, json 4.2; hidden reasoning per record: medium 4.7 (profile), low 1.0.")
    print("faulty = 2% 5xx, 10% malformed replies, 400 above 12,000 prompt chars, provider limit 12 in flight, 429 burst at 30-34 s.")
    print("A failed job is called again in a new cell, as its error suggests (cells column).\n")
    head = f"{'config':32} {'scenario':8} {'cells':>5} {'calls':>6} {'failed':>6} {'input tok':>10} {'output tok':>10} {'virtual s':>9} {'acc':>6}"
    print(head)
    print("-" * len(head))
    for name, scen, cells, calls, failed, tin, tout, secs, acc, ok in table:
        print(f"{name:32} {scen:8} {cells:5d} {calls:6d} {failed:6d} {tin:10.0f} {tout:10.0f} {secs:9.1f} {acc:6.3f}" + ("" if ok else "  GAVE UP"))
    base = next(r for r in table if r[0].startswith("LEGACY") and r[1] == "clean")
    print("\nclean, relative to LEGACY: " + "; ".join(
        f"{r[0].replace('NEW ', '')}: calls {r[3] / base[3]:.2f}x, input {r[5] / base[5]:.2f}x, output {r[6] / base[6]:.2f}x, time {r[7] / base[7]:.2f}x"
        for r in table if r[1] == "clean" and r[0].startswith("NEW") and "refuses" not in r[0]))
    print("accuracy is against the records' own labels; the fake model is wrong on a fixed 3% of texts in every arm.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
