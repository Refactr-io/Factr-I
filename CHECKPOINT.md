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
