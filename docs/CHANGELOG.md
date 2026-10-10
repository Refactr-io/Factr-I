# Changelog

## v0.0.5-dev.1 (unreleased; the release will be v0.0.5)

The 0.0.4 engine plus the sign-in, onboarding and UI work from pull request #2. The engine's labelling and classify
behaviour is unchanged from 0.0.4.

- Sign-in and onboarding: first-run sign-in works for every method in the desktop gateway's chat-only (Python-off)
  mode. User-initiated writes (model and API-key saves, endpoint validation, the recommended-default read after
  sign-in, voice, profile soul) start the Python runtime on demand instead of returning 404 `feature_not_requested`;
  boot probes and `POST /api/factr/update` stay gated. Onboarding saves the provider the user signed in with, never the
  first row of the model list, and errors show readable text instead of a raw "Error invoking remote method ... 404".
  Without `--provider` the engine now resolves to auto, so a fresh install without Ollama no longer lands on an API-key
  provider the backend does not know.
- Model picker and provider ids: `POST /api/model/set` rewrites engine provider ids to the runtime's ids and refuses,
  with a plain message, providers the runtime cannot save (listed as `settable: false`); Settings and the onboarding
  "Change" picker list only providers that can be saved. A default model saved from a chat is written under the
  runtime's id (`openai-codex` for the ChatGPT login, `openai-api` with only a key, `anthropic`, `muse`; `gemini-api`
  keeps its id) and the engine accepts these ids at boot. The onboarding confirm card shows the price for a pick saved
  under `openai-codex`.
- Packaging: `stage:backend-python` copies symlinks verbatim and fails the build if a bundled symlink is absolute or
  escapes the staged tree (the macOS app used to carry absolute links into the build folder and failed code signing).
- UI: the right sidebar opens on a Browser whose blank page lists Files, Terminal and Changes, each as a closeable tab
  in one tab strip; the sidebar toggle keeps the user's hidden-tab choice, and folding a side by dragging updates its
  toggle. Dragging a sidebar too narrow folds it without collapsing the other. The empty chat shows one short greeting
  for the time of day or the weekend instead of the large wordmark and tagline. Sidebar projects use a folder icon.
  Settings sub-page tabs hug their labels and share one width. The dark theme uses a lighter gray set for the window,
  menus, dialogs and Settings; the dithered and lattice-grid marks are replaced by a plain spinner.
- Tests: new gateway tests replay every onboarding flow in chat-only and feature mode; two intermittent test failures
  are fixed (an OpenAI endpoint test that read env vars without the test env lock, and a Radix focus timer that fired
  after jsdom teardown). A real sign-in against a live provider is not covered by tests.
- The Windows, Linux and macOS builds of the app are not signed (no Developer ID, notarization or Authenticode).

## v0.0.4 (2026-10-10)

Design and evidence: `docs/design/v004.md`; cost work: `docs/design/v004-cost.md`.

- Classify defaults (see `docs/design/v004-cost.md`, "Defaults and evidence"): reasoning effort inherits the user's setting; the format is a JSON
  object (compact `id:letter` codes are opt-in, `FACTR_CLASSIFY_FORMAT=codes`); dedupe and the cache are opt-in (`FACTR_CLASSIFY_DEDUPE=1`);
  chunk size, when `FACTR_CLASSIFY_CHUNK_ITEMS` / `FACTR_CLASSIFY_CHUNK_CHARS` are unset, is 80 items / 48000 characters for at most 6 distinct labels and a
  median record length of at most 600 characters, else 40 / 24000; classify log rows carry `chunk_source` (`default-80`, `default-40`, `user`).
  Evidence is dev slices of short records with 2 and 6 labels: first test spam -0.15, trec -0.22 points (lower bound exactly -1.00, not passing the strict
  rule); replication on 13 fresh windows spam +0.19 (lower bound +0.00), trec +0.02 (lower bound -0.20); run-to-run noise about 0.3 points; 49% fewer
  sub-calls, 7 to 14% fewer output tokens, 28% less wall time. No measurable difference within about 1 point on tested data. Workloads with more than
  6 labels (about 10 labels, say) stay at 40 / 24000 and see no call reduction; the rule must not be widened without new evidence on more-label data.
  Users with long records, many labels or small local models should set `FACTR_CLASSIFY_CHUNK_ITEMS=40`. Tried and rejected: low effort (-3.5 points),
  compact codes as default, 16-wide concurrency, omitting the reasoning summary, one-call wording in the guidance.
- Cheaper, faster long-context labelling (each piece has its own switch; `FACTR_COST_LEGACY=1` restores 0.0.3):
  `classify` sub-calls use the main agent's reasoning effort by default (`inherit`); a lower effort is an explicit, opt-in
  setting (`FACTR_REPL_SUB_EFFORT`, `agents.repl_sub_effort`: `none|low|medium|high|xhigh|inherit`; `minimal` is rejected), logged once per session when active,
  and `llm_query` keeps the main effort unless `FACTR_REPL_QUERY_EFFORT` is set. The classify log carries `requested_effort` (configured) and `effort` (used),
  and `effort_fallback: true` when the model refused the configured value. Measured on dev data, `low` cut output tokens per record 41% but lowered record accuracy
  94.9% -> 91.4% (trec_coarse entity recall 0.89 -> 0.70), so it is not recommended by default; `none` is not supported by gpt-6-luna;
  `classify` has an opt-in `id:letter` line format (quoted code table) and strict validation (no more "not spam" accepted as "spam") and still
  reads a JSON object; transport errors retry the same chunk with backoff and never halve the other chunks,
  validation failures re-ask only the failing ids; opt-in dedupe labels identical records once and a per-session cache
  makes repeats free (`FACTR_CLASSIFY_DEDUPE=1`); `votes=1` is the documented default; concurrency and chunk size
  are configurable (`FACTR_BATCH_CONCURRENCY`, `FACTR_CLASSIFY_CHUNK_ITEMS`, `FACTR_CLASSIFY_CHUNK_CHARS`); `FACTR_CLASSIFY_LOG=<path>` writes per-chunk tokens, latency and per-record hashes (no text); the
  environment line can show the head of the first large non-code file (`FACTR_ENV_HEAD=1`, off by default) and the REPL guidance
  asks for one classify cell. Environment-line heads are limited to regular, non-secret text data files and marked untrusted. First-request prefix 4727 -> 4791 tokens.
  Hardening after an adversarial pass: size and content-filter errors halve down to single records (a refusal of a
  whole wave stops), authentication / unsupported-setting errors stop at once, a transport error no longer spends the
  invalid-reply budget, a failed job names the unlabelled items and keeps the labels it got (votes=1: a second call asks
  only for the missing ones), backoff waits are bounded by the cell's remaining wait; `1. A` lines, symbol-only labels,
  numeric labels in JSON and single-letter labels are handled; one label needs no sub-call; lone surrogates no longer
  break the worker protocol. The log now writes the row types and names a record-level comparison reads (`type=call` per
  sub-call with its own timing and an engine-made `error` category, `type=occ` per occurrence with `h`, `occ`, `chunk`,
  `pos`), for the legacy path too. A free simulator (`factr-learn/tests/classify_sim.py`) runs `classify` over 2,000
  synthetic records against a scripted provider on a virtual clock (a model, not a measurement).
  Second hardening pass: an API refusal of the sub-call effort is remembered for the session (one refused request,
  not one per call; logged as its own row; cache and log use the effort really used); the host wait allowance no
  longer shrinks with higher concurrency; context-length refusals halve until they fit and never count as a systemic
  stop (only a whole wave of other refusals does); a content-filtered record is isolated alone; a fatal error stops
  the later batches of the wave; preflight counts the worst case of voting; rejection messages carry categories and
  ids only; chunks are balanced and long prompts go first. Log rows gain `session`, `cell`, `run_id`
  (`FACTR_RUN_ID`), the chunk settings, `prompt_chars` / `reply_chars`, `requested_effort`, per-batch timestamps,
  per-occurrence provenance without dedupe, and a `job` row; a failed job still logs every occurrence; an existing log
  file that is not mode 600 is refused. The environment-line file head is now opt-in (`FACTR_ENV_HEAD=1`) and is read
  through one descriptor (`O_NOFOLLOW`, `fstat`, at most 4 KB).

- Lighter session venv creation: no pip is installed into the venv. uv path: `uv venv --system-site-packages` (no
  `--seed`); fallback: `python -m venv --system-site-packages --without-pip`, with ensurepip seeding only when the base
  interpreter cannot import pip (one tiny `python -c` check, only in that branch). pip stays reachable through the
  system site packages (`<venv python> -m pip install`, and `pip`/`pip3` shims in the venv `bin`). Process-tree peak at
  engine start (macOS, median of 8): uv 70.9 -> 53.8 MiB, no-uv 141.7 -> 44.9 MiB; the child is visible for ~0.1 s
  instead of ~2 s (no more 8 s outliers seen: 2 of 8 before, 0 of 8 after); ready time unchanged (~0.14 s); the
  install-then-import scenario passes with and without uv (wall 4.3 s -> 1.8 s with uv).

## v0.0.3 (released 2026-10-08)

Design and evidence: `docs/design/v003.md`.

- REPL imports after an install: before every cell the worker drops the import finders' caches and adds a user-site or
  site-packages directory that appeared after it started (`pip install --user`). The sandbox may read the user site
  before it exists.
- One session environment: with no `FACTR_REPL_PYTHON`, the engine builds one writable virtualenv (system site packages
  included, pip seeded) on a background thread at start (the ready line is not delayed; the first command, REPL cell,
  environment line and hint wait for it, at most 15 s, then fall back to the system interpreter), puts it first on `PATH` and uses it for bash, the REPL and the environment line.
  `FACTR_REPL_PYTHON` is never replaced. `FACTR_GUARD_SESSION_VENV=0` disables it.
- Environment line: the tool list is computed after the session venv is ready, so it no longer says `installers: pip`
  together with `missing: pip`. A failed or timed-out venv build no longer mutates the process environment from a
  background thread; commands the engine starts drop the dead venv's `PATH` entry and variables per command, and the
  auto-verify gate and the `!` shell wait for the venv like `bash`.
- The session venv is removed at shutdown and exit; stale `factr-session-venv-<pid>` directories of dead engines are
  swept at start.
- The missing-module hint names exactly one install command for the REPL's own interpreter; a failed install with a
  structural cause (unwritable prefix, externally managed) gets a one-line pointer to that command.
- Windows and CI test fixes: the agentgrep, replace and debug-socket tests are platform-neutral, the REPL toolset check
  is skipped where the REPL sandbox is absent, the Windows cmd.exe shell tool description is shortened to fit the
  tool-schema token cap, and CI runs the engine tests on two threads with a timeout (the Windows leg skips two hanging groups).

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
