# Factr-I bots checkpoint

## 2026-10-18: setup in progress

- Branch: `feat/bots-step2`, starting commit `9fffe4a4cd`; working tree was clean.
- Read COMMON-RULES.md, bots-handoff.md, pause-notes/bots.md, design sections 1-13, and the research notes. No credential files read.
- Disk check: `rtk proxy df -h /` reported 93 GiB available.
- Test isolation root: `/private/tmp/bots-handoff-0h03oq2o`; fresh HOME, FACTR_HOME, FACTR_CONFIG_HOME and Electron userData directory. Cargo uses only this worktree's `target`.
- Dependencies installed in this worktree rather than the app worktree, to honor the common rule restricting writes. Python will use an independent venv, not the formerly shared venv; no `uv sync` or `uv run` in this worktree.
- Desktop install: first npm invocation rejected duplicate config paths before installation; corrected to distinct empty user/global config paths. Corrected `npm ci` is running.
- Baseline running: isolated `CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=/Users/rameelmalik/factri-bots-wt-s2/target cargo test -p factr-gateway --locked` from `engine/`.
- Graph: Verify tier. Nearest existing index `factri-bots-app` is ready and covers the app worktree at the same starting commit; this worktree is not indexed. Graph findings will be checked against exact local source and coverage, with fallback for excluded tests and Rust additions absent from the older whole-repo index.
- No implementation milestone validated yet. No push, remote changes, release-checkout changes, or benchmark access.

## Next

1. Complete dependency setup, baseline gateway tests and engine test-profile build; record exact results and commit validated setup.
2. Fix first-turn hang, default-chat messaging and idle LRU eviction in that order, with regressions.
3. Continue handoff tasks 3-9 in order; keep upstream skeleton and additive Rust seams.

### Setup validation so far

- Isolated `npm ci` in `desktop/`: exit 0, 1191 packages installed. npm reported 37 audit findings (9 low, 13 moderate, 11 high, 4 critical); no unrelated dependency changes made.
- Isolated `uv export --locked --extra dev --no-emit-project --output-file /private/tmp/bots-handoff-0h03oq2o/requirements.txt`, `uv venv --python /Users/rameelmalik/.local/bin/python3.11 backend/.venv`, and `uv pip sync --python backend/.venv/bin/python <requirements>`: exit 0, 84 packages installed. No shared venv modified.
- `PYTHONPATH=$PWD .venv/bin/python -m pytest tests/tools/test_message_agent_mcp.py -q` from backend with isolated HOME/FACTR dirs: 6 passed.
- `../node_modules/.bin/vitest run --project electron electron/pool-retire.test.ts electron/pool-retire-http.test.ts` from desktop/app with isolated HOME/FACTR dirs: 2 files, 10 tests passed.
- `../node_modules/.bin/vitest run --project ui src/plugins/factr-bots` in the same isolated environment: 93 files, 762 tests passed.
- Full gateway baseline completed; no source changes preceded it.

- Gateway baseline `cargo test -p factr-gateway --locked`: exit 0. Unit tests: 369 passed, 0 failed, 1 ignored. All integration suites passed. No source changes made before this green baseline. Test-profile engine binary build now running: same isolated environment and target, `cargo build --profile test --bin factr --locked`.

### Milestone 1 validated

- `cargo build --profile test --bin factr --locked`: exit 0 (57.05 s), binary `target/debug/factr`.
- Gateway total: 396 passed, 0 failed, 1 ignored (369 unit passes plus 27 integration passes).
- `git diff --stat a626cbae -- engine`: existing branch changes are additive bot modules and e2e scripts, plus 4 registration lines in rpc.rs.
- Disk after setup/build: 82 GiB free, above 20 GiB threshold.
- Next: reproduce the creation-time intro hang using the desktop's exact create/title/claim/mount/submit sequence, then fix default-chat MCP and pool idle eviction. Tasks 2-9 remain.

## Resumed run, 2026-10-18: recovered intro regression harness

- Read the checkpoint, Git status/log and the sole uncommitted diff before continuing. The diff extends `bot-chat-deliver.mjs` with the exact create/title/claim/window-resume/submit sequence, an optional real Python teammate MCP, bounded RPC waits and engine cleanup. Kept it after validation.
- Re-read common rules, handoff, pause notes, design sections 1-13 and research Markdown notes. Parent advice file: none yet. No credential files read.
- Disk: 81 GiB free. Existing setup baseline remains commit `adcb5e3a`.
- `node --check engine/crates/factr-gateway/e2e/bot-chat-deliver.mjs`: exit 0.
- Real-engine harness: `FACTR_BIN=$PWD/target/debug/factr FACTR_BACKEND_PYTHON=$PWD/backend/.venv/bin/python FACTR_BACKEND_PYTHONPATH=$PWD/backend node engine/crates/factr-gateway/e2e/bot-chat-deliver.mjs`, with HOME/FACTR_HOME/FACTR_CONFIG_HOME/Electron userData/SCRATCH isolated below `/private/tmp/bots-resume-vrkpu2a8`: exit 0, all checks passed. The real Python MCP was used for the fresh-bot sequence. The intro reached the model with message_agent available and completed in the mounted window. This does NOT reproduce or establish a fix for the historical 13-minute packaged intro hang.
- Graph tier Verify; app index ready in the app worktree, older whole-repo index generation `2026-10-18T11:06:56Z`. Coverage for the new delivery module/harness is missing, so exact local source is authoritative.
- Next: investigate task 2(a) beyond the simple engine sequence, then default MCP and idle pool eviction. Tasks 2-9 remain open.

## Resumed milestone: default-chat teammate MCP and real creation coverage

- Changed `ensure_teammate_mcp` to add the server to default only when at least one Bot Mode profile exists. Plain non-bot profiles remain untouched. Missing configs are created for managed bots and default; existing model settings and other MCP servers are preserved.
- Real `profiles.create` + `profiles.configure` exposed a new managed bot without `config.yaml`; the old installer skipped it. Added that regression and fixed it.
- Expanded delivery harness: real Python feature backend creates/configures the profile, default engine creates a user chat after bot creation without restarting, and both default/fresh-bot first prompts expose message_agent. Mounted bot intro completes. Wait for completion rather than only the first model request to avoid an assertion race.
- Expanded MCP-to-engine relay harness: default sends to B, B receives the existing `@factr` sender handle, and the reply returns. No Python CLI turn for B.
- Tests (every run has fresh HOME/FACTR_HOME/FACTR_CONFIG_HOME/Electron userData under `/private/tmp`):
  - Backend: `PYTHONPATH=$PWD .venv/bin/python -m pytest tests/tools/test_message_agent_mcp.py -q`, from backend: **9 passed**, exit 0 (2.20 s).
  - Engine create/default/intro/delivery harness: `FACTR_BIN=$PWD/target/debug/factr FACTR_BACKEND_PYTHON=$PWD/backend/.venv/bin/python FACTR_BACKEND_PYTHONPATH=$PWD/backend node engine/crates/factr-gateway/e2e/bot-chat-deliver.mjs`, root `/private/tmp/bots-default-aa392_x0`: **all checks passed**, exit 0.
  - Default and bot MCP -> relay -> real B engine: `FACTR_BIN=$PWD/target/debug/factr PYTHON=$PWD/backend/.venv/bin/python BACKEND=$PWD/backend node engine/crates/factr-gateway/e2e/bot-message-agent.mjs`, root `/private/tmp/bots-handoff-rut950ej`: **all checks passed**, exit 0.
  - `git diff --check`: exit 0.
- Test mistakes corrected before conclusions: the first wrapper used Path.resolve on the venv interpreter, following its symlink to the base interpreter (no yaml). Preserve the absolute venv path WITHOUT resolving its symlink. Thus the recovered harness's earlier claim of using a fully installed Python backend was too strong. Subsequent real-feature runs used the venv correctly. An attribution assertion initially expected @default; source establishes default's existing public handle is @factr. Corrected the test; full rerun passed.
- No Rust source changes, no UI changes, no new Rust port. Setup's gateway baseline still stands (396 passed, 1 ignored); it was not rerun for Python/test-only edits.
- Task 2(a): historical intro hang remains **unreproduced and unresolved**, including after real profile creation. Old renderer log confirms the observation but does not include engine pre-model traces. Do not report the missing-config fix as the 13-minute hang's root cause.
- Task 2(b): default MCP installation and fresh default-chat tool availability/delivery are verified at engine/transport level. Existing already-created default sessions and the packaged @mention UI still need QA.
- Task 2(c): no new fix committed. Existing pool code uses renderer activeTurn as an early veto, and Rust retirement authority is two session-list snapshots (no admission freeze). Both need investigation before calling pool eviction safe or fixed.

### Advice / blocker

- Required advice command attempted: `claude -p --model claude-fable-5-1 'Factr-I bot intro hang investigation: an exact isolated real-engine create/title/claim, second WebSocket resume, then prompt.submit with the real Python MCP completes. Historical packaged first intro never reached Ollama until Stop. Before changing architecture, which next narrow diagnostic distinguishes an endpoint inheritance mistake from competing session attachment or MCP initialization? Please advise in at most 8 lines. No repo review.' < /dev/null` (noninteractive runner stdin).
- Result: exit 1, `rtk: Failed to execute command: claude: No such file or directory (os error 2)`. No Fable answer obtained and no Claude account/credential files accessed. The usual executable paths `/Users/rameelmalik/.local/bin/claude`, `/opt/homebrew/bin/claude`, `/usr/local/bin/claude` do not exist.
- User clarification pending: provide executable path or waive the Fable advice requirement. COMMON-RULES.md requires Fable before architectural/design forks. The next material pool choice is whether to keep the Rust session-snapshot fallback or add a Rust retirement permit that freezes admission through the existing retirement protocol. Do not choose or implement that protocol change without satisfying the advice instruction.
- Parent advice file re-read after the recovered milestone and before this one: none yet; no STOP.
- Remaining in order: finish task 2(a), finish default existing-session/packaged QA, fix/verify 2(c), then tasks 3-9. No claim that task 2 is complete.

## Blocked handoff and cleanup

- Source milestone committed as `7c5d220`; recovered harness milestone is `a185e02`. Both include the required co-author trailer.
- Wrote REPORT.md and NEEDS-GUI-QA.md with precise remaining scope, test limits, the Fable blocker and isolated resume commands. No task 2 completion claimed.
- Re-read the full supplied design law against this milestone: no UI/design changes or controls were introduced; no visual verification is claimed.
- Checked process executable paths: no running process from this worktree's target. No Docker resources were created. Removed the 3.9 GiB worktree target as generated build output. Dependencies remain for resumption. Rebuild the test-profile engine before running the e2e harnesses again.
- Copied the final report to `/Users/rameelmalik/Documents/Sovereign AI/handoff/bots-REPORT.md` under the explicit handoff instruction.
- Stopping because the required advice executable is unavailable at the next protocol/design fork; user executable-path/waiver response remains pending.

## Resumed with parent advice, 2026-10-18

- Read ADVICE-FROM-PARENT.md first, then checkpoint/report, clean status and log, COMMON-RULES, handoff, pause notes, design sections 1-13 and research notes. Parent removes the advice blocker: write questions and continue. No STOP.
- Advice: follow Q1 diagnostic order and Q2 retirement permit with drain. Questions recorded in handoff/QUESTIONS-bots.md; no advice CLI calls.
- Verify graph: deferred tools are available. App generation 2026-10-17T21:28:43Z; whole-tree generation 2026-10-18T11:06:56Z in sibling worktrees. Coverage reports changed app files and missing new bot Rust modules; local source is authoritative for those.
- Disk before build: 81 GiB free. Rebuilt test-profile engine with one target inside this worktree, fresh CARGO_HOME and isolated HOME/FACTR dirs: exit 0 in 2m16s. Build environment root /private/tmp/bots-resume-xu0w5h5e. Gateway baseline running before source validation.
- Task 2(a) investigation: added an unpinned-launch probe (no provider/model CLI overrides). Read MCP client: stderr is already drained; teammate MCP sets 900s for initialization and tool discovery as well as tool calls. This is a potential long stall, not established historical causation. First-turn watchdog implementation in progress, not validated.

### Task 2 investigation / implementation in progress

- Fresh baseline: cargo test -p factr-gateway --locked, isolated root /private/tmp/bots-check-492jpwex: exit 0, 396 passed, 1 ignored.
- Unpinned launch (no provider/model CLI flags), /private/tmp/bots-unpinned-intro.mjs: all checks passed, exit 0, root /private/tmp/bots-check-1bk2ft75. Historical intro hang still not reproduced.
- First-response regression: a stalled HTTP model response on a new titled Bot Chat now emits preamble_timeout to a second mounted client at 60s; the next prompt reaches the model. Confirmed in /private/tmp/bots-check-16h3bfg5. Whole suite did not pass: pre-existing default tool availability failed and a later registry ensure timed out; no full-suite success claimed. Initial watchdog attempt failed because rename sets custom_title rather than title. Fixed and rerun.
- Reproduced existing default sessions missing message_agent after creating a roster; a second attachment alone does not refresh tools. Following the existing SDK custom-tool callback seam in a new additive Rust module, with queued ack, upstream relay envelope fields, and reply wake through the turn API. Advice question written; continue by parent rule.
- Added retirement door with admission freeze and typed retiring error; removed snapshot fallback; Rust candidates ignore stale renderer busy flags, while Python keeps its veto. Opens of a retiring generation wait for stop; slots remain held until physical child exit. Delivery results persisted for restart idempotency. Validation in progress.
- Production pool fake tests: isolated vitest electron/pool-retire.test.ts electron/pool-retire-http.test.ts electron/pool-spawn-coordinator.test.ts: 3 files / 26 passed, exit 0. UI relay unit tests: 2 files / 40 passed, exit 0. Electron and renderer tsc each exit 0 (before final relay envelope type additions).
- Rust bot module tests: 39 passed, exit 0 (before final Python golden assertion). Golden envelope generated by the existing Python enqueue_envelope in fresh isolated /private/tmp/bots-check-g7nyxs1k; no credential files read.
- New real-engine native/pool harness added; running. No milestone commit yet because new runtime checks are not green.

### Parent second-round conditions and corrective validation

- Parent approves the watchdog and SDK callback seam, requires a small hard MCP initialization/tools-list timeout, visible typed failure, exact upstream tool contract, and continued task execution. Applied: installer timeout_secs=10; warm-up clamps legacy 900s entries to 10s per handshake request and 20s overall, emitting teammate_mcp_timeout. Legacy synchronous MCP fallback remains compatibility-only; new engine turns use native SDK message_agent.
- Captured message_agent schema from upstream e6cf2a9b1e by executing only its pure schema function. Only Factr's default handle and punctuation are normalized (common rules override em dashes); native tool uses this fixture. Roster names/roles ride the uncached reminder slot.
- Fixed a source regression before commit: three recursive dispatch wrappers overflowed the contract-sweep worker stack. Replaced them with admission/tool/preamble hooks before dispatch. The unmodified full contract sweep now passes (exit 0).
- Fixed retirement test setup: the temporary probe connection had no control link, so its fail-closed busy response was correct. Open the control link before checking. Retirement uses the public in-process active-turn registry, because shared stored session statuses can belong to another engine.
- Corrected harness parsing after adding the uncached roster reminder: ignore the reminder-only user row and detect the tool result anywhere in the request, rather than assuming the last row is always a tool. Also count the target's exact message when asserting no duplicate delivery; the sender's attributed reply contains the same text and is a separate, expected model request.
- Full cargo tests are running after these corrections. The ordinary delivery/create/existing-default harness is green (exit 0, /private/tmp/bots-check-9u8anq3k).

### Milestone: startup/default messaging and pool protocol validated

- Full gateway: `python3 /private/tmp/bots-test.py engine cargo test -p factr-gateway --locked --quiet`, root /private/tmp/bots-check-p8f23bau: **401 passed, 1 ignored, exit 0** (374 unit + 27 integration). Engine rebuild exit 0. Core agent loop, tool implementations, prompts and providers untouched; new gateway modules plus registration/pre-dispatch/event hooks and one REST route.
- Real engine + production pool harness: `python3 /private/tmp/bots-test.py . desktop/node_modules/.bin/tsx engine/crates/factr-gateway/e2e/bot-runtime.mts`, root /private/tmp/bots-check-iseg7rze: **exit 0**. Native queued ack before target execution; default session predating roster gains the tool; reply wakes original sender; callback idempotency; busy engine refused; Draining rejects prompt/bot delivery with typed retiring; real engine exit then reopen same Bot Chat; persisted envelope outcome prevents duplicate target turn; rerouted message delivered; repeated churn across five bots stays at 3 pooled engines and evicts despite a stale renderer busy flag. A deliberately hung MCP with legacy timeout_secs=900 fails visibly with teammate_mcp_timeout within 23s.
- App pool/exit/stop tests: vitest electron/pool-retire.test.ts electron/pool-retire-http.test.ts electron/pool-spawn-coordinator.test.ts electron/pool-child-exit.test.ts electron/pool-stop.test.ts: **5 files, 37 passed, exit 0**, /private/tmp/bots-check-98x66uwq. UI relay tests include typed retirement retry and native sender wake: **2 files, 42 passed, exit 0**, /private/tmp/bots-check-_1pc1xx7.
- Backend teammate config/protocol tests: **9 passed, exit 0**, /private/tmp/bots-check-01qgn7f3. Native queue fields checked against Python-produced golden JSON; schema captured from audited upstream.
- Ordinary delivery harness with real feature create/configure path and no CLI provider/model overrides: **all checks passed, exit 0**, /private/tmp/bots-check-9u8anq3k.
- Limits: historical 13-minute packaged stall has not been uniquely reproduced; reproduced same-class long MCP handshake now bounded and typed, plus first-response watchdog mitigation. No packaged Electron or real Ollama verdict yet. Pool cap cancellation/partial transcript after a late autonomous turn still needs real-engine QA; no forced retirement of an already-busy candidate.
- Next task 3: finish the upstream audit, durable delayed/expired reply wake and any remaining alignment. Then task 4 in order. No push or credential-file access.

- Final renderer TypeScript check: isolated root /private/tmp/bots-check-mvl6zj5o, tsc -p tsconfig.json --noEmit, exit 0. Final watchdog rerun logging to /private/tmp/bots-watchdog-verified.log; no full-suite verdict until it finishes.

## Task 3: upstream parity and durable native replies

- Task 2 source milestone committed locally as dd06d95f. Final CHECK_PREAMBLE=1 ordinary delivery suite passed in /private/tmp/bots-check-gaj4_zyl (all checks passed, exit 0; log /private/tmp/bots-watchdog-verified.log). It observes the typed error on the mounted second socket at 60s and a successful next prompt. Historical packaged root cause remains unproven.
- Wrote the requested item-by-item upstream audit at /Users/rameelmalik/factri-ogb-notes/bot-parity-upstream.md. All 17 contracts have upstream file:line references, current entry points, deviations, explicit decisions and verification boundaries. Keep hop cap 2 for autonomous-loop safety; default ordinary-chat messaging is the user's explicit extension of upstream's canonical-only sender gate.
- Native sender watches the existing durable relay reply files. Desktop persists a result before waking the engine; interrupted/retired senders recover the original session. Unclaimed stale envelopes are atomically taken out of the courier queue and expired without a Python waiter. Claimed mail gets an ambiguity-aware delivery_timeout at the existing 3120s reply budget. Settled native records are eligible for cleanup after the existing six-hour spool retention.
- Registered peers now use an additive Rust REST adapter over the existing hidden Bot Chat lookup/chat POST contract. Queued acknowledgment remains immediate. Peer credentials come only from already-exported engine environment variables; no credential-file reads. Cross-origin redirects are refused; a crash after acceptance is not blindly replayed. A real registered gateway/secret-store integration remains unverified.
- Python peer HTTP golden captured by isolated existing _peer_dm with fake transport: /private/tmp/bots-check-li7hxch7, exit 0, tests/fixtures/peer_dm.json. Existing native relay envelope golden remains unchanged. Upstream name/schema/description fixture remains authoritative.
- Aligned friendly-title lookup, native protocol toggle, Unicode-safe long-reply tail and 10-minute attempt cap. No core loop/tool/provider/prompt edits.
- Real runtime harness including Python peer HTTP golden, sender restart + expired reply wake, autonomous unclaimed-mail expiry, registered peer delivery + wake, protocol disabled tool absence, prior pool/restart/idempotence checks: exit 0, /private/tmp/bots-check-muivc1_0, log /private/tmp/bots-parity-final-runtime.log.
- Relay vitest: 42 passed, exit 0 (/private/tmp/bots-parity-relay.log). Renderer tsc: exit 0, /private/tmp/bots-check-dgfzulvw. Backend MCP: 9 passed, exit 0, /private/tmp/bots-check-d7bgorhw. Engine build exit 0 (/private/tmp/bots-parity-validated-build.log). Final full gateway suite finishing; do not infer its result from the earlier green runs.
- Parent advice re-read after dd06d95f and before this milestone: no STOP. Next task 4: routines, engine-owned group sessions/durable room integration, complete model inheritance, child cancellation, Description/blank routine name/delete verification; then tasks 5-9.

- Final full gateway suite: 401 passed, 1 ignored, exit 0, /private/tmp/bots-parity-release-cargo.log (fresh isolated root recorded at its top). git diff --check exit 0.

## Resumed account-change run: recovered task 4 milestone

- Read parent advice first, then checkpoint/report/status/log, full common rules and handoff, pause notes, design sections and research. REPORT.md was stale: task 3 is already committed as b5bc491e. Inspected the 520-line backup patch and every recovered untracked module; kept the sound implementation. Existing target/debug/factr was present. Disk: 87 GiB free. No credential files read, no push.
- Graph Verify: nearest whole-tree sibling generation 2026-10-18T11:06:56Z. Checked all recovered paths; new bot_routine/group/stop modules absent from sibling graph, read exact worktree source. Traced model inheritance callers.
- Routines now submit literal instructions on the owner engine; an additive Rust parser upgrades safely quoted saved v2 wrappers without a shell. Owner-engine group session pointers survive restart/concurrent opens. Model inheritance carries endpoint/context even when voice created config first. Edit Description fetches stored metadata without overwriting typing; blank routine names are rejected. Existing delete route and auth/silence behavior retained.
- Parent cancellation now precedes descendant cancellation to stop further parent work first. Real delegate streams close and subagent.list becomes interrupted. Core loop/tools/prompts/providers untouched.
- Corrected recovered harness failure: its local Stop result shadowed the eviction-count array. Also made churn deterministic by requesting a nonresident profile and asserting exactly one reclaimed full slot each round. This was a harness defect, not evidence of failed pool eviction.
- Validation, every command via rtk proxy and /private/tmp/bots-test.py with fresh HOME/FACTR_HOME/FACTR_CONFIG_HOME/Electron userData:
  - engine cargo test -p factr-gateway --locked --quiet: 404 passed, 1 ignored, exit 0 (377 unit + 27 integration), root /private/tmp/bots-check-g2s9q2uu. Repeated after parent-cancel ordering below.
  - engine cargo build --profile test --bin factr --locked --quiet: exit 0, /private/tmp/bots-check-u3gkiu46.
  - desktop/app ../node_modules/.bin/vitest run --project ui src/plugins/factr-bots: 94 files, 762 passed, exit 0, /private/tmp/bots-check-hjkmgeyf.
  - backend .venv/bin/python -m pytest tests/tui_gateway/test_profiles_inherit_launch_model.py tests/tools/test_message_agent_mcp.py -q: 11 passed, exit 0, /private/tmp/bots-check-z05b7kxt.
  - backend profile configure/delete settlement/off-loop/tombstone suites: 36 passed, exit 0, /private/tmp/bots-check-d8kvvuxg.
  - renderer tsc -p tsconfig.json --noEmit: exit 0, /private/tmp/bots-check-vm8fuw9o.
  - production-pool/real-engine runtime harness: exit 0, /private/tmp/bots-check-y66skg1p; group concurrency/restart, saved routine literal execution, two delegated child streams cancelled, native replies/restart/expiry/peer, 7 forced pool evictions across 5 bots, hung MCP typed timeout.
- Advice: parent second-round rules still apply; no STOP. Recorded hosted-group transport question and continue. Owner-engine group sessions work; full gateway-owned room-round execution after Desktop closes is still a separate parity gap (Rust driver=false). This milestone does not claim that durable room behavior or packaged GUI clicks.
- Next: finish task 4 group-boundary verification, then task 5 automated 17-item engine checklist, followed by computer branch integration and setup/sandbox/platform work.

- Final repeated gateway suite after parent-first Stop: 404 passed, 1 ignored, exit 0, /private/tmp/bots-check-h1_cnkol.

### Task 4/5: cron output on the canonical engine chat

- New real profile/schedule harness bot-schedules.mjs uses only isolated generated config and an OpenAI-compatible stub. Exercises real create/describe/clone/SOUL, cron create/pause/restart/resume/fire/delete, output storage and denied scheduling tools. Initial 404 was a test mistake: feature HTTP routes require feature=1; corrected.
- The output-delivery extension exposed the remaining Python CLI fallback for cron bot-chat output. Added a small authenticated /api/bot/deliver adapter over the existing Rust canonical delivery path, and an owner-engine Python HTTP adapter. Existing delivery id derives from execution id and preserves restart deduplication. Suppressed warning policy remains respected. Cross-profile output routing remains a separate test/integration item.
- backend pytest tests/cron/test_engine_bot_chat.py tests/cron/test_cron_bot_chat_delivery.py tests/cron/test_bot_chat_pending.py tests/cron/test_cron_delivery_redaction.py tests/cron/test_warning_delivery_policy.py -q: 38 passed, 3 skipped, exit 0, isolated /private/tmp/bots-check-r9w_cu8p. The three existing skips were reported by pytest; not counted as passes.
- Engine build after private RPC error type correction: exit 0, /private/tmp/bots-check-72_kyj6h. No visibility widening of the shared error type.

- Final gateway after HTTP output adapter: 404 passed, 1 ignored, exit 0, /private/tmp/bots-check-hf_8oeyp.
- bot-schedules.mjs including bot-chat output: exit 0, /private/tmp/bots-check-7uyvt85f. Saved output is processed in the pre-existing canonical Bot Chat, with no per-output CLI chat.
- New combined bot-checklist.mjs: four real-engine suites passed, exit 0, isolated /private/tmp/bots-check-mxvq6m5w; artifact /private/tmp/bots-check-mxvq6m5w/bots-checklist-5mI0wJ/checklist.json. All 17 flows enumerated with narrower engine evidence and remaining GUI/external/implementation boundaries, not 17 full passes. Electron tsc: exit 0, /private/tmp/bots-check-sgbunz_3.
- Rewrote stale NEEDS-GUI-QA.md and active REPORT.md. Task 4's member sessions and native routine/output execution are validated; durable close-Desktop room rounds remain explicitly partial. Task 5 harness is implemented and green at its stated engine scope. Real UI/external checklist is pending. Next ordered implementation: useful computer branch commits and real isolated computer tests.

## Task 6: computer branch integration, source milestone

- Read and applied useful changes from 0777f66d, 90fcc7a2, d7b764e8, 7fc5bbed, selected 1f5d990c (Run now/i18n/contracts/real test), and a9b012e2. Conflicts resolved to preserve native queued messaging, canonical delivery, retiring retry and reply wake. Skipped the optional new allowlist feature because it was not requested. No Rust core/source changes from the computer branch survived its final poll-protocol reversal.
- Computers contribute live courier connections; stopped computers remain targets without being polled awake. Enabled schedules protect computers from idle/cap stop and start them at app launch; unscheduled computers stop on quit. Stopped computers recreate on spec/image changes. Host timezone is passed. Run now uses existing cron action and stays visible/focusable.
- Added persistent relay spool symlink into each profile so queued native mail/receipts survive container recreation. Removed duplicate hop helpers/constants introduced by overlapping cherry-picks.
- Extended scheduled bot-chat delivery to another profile via the same Desktop courier with a deterministic execution-derived envelope id. Same-host computer roster entries may resolve an unmounted recipient; unrelated remote machines remain excluded. Duplicate ids cannot be reused with another payload. Existing API without a supplied id retains its golden shape.
- Validation (fresh isolated roots via /private/tmp/bots-test.py):
  - UI relay/Run now: 56 passed, exit 0, /private/tmp/bots-check-5sta4hlu. Full bots UI: 94 files, 770 passed, exit 0, /private/tmp/bots-check-1e0quyso.
  - Electron computer lifecycle + five pool suites: 6 files, 84 passed, exit 0, /private/tmp/bots-check-txzfng5y. Renderer and Electron tsc both exit 0, /private/tmp/bots-check-_9u_1ns2 and /private/tmp/bots-check-vhggtl35.
  - Backend computer guard/relay/schedule scope/MCP: 51 passed, exit 0, /private/tmp/bots-check-rahfc1jz. Extended delivery/redaction/relay suites: 107 passed, 3 existing skips, exit 0, /private/tmp/bots-check-7_2h5ytp. New cross-profile/id validation tests: 4 passed, exit 0, /private/tmp/bots-check-_20i1172.
  - Real engine bot-schedules.mjs now includes own Bot Chat plus Alpha-to-Beta scheduled output over the courier: exit 0, /private/tmp/bots-check-jm6yuv4v. Initial courier attempt lacked its empty outbox directory (test bug); created it and reran green.
- Real Docker QA remains RUNNING/PENDING, not claimed: fresh source image build log /private/tmp/bots-computer-build-resume.log. Isolated HOME lacks Docker Desktop socket/buildx discovery, so set DOCKER_HOST=unix:///Users/rameelmalik/.docker/run/docker.sock and generated non-secret DOCKER_CONFIG=/private/tmp/bots-docker-client-15cllvq4 with only cliPluginsExtraDirs. No user Docker config/credentials read. Stopped only this run's legacy Docker build and retried with bundled buildx.
- Hardened existing image test before running it: unique bots- names, cleanup trap only after name checks, default scratch under tmp, stop/restart B for the read-only/C probes so at most one pair runs. One extra health assertion added. bash -n passed. Real two-computer test updated for queued native replies and no invalid exec against a stopped B; full runtime verdict still pending.
- Advice: no STOP; group transport question pending without blocking progress. Continue image QA and task 7 platform setup/fallback, then task 8 and final docs/cleanup.

### Milestone: Docker setup and bounded status

- Added main-process runtime-status/setup IPC with five explicit states, a native consent dialog showing fixed commands, joined concurrent clicks, bounded installation/startup waits, and actionable retry. Existing Computer gate invokes it and starts the bot only on ready. No licence acceptance flags or unconfined fallback in the setup controller.
- Platform plans: macOS installed Docker Desktop/Colima start, Homebrew install or official download; Windows WSL >=2.1.5 check, elevated WSL prerequisite or interactive winget Docker installer; Debian/Ubuntu pkexec apt-get Docker Engine, other distributions explicit official instructions. These plans are fake-tested, not actual machine installations.
- Isolated tests: 11 setup/controller tests passed (/private/tmp/bots-check-6t9xw3qj), 2 Computer gate tests passed (/private/tmp/bots-check-k0xx27yx); renderer and Electron tsc passed before commit. Actual macOS read-only readiness probe passed, no installation/restart (/private/tmp/bots-check-kcwx1mfc).
- Full gateway rerun had one timing-sensitive skills_seed_paths failure while container builds/model load ran; focused unchanged test passed in 6s (/private/tmp/bots-check-pzrcl9j6). Full-suite verdict remains the earlier 404 passes/1 ignored until a clean rerun.
- Real lean-image test passed 57/57 including a local model turn; exact test containers removed. Two-computer real-model relay/schedule test still running, no verdict. Disk 70 GiB free.
- Advice: parent re-read, no STOP or new answer. Continue local OS sandbox, browser profile, explicit Stop gate, then OS/manual QA and final documentation.

## Restart recovery: explicit Stop milestone

- Read parent advice first, complete common rules/handoff, checkpoint/report/status/log, and inspected all three interrupted files against the backup patch (identical). Kept and completed them. No advice blocker or STOP.
- Engine target/debug/factr is present (192 MiB); no rebuild needed for TypeScript-only recovery. Disk 81 GiB free. docker info: 29.8.0, Linux; no bots- containers left from the interrupted run. Recreated missing /private/tmp/bots-test.py with fresh HOME/FACTR_HOME/FACTR_CONFIG_HOME/Electron userData per test.
- Verify graph: factri-bots-app generation 2026-10-17T21:28:43Z, sibling root; recovered bot-computers paths not tracked, main metadata changed. Exact local source/callers read; no negative graph claim.
- Explicit Stop now persists across reconnects/restarts and scheduled startup until explicit Start. Removed containers drop stale routes. A missing Docker daemon with a computer marker refuses host fallback to prevent two writers. Corrected recovered status shortcut so a failed Stop cannot report a live container as off.
- Validation: rtk proxy python3 /private/tmp/bots-test.py desktop/app ../node_modules/.bin/vitest run --project electron electron/bot-computers.test.ts electron/bot-docker-setup.test.ts: 62 passed, exit 0, /private/tmp/bots-check-4x7i_v_t. Electron tsc -p tsconfig.electron.json --noEmit: exit 0, /private/tmp/bots-check-18ghvzdp. git diff --check: exit 0.
- Next task 7: local sandbox fallback/browser profile, finish real computer relay/schedule QA, then task 8 platform checks and task 9 docs/cleanup. No credential files read or push.

### Task 7 follow-up: real Docker QA found a native-title defect

- The resumed real two-computer suite completed with 3 passed / 5 failed, exit 1, isolated /private/tmp/bots-check-kkeam_5y (synthetic computer files /private/tmp/real-5cac3f). Containers were removed immediately by afterAll; docker ps confirms none remain. Do not claim that run passed.
- Inspected only synthetic session data: the headless API's first prompt overwrote custom_title=Bot Chat with a prompt title, then native message_agent's callback correctly rejected the renamed session. Added the real headless regression to bot-runtime.mts and observed it fail against the old binary (/private/tmp/bots-check-vs9j8x42).
- Fixed once in additive bot_message.rs: populate the connection's canonical title before the existing auto-naming check. No core loop/tools/prompts/providers changed. The same regression now passes, including the full runtime/pool suite: exit 0, /private/tmp/bots-check-051rqf8f, /private/tmp/bots-headless-title-regression.log.
- Full gateway: rtk proxy python3 /private/tmp/bots-test.py engine cargo test -p factr-gateway --locked --quiet: 404 passed / 1 ignored, exit 0, /private/tmp/bots-check-q4x5ngkd, /private/tmp/bots-resume-gateway.log. Engine build with --profile test --bin factr --locked --quiet: exit 0, /private/tmp/bots-check-0piy2bxt.
- Image rebuilding with this fix, one build; real computer rerun pending. Two additional harness defects identified separately: the network probe's 5s test budget was too short; an advanced fake idle clock legitimately stopped the paused computer. Correcting these before a fresh run.
- Local sandbox implementation is in progress and uncommitted: real macOS native shell/file boundaries and Python feature call pass, Windows lifecycle fakes pass, browser runtime still needs work. Do not count task 7 complete. Advice re-read: no STOP.
