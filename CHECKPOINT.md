# Factr-I i18n checkpoint

## Scope and constraints
- Worktree: /Users/rameelmalik/factri-i18n-wt; branch feat/0.0.10-i18n.
- Read COMMON-RULES.md, ADVICE-FROM-PARENT.md and i18n-handoff.md completely, in order.
- Wording only. No layout, style, icon, engine, version, remote or release-checkout changes. No push or credential reads.
- Milestones: (1) stale locale cloud/gateway wording and regression tests; (2) English em dashes; (3) product names; (4) final validation and report.
- Validation: app typecheck, ESLint on touched source, Vitest ui project restricted to i18n. Tests use /private/tmp/factri-i18n-qa for isolated home, Factr/config and Electron data.

## Discovery
- Git status initially clean. Disk: 73 GiB available before dependency install.
- Verify tier: list_projects found no index for this worktree. Nearest related index factri-main belongs to /Users/rameelmalik/factri-integ-w19, generation 2026-10-12T11:52:27Z. search_graph returned no locale symbols; check_index_coverage confirms i18n excluded and create-dialog partial (795, 1079). Use local source and literal scans for all evidence; no completeness claims from that graph.
- Existing defineLocale merges partial overrides over English. runtime.translateNow also falls back to English for missing values. Reuse both; no runtime logic changes needed.
- No node_modules installed. Installing locked dependencies with scripts disabled for source-only validation in isolated environment.

## Advice
- Remote gateway wording remains in current English. Remove obsolete cloud branding, cloud discovery claims and outdated direct-network wording; preserve valid remote/SSH functionality. Question recorded for parent; proceed with current English as authority.

## Milestone 1: locale cloud and gateway wording (validated)
- Removed 23 stale overrides: zh (9), ru (8), ja/ar/zh-hant (2 each). Existing English merge supplies current remote/SSH and hosted-registry wording. No runtime/helper/type changes.
- Added 11 parameterized runtime regressions: all six catalogs contain no obsolete cloud brand, and all five non-English locales resolve removed gateway keys to English, including the intentionally empty cloudAddHint.
- Regression proof before source changes: `npm run test:ui -- src/i18n/runtime.test.ts` failed 7/16 tests (zh/ru brand plus five stale-locale cases). After changes: all 36 i18n tests passed in 4 files, twice (second run after concise failure-assertion edit).
- `npm run typecheck`: passed all three tsc projects (renderer, Electron, e2e).
- `../node_modules/.bin/eslint src/i18n/ar.ts src/i18n/ja.ts src/i18n/ru.ts src/i18n/zh.ts src/i18n/zh-hant.ts src/i18n/runtime.test.ts`: passed.
- `git diff --check`: passed. Reviewed all removed keys against current English and their settings/sidebar consumers.
- Installed 1191 locked packages with `npm ci --ignore-scripts --no-audit --no-fund`; no manifest or lock changes. Only existing dependency deprecation warnings.
- Initial checks used isolated /private/tmp/factri-i18n-qa directories; subsequent checks use `rtk proxy python3 /private/tmp/factri-i18n-qa/run.py <command>`, which creates fresh HOME/FACTR_HOME/FACTR_CONFIG_HOME/XDG_CONFIG_HOME/ELECTRON_USER_DATA_DIR directories per command and removes them afterward. No app launched.
- Non-fatal existing Vitest warnings: Vite config __dirname/native-loader compatibility, Node 26 localStorage (handled by existing test setup).
- Re-read parent advice after validation: no STOP and no i18n-specific correction.
- Next: replace English em dashes in actual UI text only; preserve comments, non-English translations, model prompts and test data.


## Milestone 2: English punctuation (validated)
- Replaced 158 visible English em dashes in 157 string/JSX-text spans across 58 src files. Includes English Bot Mode/Kanban bundles, settings, chat/status text, and user-visible error/tool feedback. en.ts already contains no literal, escaped or HTML-entity em dash.
- Left non-English plugin bundles, comments (including injected JS/CSS comments), regex punctuation handling, debug console output and internal model prompts untouched. No engine code or prompt changes.
- `npm run typecheck`: passed all three tsc projects.
- ESLint on all 58 touched source files: exit 0; 12 existing TS warnings (padding and hook dependencies) plus one ignored Radio JS warning. Forced `eslint --no-ignore src/plugins/radio/plugin.js`: seven existing errors; running the committed HEAD content through the same ESLint stdin command reproduces all seven at identical lines. No lint configuration or unrelated code changed.
- Extended check: `npm run test:ui -- src/i18n <24 adjacent test files> --maxWorkers=2`: 454 passed, four failures limited to old punctuation expectations in session-tile.test.ts and group-activity.test.ts. Updated exactly four assertion strings; no fixtures or test inputs changed.
- `npm run test:ui -- src/i18n src/app/chat/session-tile.test.ts src/plugins/factr-bots/group-activity.test.ts`: all 54 tests passed in six files after assertion updates. The other 454 tests had already passed; no implementation changes since that run.
- ESLint on the two edited assertion files: passed. `git diff --check`: passed.
- Asserted every diff against milestone 1 using Python SequenceMatcher: exactly 162 replacements, each a literal/escaped em dash to a hyphen, across 60 files; no other byte-level edits.
- Re-read parent advice: no STOP or i18n-specific correction. Next: normalize product naming and document genuinely ambiguous descriptors.

## Milestone 3: product naming (validated)
- Normalized 29 visible names in 11 files: Factr-I Desktop / Factr Desktop to Factr-I, three bare Factr mentions to Factr-I, and two mixed-language settings labels from Factr-I desktop to Factr-I. Includes fallback/SDK errors and all four Bot Mode locale bundles. No identifiers, command names, interfaces, types or control logic changed.
- Ambiguous descriptive wording left for the user: lowercase "Factr-I agent" in uninstall-section.tsx distinguishes the backend from the GUI; lowercase "Factr-I desktop app" in en.ts and "Factr-I desktop bridge" in api/plugins.ts describe the package/bridge. Removing these descriptors could obscure which installed part the message means. Recorded as question 2 in QUESTIONS-i18n.md; continued without waiting.
- Syntax audit of 1,377 non-test TS/TSX/JS source files: no obsolete capitalized product-name candidates (Factr-I Agent, Factr-I Desktop, Factr Desktop, Hermes, jcode, Prime) or cloud brand remain in literal/JSX-text nodes. No unclassified English UI em dashes remain. Exclusions: 62 internal model-prompt nodes, two developer-console nodes, three comment-containing injected-code nodes, 401 non-English core-locale nodes, 130 non-English plugin-locale nodes.
- `npm run typecheck`: passed all three tsc projects.
- ESLint on the 11 touched source files: passed without warnings.
- `npm run test:ui -- src/i18n <26 adjacent test files> --maxWorkers=2`: all 472 tests passed in 30 files. Exact expanded command and adjacent file list will be included in REPORT.md.
- `git diff --check`: passed. Reviewed entire naming diff against message consumers and preserved descriptive/technical names.
- Existing jsdom canvas warnings in broader tests are non-fatal; no native canvas dependency added for this wording task.
- Re-read parent advice: no STOP or i18n correction. Next: final source/diff review, QA notes, report copy and local documentation commit.

## Milestone 3 follow-up: shared gateway error (validated)
- Final adjacent-source search found one additional Factr Desktop name in desktop/shared/src/websocket-url.ts, surfaced by all desktop WebSocket consumers. Changed only that error string to Factr-I.
- Read helper, callers and existing app gateway-ws-url tests. Graph coverage has no recorded issue for this file in the related index; source checked locally because the index is for another worktree.
- Shared `npm run typecheck` and `../node_modules/.bin/eslint src/websocket-url.ts`: passed.
- App `npm run test:ui -- src/i18n src/lib/gateway-ws-url.test.ts`: passed 59 tests in five files.
- No source change in the helper's auth/ticket behavior. Final check also found a few Electron UI copy leftovers; handle those before the final report.

## Milestone 2 follow-up: Electron-visible English copy (validated)
- Final desktop-source audit found 11 visible English em dashes in seven Electron files: install repair copy, plugin removal error, update/wait/recovery/About copy, plugin compatibility list, missing-renderer-assets HTML text, SSH validation and foreign-lockfile refusal. Changed only those string spans to hyphens.
- Preserved main-process logs, generated-script comments, and the preview-reach e2e fixture. No lifecycle/auth/SSH behavior changed.
- App `npm run typecheck`: passed all three tsc projects.
- ESLint on the seven touched Electron files: passed without warnings.
- `npm run test:desktop:platforms -- electron/bootstrap-failure-copy.test.ts electron/desktop-plugin-remove.test.ts electron/plugin-compat-notice.test.ts electron/renderer-load-error-page.test.ts electron/ssh-connection.test.ts electron/remote-lifecycle.test.ts`: 174 tests passed across the five existing matching files. bootstrap-failure-copy.test.ts does not exist; bootstrap copy and the three main.ts messages received source/diff checks only, with real dialogs left for GUI QA.
- Line-by-line check confirms exactly 11 punctuation substitutions and no other Electron edits. `git diff --check`: passed.
