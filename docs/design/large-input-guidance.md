# Large-input guidance (v0.0.1)

General engine changes so the model looks at a large input before computing on it.

## What changed

- Environment line: each cwd entry carries its size, and text files a line count; files over 20,000 bytes (the
  threshold of the system prompt's large-input rule) are marked. Example: `records.txt (72 KB, 1237 lines, large input)`.
- System prompt: "inspect size and structure first, then process"; judgment work goes to the model per slice rather
  than to string matching; a property that is not literally in the data is inferred per record, not counted by name;
  a tiny result or a tie means the method failed; give a best estimate with a caveat rather than "cannot determine".
  The todo rule now says multi-step work only, after a first look at the inputs.
- REPL guidance (only when the REPL is available): judgment work via `llm_query`/`llm_query_batch` on focused
  slices, the 200000-character prompt limit, `repl` rather than `python3 -c` for data work.
- REPL tool description (sent once the deferred tool is loaded): persistence, `load(path, start=0, length=None)`
  (byte offsets), batch limits (64 prompts, 8 concurrent, 2000000 bytes), per-prompt limit, per-cell limits
  (16 host calls, 20 s compute, 8000-char output clip). A test asserts the numbers against the constants.
- `llm_query` / `llm_query_batch`: a prompt over 200000 characters is an explicit error (per item in a batch), no
  longer silently cut. Running out of the host-call time budget now returns an error to the cell (worker and
  variables survive); a batch earns 30 s extra per wave of 8 prompts.
- `read` with no range on a file over 20,000 bytes returns size, line count, head, tail and evenly spaced sample
  lines, with a pointer to ranges and, when available, the REPL.
- Stop gates: a todo item self-declared `verified` with no non-todo tool call in the turn gets one nudge (todo
  schema text says `verified` needs a tool-checked result); a text-only stop in a headless run with a large
  non-code file in the cwd and at most one analysis call gets one "what did you measure" nudge; the decline nudge
  recognises more "cannot determine" phrasings; the format nudge also recognises Title-case labels (`Result:`) when
  the task states the answer's form, and ignores prose openers such as `Note:`.

## Measured (scripted fake model, release build, request body of the first model call)

Before: pinned `factr-0.0.0-d3b499d`, run in place. After: this tree.

- System prompt: 2620 -> 3474 chars, of which the largest part is already-shipped REPL text now measured in; the
  tool list is identical (8909 chars of tool schemas, same names).
- Always-on prefix estimate (`agent_tests`, REPL available): 4371 -> 4497 tokens (+126: system 2215 -> 2341, tool
  schemas unchanged at 2156). The ceiling test measured value was updated to 4497 with its previous margin.
- Env line before: `files: notes.txt records.txt`. After: `files: notes.txt (5 B, 1 lines) records.txt (72 KB, 1237 lines, large input)`.
- `read records.txt` (72983 bytes, 1237 lines) returns a ~2 KB overview instead of the first 5000 lines.

## Whole-set judged counts (follow-up)

Observed failure classes: a model labelling records itself and counting by hand (errors grow with record count);
one-label-at-a-time prompts that invent their own definitions and under-count; sub-model replies with the wrong number
of labels used anyway; records or sub-model output retyped by hand; empty batch calls; answers that do not follow from the
model's own counts; a keyword method that finds one class accepted.

- REPL helper `await classify(items, labels, guidance=None, votes=1)` (`python_worker.py`): items are numbered and chunked
  (40 items or 24000 chars), the sub-model sees the whole label list and replies with a JSON object id -> label. A reply is
  accepted per id only when every id is in range, none repeats and the label is allowed after normalising case,
  punctuation and unambiguous whole-word fragments; a reply with unknown or repeated ids is rejected whole. Failing items
  are re-asked in smaller chunks, at most 2 retries, then a clear error (no guessed labels). Returns a list aligned with
  `items`. `votes>1`: two full passes with the label order rotated, then only items where they disagree get the
  remaining votes (at least one tie-break); majority wins, ties go to the first pass.
- Budget: all chunks of a wave go in one `llm_query_batch` call (64 prompts, 1.8 MB per call, so about 2500 items per
  call); a wave is one host call, the retries are at most two more. Typical use is 1 call of the cell's 16. The helper
  refuses to start a call when the cell budget is used up.
- `llm_query_batch([])` is an error telling the model to pass one prompt per record; a plain `llm_query` is unchanged.
- System prompt: judge-and-count over more than a few dozen records goes through sub-calls over the full label set even
  for a readable file; the sub-model sees only the prompt, so the records go in it. The "tiny result or tie means the
  method failed" rule is replaced by: print the table over every allowed label, check the sum and labels, take min/max/
  lookup in code; a zero or one-class split from a keyword/regex failed; a tie or small count can be real.
- Stop nudge: typographic apostrophes are normalised before decline matching; "can't/cannot calculate",
  "cannot be verified" and "can't verify" are give-ups.
- Cost: REPL tool description cap 190 -> 215 tokens (deferred tool); first-request prefix 4497 -> 4682 tokens (+185,
  system 2341 -> 2526); the prefix ceiling test was re-measured.
