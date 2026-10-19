# UI Activity checkpoint

## Scope and rules
- Task: handoff/ui-batch/ui-activity-handoff.md, steps C, icons, B, verification, in order. No push; no engine core or font-family changes; no credentials read.
- Read COMMON-RULES.md, ADVICE-FROM-PARENT.md, task, REQUIREMENTS.md, claude-code-hands-on.md, FABLE-ADVICE.md, OPUS-PLAN.md, DESIGN-refactr.md, RTK.md. Design brief takes precedence over conflicting default law. Operate mode, existing tokens/components.
- Graph Verify: factri-int10-app generation 2026-10-19T21:05:07Z, best effort, integration sibling checkout. Searched CommandCenter/RunHistory/Rail/ProfileMenu/openAgents; trace CommandCenterView inbound found no recorded callers (JSX needs source fallback). Coverage checked command-center/, agents/, rail, nav-model, profiles/, contrib/, route candidate, profile candidate and electron/desktop-profile.ts. Direct local source used for all material edits; graph directory coverage not tracked and tests/i18n excluded.
- Initial git clean on feat/ui-activity. Disk: 42 GiB available. Before commit: ffe8911 (before: merge Activity).
- One npm ci running in desktop, isolated HOME/FACTR_HOME/FACTR_CONFIG_HOME under /private/tmp/factri-ui-activity; ignore scripts, no dependency changes.

## Advice
- Existing ledger has more working controls than the plan describes. Preserve the ledger and embed it; preserve live subagent tree. Scheduled reuses CronView in embedded mode; System flattens existing actions and sessions, removes Usage dashboard. Question written to handoff/QUESTIONS-ui-activity.md; continue safely without waiting.

## Next
1. Activity merge with alias redirect, tests/typecheck/lint then commit.
2. Rail icon normalization and naming sweep.
3. Account row and optional validated profile picture.
4. Full UI tests and real GUI attempt, detailed manual scenarios for anything unavailable; REPORT.md and handoff copy.

## Step 1: Activity validated
- Runs is default. Existing ledger, live workers, details/spans, stop, jump-to-session, filters and approval/alert/fact/memory audit retained. One loaded-list totals line; existing monitor/budget warning strip retained. System flattens engine/logs, maintenance/storage, sessions and Manage profiles. Usage charts/model/skill analytics preserved; removed only Usage decorative stat tiles.
- Parent amendment read before commit: no Scheduled tab, retain Scheduled rail destination. SOL-ADVICE.md read. Supersedes initial embedding choice above; no scheduler change remains.
- Compatibility: `/agents` redirects with replace to `/command-center?section=runs`; openAgents/nav.agents API paths unchanged. Legacy system section aliases enter System. Activity occupies old Command Center slot; separate agents nav row removed.
- Validation (isolated env under /private/tmp/factri-ui-activity): `npm run typecheck` PASS (all 3 TS projects); `npm exec -- vitest run --project ui src/app/command-center/ src/app/agents/run-history.test.tsx src/app/chat/sidebar/run-history-nav.test.tsx src/app/routes.test.ts --maxWorkers 2` PASS, 6 files / 16 tests. First new suite failed due to an incomplete test mock, fixed with importOriginal. `npm exec -- eslint` on all touched TS/TSX with --fix PASS. `git diff --check` PASS.
- Disk after npm ci: 37 GiB. npm install completed, no other install active. No GUI claim yet; actual-pointer QA follows after account row.
