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

## Milestone 4 validation in progress: bots merge

- Only conflicts: CHECKPOINT.md and NEEDS-GUI-QA.md (both add/add). This checkpoint retains integration history; the original bots checkpoint/report are archived in /private/tmp/integration-tool-state. NEEDS-GUI-QA.md concatenates the integration/Rust list and the complete original bots matrix, preserving both.
- Shared gateway lib.rs/rpc.rs registrations and generated contracts auto-merged. Inspected dispatch hooks, OpenCode registration, native UI route, feature reconnection and pet predicate. Contract generator --check passes after combined merge. Renderer and Electron TypeScript both pass.
- Sibling graph lacks the new bots modules; structural search and coverage attempted for every merged path, current exact source/diffs used for shared hooks and new module entry points. No claim of current whole-worktree graph coverage.
- Inherited lint: 77 errors, mostly braces/import ordering, plus unused test import and two redundant escapes. Applied only required error fixes, leaving spacing and hook warnings visible. One retired name in a source comment changed to compatible peer gateways. No runtime naming/schema change.
- Full UI/Electron run, full serial gateway tests and affected backend tests running.
- Runner correction: TMPDIR initially inherited the system temporary folder, so some test-created scratch used the OS temp location. New invocations pin TMPDIR/TMP/TEMP beneath each fresh /private/tmp isolation root. HOME/FACTR/userData were isolated throughout; no credential access.

### Combined validation findings

- Full serial gateway suite: 414 passed, 1 ignored, exit 0 (142.54s); /private/tmp/integration-check-svbvvkdl/output.log.
- Full touched-file ESLint: exit 0, zero errors; 176 inherited warnings (175 spacing, one hook-dependency warning) remain deliberately outside behavior changes. Full name gate rerun: OK, zero hits.
- Backend affected suites: 83 passed, 1 failed. Failure was a stale test assuming every CLI prefix is Python -m, while this machine resolves a standalone executable. The runner itself was mocked and no real child ran. Parameterized that test over both supported prefixes and kept profile validation, alias mapping and exact transport/cap assertions. Rerun in progress. No runtime change.
- Corrected graph coverage batch limit: original 140-path call was rejected; split into 100+40 paths. Generation remains sibling-only; new modules missing and create-dialog parser ranges require direct source, inspected where relevant.
- Building the integrated test-profile engine next for real feature-idle reconnect and canonical bot smoke, never the packaged app.

- Backend transport fixture rerun: 26 passed, exit 0, both Python and standalone prefixes. Other 59 affected backend tests passed in the initial run.
- Full desktop run hit the unchanged updater slow-launch test at the default five-second test deadline (5.156s). Source fixture deliberately sleeps four seconds, then performs shell polling/process checks. Isolated unchanged file passed all 24 tests, slow case 4.844s. Set only that test deadline to 15s, preserving all product assertions and the script deadline. Full Electron rerun started; no updater runtime change.

- Full UI project: all 987 files / 8217 cases passed in the combined run. The command failed only in Electron: the five-second updater deadline above and two suites unable to import the skipped Electron binary. The next Electron run passed all 2460 loaded tests but still lacked those two imports. Installing the worktree-local Electron binary to complete those checks, without launching it. Initial installer command used the wrong workspace path and failed harmlessly; corrected to app/node_modules/electron/install.js.
- Integrated engine canonical registry smoke: all checks pass (13.56s). Persistent feature socket reconnects across two actual Python idle exits: pass (27.70s). Both use synthetic isolated state. Renderer production bundle: pass (17.82s). E2E TypeScript config: pass.

### Final desktop checks

- Full Electron after local dependency setup: 231 files passed, 3 skipped; 2484 tests passed, 19 skipped, exit 0 (33.94s), /private/tmp/integration-check-6vo_3goy/output.log. Skips: 8 opt-in real Docker tests, 1 opt-in live retirement test, 10 WSLg process tests on macOS. No skipped check counted green.
- Full UI: 987 files / 8217 passed in the combined run; its failures were exclusively Electron and are resolved by the final Electron run. Post-lint affected rerun: 140 passed / 8 opt-in Docker skipped.
- Final renderer, Electron and E2E TypeScript all pass. Combined touched-file ESLint exits 0 with 187 inherited warnings (186 spacing, one hook dependency). No warnings suppressed in the final lint.
- Final contract generation check, shell/JS syntax, full naming gate and complete diff whitespace check pass. Removed one imported trailing blank line from computer/README.md; root README is unchanged.
- Confirmed the full bots GUI-QA document is preserved verbatim as the suffix of NEEDS-GUI-QA.md. No non-gateway engine crate source changed. Disk before cleanup: 67 GiB. No engine process from this target remains.

## Milestone 4 validated

- Repeated all affected backend suites after the fixture fix: 85 passed, exit 0 (52.50s), /private/tmp/integration-check-chijm0n_/output.log. Includes the full 22-case Python UI golden comparison.
- All required integration checks now pass at their documented scopes. Gateway ignored test is observability::tests::measure_overhead (manual release measurement); no benchmarks were run.
- No architectural advice fork was needed. Parent guidance remains continue, no STOP. Next: final detailed source/design review, report, cleanup and report copy.

## Final report, review and cleanup

- Bots merge committed as 8fc57953; parent advice re-read afterward, no STOP. All four requested branch tips are ancestors in the specified first-parent merge order. Every integration commit has the required co-author trailer.
- Re-read the entire supplied design law, checking identity/type, color/atmosphere, icons/marks, surfaces/glass/shadows, ornaments, page compositions, clipping/gutters/centering, responsive alignment, motion/reduced motion/visible defaults, labels/copy, interactions and specificity. Integration retains the existing application design; no new marketing composition or visual system. New source remains visible without optional chunks; sidebar spacing and scrollbar motion are explicit. Pixel contrast/centering/clipping, actual pointer controls and platform behavior remain unverified under the explicit no-GUI instruction. Detailed boundaries and exact scenarios are in REPORT.md and NEEDS-GUI-QA.md. No visual certification claimed.
- REPORT.md includes merge order/conflict decisions, all validation results, resolved failures, exact chronological command/log ledger, inherited half-done upstream scope and resume commands. Copied verbatim and byte-compared to integration-REPORT.md in the authorized handoff directory.
- Removed only generated worktree target/, desktop/app/dist/ and tsconfig.e2e.tsbuildinfo after checking no tracked files, external directory symlinks or own engine/backend processes. Dependencies and isolated logs/cache/runner retained for QA. No Docker resources created, no benchmark run.
- Root README, package versions, Cargo.lock and non-gateway engine crates match the starting commit. Complete diff whitespace check passes. Report-inclusive name gate passes with zero hits. Final source milestone checks: UI8217, Electron2484 (19 skipped), gateway414 (1 manual measurement ignored), backend85.
- No remaining integration blocker or open architectural question. Manual/release limits are retained, including desktop-owned group rounds, Windows/native-Linux QA, real credential/provider paths, inherited browser/pet issues and the full GUI matrix. No push.

- Post-cleanup disk: 71 GiB free. Final report copy verified byte-for-byte.

## Parent: merged feat/0.0.10-i18n (wording and translation leftovers); its checkpoint/report are in handoff/i18n-REPORT.md

## Merge2: flaky engine test merge

- Started clean at 11e5b0ee on feat/0.0.10-integration; merged all seven fix/flaky-engine-tests commits with --no-ff. No conflicts occurred, so restored the original integration checkpoint/report and appended the flaky report under Flaky engine test fixes. Reviewed the complete six-file Rust diff: all changes are in test modules. No production port or UI change.
- Read COMMON-RULES.md, ADVICE-FROM-PARENT.md, merge2 handoff, RTK and Ponytail instructions. Parent says continue, no STOP. No advice fork needed. Verify tier: this worktree is not indexed; read the exact merge diff directly. Disk before builds: 71 GiB free. Evidence: /private/tmp/factri-merge2-etue5ry_.
- Next: three locked 64-thread workspace runs in sequence, desktop TypeScript/UI/Electron sanity checks, name gate, changelog, whitespace, local commit and final report copy. No GUI, packaged app, Docker, benchmark or push.

### Merge2 advice and early checks

- Production-prefix assertion passes for all five Rust test-module files; bash_tests.rs remains included only under cfg(all(test, not(windows))).
- First name gate fails solely on four matches in three personal paths copied from the flaky report. Replaced only these paths in the appended section with portable placeholders; archived verbatim source under the evidence directory. Question recorded in handoff/QUESTIONS-merge2.md, continuing with safest judgment. No source suppression or gate change.
- Test runner reuses the prior integration isolation pattern with an allowlisted environment, fresh HOME/FACTR/config/TMPDIR/userData per command, task-local Cargo cache, CARGO_BUILD_JOBS=4 and the only target at target/.

### Merge2 harness correction

- First full workspace attempt: exit 101, 359.55s, log /private/tmp/factri-merge2-etue5ry_/merge2-check-zqrg6die/output.log. Four failures across gateway projects_store and harness-api-server are InvalidInput: path must be shorter than SUN_LEN. The runner nested its scratch beneath the evidence directory, exceeding macOS Unix-socket path limits. All merged test modules passed.
- Corrected only the temporary runner to use fresh short /private/tmp/m2-* roots, still isolated for every command and still one target/. No repository source fix or skip. Restarting the required three consecutive full workspace runs; the failed attempt remains evidence.

- Corrected workspace run 1: cargo test --workspace --locked --no-fail-fast -- --test-threads=64, exit 0, 143.64s, /private/tmp/m2-uq03wfyw/output.log. All four prior socket-path failures pass; no source changes. Run 2 started with fresh isolation.
- Corrected workspace run 2: identical command, exit 0, 148.60s, /private/tmp/m2-2etxzwad/output.log. Run 3 started with fresh isolation.

## Merge2 milestone 1 validated: three consecutive full workspace passes

- Third run: identical locked 64-thread workspace command, exit 0, 143.57s, /private/tmp/m2-ow6nxcz0/output.log. Final streak is 143.64s / 148.60s / 143.57s, all green. Each completed all 147 Cargo targets. Main unit suites each run: app-core 1108 passed / 4 ignored, base 1366 / 3, gateway 387 / 1. Existing opt-in ignores unchanged, no retries or skips added.
- Exact command for each: rtk proxy python3 /private/tmp/factri-merge2-etue5ry_/run.py engine cargo test --workspace --locked --no-fail-fast -- --test-threads=64. Full command/environment ledger: runs.jsonl and run.py in that evidence directory.
- The runner-path failure is documented above and excluded from the streak. All four affected socket tests pass in every corrected run. No repository source adjustments were needed. Name gate passed after portable report paths; git diff --check passes. Disk: 66 GiB.
- Next: amend this task's merge commit to preserve integration records with the required co-author trailer, then desktop TypeScript/UI/Electron sanity, changelog, final report and cleanup.

- Merge milestone committed locally as dafd673e (two parents retained; required co-author trailer present). Parent advice re-read, no STOP or new merge2 answer.
- Desktop typecheck: npm run typecheck from desktop/app through the isolated runner, exit 0, 18.00s, /private/tmp/m2-4dct4x_7/output.log. Includes renderer, Electron and E2E tsc --noEmit. Full UI project started with four workers.

### Desktop sanity findings

- First full UI run: exit 1, 235.66s, 987 files / 8228 tests, 985 files and 8224 tests passed; four failures in two files. Log /private/tmp/m2-ic_4wipq/output.log. Capabilities scope and three MoA progress assertions still expected em dashes after the already-merged wording branch changed the rendered copy to hyphens. Confirmed exact received strings and the 3da2131a..11e5b0ee source diff. Updated only the five stale expected strings; no production UI change.
- Graph follow-up: nearest factri-main sibling generation 2026-10-12T11:52:27Z, different worktree. Coverage checked all six Rust paths, docs and the relevant UI paths. UI tests/docs are excluded or missing there; exact local test sources and wording diffs are authoritative. No stale graph claim or reindex.
- Re-running the complete UI project after the assertion-only fixes, then Electron once. Rust source is unchanged from the three-run streak.

- Full UI rerun after only five expectation fixes: exit 0, 233.50s, all 987 files / 8228 tests pass, /private/tmp/m2-c60z_jj7/output.log. Complete Electron project started next with four workers; no GUI launched.

## Merge2 milestone 2 validated: desktop sanity

- Electron full project: exit 0, 30.32s, 231 files passed / 3 skipped, 2484 tests passed / 19 skipped. Log /private/tmp/m2-8xh2v4kw/output.log. Skips are unchanged: 8 opt-in Docker, 1 opt-in live retirement, 10 WSLg process tests on macOS. No GUI or Docker run.
- Exact desktop commands from desktop/app, via the isolated runner: npm run typecheck; node ../node_modules/vitest/vitest.mjs run --project ui --maxWorkers=4; node ../node_modules/vitest/vitest.mjs run --project electron --maxWorkers=4. All final checks exit 0.
- Name gate after test fixes: bash scripts/gate-names.sh, exit 0, 2.68s, /private/tmp/m2-ydjeb6v6/output.log. git diff 11e5b0ee --check: exit 0.
- Added one unreleased changelog line for engine test isolation fixes. No production change, dependency, version, lockfile or README edit. Existing NEEDS-GUI-QA.md is unchanged. Parent guidance re-read after desktop milestone, no STOP or new merge2 answer.
- Next: clean this task's generated target/cache/scratch, finish and copy the report, run report-inclusive final gates, commit the fixture/docs follow-up and confirm clean status.

## Merge2 final report and cleanup

- Removed only this task's target/, isolated Cargo cache, per-run scratch and untracked TypeScript build-info caches after verifying no tracked target files or active target executables. Logs, run.py, runs.jsonl, check-test-only.py and original report copies remain. Free space returned to 71 GiB.
- REPORT.md now leads with merge2 results, both resolved validation failures, full command/log ledger, unchanged-source proof, cleanup, limits and repeat commands; prior integration history and final Flaky engine test fixes section preserved. No requested work remains half-done.
- Design-law applicability re-check: no rendered UI change, only expected-string alignment in tests. Existing GUI matrix unchanged; no visual or pointer certification. Final report-inclusive gates and report copy verification next.

- Final report-inclusive name gate: exit 0, zero hits, 2.45s, /private/tmp/m2-wvbxqfgo/output.log. Full baseline-to-worktree whitespace check and production-prefix assertions pass. Final scope assertion matches exactly six Rust test files, two UI test fixtures and three documentation files.
- Integration report body remains intact; final source-report appendix differs only in the documented three path substitutions. REPORT.md copied to handoff/merge2-REPORT.md and byte-compared successfully. All requested merge2 checks complete; no blocker or half-done work. No push.

## Bots-create: scope and diagnosis (2026-10-19)

- Read COMMON-RULES.md, ADVICE-FROM-PARENT.md and bots-create-handoff.md completely, plus RTK, Ponytail and Codebase Memory instructions. Branch fix/bots-second-create started clean. No graph tools are callable in this session, so Verify uses exact local source and caller searches; graph project/generation/coverage cannot be certified.
- Root cause: SDK host.request reads the live $gateway; createCanonicalChat activates the created bot, so the next dialog's requestForTarget sends profiles.create to that bot's isolated config home. This is intended SDK session routing, not a primary pool ownership failure. data.ts already uses an explicit local default-profile route for roster reads after a bot activates.
- Scope: reuse the same explicit source route in the dialog's shared requestForTarget for local management RPCs after bot focus; retain remote and legacy routing and session routing. Cover consecutive creates, actual isolated filesystem destinations, root-route failure, existing lazy/MCP/remote-target behavior. No engine, dependency, layout or version changes planned.
- Disk: df -h / reports 63 GiB available. No builds requiring Rust or Docker. No credential files read. No GUI or packaged app permitted.
- Next: add regression and record its pre-fix failure; apply smallest fix, run affected UI/Electron projects, tsc and touched-file lint, commit locally, finish REPORT.md and copy it to handoff/bots-create-REPORT.md.

### Advice

- Parent guidance has no STOP. No architectural change needed: follow the existing explicit root-profile routing pattern. Missing advice will be recorded in QUESTIONS-bots-create.md if needed, never used as a reason to stop.

### Bots-create regression reproduced before the fix

- No dependencies were installed initially (first command failed MODULE_NOT_FOUND). Ran isolated npm ci --ignore-scripts --no-audit --no-fund with the existing lockfile: 1191 packages installed, exit 0. No lockfile/dependency edit.
- Pre-fix command from desktop/app: node ../node_modules/vitest/vitest.mjs run --project ui src/plugins/factr-bots/create-agent-lazy-profile.test.tsx --maxWorkers=1, through /private/tmp/bc-xx9sync2/run.py. Exit 1: exactly 3 new regressions failed, 7 existing tests passed. Both consecutive-create variants wrote Beta beneath Alpha's sandbox config; root-backend failure was ignored because the request went to the active bot. Log: /private/tmp/bc-xx9sync2/regression-before.log.
- Applied the existing local-root pattern to the shared requestForTarget helper only. No pool/session/core routing change. Remote-target routing remains the first choice, legacy SDKs keep their existing path. A routed root error is propagated, never retried into the sandbox.
- Next: focused green check, then local milestone commit and full affected checks.

## Bots-create milestone 1 validated

- Post-fix focused suite: all 10 tests pass, including consecutive Alpha/Beta creates through the button and lazy Capabilities path, filesystem destination assertions, visible root-route error with no sandbox fallback, and remote targeting while Alpha is focused. Final focused run: 6.99s, exit 0, /private/tmp/bc-xx9sync2/focused-final.log. Existing React act warnings remain in this test harness; no unhandled rejection or test failure.
- npm run typecheck: renderer, Electron and E2E tsc --noEmit all pass, exit 0 (19.56s), typecheck.log. Touched-file ESLint: exit 0, zero warnings/errors (0.97s), lint-final.log. Two new test spacing warnings from the initial lint were corrected.
- Test commands use an allowlisted environment with fresh /private/tmp/bc-run-* HOME, FACTR_HOME, FACTR_CONFIG_HOME, TMPDIR and FACTR_DESKTOP_USER_DATA_DIR for each command. Logs/runner/command ledger: /private/tmp/bc-xx9sync2.
- Parent advice re-read: no STOP or bots-create-specific correction. Next: commit this validated fix, run the full UI and Electron projects (includes bots, routing and pool tests), document manual QA without driving the GUI, finalize report and cleanup.
