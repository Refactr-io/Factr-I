# Running Factr-I in a benchmark: the exact headless configuration

This is the one recommended configuration for a headless benchmark run. Anything not listed here is left at its default. The engine never spends the ChatGPT/Codex refresh token. The Factr Python backend is the only refresher, and the engine can only call it when it finds it (it does not, under the per-task `HOME` below), so the per-task home is read-only with respect to credentials.

## 1. The binary

Build the release binary (`cargo build --release --locked -p factr-cli`, see [BUILDING.md](BUILDING.md)) and copy `engine/target/release/factr` to a versioned name, for example `factr-<short-sha>`. Record in the run's `versions.txt`: the sha256 of the copy, the commit, and the output of `factr-<sha> __version` (one JSON line: `{"db_schema":..,"sha":"<build sha>","version":".."}`). Never reuse a home across engine versions (the database schema is not readable by older builds).

## 2. Per-task home layout

One fresh set of directories per task. Nothing is shared between tasks.

```
$TASK/home/      HOME
$TASK/engine/    FACTR_HOME          (engine state: sessions db, token file; starts empty)
$TASK/config/    FACTR_CONFIG_HOME
    auth.json    copy of the master login (see section 5), mode 0600 then 0400
    config.yaml  approvals: {mode: off}   checkpoints: {enabled: false}
```

`FACTR_HOME` defaults to `~/.factr/engine` and `FACTR_CONFIG_HOME` to `~/.factr`; both are set explicitly so nothing under the real home is read or written. `FACTR_CONFIG_HOME/auth.json` is the only place the engine looks for the ChatGPT/Codex login.

`config.yaml`:

```yaml
approvals:
  mode: off
checkpoints:
  enabled: false
```

Do not create `SOUL.md`, `skills/`, `.env` (the engine reads `FACTR_CONFIG_HOME/.env` first for every API key it defines), `active_profile`, or any memory or cron files. The Factr backend seeds `SOUL.md`, `skills/` and other directories into whatever `FACTR_CONFIG_HOME` it runs against, so run the login commands (section 5) only against the master directory and copy just `auth.json` and `config.yaml` into a task. The copied `auth.json` carries the refresh token, so the master login must be a Factr-I-only account: the task's tools can read the copy.

`approvals.mode: off` (unquoted `off` is read correctly) makes unattended runs run approval-gated commands. Without it the same run gets `headless-deny` on such commands (checked live). Catastrophic commands are still blocked. `cron_mode`/`unattended_mode: approve` and `command_allowlist` are the other approval keys the engine honours; leave them out. `checkpoints.enabled: false` is written explicitly so the setting is visible in the run; headless runs already default to off, an explicit `true` in `config.yaml` would turn it on. After a run, `undo/<session>.json` in `FACTR_HOME` shows `"checkpoint":null`.

With `HOME` pointing at the empty task directory the engine finds no Factr Python backend (it looks for `$HOME/.factr/factr-backend/venv/bin/factr`, `FACTR_BACKEND_PYTHON` and `FACTR_BACKEND_CMD`). That is what keeps the cron ticker, the bot and feature processes and the Codex refresh command off for a task. Do not put a backend where a task can reach it.

## 3. Environment

Start from the runner's environment and strip first:

- every variable starting with `FACTR_`
- every variable ending in `API_KEY`, `TOKEN` or `SECRET`
- `FIRECRAWL_API_URL` and `TAVILY_BASE_URL` (a Firecrawl URL alone selects the Firecrawl web backend, and these two match neither pattern above)

Then set exactly the following. Accepted values are exact: the engine compares the string, so write them as shown.

| Variable | Value | Why, and what the engine does with it |
|---|---|---|
| `HOME` | the per-task `home` directory | isolation; the engine, its tools and the Factr backend lookup all resolve `~` here |
| `FACTR_HOME` | the per-task engine directory | sessions db, spans, token file (engine, `factr-storage`) |
| `FACTR_CONFIG_HOME` | the per-task config directory | `config.yaml`, `auth.json`, `.env` (engine, `factr-base` config) |
| `FACTR_MEMORY_ENABLED` | `0` | no automatic memory extraction or recall. Accepts `0/false/no/off` to disable, `1/true/yes/on` to enable; default on. Read by `factr-base` (`memory_extract`, `config`). Live: with `0` no `memory.*` span and no extra model call; unset, a `memory.recall` span per turn and a `memory.extract` model call at session end |
| `FACTR_LEARNING_ENABLED` | `0` | no learning reviews. Only the exact string `0` disables; default on. Read by `factr-learn`. A `/api/agent/run` session never auto-reviews anyway (`learning.skip` span, reason `headless`, or `disabled` with this variable). A review the model asks for explicitly (the REPL `refine` function) still runs; not exercised live |
| `FACTR_HEADLESS` | `1` | only the exact string `1`. `/api/agent/run` already marks its own session headless (no persona block unless a `SOUL.md` exists, checkpoints off, unattended approval policy, environment snapshot, auto-verify gate). This variable extends it to the whole process: sessions the engine creates itself (sub-agents), the persona of sessions created over RPC, the checkpoint default. Read by `factr-base`, `factr-gateway`, `factr-app-core` |
| `FACTR_TURN_DEADLINE_S` | the task's time limit in seconds | a positive number (fractions allowed), counted from the start of each turn; `FACTR_HARD_DEADLINE_UNIX` (epoch seconds) is the alternative. One reminder at 70% and one at 90% is injected before the next model request (`loop.guard` span, reason `deadline_70`/`deadline_90`; checked live). Read by `factr-app-core` |
| `FACTR_AUTO_VERIFY` | `0` for the headline row; unset for the labelled gate-on row | `0` forces the gate off, `1` forces it on, anything else or unset means on for headless runs. The gate runs the project's detected test command (`cargo test`, `go test ./...`, `npm test` when `package.json` has a `test` script, gradle, mvn, cmake, python `unittest`/`pytest`) when a turn ends after code edits that were not followed by a passing test run; up to 3 rounds of 120 s, failures are fed back. No detectable test command: skipped (`auto_verify_skip_no_marker`). Section 7 |
| `FACTR_WAIT_FOR_USAGE_MAX_S` | `0` | longest a turn parks on a usage-limit reply with a known reset time. Seconds; `0` turns parking off; unset parks up to 7200 s, at most 5 times per turn. Read by `factr-app-core` |
| `FACTR_OPENAI_REASONING_EFFORT` | the effort the other arms use (arm-matched) | `none`, `minimal`, `low`, `medium`, `high`, `xhigh` or `max` (anything else is ignored with a warning, an empty value too). Unset is `low`. Recorded on each `Model call` span as `gen_ai.request.reasoning_effort` and sent as `reasoning.effort` (checked live on the Responses path) |
| `FACTR_OPENAI_SERVICE_TIER` | `default` | `default`, `auto`, `none`, `off` and `standard` send no `service_tier`; `priority` and `fast` send `priority`; `flex` sends `flex`. Unset is `priority` (checked live: unset sends the field, `default` omits it) |

Never set `FACTR_BACKEND_PYTHON`, `FACTR_BACKEND_CMD`, `FACTR_REPL_PYTHON`, `FACTR_DEPLOYMENT`, `FACTR_PROVIDER`, `FACTR_MODEL` or `FACTR_WEB_BACKEND` in a benchmark run.

- `FACTR_BACKEND_PYTHON` and `FACTR_BACKEND_CMD` make the Factr backend discoverable: the engine would then refresh the login itself, start the cron ticker and bots, and seed `skills/` and `SOUL.md` into the config directory.
- `FACTR_DEPLOYMENT=private` switches every login off (keys from the environment only), so the ChatGPT/Codex login stops working.
- `FACTR_PROVIDER` and `FACTR_MODEL` are only launch-time defaults for `--provider` and `--model`; use the flags.
- `FACTR_REPL_PYTHON` names the REPL's interpreter (default: the first `python3` on `PATH`).
- `FACTR_WEB_BACKEND` picks the web search backend; with none set and the key variables stripped, none is configured.

Optional, leave unset unless the row is labelled:

- `FACTR_VERIFY_ON_STOP`: only `0` disables. On by default. It covers every stop nudge (verify after edits, failed check, text-only "I will", question or offer in a headless run, snippet-only answer, decline, missing output label), at most two per turn, each recorded as a `loop.guard` span. It is independent of the auto-verify gate (checked live: the gate still ran with `FACTR_VERIFY_ON_STOP=0`).
- `FACTR_ENV_SNAPSHOT`: `0` disables the one-line environment block (cwd, tools on `PATH`, top-level files) appended to a headless session's first user message; `1` forces it on for non-headless sessions. It is skipped when the cwd is missing, `/` or the home directory.
- `FACTR_BEDROCK_MAX_TOKENS`: Bedrock only, a positive integer output cap (default 32000).
- `FACTR_SWARM_ENABLED=0`, `FACTR_REPL=0`: turn off the sub-agent (swarm) tools and the REPL tool; both are on by default, so label them if left on.
- `FACTR_RESUME_GOALS`: leave unset; a goal or loop left active across an engine restart is paused unless it is `1`.

## 4. Launch and run

Launch the engine with explicit provider and model flags (`--provider openai` selects the ChatGPT/Codex login; the other flags are `--host`, `--port`, `--token-stdin`, `--allow-remote`):

```
factr-<sha> --provider openai --model gpt-6-luna serve --host 127.0.0.1 --port 0
```

- The port is on stdout: the exact line `FACTR_BACKEND_READY port=<n>` (stdout is flushed after it). The ready line is printed even when the provider is not ready.
- The bearer token is the 0600 file `$FACTR_HOME/factr-gateway.token` (64 characters; stderr says `factr: token written to <path>`). The runner can instead supply its own, at least 32 characters (shorter: the engine refuses to start): preferably on stdin with `--token-stdin` (first line), so it is in neither the environment nor argv, or through `FACTR_DASHBOARD_SESSION_TOKEN`. Both are removed from the engine's own environment at startup, and no token file is written. If you use the environment variable, it is an exception to the strip rules above and to the audit in section 8.
- Authenticate with the header `x-factr-session-token: <token>` (checked first) or `Authorization: Bearer <token>`. A missing or wrong token gets `401`. `GET /api/status` and `GET /api/health` need no token.
- Pre-flight, before the first task: `GET /api/status` has a `missing_login` field, but do not rely on it. The engine only records it when it builds the provider after finding a login it then cannot load, which the startup check rules out; it stayed `null` in the live check with no login at all and with an expired one. Check instead (a) stderr for `factr: model provider not ready (...)`, and (b) `GET /api/model/options` (with the token): the entry in `providers` with `is_current: true` must have `authenticated: true`. Live, with no or an expired login it was `false` and `/api/agent/run` answered `503` with the error `OpenAI credentials not available...`. The server starts even when the provider is not ready.
- Run each task with `POST /api/agent/run` and a JSON body. `prompt` is required (a missing or blank prompt gets `400`). `cwd` is the working directory (default: the engine's start directory). `timeout_s` defaults to 600 when absent and is clamped to 1..3600, so always send it. Optional keys: `title`, `session_key`, `surface` (`bot` or anything else means `cron`), `instructions`, `model`, `provider`, `enabled_toolsets`, `disabled_toolsets`, `run_id`. The body may be up to 4 MiB.
- The reply is JSON: `ok`, `text`, `error`, `interrupted`, `session_id`, `usage` (`input_tokens`, `output_tokens`, `cached_tokens`). A turn that exceeds `timeout_s` is interrupted and answered with HTTP 200 and `ok: false`, `error: "timed out waiting for the turn to finish"`. A turn that cannot start (provider not ready) is HTTP 503 with `ok: false`. A foreground command that hits its timeout is promoted to a background task; background tasks of a `cron` run are cancelled when its reply is sent.
- As soon as the reply arrives, send SIGTERM to the engine's process group. Goal and heartbeat drivers keep running after a reply otherwise. Checked live: the group exits with code 0 within a second, and a `sleep` the model left running is gone.

## 5. The ChatGPT/Codex login

One master login serves the whole window. The engine never spends its refresh token.

The commands below are the engine's wrappers around the Factr Python backend (`factr login openai` is `auth add openai-codex`; `openai`, `chatgpt` and `codex` are accepted as aliases for `openai-codex`). They need that backend. Run them with `HOME` and `FACTR_HOME` of your choice, but point them at the master directory:

- the managed install at `$HOME/.factr/factr-backend/venv/bin/factr` is found on its own; otherwise
- use the desktop bundle's runtime, as the desktop does: `FACTR_BACKEND_PYTHON=<Factr-I.app>/Contents/Resources/backend-python/runtime/bin/python3 FACTR_BACKEND_PYTHONPATH=<...>/backend-python/source:<...>/backend-python/packages`.

Never point `FACTR_BACKEND_CMD` at the engine binary or at the bundle's `Contents/Resources/factr/factr` (that is the engine): the engine then runs itself as its own backend and forks without end (seen live; a fork bomb). Without any backend the commands print `the bundled Factr backend was not found` and exit 1.

1. Create the master once, in a directory only the runner uses: `FACTR_CONFIG_HOME=<master-dir> factr login openai` (a Factr-I-only account; the flow is interactive, `--no-browser` and `--browser` select the sign-in method). The login lands in `<master-dir>/auth.json` under `credential_pool."openai-codex"`.
2. Before each benchmark window, refresh it: `FACTR_CONFIG_HOME=<master-dir> factr auth refresh openai-codex` (with a single stored credential no target is needed; with several, pass the index, id or label). `factr auth` lists the connected providers by name only.
3. Read the access token's expiry and end the window 15 minutes before it. The engine takes the expiry from `expires_at_ms`, else `expires_at`, else the access token's JWT `exp` claim; read the same field (decode the payload only; do not print the token). A task that starts with less than 15 minutes left must not start.
4. For each task, copy `auth.json` from the master into the task's `FACTR_CONFIG_HOME`, mode 0600 then 0400 (the engine reads a 0400 file; the entry needs a non-empty `refresh_token` or it is ignored).
5. If the grant expires mid-window the engine does not refresh it (no backend is reachable): the call fails with the "Codex login expired" message. That is the signal the window was too long, not a task result.

## 6. Defaults that differ from other arms (label these in the results)

| Default | Engine value | Treatment |
|---|---|---|
| Reasoning effort | `low` | set `FACTR_OPENAI_REASONING_EFFORT` to the arm-matched value |
| Service tier | `priority` | set `FACTR_OPENAI_SERVICE_TIER=default` |
| Usage-limit wait | parks up to 7200 s, five times | `FACTR_WAIT_FOR_USAGE_MAX_S=0` |
| Auto-verify gate | on for headless runs, when a test command is detected | `FACTR_AUTO_VERIFY=0` for the headline row; unset is the labelled gate-on row |
| Verify-on-stop nudges | on (all seven kinds, two per turn at most) | label if left on |
| Environment snapshot | on for headless | label if left on |
| Web backends | chosen by `FACTR_WEB_BACKEND`, `config.yaml` `web.search_backend`, else the keys present | none is present after the env strip (including `FIRECRAWL_API_URL`), so none is configured; record that |
| Sub-agents (swarm) and REPL tool | on | label if left on (`FACTR_SWARM_ENABLED=0`, `FACTR_REPL=0`) |
| Native compaction | automatic | label |
| Hosted `image_generation` tool | present in ChatGPT (Codex login) mode | label |

## 7. Separate labelled rows

- Headline: the configuration above, `FACTR_AUTO_VERIFY=0`.
- Gate-on: the same with `FACTR_AUTO_VERIFY` unset. Even in the headline row the verify-on-stop nudge can send the model back once after code edits with no test run (a `verify_nudge` span); report it as a label or set `FACTR_VERIFY_ON_STOP=0` in both rows. The gate internalizes a retry that stock harnesses do not have: report gate-off, gate-on and after-retry separately, with extra tokens and time per round.
- Memory and learning: a separate, clearly labelled run with `FACTR_MEMORY_ENABLED` and `FACTR_LEARNING_ENABLED` at their defaults and a persistent home across tasks.

## 8. Pre-flight checklist (all before the first scored task)

1. Versioned binary copy exists; sha256 recorded; `factr-<sha> __version` shows the expected version, sha and `db_schema`.
2. Master login refreshed; JWT `exp` read; window end set 15 minutes before it.
3. One smoke task through the full path (fresh per-task home, scrubbed env, `/api/agent/run`), then check:
   - the reply has `ok: true`;
   - every `Model call` span in `$FACTR_HOME/factr.db` (`spans` table) names `gpt-6-luna` and carries `gen_ai.request.reasoning_effort` equal to the expected effort;
   - no `checkpoints` directory in `FACTR_HOME`;
   - no `skills` directory in `FACTR_CONFIG_HOME`, and none in `FACTR_HOME` unless a REPL cell ran with learning enabled (with `FACTR_LEARNING_ENABLED=0` the REPL no longer creates it);
   - no `headless-deny` rows in the `approvals` table (`actor` column) of `$FACTR_HOME/factr.db`;
   - no `memory.extract` or `memory.recall` span, and `learning.skip` spans only;
   - the pre-flight of section 4 passed (no `model provider not ready` on stderr, `authenticated: true` for the current provider).
4. Environment audit: print the engine's environment (names only) and confirm nothing outside section 3 matches `FACTR_`, and nothing ends in `API_KEY`, `TOKEN` or `SECRET`. Also confirm `ls` of the task's `HOME`, `FACTR_HOME` and `FACTR_CONFIG_HOME` shows nothing the run should not have created, and that the real `~/.factr` is unchanged.
5. Label settings recorded in the results header: every row of section 6 with its chosen treatment.
6. An explicit go from the operator. A smoke task is a paid run; do not start the scored window without it.

## 9. Runner rules (what the runner must do; the engine does not do these for you)

These come from an independent review of the headless run path.

0. **Grade `final_text`, not `text`.** The `/api/agent/run` reply carries `final_text` (the last assistant message of the turn) next to `text`, which joins the text from before and after any stop nudge. Graders should take `final_text`.
1. **Empty or notice-only text is "no answer", never a result.** After repeated empty model turns the engine can still return `ok:true` with `text:""` or with a text that starts with `[provider guardrail]`. Treat both as "no result" and re-queue or record them as infrastructure failures.
2. **Classify infrastructure errors and re-queue them, do not score them.** An `error` that mentions a usage limit, `429` or a rate limit; `Codex login expired`; `OpenAI rejected the access token...`; and `ok:false` with `text:""` after a timeout are not task results. With `FACTR_WAIT_FOR_USAGE_MAX_S=0` a usage limit fails the task immediately.
3. **Run tasks sequentially, or with at most 2 engines at once.** Every engine shares one account's quota, so parallel engines hit a limit together.
4. **Isolate task directories.** The file and search tools have no workspace sandbox: `ls ../` or a search with `path:".."` can reach sibling task directories and their answers. Put each task's `cwd` under a parent that holds nothing else, and delete finished task directories before the next task starts.
5. **Token window.** The engine never spends the refresh token. Refresh the master login once per benchmark window, read the JWT `exp`, and start a task only if `exp - now > 15 minutes + timeout_s`. Copy `auth.json` fresh for every task.
6. **Never configure `fallback_providers`.** It makes the engine silently switch to another provider on non-context errors. Assert it is absent from every task `config.yaml` and grep the spans for `reason=provider_fallback` in the smoke task.
7. **Shutdown.** Wait at least 1 second after the reply before sending SIGTERM (spans are written by a batched writer about 0.3 to 0.5 seconds after the reply), then make sure no descendants survive: commands started with `nohup` or `setsid` can outlive the engine's group kill, so kill leftover descendants of the engine by process tree.
8. **Tool policy per task.** The web tools (`websearch`, `webfetch`) are available by default and reach public search engines and pages without API keys. Pass `enabled_toolsets` per task and record the exact set and the tool list from the request body. Recommended sets:
   - GAIA: `["terminal","file","code_execution","web","todo"]`. Run a free live smoke of `websearch` and `webfetch` first (a scripted fake model calls them; no paid call), because they scrape public pages.
   - TBLite, Aider polyglot, OOLONG, OOLONG-Pairs, LongCoT-Mini: `["terminal","file","code_execution","todo"]`. The REPL has no network.
   - Leave out `browser`, `delegation`, `memory`, `skills`, `session_search`, `cronjob` and `clarify` everywhere.
   - A call to a tool that is off the list is returned to the model as a tool error and the run continues; `disabled_toolsets:["web"]` behaves the same way.
   - Check whether the hosted `image_generation` tool still appears in the request and record it.
9. **Memory cap.** Set `terminal.max_memory_mb` in the task `config.yaml` (a few GB, and above 256 MiB so the REPL's own 256 MiB cap is not lowered) so a runaway command is killed at a fixed threshold; without it the engine kills only under real memory pressure. Record the value.
10. **Linux.** The REPL is off on Linux unless `FACTR_REPL_PYTHON` is set; set it there (the only case where this variable is set) and record it. If `HTTPS_PROXY` is set, add `NO_PROXY=127.0.0.1`.

## 10. Behaviours to label in the results

- **Repeat guard.** The 6th identical `(tool, args, result)` call gets a tool error and the 10th ends the turn (`ok:false`). A model re-running the same failing command ten times is stopped by this rule. Identify such runs by the `loop.guard action=stop` span and do not count them as model failures without looking.
- **Reflection turns.** Commands such as `rm -r`, `find -delete`, `git clean`, `chmod -R`, `dd`, `truncate` and `xargs rm` always cost one reflection turn before the retry, regardless of `approvals.mode`. Label if relevant.
- **Tool output caps.** Tool results over about 50 KB are cut (40% head, 60% tail) with a note and a spill file; `repl` output is capped at 8,000 characters. These announce themselves. REPL-heavy tasks will page a lot.
- **A timeout** interrupts the session and returns `ok:false` with `text:""`; files already written stay on disk (relevant to graded file outputs).

## Integrity

Only general capabilities were added. No benchmark task ids, answers or per-task formats appear in the code. Edits to test files are flagged in the `loop.guard` spans (`tests_touched`), so those passes can be excluded. The format-compliance nudge reads an explicitly stated output label from the task text only.

## Verification

Verified against engine commit 7e22758 (source; the later gateway edits in 286bff9 touch the effort pill and the cron run-history route, nothing above) and the release binary built from c616fec (`factr __version` reports sha `c616fec`, `db_schema` 9) on 2026-10-04.

Read in the source: every variable and accepted value in section 3, the token file and its mode, the readiness line, the `/api/status` and `/api/agent/run` handlers and the 3600 clamp, the auth headers, the login and refresh code paths (`auth.json` location, entry shape, expiry rules, the single-refresher rule), and the `factr login`, `factr auth ...` mapping and the backend `auth refresh` command.

Run live, with temporary directories only, a scripted fake model (no paid calls), the doc's scrubbed environment and `--provider openai-compatible` (Chat Completions) or `--provider openai-api` (Responses) against it: server start and the ready line; token file mode 0600, 64 characters; a supplied token on stdin and in the environment (both accepted, under 32 characters refused); both auth headers and `401` without; `/api/status`; `/api/agent/run` with a plain and a tool-using prompt, `ok: true`; no `checkpoints` or `skills` directory, `checkpoint: null` in `undo/`; no `memory.*` span and only `learning.skip` with the two variables at `0`, against `memory.recall` and `memory.extract` with them unset; `FACTR_AUTO_VERIFY=0` against unset against `1` with a failing unit test in the project; `FACTR_VERIFY_ON_STOP=0`; the deadline reminder; reasoning effort and service tier on the wire; `headless-deny` without `approvals.mode: off` and policy approvals with it; `missing_login` and `authenticated` with no login and with an expired fake one; SIGTERM to the process group; nothing under the real `~/.factr` or the other legacy home directories on the machine changed (file lists and mtimes compared before and after); `factr auth`, `factr login openai` and `factr auth refresh openai-codex` with and without a backend.

Not verified live: a real ChatGPT/Codex login (the positive `authenticated: true` and `missing_login` cases, the refresh and the expiry window, the `Codex login expired` message), the Bedrock cap, web backends, a model-called `refine`, and the `factr login openai` sign-in flow itself (only its argument mapping and `--help`).
