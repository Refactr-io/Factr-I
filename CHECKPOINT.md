# UI-LOAD checkpoint

## Rules and scope
- Worktree: `/Users/rameelmalik/factri-ui-load-wt`, branch `feat/ui-loader`. No push, no credentials, no engine-core changes.
- Read COMMON-RULES, ADVICE-FROM-PARENT, UI loader handoff, REQUIREMENTS, Claude hands-on notes, FABLE-ADVICE, OPUS-PLAN, supplied loading component, DESIGN-refactr and RTK. Inspected both provided Claude icon-strip images; no loader-current image exists.
- Handoff design authority: DESIGN-refactr, existing theme tokens, pixel reference. Font families remain untouched.
- Graph tier Verify: `factri-int10-app`, generation `2026-10-19T21:05:07Z`, fast index of integration worktree. Symbol searches and one-hop traces used first. Exact candidate coverage reports no recorded gaps; JSX callers and excluded tests checked in this worktree's source because graph calls omit JSX relationships. Broad coverage includes ignored tests; no completeness claim relies on it.
- Disk initial check: 42 GiB available. One npm install running, no cargo work.
- Tests run with isolated HOME, FACTR_HOME, FACTR_CONFIG_HOME and XDG_CONFIG_HOME under `/private/tmp/factri-ui-loader`; npm user config disabled. Electron userData will be isolated explicitly.

## Step 0: before and discovery
- Before commit: `ff34ab1` (`before: replace inline loaders and startup screens`).
- Inline orb callers: thread/status (loading session, response, quiet-turn activity), thread/message-parts (reasoning), tool/fallback, tool/delegate, page-loader, chat-swap overlay + sync badge, composer voice controls, terminal connection, onboarding/flow, session-import, updates, desktop-install, screenshot-settings, boot-failure settings fallback.
- Ring callers: LatticeLoader in sidebar load-more, cron, agents, composer status stack/trigger/model pill, model visibility, statusbar. Shared Button/SearchField/ActionStatus/Favicon and individual async controls also use Loader2.
- SDK GlyphSpinner is a separately exported legacy glyph strip with its own tests; keep its public props compatible while routing to the new visual.
- Startup: GatewayConnectingOverlay currently decodes CONNECTING for up to 1.18 seconds after open. BootFailureOverlay already has retry, repair, connection settings, sign-in and logs; preserve recovery logic. ChatSwapOverlay covers profile wake. Electron index pre-paints theme; update-splash is already the approved Factr-I helmet, not Hermes.
- Package evaluation: `npm view thinking-orbs@0.3.2 version license dependencies peerDependencies repository --json` passed: MIT, zero runtime dependencies, React >=18. Existing manifest already has ^0.3.1. README/source evaluation pending install.
- Next: pixel Loader + shared compatibility routes + timer, validated commit; then boot orb/reconnect/failure design, validated commit; final checks/screenshots/report.

## Advice
- Parent says claude CLI unavailable; no calls attempted. Existing Fable decision applies: pixel grid inline, one boot orb, 3-second elapsed threshold, reduced motion.
- No new architecture: reuse existing elapsed hook, pane visibility, theme and recovery actions.

## Validation
- `git status --short`: clean before work.
- `df -h /`: above 20 GiB.
- Install/test and browser QA results to be appended after milestones.

## Step 1: pixel loader complete
- Replaced the old 578-line curve Loader with the supplied Drive/Dots/Orbit grid, keeping exported LoaderType and legacy curve props accepted for SDK/Kanban compatibility.
- Added one shared SVG pixel mark behind Loader2/Loader2Icon/Loader icon exports. All existing async buttons, prompts, search, settings, onboarding and tool spinners get the same mark without caller rewrites; SVG props/refs stay compatible and caller animate-spin classes cannot rotate the grid.
- Replaced every app AgentOrb caller outside its definition with Loader. Kept LatticeLoader/GlyphSpinner entry points as small compatibility wrappers. Removed unused glyph-strip stylesheet. Orb module is reserved for the next boot-screen step.
- Existing elapsed hook retained: timers only run while viewed. Timer display hidden before 3s, same 8ch UI-font tabular slot before and after; existing tool/thinking shimmer classes now use reference shimmer. Reduced motion removes shimmer/pixel animation in CSS; hidden panes and renderer pause stop pixels.
- Tests: `python3 /private/tmp/factri-ui-loader/run.py npm run typecheck` passed (renderer, Electron, e2e).
- Tests: isolated `vitest run --project ui ... --maxWorkers=2` over loader, glyph-spinner, response status, turn-gap, duplicate activity, status-tail, fallback, tool-group, swap, connector-card, activity-timer: 11 files / 65 tests passed (11.90s).
- ESLint --fix on changed TS/TSX and Prettier applied; no errors. One test-only restricted document warning corrected to globalThis.document. `git diff --check` passed.
- Native/browser reduced-motion and geometry assertions still pending the shared final visual pass; Chromium download underway in isolated HOME. No full backend run yet.
- Package install completed serially; after install disk has 37 GiB free. Evaluated pinned thinking-orbs 0.3.2 installed README and implementation: color tint accepts RGB/hex, reduced motion paints one representative frame, offscreen/document hidden pauses. Upgrade staged with boot step, not this milestone.
- Parent advice reread; UI batch amendment concerns panes/Activity, does not change loader scope. No STOP.
- Next: new boot/status surface, fast interruptible settle/fade, non-blocking reconnect with retry, retain all failure recovery actions.
