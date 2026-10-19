# Factr-I 0.0.10 integration checkpoint

## Setup and scope

- Read common rules, parent advice, and the entire integration handoff in order. Read the bots branch report, checkpoint and GUI QA matrix.
- Branch starts clean at 39ca7e9f. Merge order: scrollbar-overlay, rust-ram, opencode-auto, bots-step2. Local commits only, no push, version change, tag, benchmark, packaged app or GUI driving.
- Scope includes branch changes, shared gateway registrations/contracts, renderer and Electron callers/tests, changelog and QA documentation. Engine core stays unchanged. README stays unchanged.
- Disk: 74 GiB free before setup. One Cargo target at worktree root. Dependencies are installed within this worktree.
- Test runner: /private/tmp/integration-run.py. Every invocation creates fresh HOME, FACTR_HOME, FACTR_CONFIG_HOME, XDG and Electron userData directories; npm and Cargo caches are isolated under /private/tmp/integration-tool-state. Environment is allowlisted, not copied wholesale. Logs and exact commands accumulate in runs.jsonl there.
- Graph tier: Verify. No index for this integration worktree. Nearest Rust branch graph generation 2026-10-18T12:52:21Z; branch differences and missing scrollbar symbols require exact local source fallback. Coverage checked for all eight scrollbar files; styles.css has parser gaps and was read directly. No blanket index that might inspect credential files.
- Scrollbar branch merged with --no-ff --no-commit, no conflicts. Read the complete staged source diff. Dependency installation and validation pending.

## Advice

- Parent advice says continue; no STOP. No architectural decision needed for the first merge. Missing advice will be recorded in QUESTIONS-integration.md without stopping.

## Next

1. Validate scrollbar merge and commit it.
2. Merge and validate Rust RAM, then OpenCode, then bots.
3. Full UI/Electron and gateway tests, naming gate, final QA matrix, cleanup and report copy.

## Milestone 1: scrollbar merge validated

- No conflicts. Changelog unreleased section created. No Rust files changed.
- Isolated npm ci: 1191 packages, exit 0 (32.99s).
- From desktop: node node_modules/typescript/bin/tsc -p app --noEmit: exit 0.
- From desktop/app: ../node_modules/.bin/eslint src/app/chat/sidebar/index.tsx src/app/chat/sidebar/nav-model.tsx src/app/command-palette/index.tsx src/lib/icons.ts src/lib/scroll-activity.test.ts src/lib/scroll-activity.ts src/main.tsx: exit 0.
- vitest run --project ui src/lib/scroll-activity.test.ts src/app/chat/sidebar src/app/command-palette --maxWorkers=4: 41 files, 275 passed.
- vitest run --project electron electron/pool-retire.test.ts electron/pool-retire-http.test.ts --maxWorkers=4: one existing file, 2 passed. pool-retire-http is added by the later bots branch. Initial electron/platform filter matched no files (exit 1); corrected, not a product failure.
- bash scripts/gate-names.sh: OK, zero hits. git diff --check: exit 0.
- Logs: /private/tmp/integration-check-{yo6timy_,4uqpx171,0mx3w8s9,r7610r54,otnb3h2n,el27panl}/output.log.
- Design-law source review: imported scrollbar/icon changes preserve the existing interface. Pointer/visual checks deferred to GUI QA as expressly requested.
- Next: merge Rust RAM and preserve its QA notes.

## Milestone 2 validation in progress

- Rust RAM source merged without conflicts. CHECKPOINT.md add/add keeps this integration record; original branch checkpoint/report archived under /private/tmp/integration-tool-state (branch history also preserves them). Root NEEDS-GUI-QA.md is preserved.
- New pane-mirror test lacked the minWidth now required by this base. Added only minWidth to its fixture. Targeted renderer suite: 8 files, 58 passed; fixture rerun: 1 passed. TypeScript rerun exits 0 after the fix. All 17 touched renderer files pass ESLint. Electron pool smoke: 2 passed.
- First naming gate caught only personal paths in duplicated archive notes; moved those notes outside the source tree. Full naming gate rerun: OK, zero hits. No gate suppression or product rename.
- Gateway full serial tests and real Python golden parity check running. Isolated backend venv installed from locked dev export, no shared environment changed.

### Milestone 2 validated

- cargo test -p factr-gateway --locked -- --test-threads=1: exit 0, all suites pass (229.78s, including cold compile). Exact log: /private/tmp/integration-check-dwkkrgl8/output.log.
- backend .venv/bin/python -m pytest tests/factr_backend/test_ui_route_goldens.py -q: 1 passed, validates all 22 Python cases plus embedded JSON; /private/tmp/integration-check-mf7sei4h/output.log.
- Fixture compatibility fix, changelog and scrollbar QA notes included. No engine core or README changes. Next: OpenCode merge.

## Milestone 3: OpenCode validation in progress

- No conflicts; additive RPC module, provider catalog and both onboarding/model-menu entry points preserved. Exact source reviewed because the sibling graph lacks this module; coverage checked for all 14 changed paths.
- TypeScript exit 0. Onboarding/model-picker UI: 12 files, 79 passed. Electron pool smoke: 2 passed. Contract generator --check: exit 0, all three generated artifacts current. Name gate: OK, zero hits.
- ESLint found one unsorted imported name in onboarding/index.tsx; fixed that import only. All six touched renderer files pass the rerun. No credentials read or adoption attempted. OpenCode native tests running.
- Added exact OpenCode/manual-platform scenarios to NEEDS-GUI-QA.md and its entry to the unreleased changelog.

### Milestone 3 validated

- cargo test -p factr-gateway --locked opencode -- --test-threads=1: 4 passed, exit 0 (49.19s including compile). Log /private/tmp/integration-check-bzypci49/output.log.
- All milestone checks pass after the import-order correction. Next: bots last, retaining both native UI reads and OpenCode registrations/contracts.
