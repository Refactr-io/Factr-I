# Changelog

## v0.0.2 (released 2026-10-06)

Design and rationale: `docs/design/v002.md`. Every guard below has a switch `FACTR_GUARD_<NAME>=0` (default on).

- Answer templates: the label detector accepts `word = <x>` and `word: <x>` in any case, takes the first template after
  a formatting verb on word boundaries and ignores data brackets; a stub never replaces an earlier reply ending in a
  template line; decline nudges ask for the best answer in the requested form again. `TEMPLATE`.
- Final text: a turn's answer is one whole reply and is never concatenated, trimmed or appended to. With a task label
  (a literal line the task asks for) it is the latest reply that carries that label line; otherwise the last reply,
  falling back to an earlier nudged reply only when the last is empty or a bare stub (40 characters or fewer that is
  not a number, a shortfall or a question). One `StopNudge::final_text` serves the turn loops. `FINAL_TEXT`.
- Verify tracker fix: an edit made with an edit tool also changes the cwd mtime, so the snapshot diff counted it as a
  shell write at the next bash run and the verify nudge fired on nearly every edit-then-run task. Paths written by edit
  tools are now excluded from the snapshot diff, the snapshot is refreshed after each bash/REPL run (a shell write gets its
  own sequence number), and an exec counts only when strictly after the write. `VERIFY`.
- Stale-process note: a successful edit/write/apply_patch/replace result gets "bg task <id> started before this edit still
  runs the old code; cancel and restart it, then bg wait until=<ready line>." for each bash background task of the session
  still running (once per task id per turn). `STALE_BG`.
- Service readiness recheck: at a headless text-only stop, a background task started this run that looked like a server
  (`bg` wait with `until=`, or a ready/listening line in its output; or ran over 5 s and failed) and is no longer running
  gets one nudge naming it and its exit status. Tasks the model cancelled, or whose end it already read, are dropped.
  `SERVICE`.
- Text switches (read once per process): `ENV_INSTALLERS` (the `installers` field in the environment line) and
  `PROMPT_STDLIB` (the missing-library prompt rule) are ON by default; `=0` turns each off. Reason: measured benefit on
  held-out data, only as a proxy; no tool-use change under the no-tools line (the environment block is skipped and the
  rule sits below the no-tools override). `BG_DESC=1` (newer `run_in_background` description plus the server hint in the
  background-start result) stays opt-in, default off. `PROMPT_BEST=0` restores the v0.0.1 best-answer line (default on).
  The timeout fix is unswitched. Default prefix 4727 tokens (+47 vs 4680, +45 vs v0.0.1).
- Tasks that forbid tools or code: one shared detector (`factr_base::task_policy::forbids_tools`, word-bounded, handles
  `don't`/`dont`/curly apostrophes, "no tool calls", "without running code", "tools are not allowed", by hand, in your
  head, with exclusions such as "no code changes" and "no tools/ directory") now gates every guard. With it, no verify,
  failed-check, pending, service, unverified, measure, action, snippet, unread or compute nudge is queued, the question
  nudge asks for the answer without tools, the 70% deadline reminder does not say verify, the environment block is
  omitted, and the system prompt states that the user's no-tools instruction overrides every rule. `NOTOOLS=0` disables
  the new gating and restores the old phrase list.
- Verify check: a headless turn that wrote any deliverable (an edit tool on any path, or a file changed in the cwd by
  bash or the REPL, found by an mtime snapshot) and then ran or read back nothing gets one nudge to run or read it back
  and check it against each stated requirement. A run whose output says no tests ran does not count. `VERIFY`.
- Shortfall detector (replaces decline phrase matching): a negation or shortfall word and an object word in one sentence
  of the last paragraph, before half of the time budget, once per turn; wording follows whether files were written,
  whether tools are forbidden and how many routes were tried. The unreadable-input exemption is kept. `SHORTFALL`.
- Compute-before-final guard: a bare number as the answer with no bash or REPL run since the data was read (or no tool
  at all) gets one nudge to recompute it in code. `read` adds one footer line (rows x columns, absolute path) for
  xlsx/xlsm/csv/tsv/json. `COMPUTE`, `READ_FOOTER`.
- Unread-remainder guard: a webfetch whose output was cut (its own window note) or a large-file overview that is never
  searched, paged or read in full gets one nudge naming the characters seen. `UNREAD`.
- webfetch: after three attempts on a 5xx or connection error the archive fallback is tried (when enabled) and the error
  says to get the fact from another page or site; websearch backend errors suggest one retry with a shorter query.
  `FETCH_HINT`.
- bash: `run_in_background` ignores the foreground `timeout` (a background server is no longer killed at the cap); the
  start message points to `bg` wait with `until=<ready regex>`.
- Ported from the round-3 work: one shell resolver (bash on PATH, else /bin/sh); environment line states shell, REPL and
  installers; missing-library prompt rule; actionable missing-module, NameError and compute-interrupt errors; promoted or
  background checks count as pending, with one "still running, `bg` wait" nudge (`PENDING`); skipped-test follow-up
  (`SKIPPED`); no-runner fallback text; more give-up phrases, curly apostrophes; the second nudge needs non-todo progress;
  `loop.guard reason=turn_end` with `nudges_sent`, `used_tools`, `final_len`, `stop_reason`, `answer_changed` and
  `answer_shape` (observation only, `ANSWER_SHAPE`).
- No nudge text contains hedging or answer-shape wording (a test covers every nudge); the format nudge for a task label has
  its own once-per-turn allowance outside the two-nudge cap.
- System prompt: "give your best answer in the requested form" (no caveat wording), no todo push, and one line against
  searching for or using published answers to the given task. The `factr` bridge tool is registered only when a host with
  a feature backend is installed (`BRIDGE`). First-request prefix 4682 -> 4702 tokens (+20).

## v0.0.1 (unreleased)

- websearch: the model-visible schema no longer has `engine` or `bing_market`; the operator picks the backend via
  `websearch.engine` / `FACTR_WEBSEARCH_ENGINE`. Stale arguments are accepted, ignored, and noted in the result.
- websearch: pinned SearXNG mode (`engine: searxng` plus a URL ignores fallbacks and key backends);
  `websearch.last_resort_wikipedia` / `FACTR_WEBSEARCH_LAST_RESORT_WIKIPEDIA`.
- webfetch: `webfetch.allowed_hosts` / `FACTR_WEBFETCH_ALLOWED_HOSTS` (redirect hops re-checked; `host:port` entries match
  the port too) and `webfetch.wayback_fallback` / `FACTR_WEBFETCH_WAYBACK`. See `docs/design/web-tools.md`.
- REPL: `spawn_subagent` now follows the run's tool policy (needs `delegate` allowed); `llm_query*` labelled as plain model calls.
- OpenAI Responses (ChatGPT mode): the hosted `image_generation` tool follows the run's tool policy; default unchanged
  when no policy is set. `parallel_tool_calls: false` is unchanged and documented.
- Large inputs: the environment line lists file sizes and line counts with a `large input` marker; the system prompt
  says to inspect size and structure first, infer non-literal properties per record, and give a best estimate rather
  than "cannot determine"; todo is for multi-step work after a first look at inputs. See `docs/design/large-input-guidance.md`.
- REPL: the tool description states the real limits; `llm_query` prompts over 200000 characters are an error (was a
  silent cut); host-call time exhaustion returns an error to the cell and keeps the worker; batches earn extra wait.
- read: a plain read of a file over ~20 KB returns an overview (size, lines, head, tail, samples) with a pointer to ranges.
- Stop nudges: self-declared `verified` todos with no inspecting tool call, a large-input answer after at most one
  analysis call, more "cannot determine" endings, and Title-case required answer labels.
- REPL: `await classify(items, labels, guidance=None, votes=1)` returns one label per item from the full label set (numbered
  chunks, JSON reply validated for every id exactly once and allowed labels, failing chunks re-asked up to twice, `votes>1`
  re-asks only disagreements); `llm_query_batch([])` is an explicit error. The REPL tool description cap is now 215 tokens.
- System prompt: whole-set judged counts are labelled per record over the full label set and counted in code; before
  answering from counts, print the table over every allowed label and check it sums to the record count. First-request
  prefix +185 tokens (4497 -> 4682).
- Stop nudges: typographic apostrophes match like plain ones; "can't/cannot calculate" and "cannot be verified / can't verify"
  count as give-ups.
