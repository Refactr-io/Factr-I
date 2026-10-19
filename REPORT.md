# Factr-I 0.0.10 integration report

All four authorized branches are merged and locally committed in order. Integration is complete at the automated scope below; manual/platform release QA remains open. No push, remote change, tag or version bump. Root README and all non-gateway engine crates are unchanged. No real credential files were read or copied. The release checkout and benchmark results were untouched.

## Merge order and conflict decisions

| Order | Branch / merged tip | Local merge commit | Conflicts and decision |
| --- | --- | --- | --- |
| 1 | feat/scrollbar-overlay / 391058c9 | 08e459a3 | None. Overlay scroll activity, flush sidebar and history icon retained. |
| 2 | feat/rust-ram / 78cc0144 | f6a9a3b5 | CHECKPOINT.md add/add: keep integration history; original checkpoint/report archived in the temporary evidence directory and preserved in branch history. Native reads, guarded pet forwarding, bounded caches, lazy math/disclosures, muted haptics and pane identity retained. |
| 3 | feat/opencode-auto / 509d9955 | 8d3ec44f | None. OpenCode detection, catalog and connection flow retained in native RPC and both UI entry points. |
| 4 | feat/bots-step2 / 99f76326 | 8fc57953 | CHECKPOINT.md add/add handled as above. NEEDS-GUI-QA.md add/add resolved by concatenating the integration/Rust QA list and complete original bots matrix. A byte comparison confirms the original bots document is intact. |

Shared auto-merges were inspected in gateway lib.rs/rpc.rs, Electron connection-config/main, locale types/text and all three generated contract artifacts. Native dashboard reads and pet routing coexist with bot admission, tool registration, Stop and feature reconnection. OpenCode registration remains present. Contract generation --check passes without regenerated differences. No feature was removed to resolve a conflict.

docs/CHANGELOG.md has one v0.0.10 (unreleased) section covering all four branches. App and engine versions remain 0.0.9; other package versions are unchanged.

## Final verification

| Check | Result |
| --- | --- |
| Full UI project | 987 files, 8217 tests passed. The combined run's failures were exclusively Electron setup/deadline issues described below. |
| Full Electron project, final rerun | 231 files passed, 3 skipped; 2484 tests passed, 19 skipped; exit 0. |
| Post-lint affected desktop regression | 140 passed, 8 opt-in Docker tests skipped. |
| Gateway crate after bots, locked and serial | 414 passed, 1 ignored; all integration/doc suites passed; exit 0. Earlier Rust-RAM milestone: 371 passed, 1 ignored. OpenCode filter: 4 passed. |
| Affected backend suites, final rerun | 85 passed, including real Python comparison of all 22 UI fixture cases and embedded goldens. |
| Renderer, Electron and E2E TypeScript | All noEmit checks passed. |
| ESLint on all changed app TS/TSX | Exit 0, zero errors; 187 existing warnings (186 spacing, one hook dependency) remain. Final command uses the normal rules, without suppression. |
| Generated gateway contracts | --check passed before and after the final merge. |
| Integrated engine build | Test-profile factr binary built, exit 0. |
| Real-engine canonical bot registry | All checks passed: idempotency, concurrent ensure, distinct profiles, restart, get_many and claim. No model or account required. |
| Real feature-backend idle recovery | Persistent Desktop socket completed feature RPC after two real Python idle exits, exit 0. |
| Production renderer bundle | Vite build passed in 17.82s. |
| Naming and whitespace | Full name gate: zero hits. Complete diff whitespace check passed. |
| Computer scripts | Shell syntax and Linux harness JS syntax passed. No Docker test was executed in this integration run. |

The exact chronological commands, exit codes, durations and log locations are in the appendix. Tests ran with fresh HOME, FACTR_HOME, FACTR_CONFIG_HOME, XDG directories and Electron userData using /private/tmp/integration-run.py. Npm/Cargo caches are isolated in /private/tmp/integration-tool-state. One Cargo target inside this worktree and one Cargo build at a time were used. Environment is allowlisted rather than inherited wholesale.

## Failures found and resolved

- Initial Electron filter electron/platform matched no files. Replaced it with an existing pool smoke suite; no product failure.
- The Rust-RAM pane regression fixture omitted this base's required minWidth. Added that test value; TypeScript and lifecycle regression pass.
- Archived upstream checkpoint notes initially brought personal paths into the name gate. Moved copies to the temporary evidence directory; originals remain in merged branch history. No naming gate exception was added.
- OpenCode had one unsorted imported name. Bots brought 77 lint errors, predominantly omitted braces/import ordering, one unused test import and two redundant string escapes. Fixed those errors without runtime policy changes; spacing/hook warnings are explicitly retained.
- One imported peer module comment contained a retired name. Reworded the comment; tool semantics/schema unchanged.
- Backend transport test assumed the CLI prefix was always Python -m. The existing launcher also supports a standalone executable. Parameterized the fixture over both prefixes, keeping exact argument, profile/alias validation and timeout/exit-grace assertions. No production transport change.
- Updater slow-launch fixture deliberately sleeps four seconds before shell polling. It exceeded the default five-second test budget under the full run (5.156s); unchanged isolated rerun passed (4.844s). Raised only that test's timeout to 15s; final full Electron passes, preserving all product assertions and runtime deadlines.
- Two Electron suites could not import Electron because setup skipped its binary download. Installed the worktree-local dependency without launching it; final full Electron passes. First installer attempt addressed the wrong workspace path and failed; the corrected app-local command succeeded.
- Removed an imported trailing blank line at the end of computer/README.md. Root README was not edited.
- Initial runner inherited the system TMPDIR, so some early test scratch used the OS temporary directory. HOME/FACTR/userData were isolated throughout. All later invocations pin TMPDIR/TMP/TEMP inside their fresh /private/tmp root. No real credential state was accessed.
- The first 140-path graph coverage call exceeded the tool's 128-path cap. Reissued as 100+40 paths. Nearest graph is a sibling worktree generation 2026-10-18T12:52:21Z, with missing new modules and known parser gaps; exact local source is authoritative. No current whole-worktree graph completeness claim.

## Source/design re-check and GUI handover

Re-read every supplied design-law point and checked the added source/diffs against its categories. This integration preserves the existing application design. It introduces no new font, palette, hero/footer/card composition, decorative layout, gradient, avatar, fake logo/product window, glass, shadow, grain or background artifact. The new history mark uses the existing icon export path. OpenCode/bot controls use existing components and layout flow.

Checked motion and visibility: scrollbar motion respects reduced motion; optional math/code failure retains readable content; collapsed replies alone unmount their own hidden body. No new entrance-hidden content, hover lift, animated underline, decorative status glow or fake interactivity was introduced by integration. Checked source for new user-facing retired product names and em dashes; added em dashes are comments only. The name gate passes.

Gutters, clipping/seams, text contrast, centering, narrow layouts, control hit areas, focus/keyboard access and actual pointer behavior cannot be certified from source/tests. The explicit no-GUI-driving instruction was followed. No visual or pointer pass is claimed. NEEDS-GUI-QA.md includes the full original bots matrix, Rust RAM limits, and added scrollbar/OpenCode cases. The branch's existing house styling was not redesigned to evade the merge task.

Priority manual checks:

1. Sidebar bars with system scrollbar preferences, mouse/trackpad, idle/hover, reduced motion, narrow panes and long labels; Run history in rail/palette.
2. Disposable OpenCode sign-in/key fallback, consent, model refresh, failed/offline adoption and initiating-profile ownership. Real credential adoption/inference was not exercised here.
3. Bot create/default/reopen/relaunch, exactly one intro/chat, attachments and clarify/approval ownership; active-room rename/disband with late replies; schedules and Stop; noVNC takeover/hand-back.
4. Rust RAM Settings/jobs native reads, math failure fallback, disclosure images, long chat/find/selection, browser-tab lifecycle, enabled-pet visuals/shutdown, microphone/voice and multi-window.
5. Windows and native Linux setup/sandbox/browser/ACL/process/packaging matrices already retained in the QA document. Integration ran on macOS only.

## Remaining limits / half-done upstream scope

No integration implementation remains half-done. Inherited release boundaries remain: room rounds still run in Desktop (native driver=false); local schedules/courier require Desktop; bare Linux needs usable bubblewrap or Docker and fails closed otherwise. Real Windows confinement/native Linux desktop, cold browser acquisition, provider/account paths, signed packaging and published computer images were not certified here.

The Rust-RAM branch reports an inherited Electron createGuest/null console exception when returning to a loaded browser tab, and possible enabled-pet Python helper retention after quit. Both remain manual follow-up; this merge does not claim fixes. The historical packaged first-turn stall's unique cause remains unproven. See the preserved QA document for exact scenarios.

Electron's 19 skips are 8 opt-in real Docker tests, 1 opt-in live retirement test and 10 WSLg process tests. The ignored Rust test is observability::tests::measure_overhead, a manual release measurement. No GUI, packaged app, benchmark, Docker/model-account or real credential-adoption run was performed. Upstream Docker results were read as historical evidence, not relabeled as integrated validation.

## Cleanup and resumption

Generated worktree target/, desktop/app/dist/ and tsconfig.e2e.tsbuildinfo are removed after checking no tracked files or external directory symlinks. No engine process from the integration target remained. Dependencies, isolated caches, synthetic logs and the runner are retained for QA. No Docker resources were created by this task. Disk checks stayed above 20 GiB; 71 GiB is free after cleanup.

Run from the integration worktree root; WORKTREE means its absolute path. The retained runner prints a fresh isolation root each time. Rebuild before engine smoke because target is cleaned.

```sh
rtk git status --short --branch
rtk proxy df -h /
rtk proxy python3 /private/tmp/integration-run.py engine cargo test -p factr-gateway --locked -- --test-threads=1
rtk proxy python3 /private/tmp/integration-run.py engine cargo build --profile test --bin factr --locked
rtk proxy python3 /private/tmp/integration-run.py desktop node node_modules/typescript/bin/tsc -p app --noEmit
rtk proxy python3 /private/tmp/integration-run.py desktop/app ../node_modules/.bin/vitest run --project ui --project electron --maxWorkers=4
rtk proxy python3 /private/tmp/integration-run.py backend .venv/bin/python -m pytest tests/factr_backend/test_ui_route_goldens.py tests/tui_gateway/test_bot_relay_methods.py -q
rtk proxy python3 /private/tmp/integration-run.py backend .venv/bin/python scripts/gen_gateway_contracts.py --check
rtk proxy python3 /private/tmp/integration-run.py . bash scripts/gate-names.sh
```

REPORT.md is copied verbatim to the authorized handoff directory as integration-REPORT.md. CHECKPOINT.md records each validated milestone. Parent advice was re-read after each merge; no STOP or unresolved architectural question required user input. No push.

## Exact check ledger

Each command below was invoked from this worktree root. The runner's first argument selects its command's working directory. For reproducibility, ${WORKTREE} denotes this worktree and ${PYTHON311} the explicitly selected Python 3.11 interpreter. Those path substitutions remove personal home paths from the committed report; no command option is omitted. Output logs remain outside the source tree. Nonzero attempts are retained rather than counted as passes.

### Check 1: exit 0, 32.99s

```sh
rtk proxy python3 /private/tmp/integration-run.py desktop npm ci --no-audit --no-fund
```

Log: /private/tmp/integration-check-yo6timy_/output.log.

### Check 2: exit 0, 2.44s

```sh
rtk proxy python3 /private/tmp/integration-run.py . bash scripts/gate-names.sh
```

Log: /private/tmp/integration-check-el27panl/output.log.

Result: gate: OK (0 hits)

### Check 3: exit 0, 1.64s

```sh
rtk proxy python3 /private/tmp/integration-run.py desktop/app ../node_modules/.bin/eslint src/app/chat/sidebar/index.tsx src/app/chat/sidebar/nav-model.tsx src/app/command-palette/index.tsx src/lib/icons.ts src/lib/scroll-activity.test.ts src/lib/scroll-activity.ts src/main.tsx
```

Log: /private/tmp/integration-check-0mx3w8s9/output.log.

### Check 4: exit 1, 3.31s

```sh
rtk proxy python3 /private/tmp/integration-run.py desktop/app ../node_modules/.bin/vitest run --project electron electron/platform --maxWorkers=4
```

Log: /private/tmp/integration-check-2u1lni41/output.log.

### Check 5: exit 0, 16.11s

```sh
rtk proxy python3 /private/tmp/integration-run.py desktop node node_modules/typescript/bin/tsc -p app --noEmit
```

Log: /private/tmp/integration-check-4uqpx171/output.log.

### Check 6: exit 0, 15.75s

```sh
rtk proxy python3 /private/tmp/integration-run.py desktop/app ../node_modules/.bin/vitest run --project ui src/lib/scroll-activity.test.ts src/app/chat/sidebar src/app/command-palette --maxWorkers=4
```

Log: /private/tmp/integration-check-r7610r54/output.log.

Result: Test Files  41 passed (41); Tests  275 passed (275)

### Check 7: exit 0, 0.4s

```sh
rtk proxy python3 /private/tmp/integration-run.py desktop/app ../node_modules/.bin/vitest run --project electron electron/pool-retire.test.ts electron/pool-retire-http.test.ts --maxWorkers=4
```

Log: /private/tmp/integration-check-otnb3h2n/output.log.

Result: Test Files  1 passed (1); Tests  2 passed (2)

### Check 8: exit 0, 8.18s

```sh
rtk proxy python3 /private/tmp/integration-run.py desktop/app ../node_modules/.bin/vitest run --project ui src/app/chat/pane-mirror.test.tsx src/app/chat/right-rail/preview-file.test.tsx src/app/chat/right-rail/preview-pane.test.tsx src/components/assistant-ui/markdown-text.lazy-failure.test.tsx src/components/assistant-ui/markdown-text.lazy-math.test.tsx src/components/assistant-ui/thread/inter-agent-collapse.test.tsx src/components/haptics-provider.test.tsx src/lib/markdown-blocks.test.ts --maxWorkers=4
```

Log: /private/tmp/integration-check-nhz91216/output.log.

Result: Test Files  8 passed (8); Tests  58 passed (58)

### Check 9: exit 2, 17.11s

```sh
rtk proxy python3 /private/tmp/integration-run.py desktop node node_modules/typescript/bin/tsc -p app --noEmit
```

Log: /private/tmp/integration-check-qwkm5f5b/output.log.

### Check 10: exit 0, 0.89s

```sh
rtk proxy python3 /private/tmp/integration-run.py desktop/app ../node_modules/.bin/eslint src/app/chat/pane-mirror.test.tsx src/app/chat/pane-mirror.ts src/app/chat/right-rail/preview-file.test.tsx src/app/chat/right-rail/preview-file.tsx src/app/chat/right-rail/preview-pane.test.tsx src/app/chat/right-rail/preview-pane.tsx src/components/assistant-ui/markdown-text.lazy-failure.test.tsx src/components/assistant-ui/markdown-text.lazy-math.test.tsx src/components/assistant-ui/markdown-text.tsx src/components/assistant-ui/thread/assistant-message.tsx src/components/assistant-ui/thread/inter-agent-collapse.test.tsx src/components/haptics-provider.test.tsx src/components/haptics-provider.tsx src/lib/markdown-blocks.test.ts src/lib/markdown-blocks.ts src/lib/use-math-plugin.ts
```

Log: /private/tmp/integration-check-cnuvshqf/output.log.

### Check 11: exit 0, 0.44s

```sh
rtk proxy python3 /private/tmp/integration-run.py desktop/app ../node_modules/.bin/vitest run --project electron electron/pool-retire.test.ts --maxWorkers=4
```

Log: /private/tmp/integration-check-kyhxns8h/output.log.

Result: Test Files  1 passed (1); Tests  2 passed (2)

### Check 12: exit 1, 2.8s

```sh
rtk proxy python3 /private/tmp/integration-run.py . bash scripts/gate-names.sh
```

Log: /private/tmp/integration-check-gikql6c8/output.log.

Result: gate: FAIL (9 lines with retired names)

### Check 13: exit 0, 0.23s

```sh
rtk proxy python3 /private/tmp/integration-run.py backend uv export --locked --extra dev --no-emit-project --output-file /private/tmp/integration-tool-state/requirements.txt
```

Log: /private/tmp/integration-check-btrmddn2/output.log.

### Check 14: exit 0, 0.04s

```sh
rtk proxy python3 /private/tmp/integration-run.py backend uv venv --python ${PYTHON311} .venv
```

Log: /private/tmp/integration-check-khmw22pj/output.log.

### Check 15: exit 0, 0.56s

```sh
rtk proxy python3 /private/tmp/integration-run.py desktop/app ../node_modules/.bin/eslint src/app/chat/pane-mirror.test.tsx
```

Log: /private/tmp/integration-check-m9v_zyok/output.log.

### Check 16: exit 0, 3.49s

```sh
rtk proxy python3 /private/tmp/integration-run.py . bash scripts/gate-names.sh
```

Log: /private/tmp/integration-check-zncnnm5b/output.log.

Result: gate: OK (0 hits)

### Check 17: exit 0, 1.98s

```sh
rtk proxy python3 /private/tmp/integration-run.py desktop/app ../node_modules/.bin/vitest run --project ui src/app/chat/pane-mirror.test.tsx --maxWorkers=4
```

Log: /private/tmp/integration-check-efzcxyyv/output.log.

Result: Test Files  1 passed (1); Tests  1 passed (1)

### Check 18: exit 0, 20.16s

```sh
rtk proxy python3 /private/tmp/integration-run.py desktop node node_modules/typescript/bin/tsc -p app --noEmit
```

Log: /private/tmp/integration-check-q28r48fh/output.log.

### Check 19: exit 0, 15.49s

```sh
rtk proxy python3 /private/tmp/integration-run.py backend uv pip sync --python .venv/bin/python /private/tmp/integration-tool-state/requirements.txt
```

Log: /private/tmp/integration-check-40q_4zmx/output.log.

### Check 20: exit 0, 45.75s

```sh
rtk proxy python3 /private/tmp/integration-run.py backend .venv/bin/python -m pytest tests/factr_backend/test_ui_route_goldens.py -q
```

Log: /private/tmp/integration-check-mf7sei4h/output.log.

Result: 1 passed in 45.07s

### Check 21: exit 0, 229.78s

```sh
rtk proxy python3 /private/tmp/integration-run.py engine cargo test -p factr-gateway --locked -- --test-threads=1
```

Log: /private/tmp/integration-check-dwkkrgl8/output.log.

Result: test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.06s; test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 4.94s; test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.58s; test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

### Check 22: exit 1, 0.8s

```sh
rtk proxy python3 /private/tmp/integration-run.py desktop/app ../node_modules/.bin/eslint src/app/shell/model-menu-panel.tsx src/components/onboarding/index.tsx src/components/onboarding/providers.tsx src/i18n/en.ts src/i18n/types.ts src/store/onboarding.ts
```

Log: /private/tmp/integration-check-il72tjuc/output.log.

### Check 23: exit 0, 5.1s

```sh
rtk proxy python3 /private/tmp/integration-run.py desktop/app ../node_modules/.bin/vitest run --project ui src/components/onboarding src/store/onboarding src/app/shell/model-menu --maxWorkers=4
```

Log: /private/tmp/integration-check-eh5wocul/output.log.

Result: Test Files  12 passed (12); Tests  79 passed (79)

### Check 24: exit 0, 0.71s

```sh
rtk proxy python3 /private/tmp/integration-run.py desktop/app ../node_modules/.bin/eslint src/components/onboarding/index.tsx --fix
```

Log: /private/tmp/integration-check-24ycs4jb/output.log.

### Check 25: exit 0, 0.67s

```sh
rtk proxy python3 /private/tmp/integration-run.py backend .venv/bin/python scripts/gen_gateway_contracts.py --check
```

Log: /private/tmp/integration-check-ehb5qnen/output.log.

### Check 26: exit 0, 0.44s

```sh
rtk proxy python3 /private/tmp/integration-run.py desktop/app ../node_modules/.bin/vitest run --project electron electron/pool-retire.test.ts --maxWorkers=4
```

Log: /private/tmp/integration-check-0iz9b1o5/output.log.

Result: Test Files  1 passed (1); Tests  2 passed (2)

### Check 27: exit 0, 4.25s

```sh
rtk proxy python3 /private/tmp/integration-run.py . bash scripts/gate-names.sh
```

Log: /private/tmp/integration-check-kekzpyjs/output.log.

Result: gate: OK (0 hits)

### Check 28: exit 0, 19.64s

```sh
rtk proxy python3 /private/tmp/integration-run.py desktop node node_modules/typescript/bin/tsc -p app --noEmit
```

Log: /private/tmp/integration-check-6b4s68nf/output.log.

### Check 29: exit 0, 1.02s

```sh
rtk proxy python3 /private/tmp/integration-run.py desktop/app ../node_modules/.bin/eslint src/app/shell/model-menu-panel.tsx src/components/onboarding/index.tsx src/components/onboarding/providers.tsx src/i18n/en.ts src/i18n/types.ts src/store/onboarding.ts
```

Log: /private/tmp/integration-check-es4lmwke/output.log.

### Check 30: exit 0, 49.19s

```sh
rtk proxy python3 /private/tmp/integration-run.py engine cargo test -p factr-gateway --locked opencode -- --test-threads=1
```

Log: /private/tmp/integration-check-bzypci49/output.log.

Result: test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 1 filtered out; finished in 0.00s; test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 1 filtered out; finished in 0.00s; test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 1 filtered out; finished in 0.00s; test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 1 filtered out; finished in 0.00s

### Check 31: exit 0, 0.61s

```sh
rtk proxy python3 /private/tmp/integration-run.py backend .venv/bin/python scripts/gen_gateway_contracts.py --check
```

Log: /private/tmp/integration-check-24l5mvy6/output.log.

### Check 32: exit 0, 4.72s

```sh
rtk proxy python3 /private/tmp/integration-run.py desktop/app ../node_modules/.bin/tsc -p tsconfig.electron.json --noEmit
```

Log: /private/tmp/integration-check-6__eeirr/output.log.

### Check 33: exit 1, 3.59s

```sh
rtk proxy python3 /private/tmp/integration-run.py . bash scripts/gate-names.sh
```

Log: /private/tmp/integration-check-ngzmhi43/output.log.

Result: gate: FAIL (1 lines with retired names)

### Check 34: exit 0, 20.65s

```sh
rtk proxy python3 /private/tmp/integration-run.py desktop node node_modules/typescript/bin/tsc -p app --noEmit
```

Log: /private/tmp/integration-check-6gdf1unv/output.log.

### Check 35: exit 1, 4.86s

```sh
rtk proxy python3 /private/tmp/integration-run.py desktop/app ../node_modules/.bin/eslint electron/bot-computers.real.test.ts electron/bot-computers.test.ts electron/bot-computers.ts electron/bot-docker-setup.test.ts electron/bot-docker-setup.ts electron/bot-local-sandbox.test.ts electron/bot-local-sandbox.ts electron/connection-config.ts electron/host-backend-singleton.test.ts electron/host-backend-singleton.ts electron/main.ts electron/pool-retire-http.test.ts electron/pool-retire-http.ts electron/pool-retire.test.ts electron/pool-retire.ts electron/preload.ts src/api/profiles.ts src/app/cron/deliver-checkboxes.test.tsx src/components/ui/codicon.tsx src/factr.test.ts src/global.d.ts src/i18n/ar.ts src/i18n/en.ts src/i18n/ja.ts src/i18n/ru.ts src/i18n/types.ts src/i18n/zh-hant.ts src/i18n/zh.ts src/lib/hidden-features.test.ts src/lib/hidden-features.ts src/plugins/factr-bots/bot-row.tsx src/plugins/factr-bots/canonical-chat-creation.test.ts src/plugins/factr-bots/canonical-chat-engine-registry.test.ts src/plugins/factr-bots/canonical-chat-local-engine.test.ts src/plugins/factr-bots/canonical-chat-registry.test.ts src/plugins/factr-bots/canonical-chat-topology.test.ts src/plugins/factr-bots/canonical-chat.ts src/plugins/factr-bots/chat-empty.tsx src/plugins/factr-bots/computer-setup.test.tsx src/plugins/factr-bots/computer.test.tsx src/plugins/factr-bots/computer.ts src/plugins/factr-bots/create-dialog.tsx src/plugins/factr-bots/cron-detail.test.tsx src/plugins/factr-bots/cron-prompt.test.ts src/plugins/factr-bots/cron.tsx src/plugins/factr-bots/cross-connection-bots.test.ts src/plugins/factr-bots/data.engine-bot-chat.test.tsx src/plugins/factr-bots/data.roster.test.tsx src/plugins/factr-bots/data.ts src/plugins/factr-bots/edit-profile-dialog.test.tsx src/plugins/factr-bots/edit-profile-dialog.tsx src/plugins/factr-bots/engine-bot-chat.test.ts src/plugins/factr-bots/engine-bot-chat.ts src/plugins/factr-bots/group-chat.test.ts src/plugins/factr-bots/group-chat.ts src/plugins/factr-bots/group-compress.test.tsx src/plugins/factr-bots/group-rounds.test.ts src/plugins/factr-bots/group-turns.test.ts src/plugins/factr-bots/group-turns.ts src/plugins/factr-bots/hide-bot-chats.runtime.test.ts src/plugins/factr-bots/i18n.ts src/plugins/factr-bots/profile-config.tsx src/plugins/factr-bots/relay.test.ts src/plugins/factr-bots/relay.ts src/plugins/factr-bots/roster-pane-content.test.tsx src/plugins/factr-bots/roster-pane-content.tsx src/plugins/factr-bots/roster-pane.tsx src/plugins/factr-bots/routing.ts src/plugins/factr-bots/screen-autoraise.ts src/plugins/factr-bots/screen-connection.ts src/plugins/factr-bots/screen-connection.url.test.ts src/plugins/factr-bots/screen-pane.tsx src/sdk/index.ts src/store/gateway-turn-settled.test.ts src/store/gateway.ts
```

Log: /private/tmp/integration-check-jifp612w/output.log.

Result: ✖ 251 problems (77 errors, 174 warnings)

### Check 36: exit 1, 6.4s

```sh
rtk proxy python3 /private/tmp/integration-run.py desktop/app ../node_modules/.bin/eslint electron/bot-computers.real.test.ts electron/bot-computers.test.ts electron/bot-computers.ts electron/bot-docker-setup.test.ts electron/bot-docker-setup.ts electron/bot-local-sandbox.test.ts electron/bot-local-sandbox.ts electron/connection-config.ts electron/host-backend-singleton.test.ts electron/host-backend-singleton.ts electron/main.ts electron/pool-retire-http.test.ts electron/pool-retire-http.ts electron/pool-retire.test.ts electron/pool-retire.ts electron/preload.ts src/api/profiles.ts src/app/cron/deliver-checkboxes.test.tsx src/components/ui/codicon.tsx src/factr.test.ts src/global.d.ts src/i18n/ar.ts src/i18n/en.ts src/i18n/ja.ts src/i18n/ru.ts src/i18n/types.ts src/i18n/zh-hant.ts src/i18n/zh.ts src/lib/hidden-features.test.ts src/lib/hidden-features.ts src/plugins/factr-bots/bot-row.tsx src/plugins/factr-bots/canonical-chat-creation.test.ts src/plugins/factr-bots/canonical-chat-engine-registry.test.ts src/plugins/factr-bots/canonical-chat-local-engine.test.ts src/plugins/factr-bots/canonical-chat-registry.test.ts src/plugins/factr-bots/canonical-chat-topology.test.ts src/plugins/factr-bots/canonical-chat.ts src/plugins/factr-bots/chat-empty.tsx src/plugins/factr-bots/computer-setup.test.tsx src/plugins/factr-bots/computer.test.tsx src/plugins/factr-bots/computer.ts src/plugins/factr-bots/create-dialog.tsx src/plugins/factr-bots/cron-detail.test.tsx src/plugins/factr-bots/cron-prompt.test.ts src/plugins/factr-bots/cron.tsx src/plugins/factr-bots/cross-connection-bots.test.ts src/plugins/factr-bots/data.engine-bot-chat.test.tsx src/plugins/factr-bots/data.roster.test.tsx src/plugins/factr-bots/data.ts src/plugins/factr-bots/edit-profile-dialog.test.tsx src/plugins/factr-bots/edit-profile-dialog.tsx src/plugins/factr-bots/engine-bot-chat.test.ts src/plugins/factr-bots/engine-bot-chat.ts src/plugins/factr-bots/group-chat.test.ts src/plugins/factr-bots/group-chat.ts src/plugins/factr-bots/group-compress.test.tsx src/plugins/factr-bots/group-rounds.test.ts src/plugins/factr-bots/group-turns.test.ts src/plugins/factr-bots/group-turns.ts src/plugins/factr-bots/hide-bot-chats.runtime.test.ts src/plugins/factr-bots/i18n.ts src/plugins/factr-bots/profile-config.tsx src/plugins/factr-bots/relay.test.ts src/plugins/factr-bots/relay.ts src/plugins/factr-bots/roster-pane-content.test.tsx src/plugins/factr-bots/roster-pane-content.tsx src/plugins/factr-bots/roster-pane.tsx src/plugins/factr-bots/routing.ts src/plugins/factr-bots/screen-autoraise.ts src/plugins/factr-bots/screen-connection.ts src/plugins/factr-bots/screen-connection.url.test.ts src/plugins/factr-bots/screen-pane.tsx src/sdk/index.ts src/store/gateway-turn-settled.test.ts src/store/gateway.ts --fix --rule 'padding-line-between-statements: off'
```

Log: /private/tmp/integration-check-zwfk5mwl/output.log.

Result: ✖ 3 problems (2 errors, 1 warning)

### Check 37: exit 1, 62.18s

```sh
rtk proxy python3 /private/tmp/integration-run.py backend .venv/bin/python -m pytest tests/factr_backend/test_ui_route_goldens.py tests/tools/test_message_agent_mcp.py tests/tools/test_local_relay_delivery.py tests/tools/test_bot_computer_guard.py tests/tools/test_bot_desktop_takeover.py tests/tools/test_windows_bot_sandbox.py tests/tui_gateway/test_bot_relay_methods.py tests/tui_gateway/test_profiles_inherit_launch_model.py tests/cron/test_engine_bot_chat.py tests/cron/test_cron_manage_profile_scope.py -q
```

Log: /private/tmp/integration-check-ycq9wzcu/output.log.

### Check 38: exit 0, 4.68s

```sh
rtk proxy python3 /private/tmp/integration-run.py . bash scripts/gate-names.sh
```

Log: /private/tmp/integration-check-bh9s_0ea/output.log.

Result: gate: OK (0 hits)

### Check 39: exit 0, 7.87s

```sh
rtk proxy python3 /private/tmp/integration-run.py desktop/app ../node_modules/.bin/eslint electron/bot-computers.real.test.ts electron/bot-computers.test.ts electron/bot-computers.ts electron/bot-docker-setup.test.ts electron/bot-docker-setup.ts electron/bot-local-sandbox.test.ts electron/bot-local-sandbox.ts electron/connection-config.ts electron/host-backend-singleton.test.ts electron/host-backend-singleton.ts electron/main.ts electron/pool-retire-http.test.ts electron/pool-retire-http.ts electron/pool-retire.test.ts electron/pool-retire.ts electron/preload.ts src/api/profiles.ts src/app/chat/pane-mirror.test.tsx src/app/chat/pane-mirror.ts src/app/chat/right-rail/preview-file.test.tsx src/app/chat/right-rail/preview-file.tsx src/app/chat/right-rail/preview-pane.test.tsx src/app/chat/right-rail/preview-pane.tsx src/app/chat/sidebar/index.tsx src/app/chat/sidebar/nav-model.tsx src/app/command-palette/index.tsx src/app/cron/deliver-checkboxes.test.tsx src/app/shell/model-menu-panel.tsx src/components/assistant-ui/markdown-text.lazy-failure.test.tsx src/components/assistant-ui/markdown-text.lazy-math.test.tsx src/components/assistant-ui/markdown-text.tsx src/components/assistant-ui/thread/assistant-message.tsx src/components/assistant-ui/thread/inter-agent-collapse.test.tsx src/components/haptics-provider.test.tsx src/components/haptics-provider.tsx src/components/onboarding/index.tsx src/components/onboarding/providers.tsx src/components/ui/codicon.tsx src/factr.test.ts src/global.d.ts src/i18n/ar.ts src/i18n/en.ts src/i18n/ja.ts src/i18n/ru.ts src/i18n/types.ts src/i18n/zh-hant.ts src/i18n/zh.ts src/lib/hidden-features.test.ts src/lib/hidden-features.ts src/lib/icons.ts src/lib/markdown-blocks.test.ts src/lib/markdown-blocks.ts src/lib/scroll-activity.test.ts src/lib/scroll-activity.ts src/lib/use-math-plugin.ts src/main.tsx src/plugins/factr-bots/bot-row.tsx src/plugins/factr-bots/canonical-chat-creation.test.ts src/plugins/factr-bots/canonical-chat-engine-registry.test.ts src/plugins/factr-bots/canonical-chat-local-engine.test.ts src/plugins/factr-bots/canonical-chat-registry.test.ts src/plugins/factr-bots/canonical-chat-topology.test.ts src/plugins/factr-bots/canonical-chat.ts src/plugins/factr-bots/chat-empty.tsx src/plugins/factr-bots/computer-setup.test.tsx src/plugins/factr-bots/computer.test.tsx src/plugins/factr-bots/computer.ts src/plugins/factr-bots/create-dialog.tsx src/plugins/factr-bots/cron-detail.test.tsx src/plugins/factr-bots/cron-prompt.test.ts src/plugins/factr-bots/cron.tsx src/plugins/factr-bots/cross-connection-bots.test.ts src/plugins/factr-bots/data.engine-bot-chat.test.tsx src/plugins/factr-bots/data.roster.test.tsx src/plugins/factr-bots/data.ts src/plugins/factr-bots/edit-profile-dialog.test.tsx src/plugins/factr-bots/edit-profile-dialog.tsx src/plugins/factr-bots/engine-bot-chat.test.ts src/plugins/factr-bots/engine-bot-chat.ts src/plugins/factr-bots/group-chat.test.ts src/plugins/factr-bots/group-chat.ts src/plugins/factr-bots/group-compress.test.tsx src/plugins/factr-bots/group-rounds.test.ts src/plugins/factr-bots/group-turns.test.ts src/plugins/factr-bots/group-turns.ts src/plugins/factr-bots/hide-bot-chats.runtime.test.ts src/plugins/factr-bots/i18n.ts src/plugins/factr-bots/profile-config.tsx src/plugins/factr-bots/relay.test.ts src/plugins/factr-bots/relay.ts src/plugins/factr-bots/roster-pane-content.test.tsx src/plugins/factr-bots/roster-pane-content.tsx src/plugins/factr-bots/roster-pane.tsx src/plugins/factr-bots/routing.ts src/plugins/factr-bots/screen-autoraise.ts src/plugins/factr-bots/screen-connection.ts src/plugins/factr-bots/screen-connection.url.test.ts src/plugins/factr-bots/screen-pane.tsx src/sdk/index.ts src/store/gateway-turn-settled.test.ts src/store/gateway.ts src/store/onboarding.ts
```

Log: /private/tmp/integration-check-rl2n5yzo/output.log.

Result: ✖ 176 problems (0 errors, 176 warnings)

### Check 40: exit 0, 142.54s

```sh
rtk proxy python3 /private/tmp/integration-run.py engine cargo test -p factr-gateway --locked -- --test-threads=1
```

Log: /private/tmp/integration-check-svbvvkdl/output.log.

Result: test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.37s; test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 5.18s; test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.67s; test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

### Check 41: exit 0, 8.21s

```sh
rtk proxy python3 /private/tmp/integration-run.py backend .venv/bin/python -m pytest tests/tui_gateway/test_bot_relay_methods.py -q
```

Log: /private/tmp/integration-check-oxtz5be0/output.log.

Result: 26 passed in 7.64s

### Check 42: exit 0, 8.34s

```sh
rtk proxy python3 /private/tmp/integration-run.py desktop/app ../node_modules/.bin/vitest run --project ui --project electron electron/bot-computers.real.test.ts electron/bot-computers.test.ts electron/bot-computers.ts electron/bot-docker-setup.test.ts electron/bot-docker-setup.ts electron/bot-local-sandbox.test.ts electron/bot-local-sandbox.ts electron/main.ts electron/pool-retire-http.test.ts electron/pool-retire-http.ts electron/pool-retire.ts src/plugins/factr-bots/canonical-chat-local-engine.test.ts src/plugins/factr-bots/computer-setup.test.tsx src/plugins/factr-bots/computer.ts src/plugins/factr-bots/cron-prompt.test.ts src/plugins/factr-bots/cron.tsx src/plugins/factr-bots/edit-profile-dialog.test.tsx src/plugins/factr-bots/edit-profile-dialog.tsx src/plugins/factr-bots/group-chat.ts src/plugins/factr-bots/group-turns.ts src/plugins/factr-bots/relay.test.ts src/plugins/factr-bots/relay.ts src/plugins/factr-bots/screen-pane.tsx --maxWorkers=4
```

Log: /private/tmp/integration-check-yzdboov0/output.log.

Result: Test Files  9 passed | 1 skipped (10); Tests  140 passed | 8 skipped (148)

### Check 43: exit 0, 13.6s

```sh
rtk proxy python3 /private/tmp/integration-run.py desktop/app ../node_modules/.bin/vitest run --project electron electron/update-swap.test.ts --maxWorkers=1
```

Log: /private/tmp/integration-check-jt94mmfz/output.log.

Result: Test Files  1 passed (1); Tests  24 passed (24)

### Check 44: exit 0, 1.36s

```sh
rtk proxy python3 /private/tmp/integration-run.py desktop/app ../node_modules/.bin/eslint electron/update-swap.test.ts
```

Log: /private/tmp/integration-check-vrghqg79/output.log.

Result: ✖ 11 problems (0 errors, 11 warnings)

### Check 45: exit 0, 2.48s

```sh
rtk proxy python3 /private/tmp/integration-run.py desktop/app ../node_modules/.bin/tsc -p tsconfig.electron.json --noEmit
```

Log: /private/tmp/integration-check-zkgxwg0b/output.log.

### Check 46: exit 0, 61.09s

```sh
rtk proxy python3 /private/tmp/integration-run.py engine cargo build --profile test --bin factr --locked
```

Log: /private/tmp/integration-check-nx6aaanp/output.log.

### Check 47: exit 1, 38.74s

```sh
rtk proxy python3 /private/tmp/integration-run.py desktop/app ../node_modules/.bin/vitest run --project electron --maxWorkers=4
```

Log: /private/tmp/integration-check-te5ad0az/output.log.

Result: Test Files  2 failed | 229 passed | 3 skipped (234); Tests  2460 passed | 19 skipped (2479)

### Check 48: exit 0, 13.56s

```sh
rtk proxy python3 /private/tmp/integration-run.py . env FACTR_BIN=${WORKTREE}/target/debug/factr node engine/crates/factr-gateway/e2e/bot-chat-registry.mjs
```

Log: /private/tmp/integration-check-kvo_qci2/output.log.

Result: all checks passed

### Check 49: exit 0, 18.4s

```sh
rtk proxy python3 /private/tmp/integration-run.py desktop/app ../node_modules/.bin/vite build
```

Log: /private/tmp/integration-check-vnqhszli/output.log.

### Check 50: exit 0, 27.7s

```sh
rtk proxy python3 /private/tmp/integration-run.py . env FACTR_BIN=${WORKTREE}/target/debug/factr FACTR_BACKEND_PYTHON=${WORKTREE}/backend/.venv/bin/python FACTR_BACKEND_PYTHONPATH=${WORKTREE}/backend node engine/crates/factr-gateway/e2e/bot-feature-reconnect.mjs
```

Log: /private/tmp/integration-check-yvap056p/output.log.

Result: PASS persistent Desktop socket reconnects feature RPC after two real idle backend exits

### Check 51: exit 1, 0.04s

```sh
rtk proxy python3 /private/tmp/integration-run.py desktop env ELECTRON_SKIP_BINARY_DOWNLOAD=0 ELECTRON_CACHE=/private/tmp/integration-tool-state/electron-cache node node_modules/electron/install.js
```

Log: /private/tmp/integration-check-95lwyxus/output.log.

### Check 52: exit 1, 295.52s

```sh
rtk proxy python3 /private/tmp/integration-run.py desktop/app ../node_modules/.bin/vitest run --project ui --project electron --maxWorkers=4
```

Log: /private/tmp/integration-check-hypc9wtj/output.log.

Result: Test Files  3 failed | 1215 passed | 3 skipped (1221); Tests  1 failed | 10676 passed | 19 skipped (10696)

### Check 53: exit 0, 0.97s

```sh
rtk proxy python3 /private/tmp/integration-run.py desktop/app ../node_modules/.bin/tsc -p tsconfig.e2e.json --noEmit
```

Log: /private/tmp/integration-check-t87zrx4m/output.log.

### Check 54: exit 0, 19.9s

```sh
rtk proxy python3 /private/tmp/integration-run.py desktop/app env -u ELECTRON_SKIP_BINARY_DOWNLOAD ELECTRON_CACHE=/private/tmp/integration-tool-state/electron-cache node node_modules/electron/install.js
```

Log: /private/tmp/integration-check-dda66akl/output.log.

### Check 55: exit 0, 4.59s

```sh
rtk proxy python3 /private/tmp/integration-run.py desktop/app ../node_modules/.bin/eslint electron/bot-computers.real.test.ts electron/bot-computers.test.ts electron/bot-computers.ts electron/bot-docker-setup.test.ts electron/bot-docker-setup.ts electron/bot-local-sandbox.test.ts electron/bot-local-sandbox.ts electron/connection-config.ts electron/host-backend-singleton.test.ts electron/host-backend-singleton.ts electron/main.ts electron/pool-retire-http.test.ts electron/pool-retire-http.ts electron/pool-retire.test.ts electron/pool-retire.ts electron/preload.ts electron/update-swap.test.ts src/api/profiles.ts src/app/chat/pane-mirror.test.tsx src/app/chat/pane-mirror.ts src/app/chat/right-rail/preview-file.test.tsx src/app/chat/right-rail/preview-file.tsx src/app/chat/right-rail/preview-pane.test.tsx src/app/chat/right-rail/preview-pane.tsx src/app/chat/sidebar/index.tsx src/app/chat/sidebar/nav-model.tsx src/app/command-palette/index.tsx src/app/cron/deliver-checkboxes.test.tsx src/app/shell/model-menu-panel.tsx src/components/assistant-ui/markdown-text.lazy-failure.test.tsx src/components/assistant-ui/markdown-text.lazy-math.test.tsx src/components/assistant-ui/markdown-text.tsx src/components/assistant-ui/thread/assistant-message.tsx src/components/assistant-ui/thread/inter-agent-collapse.test.tsx src/components/haptics-provider.test.tsx src/components/haptics-provider.tsx src/components/onboarding/index.tsx src/components/onboarding/providers.tsx src/components/ui/codicon.tsx src/factr.test.ts src/global.d.ts src/i18n/ar.ts src/i18n/en.ts src/i18n/ja.ts src/i18n/ru.ts src/i18n/types.ts src/i18n/zh-hant.ts src/i18n/zh.ts src/lib/hidden-features.test.ts src/lib/hidden-features.ts src/lib/icons.ts src/lib/markdown-blocks.test.ts src/lib/markdown-blocks.ts src/lib/scroll-activity.test.ts src/lib/scroll-activity.ts src/lib/use-math-plugin.ts src/main.tsx src/plugins/factr-bots/bot-row.tsx src/plugins/factr-bots/canonical-chat-creation.test.ts src/plugins/factr-bots/canonical-chat-engine-registry.test.ts src/plugins/factr-bots/canonical-chat-local-engine.test.ts src/plugins/factr-bots/canonical-chat-registry.test.ts src/plugins/factr-bots/canonical-chat-topology.test.ts src/plugins/factr-bots/canonical-chat.ts src/plugins/factr-bots/chat-empty.tsx src/plugins/factr-bots/computer-setup.test.tsx src/plugins/factr-bots/computer.test.tsx src/plugins/factr-bots/computer.ts src/plugins/factr-bots/create-dialog.tsx src/plugins/factr-bots/cron-detail.test.tsx src/plugins/factr-bots/cron-prompt.test.ts src/plugins/factr-bots/cron.tsx src/plugins/factr-bots/cross-connection-bots.test.ts src/plugins/factr-bots/data.engine-bot-chat.test.tsx src/plugins/factr-bots/data.roster.test.tsx src/plugins/factr-bots/data.ts src/plugins/factr-bots/edit-profile-dialog.test.tsx src/plugins/factr-bots/edit-profile-dialog.tsx src/plugins/factr-bots/engine-bot-chat.test.ts src/plugins/factr-bots/engine-bot-chat.ts src/plugins/factr-bots/group-chat.test.ts src/plugins/factr-bots/group-chat.ts src/plugins/factr-bots/group-compress.test.tsx src/plugins/factr-bots/group-rounds.test.ts src/plugins/factr-bots/group-turns.test.ts src/plugins/factr-bots/group-turns.ts src/plugins/factr-bots/hide-bot-chats.runtime.test.ts src/plugins/factr-bots/i18n.ts src/plugins/factr-bots/profile-config.tsx src/plugins/factr-bots/relay.test.ts src/plugins/factr-bots/relay.ts src/plugins/factr-bots/roster-pane-content.test.tsx src/plugins/factr-bots/roster-pane-content.tsx src/plugins/factr-bots/roster-pane.tsx src/plugins/factr-bots/routing.ts src/plugins/factr-bots/screen-autoraise.ts src/plugins/factr-bots/screen-connection.ts src/plugins/factr-bots/screen-connection.url.test.ts src/plugins/factr-bots/screen-pane.tsx src/sdk/index.ts src/store/gateway-turn-settled.test.ts src/store/gateway.ts src/store/onboarding.ts
```

Log: /private/tmp/integration-check-9ovhhbbb/output.log.

Result: ✖ 187 problems (0 errors, 187 warnings)

### Check 56: exit 0, 0.71s

```sh
rtk proxy python3 /private/tmp/integration-run.py backend .venv/bin/python scripts/gen_gateway_contracts.py --check
```

Log: /private/tmp/integration-check-jv5llbi4/output.log.

### Check 57: exit 0, 17.38s

```sh
rtk proxy python3 /private/tmp/integration-run.py desktop node node_modules/typescript/bin/tsc -p app --noEmit
```

Log: /private/tmp/integration-check-qadpdu2m/output.log.

### Check 58: exit 0, 0.01s

```sh
rtk proxy python3 /private/tmp/integration-run.py . bash -n computer/entrypoint.sh computer/tests/test_computer_image.sh computer/tests/test_local_sandbox_linux.sh
```

Log: /private/tmp/integration-check-ifmynal8/output.log.

### Check 59: exit 0, 0.06s

```sh
rtk proxy python3 /private/tmp/integration-run.py . node --check computer/tests/test_local_sandbox_linux.mjs
```

Log: /private/tmp/integration-check-k7h86j7r/output.log.

### Check 60: exit 0, 3.3s

```sh
rtk proxy python3 /private/tmp/integration-run.py . bash scripts/gate-names.sh
```

Log: /private/tmp/integration-check-_sngmsj9/output.log.

Result: gate: OK (0 hits)

### Check 61: exit 0, 33.94s

```sh
rtk proxy python3 /private/tmp/integration-run.py desktop/app ../node_modules/.bin/vitest run --project electron --maxWorkers=4
```

Log: /private/tmp/integration-check-6vo_3goy/output.log.

Result: Test Files  231 passed | 3 skipped (234); Tests  2484 passed | 19 skipped (2503)

### Check 62: exit 0, 53.25s

```sh
rtk proxy python3 /private/tmp/integration-run.py backend .venv/bin/python -m pytest tests/factr_backend/test_ui_route_goldens.py tests/tools/test_message_agent_mcp.py tests/tools/test_local_relay_delivery.py tests/tools/test_bot_computer_guard.py tests/tools/test_bot_desktop_takeover.py tests/tools/test_windows_bot_sandbox.py tests/tui_gateway/test_bot_relay_methods.py tests/tui_gateway/test_profiles_inherit_launch_model.py tests/cron/test_engine_bot_chat.py tests/cron/test_cron_manage_profile_scope.py -q
```

Log: /private/tmp/integration-check-chijm0n_/output.log.

Result: 85 passed in 52.50s

### Check 63: exit 0, 2.63s

```sh
rtk proxy python3 /private/tmp/integration-run.py . bash scripts/gate-names.sh
```

Log: /private/tmp/integration-check-0_umfhw4/output.log.

Result: gate: OK (0 hits)

Other final assertions (exit 0): git diff --check 39ca7e9f; generated contract parity; byte-preservation assertion for the bots QA document; no root README/non-gateway engine source differences; local merge-parent ancestry checks; no integration-target process; final clean Git status. Exact merge commands were git merge --no-ff --no-commit followed by git commit only after validation, in the order above. Every new commit carries the required co-author trailer.
