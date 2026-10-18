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

