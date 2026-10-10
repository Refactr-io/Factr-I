# 0.0.4: cost and time of long-context labelling

What changed, why, how to switch each piece off, and how to A/B it. All defaults are the new behaviour;
`FACTR_COST_LEGACY=1` restores 0.0.3 exactly (opt-in sub-call effort, prompts, matching, retries, concurrency, tool
text, guidance and environment line).

## Why (measured on development slices only)

Profile of 360 items on the 0.0.3 builds, model gpt-6-luna at medium effort for the main agent and for the
sub-calls. Per item: 4.5 main turns and 5.1 sub-calls (about 48 sub-calls on a full-size scored item).

- A sub-call takes 5.5 s fixed plus 0.0164 s per output token (61 tokens/s); the mean call writes 362 tokens.
- Sub-calls are 81% of output tokens. The JSON answer itself needs about 4 (spam-style) to 6 (topic-style)
  tokens per record; observed 8.9 and 12.0. About half of the sub-call output is therefore not the answer, most
  likely hidden reasoning inherited from the main agent's `medium` effort (inferred: the stream's usage event does
  not report reasoning tokens).
- 0 of 2.6M sub-call input tokens were cached (the shared prefix is about 270 tokens, under the 1024 minimum).
- Chunks of 40 records, 8 slots, 5.0 realised: about 13.5 s of waiting per round of 8 calls.
- Waste: 3.2% retried calls (retries halved the chunk size for every pending chunk), 14% of classify items asked
  `votes>=2`, 3% re-labelled in a second cell; 2.5 inspection cells before the labelling cell (about 12% of the
  seconds); 22.5% of items needed an answer-format nudge (not touched here).
- Fuzzy label matching accepted "not spam" as "spam".

Numbers are from the profile; none of the changes below has been measured end to end yet. The A/B plan decides.

## What

| # | Change | Switch (default) | Legacy |
|---|---|---|---|
| 1 | `classify` sub-calls run on a fork of the main agent, so by default they use the main agent's reasoning effort (`inherit`, as in 0.0.3: the effort is the user's, never hard-coded); a lower effort is an explicit opt-in setting; `llm_query` / `llm_query_batch` (reading, summarising) keep the main agent's effort unless set. The main agent is unchanged. Allowed: `none`, `low`, `medium`, `high`, `xhigh`, `inherit`; `minimal` is rejected (the model does not accept it) and an unknown value is logged and the main effort used. The effective effort is resolved once per session (a refusal or fallback is logged once) and is what the cache key and the log carry. If the API itself refuses the pinned effort, that call is sent again at the main effort, and the session remembers it: later sub-calls go straight to the main effort (one refused request per session, not one per call); the log has a row for the refused request, and the cache key and the log carry the effort really used. | `FACTR_REPL_SUB_EFFORT` / `agents.repl_sub_effort` (`inherit`, classify; opt-in `low` etc.); `FACTR_REPL_QUERY_EFFORT` / `agents.repl_query_effort` (`inherit`, queries) | main effort |
| 2 | `classify` asks for a JSON object by default (id to label). Opt-in `FACTR_CLASSIFY_FORMAT=codes` asks for one `id:letter` line per record (`A="spam" \| B="ham"`: labels JSON-quoted in the table; letters, not numbers, so a code cannot be confused with a 1-based id; passes with votes list the labels in another order with the same letters). In codes mode JSON is used instead for more than 26 labels, a label with a non-printable character, or a single-letter label that is not its own code (labels `B`, `A` would read `A="B"`). Lines are read anywhere in the reply (ids like `01`, either case; `1. A` and `1) A` with one letter count, an echoed item line like `1. apple` does not); prose around them is ignored only if every id is covered exactly once with a letter in range, else the chunk is rejected; some invalid ids re-ask only those ids. No fragment matching; a bare number is never a label unless the number is itself one of the labels (JSON format). A JSON object reply still parses. Labels made only of symbols (an emoji, `+`) are allowed and matched on their own text. One label: every item gets it, no sub-call. An empty label list is a `ValueError`. | `FACTR_CLASSIFY_FORMAT=codes\|json` (`json`; `codes` is opt-in) | JSON prompt, fuzzy matching |
| 3 | Retry hygiene, per chunk, with separate bounds so the loop always ends: an invalid reply re-asks the failing records (at most 3 answered asks per record; failures of the same pass and ask count are regrouped, a reply rejected whole at half size). A transport error (429, 5xx, timeout, connection, overload, or an unknown error; the provider already retried the request itself) re-asks the same chunk unchanged once after a jittered backoff the engine waits out (not the cell's 20 s compute, and never longer than the cell's remaining model-call wait; no wait when nothing is asked again); it does not spend the invalid-reply budget and is never regrouped with validation failures. A size error (context length, too long/large, 413, an over-long prompt), a content filter or another 4xx is never repeated at the same size: the chunk is halved, at most ceil(log2(chunk_items)) + 1 times, so down to single records: a prompt over a limit fits and a filtered record is isolated alone. Only a whole wave of at least 4 requests all refused for something other than size stops the job (five chunks over the context limit are halved, not stopped). An authentication, permission, unsupported-setting or missing-model error, or a failed host call (budget, wait), stops at once, and the later batches of that wave are not sent. A job that does not finish raises an error that names the unlabelled items by position and why (engine categories and ids, never the reply's keys or provider text); with `votes=1` and the opt-in dedupe on, every label is cached as it arrives, so calling `classify` again (in a new cell if the budget ran out) asks only for the missing ones. Before sending, a job that cannot fit (16 host calls, 64 prompts and 2,000,000 UTF-8 bytes per batch, 200,000 characters per prompt; with votes > 1 the worst case, every record disagreeing) is refused with a clear message. | always on | halve every pending chunk |
| 4 | Knobs: concurrency, records per chunk, characters per chunk. Unless the user sets the chunk size, the worker picks it from the data (see "Defaults and evidence" below): 80 records / 48000 characters when there are at most 6 distinct labels and the median length of the records to label is at most 600 characters, else 40 / 24000. A user setting always wins (each of the two on its own) and the log says `chunk_source: user`. Chunks are balanced (100 records at 40: 34 + 33 + 33, no small tail), and within a batch the longest prompts are sent first. The host wait allowance is one wave per 8 prompts (or per the concurrency, if lower): raising the concurrency never shortens it. A record is never split; one longer than the cap goes alone. Out-of-range values fall back to the default. | `FACTR_BATCH_CONCURRENCY` (8, 1-64), `FACTR_CLASSIFY_CHUNK_ITEMS` (data-shaped, 1-500), `FACTR_CLASSIFY_CHUNK_CHARS` (data-shaped, 500-150000) | 8 / 40 / 24000, unbalanced |
| 5 | Opt-in dedupe: identical records are labelled once and the label restored for each duplicate, in order. Identity is the hash of the whole normalised text (whitespace collapsed; a lone surrogate becomes `?`), taken before the 6000-character cut (two records equal only in their first 6000 characters stay separate). A per-session cache keyed (text hash, format, model, effective effort, labels in order, guidance as text, votes) makes a second `classify` over the same records free and keeps the labels of a job that failed part way (votes=1). | `FACTR_CLASSIFY_DEDUPE=1` turns both on (off by default) | off |
| 6 | `votes=1` is the documented default (tool description and REPL guidance). A second opinion is asked only for invalid replies unless the caller passes `votes>=2`. | API unchanged | |
| 7 | Log (`FACTR_CLASSIFY_LOG=<path>`; `{pid}` expands; a directory gets `classify-<pid>.jsonl`; written by the engine, mode 0600, append-only, one row per write under a lock, never through a symlink, never to an existing file whose mode is not 600; a write failure is reported once). Three row types; the engine adds `session`, `cell` (per-session counter, survives worker restarts) and `run_id` (when `FACTR_RUN_ID` is set) to every row and prefixes `call_id` / `chunk` with `session/cell/`. `type=call`, one per sub-call attempt: `call_id`, `classify_call`, `pass`, `wave`, `attempt`, `chunk_size`, `records_count` (records accepted from the reply), `input_tokens`, `output_tokens`, `reasoning_tokens` (null: not reported), `cached_tokens`, `latency_ms`, `ts_start_ms` / `ts_end_ms` (the call's own: its batch's start plus the host's slot offset), `effort` (really used), `requested_effort`, `format`, `votes`, `model`, `legacy`, `dedupe`, `chunk_items`, `chunk_chars`, `chunk_source` (`default-80`, `default-40` or `user`), `concurrency`, `prompt_chars`, `reply_chars` (hidden output = `output_tokens` minus the reply's tokens), `error`, `refusal`, `transport_error`, `validation_failure`, `results` (`[hash16, label]` per accepted record). A request the API refused for its effort has its own row (`call_id` + `r`, `error: request failed: effort refused`). `type=occ`, one per input occurrence in order, also when the job failed: `classify_call`, `occ` (index in the caller's list), `h` (16-hex SHA-256 prefix of the full normalised text), `label` (FINAL after voting, dedupe and cache; null if unlabelled), `effort`, `truncated`, `cached`, `deduped`, `chunk` (the `call_id` that labelled this occurrence, or its first copy, in the first pass) and `pos` (its position there), null when cached. `type=job`, one per `classify`, also when it failed: `classify_call`, `status` (`ok` / `failed`), `error` (category), `records`, `distinct`, `cached`, `to_label`, `unlabelled`, `votes`, `format`, `effort`, `model`, `legacy`, `dedupe`, `chunk_items`, `chunk_chars`, `chunk_source`, `concurrency`. `error` is always an engine category, never provider text: `transport: 429` / `transport: 503` / `transport: timeout`, `request error: 400 over a size limit`, `request error: 400` (other 4xx), `request failed: 401` / `request failed: unsupported setting`, `request failed: effort refused`, `refusal: content filter`, `refusal: the reply declined`, `reject: <reason with ids only>`, `host: call budget` / `host: wait` / `host: not sent`, `refused before sending: too large for the cell`; null on success. The legacy path asks through the metered batch call too (same prompts) and logs the same row types. The engine re-checks every row (known keys, plain values, strings up to 1000 characters, hashes 16 hex) and drops the rest; no record text. Logging does not spend the cell's host-call budget. | `FACTR_CLASSIFY_LOG` (off), `FACTR_RUN_ID` | |
| 8 | Main thread, opt-in: with `FACTR_ENV_HEAD=1` the environment line gives the first 600 characters of the first large data file (txt, csv, tsv, jsonl, ndjson, md, log, xml, yaml, json) next to its size and line count, labelled `untrusted head`, JSON-escaped with `<` and `>` as escapes. Never from symlinks or paths outside the working directory, hidden files, lockfiles or code; the file is opened once without following a symlink (`O_NOFOLLOW`), checked to be a regular file on that same descriptor, and only its first 4 KB is read (no check-then-reopen race); it must be valid UTF-8 without control characters, and a head whose first bytes look like a secret (private key, api key, secret, password, `token=`, bearer authorization, aws access key) is not shown. REPL guidance and tool description: label in one `classify` cell after at most one look, pass only the judged text, never classify the same records twice to verify, and the final reply must contain the exact answer line the task asks for. No new nudges. | `FACTR_ENV_HEAD=1` adds the head (default off: no file content in the line) | off |

## Reasoning effort is the user's (default: inherit)

`classify` sub-calls use the main agent's reasoning effort unless the user sets `FACTR_REPL_SUB_EFFORT` (or
`agents.repl_sub_effort`) to `none|low|medium|high|xhigh`; `inherit` or nothing set means the main effort. The
engine never lowers it on its own. When a setting is active it is visible: the engine logs once per session
"repl classify effort pinned to X by FACTR_REPL_SUB_EFFORT / agents.repl_sub_effort", and every classify log row
carries `requested_effort` (the configured value) and `effort` (what actually ran). If the model refuses the
configured value (gpt-6-luna does not support `none`), the call runs at the main effort and the job and call rows
say so with `effort_fallback: true` (a refused request also gets its own row, `error: "request failed: effort
refused"`); nothing falls back silently.

Measured on dev data, `low` cut output tokens per record by 41% but lowered record accuracy by 3.5 points
(94.9% to 91.4%; trec_coarse entity recall 0.89 to 0.70; spam/ham unchanged). It is therefore not the default and
not recommended; `none` is not supported by gpt-6-luna.

Provider facts checked in code: the Responses request carries `reasoning.effort` from the provider's stored effort
(a fork copies it, so sub-calls inherit the main agent's effort); `low` and `none` are in the selectable
ladder; the catalog of the live model can narrow it, which is why a refusal falls back to the main effort.
Reasoning tokens are not reported by the provider layer (only input, output, cache read/write), so
`reasoning_tokens` in the log is null; the effect of the lower effort shows in `output_tokens` per record.

If the API refuses a pinned (opt-in) effort (an error naming the effort, reasoning, or the pinned value as unsupported), that call
is sent again at the main agent's effort, the session stops pinning it, and the log has a row for the refused request. An inherited effort is re-resolved when the main agent's
effort changes, so the cache key and the log follow what the requests carry.

Prefix cost: the first-request prefix grows by 64 tokens (4727 -> 4791; system 2600 -> 2664), all from the REPL
guidance; `FACTR_COST_LEGACY=1` measures 4727 again. The deferred `repl` tool description grew by about 60 tokens
(cap raised 215 -> 275).

## How to A/B

Arms differ in environment only; the engine build is the same.

- A (0.0.3): `FACTR_COST_LEGACY=1` (plus `FACTR_CLASSIFY_LOG` to get its record rows).
- B (0.0.4 defaults): nothing set (json, no dedupe, data-shaped chunk size).
- Isolate one change from B: `FACTR_REPL_SUB_EFFORT=low` (opt-in cheaper classify effort), `FACTR_CLASSIFY_FORMAT=codes` (compact format),
  `FACTR_CLASSIFY_DEDUPE=1` (dedupe and cache); `FACTR_ENV_HEAD=1` adds the head (off by default).
- Chunking and concurrency experiments on top of B: `FACTR_CLASSIFY_CHUNK_ITEMS=40 FACTR_CLASSIFY_CHUNK_CHARS=24000` (the 0.0.3 size, to switch the data-shaped default off),
  `FACTR_BATCH_CONCURRENCY=16` (watch 429s: transport retries show in the log as `transport_error`).
- Set `FACTR_CLASSIFY_LOG=/path/arm-B.jsonl` (and `FACTR_RUN_ID=<item>`) per arm. Compare arms record by record on the hash (the same record
  text has the same hash in every arm) and on the assigned label: agreement between arms, and agreement of each arm
  with independent labels on held-out or synthetic records. Output tokens per record, attempts per chunk and
  latency come from the same file.
- Validate only on development or synthetic data; the scored sets are run once with the frozen candidate.
- Not touched on purpose: the answer-format nudges, the default concurrency (8), the sub-call span attributes (effort, cached tokens and latency are in the classify log instead).

## Simulator (free, a model of the harness, not a measurement)

`engine/crates/factr-learn/tests/classify_sim.py` (run by `cargo test -p factr-learn --test classify_sim -- --nocapture`)
drives the real REPL worker over 2,000 synthetic records (four invented labels, 1,801 distinct texts, duplicates, long,
empty and reply-looking records) against a fake host that mirrors `host.rs` limits and a scripted provider on a virtual
clock: latency 5.5 s + 0.0164 s per output token and 0.364 input tokens per prompt character (the profile), ASSUMED
3.0 tokens per `id:letter` line, 4.2 per JSON pair, hidden reasoning 4.7 tokens per record at `medium` (the profile's
excess) and 1.0 at `low`. `faulty` = 2% 5xx, 10% malformed replies (dropped, duplicate and wrong ids, cut-off, empty,
declined, leading prose), 400 above 12,000 prompt characters, a provider limit of 12 calls in flight and a 429 burst
at 30-34 s, with no provider-side retries (pessimistic). A job that fails is called again in a new cell, as its error
says. It asserts that every final label equals the fake model's answer, the call/token/time bounds, one log row per
sub-call and per occurrence, and the error paths (provider down: two asks per chunk then a clear error; 401: one wave;
a job too large: refused before sending).

| config | scenario | cells | calls | failed | input tok | output tok | virtual s |
|---|---|---|---|---|---|---|---|
| LEGACY 0.0.3 (json, medium) | clean | 1 | 50 | 0 | 112,257 | 17,800 | 79.4 |
| NEW c40 k8 (medium) | clean | 1 | 46 | 0 | 100,599 | 13,868 | 63.3 |
| NEW c40 k8 (opt-in low) | clean | 1 | 46 | 0 | 100,599 | 7,204 | 48.7 |
| NEW c40 k16 (opt-in low) | clean | 1 | 46 | 0 | 100,599 | 7,204 | 24.4 |
| NEW c80 k8 | clean | 1 | 23 | 0 | 98,694 | 13,868 | 46.4 |
| NEW c80 k16 | clean | 1 | 23 | 0 | 98,694 | 13,868 | 31.0 |
| LEGACY 0.0.3 (json, medium) | faulty | 3 (gave up) | 208 | 31 | 325,778 | 54,241 | 276.2 |
| NEW c40 k8 (medium) | faulty | 2 | 74 | 22 | 101,326 | 13,913 | 87.8 |
| NEW c40 k8 (opt-in low) | faulty | 1 | 67 | 15 | 103,509 | 7,224 | 74.3 |
| NEW c40 k16 (opt-in low) | faulty | 3 | 137 | 84 | 101,296 | 7,224 | 69.1 |
| NEW c80 k8 | faulty | 2 | 80 | 37 | 100,418 | 13,899 | 106.6 |
| NEW c80 k16 | faulty | 2 | 89 | 48 | 100,217 | 13,899 | 103.6 |
| NEW c40 k8, API refuses `low` (opt-in; 8 refused requests, then memoised) | clean | 1 | 54 | 0 | 100,599 | 13,868 | 63.7 |
| NEW c40 k8, votes=2 | faulty | 2 | 245 | 37 | 405,704 | 28,903 | 241.3 |
| NEW c40 k16, slow provider at 8 | slow | 1 | 46 | 0 | 100,599 | 7,204 | 243.7 |

Further scenarios asserted: a record the provider content-filters inside a chunk of 80 is isolated alone (1 of 2,000
unlabelled, the failed job still logs every occurrence and a `job` row); the slow 16-wide wave above times out at the host
with the old allowance (one wave per 16 prompts) and finishes with the new one.

Read: on clean runs, at the inherited (main) effort, the model predicts output tokens down 22% (format only) and time 0.80x at chunk 40 / 8-wide; the opt-in `low` assumption (not recommended, see the finding above) takes output down 60% and time to 0.61x, and chunk 80 / 16-wide at the inherited effort gives 0.39x. Under a provider that allows fewer calls in
flight than the engine sends, 16-wide loses most of its gain to 429s in this model (no adaptive concurrency; the measured development test logged no 429s); with real provider-side
retries the loss would be smaller but slower. The 0.0.3 loop gives up under the same faults and relabels everything on
the next call; the new loop keeps what it labelled.

## Defaults and evidence (candidate)

Three defaults differ from the first 0.0.4 draft. Reasoning effort is unchanged: it inherits the user's setting.

- Format: JSON object replies by default. The compact `id:letter` format stays as opt-in (`FACTR_CLASSIFY_FORMAT=codes`):
  measured on dev data it cost 0.40 points of accuracy and produced more rejected replies for only 11% fewer output tokens.
- Dedupe and the per-session cache: off by default (`FACTR_CLASSIFY_DEDUPE=1` or the config knob turns them on). The saving depends on
  how often records repeat (0.3% to 22.5% on the dev slices) and it assumes a record's label depends only on its text. With it off every
  occurrence is asked and logged (`type=occ` rows still carry one row per input, `deduped` false).
- Chunk size, used only when `FACTR_CLASSIFY_CHUNK_ITEMS` / `FACTR_CLASSIFY_CHUNK_CHARS` are not set: 80 items / 48000 characters when
  there are at most 6 distinct labels and the median length of the records to label is at most 600 characters; otherwise 40 / 24000
  (the 0.0.3 values). Chunks stay balanced. The choice is made per `classify` call from its own records and labels and logged on every call and job row
  (`chunk_items`, `chunk_chars`, `chunk_source`: `default-80`, `default-40` or `user`). `FACTR_COST_LEGACY=1` is unchanged (40 / 24000).
  The 48000-character chunk fits the preflight limits (16 host calls per cell, 64 prompts and 2,000,000 bytes per batch call, 200,000 characters per prompt):
  2,000 records at 80 per chunk are 25 chunks, which the worker packs into batch calls by prompts and bytes; the first-request prefix is unchanged.

Evidence for 80 / 48000, dev slices only. First test: spam -0.15 and trec -0.22 points of accuracy, with a lower bound of exactly -1.00 on one
slice, so it did not pass the strict acceptance rule. Replication on 13 fresh windows: spam +0.19 (lower bound +0.00), trec +0.02 (lower bound -0.20);
run-to-run noise is about 0.3 points. Cost on those data: 49% fewer sub-calls, 7 to 14% fewer output tokens, 28% less wall time. The honest reading is
no measurable difference within about 1 point on tested data, not "no accuracy loss".

Limits of that evidence: short records only, 2 and 6 labels only, no longer-record data and no weak-model data. Workloads with more than 6 labels
(for example datasets of about 10 labels) run at 40 / 24000 and see no call reduction from this default. The rule must not be widened without new
evidence on data with more labels. Users with long records, many labels or small local models should set `FACTR_CLASSIFY_CHUNK_ITEMS=40`.

Tried and rejected: low reasoning effort (-3.5 points of accuracy), compact `codes` as the default (-0.40 points, more rejects, only -11% output tokens),
16-wide concurrency (measured on development data: accuracy -1.07 points on the topic task, wall time only -10%, and no 429 or 5xx in that test), omitting the reasoning summary on sub-calls (no effect), and one-call
wording in the REPL guidance (no effect).
