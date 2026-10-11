# Changelog

## v0.0.7 (unreleased)

- Bots: delivering a message to another bot (message_agent, @mentions, relay) in the installed app. Delivery ran
  `factr -p <bot> chat ...`, but the installed app has no `factr` command that can run `chat` (the bundled `factr` is the
  engine, which refuses it and reads `-p` as `--provider`), so every delivery failed. A packaged backend now runs the
  bundled Python (`<python> -m factr_backend.main -p <bot> chat ...`); the per-bot turn lock and the live-owner lookup
  recognise that command too. Source and venv installs keep `factr`. The single place that picks the command is
  `bot_cli_prefix` in `backend/tools/bot_relay.py`.

## v0.0.6 (2026-10-11)

- Voice transcription: dictation now works in the installed app. The first time you use it, Factr-I downloads the local
  speech engine (faster-whisper, about 200 MB) into your data folder (`lazy-packages`), not into the signed app. The
  install used to fail with "No virtual environment found" because the bundled runtime is not a virtualenv and the
  desktop gave the installer no target folder. The optional microphone-capture package (`sounddevice`) is no longer part
  of that install, since transcribing a recording does not need it. If no speech engine or provider key is available, the
  error now says what to do instead of a generic failure.
- Projects: a project you create in the sidebar (New project, then add a folder) now appears under Projects, with or
  without chats in it. The engine gateway built the sidebar tree from chats alone and answered `projects.list` with an
  empty list, so a project that was saved to `projects.db` was never shown. It now reads your projects from
  `projects.db` (read-only), lists each one with its folders, and files chats under the project whose folder (or repo)
  holds the chat's working folder, deepest folder first. Projects you already created show up with no migration.
- Projects: the sidebar lists only the projects you created. A git repo or a chat folder that is not one of your projects is
  no longer shown as a project (this also applies to the engine's `projects.tree`); its chats stay in Home.
- The right sidebar is closed the first time the app opens. If you have already opened or closed it, Factr-I keeps your choice.
- Spacing: paddings and gaps now follow a 4px scale (8/12/16) across the composer, Settings, shared controls, right sidebar and overlays; the Settings nav is 14rem wide.
- Icons: all line icons use one lighter 1.5 stroke weight with round caps.
- CI: the gateway slash popup test now takes the shared environment lock (a test-only race).

## v0.0.5 (2026-10-11)

Sign-in, onboarding, provider and credential handling, approvals, observability and desktop UI fixes on top of 0.0.4.
The engine's labelling and classify behaviour is unchanged from 0.0.4; the engine changes are the gateway, start-up,
provider-resolution and config items listed below.

- Engine restart mid-reply: when the engine process dies while a reply streams, the reply no longer spins forever. As
  soon as the desktop learns the engine died (or, failing that, once the restarted engine is reached), the turn ends
  with "The connection to the engine was interrupted. Your message was not completed." and a Retry button, the turn
  clock stops, and messages queued behind it wait under Resume.
- Uninstall: About > Uninstall works in the packaged app (it runs the bundled interpreter with the bundled backend
  instead of requiring `~/.factr/factr-backend/venv`) and no longer prints `SyntaxWarning: invalid escape sequence
  '\c'`. The summary, the confirmation and the deletion use one list of targets: the running app (refused, with an
  explanation, when translocated, on a disk image or in a read-only folder), this copy's real app-data folder, and for
  "everything" the config folder it runs with. The confirmation lists every path. Nothing at a standard location is
  deleted unless it is the running copy, and no target may be a link, a relative path, or a system or home folder. An
  alert is raised if removing the app fails after quit.
- A config.yaml that cannot be read is no longer ignored silently: onboarding names the file and the line, and offers
  Open the folder and Back up and start fresh (a copy is saved as `config.yaml.corrupt-<time>` before anything changes).
- The start screen behind a failed start shows the plain cause; the raw error text is only under Details.
- Approval mode: the engine refuses `smart` and defaults to `manual`, so the app defaults to and shows Manual
  everywhere (new chat, existing chat, after Settings), offers only Manual and Off, and never shows Smart (a stale
  `smart` in config reads as Manual). Opening and closing Settings no longer flips the label from Smart to Manual: an
  empty reply for `approvals.mode` (the key is not in config.yaml) was read as Manual, and an empty reply to an
  approval-mode write no longer flips the chip either. Settings > Safety explains that Manual also asks before
  deleting files in a home folder or other broad location, and shows the current working folder.
- Chat errors: an API-key 401 from the engine ("rejected the access token and no refresh token is available; run
  `factr login`...") now reads "The provider rejected the key or login. Check or replace it in Settings > Providers"
  instead of the generic card; the raw text stays under Details. A cut-off or malformed stream reads "The reply was
  cut off. Retry to continue." A final rate-limit or server error uses final wording ("Rate limited. Try again in a
  moment.") instead of "retrying". Provider failures read as a plain message in the error card and toast (no provider
  connected, rejected key, missing model, rate limit, server error), with the raw text as detail.
- Chat toasts: error toasts no longer pile up (one per session, at most three), routine provider transport progress
  ("opening websocket", retries, HTTPS fallback, cooldown notices) shows as a brief info toast instead of a sticky
  warning (real failures still warn), and "Waking up <profile>..." clears on the first token or error and no longer
  stays in the page.
- Chat: "Always allow" now says it allows this exact command (and shows it); agentgrep and ls cards read "Searched X
  in folder" / "Listed folder"; stopping a reply leaves a quiet "Stopped" line and keeps the partial text.
- Settings > Model: the main-model context-window override accepts only whole numbers from 0 to 10,000,000 (0 = detect),
  shows an inline error otherwise, and an autosave error no longer lingers when you switch Model tabs.
- Mixture of Agents: preset names are limited to 40 letters, digits, spaces, dots, dashes and underscores, and providers
  without credentials are disabled in the slot pickers with a hint.
- Providers and Tools & Keys: a saved API key shows a Remove button (with confirmation) without having to click into
  the field first. Tools & Keys shows "Home Channel" style names instead of raw upper-case env names.
- Command Center > System no longer shows "Active sessions undefined" when the status reply omits the count.
- Esc closes the "Set up provider" overlay; the app menu has Settings…; the browser address field shows a focus ring;
  each right-pane tab's close button is named "Close <tab title>"; About no longer says "just now" twice.
- Run history and Usage: a tool turn counts every model request. A server that reports usage only on the final
  chunk left the tool-call request uncounted ("Model calls 1"); it is now a model call with unknown usage, so the
  run reads 2 calls and unpriced instead of looking cheaper. Usage API calls sum the same rows. Subagent rows close
  from the session journal (they could stay "spawned"). Gateway ledger only; agent loop and providers unchanged.
- Alerts: `silent_failures` no longer fires after normal chats. Background runs (memory extraction, learning, undo)
  are not model turns; a chat or scheduled turn that completes with no model call still alerts.
- Approvals: with the working folder at home (the desktop default), `/`, `/Users`, `/home`, `~/Documents`,
  `~/Desktop`, `~/Downloads` or a `/Volumes` root, a plain delete (`rm "$HOME/x"`, `unlink`, `shred`, `> file`) now
  asks ("delete inside a broad working folder"); it ran without a prompt. An "Always allow" is bound to the folder it
  was given in (older unbound grants no longer apply in a broad folder). Project folders and `approvals.mode: off`
  are unchanged. Gateway approvals only; the bash tool, classifier and agent loop are unchanged.
- `/usage` answers from the run ledger (today's and this session's runs, model calls, tokens, cost or unpriced)
  instead of "not a command this engine serves".
- Local / custom endpoint: the onboarding's "Local / custom endpoint" (and a hand-written `provider: custom` config with
  `base_url`) now works. The readiness check judged the route by the catalog defaults and reported "No usable
  credentials found for openai-compatible." although the probe and chats worked. An endpoint on this machine or the
  local network (loopback, private ranges, `*.local`) needs no key; a remote one takes its key from `model.key_env`,
  else `model.api_key` (served from memory, never logged or exported to tools), else `OPENAI_COMPAT_API_KEY` in `.env`.
  When it is not ready the message says what is missing (no endpoint saved, or the URL needs a key).
- OpenAI API-key onboarding: saving the model no longer fails with "Could not save OpenAI". An OpenAI pick is saved as
  `openai-api` when only an API key exists and stays `openai-codex` for a ChatGPT sign-in (even with a key present);
  it used to be `openai-codex` always, which the runtime refused for a key-only user. A failed save now shows the
  first line of the server's reason.
- Desktop: "No usable credentials found for <X>" reads "No AI provider is connected yet ..." when nothing is saved and
  "Your saved provider <X> has no usable credentials ..." when one is; the raw text stays under Details.
  `setup.status` reports `saved_provider` for this. Benchmark starts (`--provider openai --model ...`) are unchanged.
- Desktop: when a provider is saved but the engine cannot start with it, the card and toast no longer say "No AI
  provider is connected yet". A saved model the provider rejects ("Unsupported OpenAI model ...") reads "The provider
  does not have this model", and "credentials not available" with a provider saved reads "Your saved provider could
  not be used" (Open Providers and model-picker buttons); "No AI provider is connected yet" stays for a first run with
  nothing saved. Display only; the raw text stays under Details.
- Desktop: a "Run history" entry in the left rail (advanced mode) and on the Command center Usage tab opens the agent run history page (runs, tokens and cost).
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
  The bundled Python is precompiled at staging (`compileall`, unchecked-hash pycs) and every spawn of it runs with
  `PYTHONDONTWRITEBYTECODE=1`, so the app no longer writes ~3,000 `__pycache__` files into its own signed bundle;
  packaging fails if the staged tree changed after staging, and `npm run verify:app <Factr-I.app>` checks an import
  smoke plus `codesign --verify --deep --strict`. The staged backend leaves out `backend/tests/` (~50 MB), staged
  files are made user-writable so `xattr -cr` and re-signing work, and release engine builds pass
  `--remap-path-prefix` so the binary no longer embeds the builder's home, Cargo and build-directory paths.
- UI: the right sidebar opens on a Browser whose blank page lists Files, Terminal and Changes, each as a closeable tab
  in one tab strip; the sidebar toggle keeps the user's hidden-tab choice, and folding a side by dragging updates its
  toggle. Dragging a sidebar too narrow folds it without collapsing the other. The empty chat shows one short greeting
  for the time of day or the weekend instead of the large wordmark and tagline. Sidebar projects use a folder icon.
  Settings sub-page tabs hug their labels and share one width. The dark theme uses a lighter gray set for the window,
  menus, dialogs and Settings; the dithered and lattice-grid marks are replaced by a plain spinner.
- Tests: new gateway tests replay every onboarding flow in chat-only and feature mode; two intermittent test failures
  are fixed (an OpenAI endpoint test that read env vars without the test env lock, and a Radix focus timer that fired
  after jsdom teardown). A real sign-in against a live provider is not covered by tests.
- Docs and release notes: first open on macOS 15+ is System Settings > Privacy & Security > Open Anyway (right-click
  Open no longer works), macOS builds are Apple Silicon only, the version and signing statements are current, and the
  Ubuntu 24.04 AppImage needs FUSE 2 and `--no-sandbox`.
- CI and config: the release macOS app is ad-hoc signed (`identity: -`) and verified, so it is no longer reported as
  "damaged"; Windows builds NSIS only (MSI rejects prerelease versions) with git long paths on; the update check backs
  off for 15 minutes after a failure and honours `Retry-After` / `x-ratelimit-reset`.
- Desktop app: `auth.adopt_external_logins` defaults to false in the packaged app's bundled defaults, so it no longer
  borrows (and by refreshing, logs out) the Claude Code or Codex CLI login; a `config.yaml` setting still wins, and the
  CLI default is unchanged.
- Provider startup (desktop): the app passes `--provider` / `--model` to the engine only when `FACTR_PROVIDER` /
  `FACTR_MODEL` are set, and never a `FACTR_PROVIDER` the engine's `--provider` flag rejects (the boot card showed
  "invalid variant"; the engine reads the variable itself and explains an unsupported value in the app). It no longer
  probes local Ollama and starts the engine on it (which sent a saved ChatGPT pick to Ollama's endpoint), and the
  quit-time Ollama unload is gone (the engine unloads its own model). Onboarding finishes only with the provider the
  user connected saved, else it shows an error.
- Provider startup (engine): a saved `model.provider` or `FACTR_PROVIDER` the engine cannot serve no longer exits
  `serve`: it is resolved through the login catalog (ids, aliases, display names), else the engine starts with no
  provider and `setup.status`, `setup.runtime_check` and `/api/status` (`provider_error`) say why; it never serves
  another provider instead. With nothing configured or signed in, a local Ollama that lists models is the auto
  chain's last step (honouring `OLLAMA_HOST`); `--provider ollama` without `--model` picks an installed model.
- Gateway: a forwarded request whose feature runtime cannot start gets a 503 JSON reply with a bounded, redacted
  stderr tail instead of a closed socket; a saved custom endpoint's `model.base_url` is applied at boot and on new
  chats when config.yaml chose the provider; a ChatGPT pick with no valid ChatGPT login and an OpenAI API key present
  is refused with a clear message instead of billing the key; `model.options` and `/api/model/options` re-read the
  login after a sign-in; `/api/status` reports `missing_login` on a fresh install. Each profile backend gets its own
  `FACTR_RUNTIME_DIR`, so a second profile no longer fails with "Another factr server process is already running".
- Skills and browser: a fresh install seeds the bundled skills once (`factr skills seed`) when the skill store is
  empty and the Python backend is configured, from the skills list and from a new desktop chat, which waits up to 3 s
  so its first turn sees them (a headless `POST /api/agent/run` never seeds or waits); the browser tool's `POST
  /api/browser/act` may start the feature runtime instead of failing with 404.
- Release hardening: the release app removes Chromium's `--remote-debugging-port`, `--remote-debugging-pipe` and
  related switches at startup (also when `desktop.electron_flags` in config.yaml asks for them), so launching the app
  with them no longer opens a DevTools port; the fuses did not cover them. A QA build (`FACTR_QA_FUSES=1`) keeps them.
  The variant is baked into the bundle at build time and the packaging step fails if the bundle and the fuses
  disagree.
- Custom endpoint key: saving a keyed custom endpoint from the app (onboarding and the model picker) now writes the key
  to `.env` (same variable the `factr model` setup uses, `FACTR_CUSTOM_<HOST>_<PORT>_API_KEY`) and config.yaml holds
  only a `${VAR}` pointer (`model.api_key`, and `key_env` on the `custom_providers` row) instead of the key itself. If
  `.env` cannot take the value, the key stays inline as before; existing configs with an inline key keep working
  unchanged and are not migrated. The engine already reads the pointer form; its behaviour is unchanged. `.env` is
  still a plain 0600 file, so this separates the secret from config.yaml but does not encrypt it.
- Benchmark isolation: none of the above applies to an explicit `--provider openai --model gpt-6-luna serve` start;
  in-repo tests pin its provider, model, auth state and status to 0.0.4's across config.yaml and environment variants.
- The Windows and Linux builds of the app are not signed, and the macOS build is ad-hoc signed only (no Developer ID,
  notarization or Authenticode).

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
