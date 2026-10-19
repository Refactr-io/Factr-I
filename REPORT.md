# Factr-I bots roster refresh report

Completed on branch `fix/bots-roster-refresh` in the authorized task worktree. Local commits: `9d88ea64` (fix and regressions), `718cb7ff` (real-engine QA and manual scenarios). Both carry the required co-author trailer. No push, version bump, packaged-app launch, GUI driving, benchmark or personal credential-file access.

## Root cause and fix

A sandboxed bot sees only its private profile tree. Alpha therefore cannot discover a subsequently created Beta by scanning profiles on disk. Desktop already publishes the other bots through `bot_relay.roster.sync`, but the Rust gateway answered that method with an empty boot-stub response until Python started. Those pushes were discarded. Desktop also waited up to 60 seconds for its next periodic roster push.

The existing gateway `bot_message` module now handles roster sync natively and atomically persists the existing `bot_relay/roster.json` contract. It validates identifiers, normalizes and bounds text, deduplicates by connection/profile, preserves boolean liveness, returns the accepted count, and reports write failures with the existing 5090 error code. This works both before and after Python starts. The existing reminder and native `message_agent` paths already read that file on each call, so they now share the refreshed roster without restarting engines or exposing sibling profile files.

Creation awaits an immediate roster push. If an older sync already captured its snapshot, creation waits for it and performs a fresh read. A new bot's intro refreshes again after its engine starts. The existing relay treats local sandbox and Docker computers alike and leaves stopped computers asleep during refresh. Unknown-target errors now include pushed peers and exclude the sender. The reminder intentionally excludes the bot itself from its teammates.

The existing relay envelope, delivery, reply wake, tool name/schema and engine core are unchanged. No new dependency or architecture was introduced. The Rust changes are confined to gateway dispatch, the obsolete boot-stub arm and the existing additive bot messaging module. Python production code is unchanged.

## Reproduction and verification

Before the fix, the engine-level regression creates Alpha's private context and running gateway connection, then supplies a Beta that exists only through the owner's roster push. It fails with accepted count 0 instead of 1. After the fix, Beta resolves, the queued envelope targets `computer-beta`, attribution names Alpha, and the next reminder includes Beta. Clearing the roster removes Beta. The same test checks warm-backend routing, private file permissions and a typed write failure.

The normalization golden was captured by running the existing Python roster writer in an isolated home. Both Rust and Python test against it, including malformed rows, whitespace, bounds, uppercase identifiers, duplicate-first ordering and liveness.

All commands below ran through `/private/tmp/factri-roster-85uqy2nn/run.py`, which gives each command fresh HOME, FACTR_HOME, FACTR_CONFIG_HOME, XDG directories, temporary files and Electron userData. Cargo used only this worktree's `target/` and a task-local public registry cache. Exact commands, exits, durations and isolation roots are in `runs.jsonl`; full outputs remain at the listed paths.

| Check | Result | Log |
| --- | --- | --- |
| Engine regression before fix | Expected failure, count 0 vs 1 | `/private/tmp/br-855befnb/output.log` |
| Desktop immediate-refresh regression before fix | 44 passed, 1 expected failure | `/private/tmp/br-p979d914/output.log` |
| Full gateway crate, serial | 416 passed across 21 unit/integration/doc targets, 1 manual measurement ignored; 126.71 s | `/private/tmp/br-8nfynpwl/output.log` |
| Native messaging tests after restoring source from baseline control | 3 passed; 12.96 s | `/private/tmp/br-dvx0w0_7/output.log` |
| Python affected suites | 148 passed, 1 Windows-only skip on macOS; 16.84 s | `/private/tmp/br-ruu2hud1/output.log` |
| Bots plugin UI | 96 files, 781 passed; 28.75 s | `/private/tmp/br-7i2n515a/output.log` |
| Full Electron tests, including pool/routing | 231 files, 2484 passed, 19 existing skips; 31.52 s | `/private/tmp/br-ws8hduxw/output.log` |
| Renderer, Electron and E2E TypeScript | Passed; 19.37 s | `/private/tmp/br-5x6hzszh/output.log` |
| ESLint on every touched desktop TS/TSX file | Exit 0, no errors, 30 spacing warnings in compact code/test formatting | `/private/tmp/br-azjf6k6c/output.log` |
| Real production sandbox launcher plus two actual engines | Passed; 20.97 s | `/private/tmp/br-6iwarbe8/output.log` |
| First report-inclusive name gate | Report paths rejected, then normalized; final rerun below | `/private/tmp/br-ccnfwj61/output.log` |
| Final report-inclusive name gate | Passed, zero hits | `/private/tmp/br-874lol_w/output.log` |

The real-engine run starts Alpha before creating Beta, sends the roster update before any Python drain, and checks the next-turn reminder, native queued handoff, attributed delivery, Beta's reply and Alpha's reply wake. It also passes the existing shell/file isolation, scheduled-turn/output, pause/restart/resume and canonical-chat persistence checks. It drives backend APIs only. Synthetic model requests are preserved at `/private/tmp/factri-roster-85uqy2nn/sandbox-seen.json`.

The initial UI suite needed mocks for the newly imported relay dependency; those fixtures were corrected and the full affected suite passed. An initial Electron run could not import its runtime because dependency install scripts were disabled; installing the worktree-local runtime resolved it without launching Electron. The initially misplaced import was corrected before the final lint pass. The report-inclusive name gate rejected personal filesystem paths; only report paths were made portable before rerunning it.

## Known validation limitation

A four-thread gateway run fails the unchanged `rpc::undo_turn::tests::the_turn_snapshot_is_lazy_and_undo_restores_from_it` assertion at `undo_turn.rs:604`: `no files restored (no record of when that turn began)`. This is inherited, not introduced by the roster change:

- Current source: 387 passed, 1 failed, 1 ignored, `/private/tmp/br-c36e72az/output.log`; repeated in `/private/tmp/br-ls2qf2hl/output.log`.
- Excluding the new roster regression: the same failure, 386 passed, 1 failed, 1 ignored, `/private/tmp/br-k4iu4yhq/output.log`.
- Control with all three Rust source files temporarily restored to the pre-task commit: the same failure, 386 passed, 1 failed, 1 ignored, `/private/tmp/br-mf6lfusw/output.log`.

The committed roster source was then restored byte-for-byte and all three native messaging tests passed. The complete serial gateway crate run passes, including that undo test. No unrelated undo change or test skip was added. The inherited parallel failure remains for a separate task.

## Remaining QA and scope

No requested roster implementation is half-done and no user blocker remains. Packaged UI, actual Docker containers, native Windows/Linux, real credentials/providers and the broader release matrix were not exercised. Existing Electron skips are eight opt-in Docker cases, one opt-in live retirement case and ten WSLg process cases on macOS. Exact manual scenarios are appended to `NEEDS-GUI-QA.md`.

The supplied design law was re-read and checked point by point for applicability. This change adds no rendered markup or styling: typography, colors, icons/logos, surfaces, shadows/glass, hero/section/footer compositions, borders, clipping/gutters, comparison alignment, centering, textures and motion remain unchanged. Content visibility and existing controls remain intact; creation and intro behavior are covered by renderer tests and real backend delivery. No pixel, pointer or visual certification is claimed under the explicit no-GUI instruction.

Graph tools were tried first. This worktree has no graph index; the nearest sibling is generation `2026-10-12T11:52:27Z`, with missing new Rust files, excluded tests/Python/E2E and partial create-dialog coverage. Coverage was checked for every touched evidence path; current local source, diffs and runnable tests supplied the verification. No completeness claim relies on that stale sibling graph.

## Resume commands

From this worktree, using the retained isolated runner:

```sh
rtk proxy mkdir -p /private/tmp/factri-roster-85uqy2nn/cargo
rtk proxy python3 /private/tmp/factri-roster-85uqy2nn/run.py engine cargo test -p factr-gateway --locked -- --test-threads=1
rtk proxy python3 /private/tmp/factri-roster-85uqy2nn/run.py engine cargo build --locked --bin factr
rtk proxy python3 /private/tmp/factri-roster-85uqy2nn/run.py . desktop/node_modules/.bin/tsx engine/crates/factr-gateway/e2e/bot-local-sandbox.mts
rtk proxy python3 /private/tmp/factri-roster-85uqy2nn/run.py backend .venv/bin/python -m pytest tests/tools/test_bot_relay.py tests/tools/test_message_agent_mcp.py tests/tools/test_bot_mode_dm.py tests/tools/test_bot_mode_probe.py tests/tui_gateway/test_bot_relay_methods.py -q -rs
rtk proxy python3 /private/tmp/factri-roster-85uqy2nn/run.py desktop/app node ../node_modules/vitest/vitest.mjs run --project ui src/plugins/factr-bots --maxWorkers=4
rtk proxy python3 /private/tmp/factri-roster-85uqy2nn/run.py desktop/app node ../node_modules/vitest/vitest.mjs run --project electron --maxWorkers=4
rtk proxy python3 /private/tmp/factri-roster-85uqy2nn/run.py desktop/app npm run typecheck
```

Cargo repopulates its isolated registry from public packages when resumed. Dependencies are retained in the worktree. Build output, task-local caches and generated per-run scratch were removed after confirming no own engine remained active; logs, runner, ledger, the prior report and synthetic model requests are retained. No Docker resource was created. Free space after cleanup: 54 GiB.

Report copy: `handoff/bots-roster-REPORT.md`.
