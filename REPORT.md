# Factr-I: second bot creation routing

Task: bots-create. Worktree branch: `fix/bots-second-create`. Implementation commit: `33d1fdc`. Local commits only; no push.

## Root cause and fix

`host.request` intentionally dispatches through the live `$gateway`. Creating Alpha calls `createCanonicalChat`, which activates Alpha's own engine. The create dialog's `requestForTarget` then used that engine for Beta's `profiles.create`, resolving Beta under Alpha's private config home. This was a renderer management-routing error, not a primary/pool ownership change.

The shared create-target helper now sends local management RPCs to the existing explicit `local/default` route whenever a non-default local bot is focused. This reuses the roster's root-routing pattern and covers normal creation, lazy draft creation, capability preview and capability configuration through that helper. An explicit remote target still wins. Root-request errors propagate into the existing visible dialog error; there is no retry into the sandbox. Existing default-profile, remote and older-SDK routing, session routing, pool logic, engine core and rendered markup are unchanged.

Only production file changed: `desktop/app/src/plugins/factr-bots/create-dialog.tsx`. Regression coverage is in the existing `create-agent-lazy-profile.test.tsx`. No new dependency, version, lockfile, Rust or backend change. Previously misplaced profiles are not migrated or deleted by this fix.

## Validation

Before editing production code, the focused suite failed exactly the three new regressions while all seven existing cases passed. Both normal and lazy consecutive creates wrote Beta into `profiles/alpha/.local-computer/config/profiles/beta`; the expected primary-backend error never appeared. After the fix all ten cases pass. The fixture uses the real dialog, a fake host that models focus switching, and actual temporary profile directories; it does not launch real bot engines.

| Check | Result | Evidence |
| --- | --- | --- |
| Pre-fix regression | Expected failure: 3 failed, 7 passed | `regression-before.log` |
| Final focused regression | Pass: 10 tests, 6.99s | `focused-final.log` |
| Renderer, Electron and E2E TypeScript | Pass: all three `tsc --noEmit`, 19.56s | `typecheck.log` |
| Touched-file ESLint | Pass: no errors or warnings | `lint-release.log` |
| Full UI project | Pass: all 987 files / 8231 tests, 242.64s | `ui-full.log` |
| Full Electron project | Pass: 231 files, 2484 tests; 3 files / 19 tests skipped, 35.68s | `electron-full.log` |

The Electron skips are eight opt-in real-Docker tests, one opt-in live-retirement test, and ten WSLg process tests on macOS. All 11 local-sandbox tests pass, including real macOS confinement. Pool limits, reclamation, retirement, coordinator, primary pin, profile routes and deletion routing pass. Existing React act and Vite warnings are retained in full logs; no warnings were suppressed.

Logs, isolated test runner and exact command/exit/duration ledger: `/private/tmp/bc-xx9sync2/`. The initial focused attempt lacked installed dependencies; isolated `npm ci --ignore-scripts --no-audit --no-fund` installed the existing 1191-package lockfile. The worktree-local Electron runtime was downloaded solely to permit Node imports in the existing Electron suites. No app was launched. Each validation command received fresh HOME, FACTR_HOME, FACTR_CONFIG_HOME, TMPDIR and Electron userData directories under `/private/tmp`, with an allowlisted environment. Personal credential files were not read.

## QA limits and remaining work

No implementation is half-done. Packaged app / real GUI confirmation is pending because this handoff explicitly prohibits those actions. Exact isolated stub-model steps for Alpha then Beta, lazy capabilities creation, primary-backend failure, remote targeting, macOS, native Linux and Windows are appended to `NEEDS-GUI-QA.md`. Real provider/credential paths, Docker, Rust tests and benchmarks were not run for this renderer-only change. No other worktree, release checkout, benchmark output, remotes or versions were changed.

Codebase Memory MCP tools are unavailable in this session. Source and caller verification used the exact local files; graph project, generation and coverage are not certified. Parent advice was read after milestones; there was no STOP or unresolved advice dependency.

Design-law re-check: the change introduces no visual design or JSX. The source from the first Dialog render through EOF was checked byte-for-byte against the pre-fix commit and is unchanged. Typography, color, spacing, iconography, motion, clipping, alignment and accessibility markup therefore have no patch changes to audit. Creation and error controls were exercised in the automated renderer tests; no real-pointer or screenshot claim is made.

## Cleanup and final state

Removed task-generated TypeScript build-info and per-command scratch, including temporary npm and Electron download caches. Retained full logs, command ledger, isolated runner and installed lockfile dependencies for review/retesting. No Rust target, Docker resource or application process was created by this task. Final whitespace and scope checks pass. `CHECKPOINT.md` records milestones; `NEEDS-GUI-QA.md` records the remaining manual checks. This report is copied verbatim to the requested handoff `bots-create-REPORT.md`.

## Commands to repeat

Run from `desktop/app` through the retained isolated runner (it supplies a fresh test environment per command):

```sh
rtk proxy python3 /private/tmp/bc-xx9sync2/run.py focused node ../node_modules/vitest/vitest.mjs run --project ui src/plugins/factr-bots/create-agent-lazy-profile.test.tsx --maxWorkers=1
rtk proxy python3 /private/tmp/bc-xx9sync2/run.py ui node ../node_modules/vitest/vitest.mjs run --project ui --maxWorkers=4
rtk proxy python3 /private/tmp/bc-xx9sync2/run.py electron node ../node_modules/vitest/vitest.mjs run --project electron --maxWorkers=4
rtk proxy python3 /private/tmp/bc-xx9sync2/run.py types npm run typecheck
rtk proxy python3 /private/tmp/bc-xx9sync2/run.py lint ../node_modules/.bin/eslint src/plugins/factr-bots/create-dialog.tsx src/plugins/factr-bots/create-agent-lazy-profile.test.tsx
```

The runner is a temporary evidence artifact. If it is no longer available, recreate the same isolated environment before running the listed underlying commands. There is no Rust build target to resume.
