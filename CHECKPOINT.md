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
