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


# Flaky engine tests task (2026-10-19)

## Scope and setup
- Read COMMON-RULES.md, ADVICE-FROM-PARENT.md and flaky-handoff.md completely, in order. Task is test-only; unrelated parent bots/RAM remaining-work notes do not widen it. No push, GUI, benchmark, production behavior change or credential-file access.
- Clean starting commit 11e5b0ee, branch fix/flaky-engine-tests. Initial disk availability 71 GiB. Single Cargo target: engine/target; one build at a time.
- Isolated runner: /private/tmp/factri-flaky-l0nrzadt/run.py. Every invocation creates fresh HOME, FACTR_HOME, FACTR_CONFIG_HOME, TMPDIR, XDG_CONFIG_HOME and Electron userData directories. Environment allowlisted; shared Cargo cache cloned into the temporary runner root without reading credentials.
- Graph Verify tier: no graph for this worktree. Nearest factri-main graph generation 2026-10-12T11:52:27Z is an older, different checkout. Discovery/coverage checked for engine; exact local source required for all claims and changes. No indexing of credential files.
- Reproduction underway: cargo test --workspace --locked -- --test-threads=16 (before-01). Cold compile in progress. Exact outputs retained under the runner root.
- Read-only CI evidence: run 38178832627 macOS /usage test failed with Today zero versus session one run; run 38144763796 Linux slash-popup test failed with Null versus true. These logs are older than this branch, whose tests already include polling/locking corrections, so they are leads rather than proof of current defects. Logs in /private/tmp/factri-flaky-ci-*.log.

## Advice
- Parent file says continue, no STOP. No architecture change or design fork so far. Claude CLI not called.

## Next
- Complete repeated pre-fix workspace runs, identify current failing tests, make separate minimal test-only fixes and validate each. Then five consecutive clean full workspace runs, final report and cleanup.

## Flaky milestone 1: pre-fix reproduction complete
- before-01: full workspace attempt, 16 threads, fail-fast; exit 101 in 212.4s. App-core risky-delete fixture failed; Cargo stopped before base/gateway.
- before-02: full workspace, --no-fail-fast, 16 threads; exit 101 in 213.7s. Same fixture failed; all other targets including gateway 414/414 passed.
- before-03: full workspace, --no-fail-fast, 64 threads; exit 101. Risky-delete fixture, passing_gate_records_skipped_count_on_the_guard_span_without_nudging, streamable_http_uses_factr_oauth_token_for_discovery_and_calls and a_pre_tool_hook_can_rewrite_the_arguments_and_the_gate_reports_it failed. Gateway a_session_grant_covers_only_its_own_risk_category hung with every other gateway unit test finished. Stopped ONLY that child binary via SIGTERM after 114s (PID/parent/path verified); Cargo continued all remaining targets.
- Measured rates: workspace attempts failed 3/3; risky-delete fixture 3/3; stop-nudge recorder assertion 1/3; MCP OAuth and hook-rewrite 1/2 executions each; approval hang 1/2 gateway executions. Do not conflate fail-fast-unreached tests with passes.
- Confirmed test causes: fixture assumes HOME is outside /tmp; two stop-nudge tests replace the recorder outside the lock held by fallback tests; MCP OAuth test mutates FACTR_CONFIG_HOME without the shared lock, colliding with locked hook tests; approval tests share config and one saves cargo publish into the permanent allowlist while another expects its prompt. No production bug established.
- Advice read again: no STOP or task-specific correction. Asked parent in QUESTIONS-flaky.md about smallest approval config isolation guard versus subprocesses; proceeding with existing ENV_LOCK and test-local RAII restoration.
- Next: separate validated fixes for fixture, MCP env lock, recorder lock and approval config/receives. Five consecutive clean full suites after the final fix.

## Flaky milestone 2: risky-delete fixture fixed
- The bash test now holds the existing environment lock and creates its unique disposable target under CARGO_MANIFEST_DIR, not mutable HOME. Its three behavioral assertions are unchanged: first attempt held, blind retry held, justified command executes. No production changes.
- Validation: isolated run fix-fixture, cargo test --workspace --locked -p factr-app-core --lib tool::bash::tests:: -- --test-threads=64: exit 0, 47 passed (12.1s including rebuild). The --workspace selector runs the filter across workspace libraries; only app-core matched. git diff --check passes.
- Advice reread: unchanged, no STOP. Next: serialize the MCP OAuth environment mutation.

## Flaky milestone 3: MCP config race fixed
- Added the existing storage::lock_test_env guard to the HTTP OAuth MCP test for its whole fixture/connect/call/restore lifetime. This is the unguarded FACTR_CONFIG_HOME writer that collided with factr_hooks Home (which already uses the same lock). No production code changed.
- First validation command incorrectly used libtest --skip with crate names, so it did not exclude app-core. It stopped on the still-unfixed turn_end_span_reports_how_the_turn_ended recorder race before reaching base. This is additional evidence for the recorder fix, not an MCP regression.
- Corrected validation: isolated fix-mcp-02, cargo test --workspace --exclude factr-app-core --exclude factr-gateway --locked --lib -- --test-threads=64: exit 0 (129.9s including feature-selection rebuild); base 1366 passed, 3 ignored. Includes both previously failing MCP OAuth transport and hook rewrite. All selected workspace libraries passed. git diff --check passes.
- Advice reread: no STOP or new answer. Next: serialize recorder installers with their existing test locks.
