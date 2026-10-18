# Factr-I bots: final resumed QA report

Branch: feat/bots-step2. Worktree: /Users/rameelmalik/factri-bots-wt-s2. Local commits only; no push or remote changes. No real credential files were read, printed or copied. The release checkout and benchmark results were untouched.

## Current verdict

The interrupted local routing/rename milestone is validated and committed as dac9c7e. The autonomous delivery watchdog correction is validated and committed as 73c6a05d; the rebuilt-image real pair passed all 8 tests. The final image probe passed all 57 checks.

Recovered changes were inspected rather than blindly restored. Local native chat/group/session/attachment/answer/approval RPCs now use the bot's owning engine; roster batches and metadata remain passive. Canonical creation and local group turns retain their connection for the entire operation. Normal local UI identity is preserved. The Sandbox status and provider-sharing explanation reflect the isolated runtime.

The failing room test was a product race: an edit under the old legacy name remained in a coalesced rename sync job, preventing its tombstone. Only rooms present in the current local snapshot receive change revisions. The direct merge test and late-clarify rename/disband/no-resurrection regression pass. Test fault injection now intercepts the actual requestProfile boundary.

Real Docker diagnosis distinguished three stages: the model emitted a queued envelope, B received its attributed turn, then B's initial autonomous response was cancelled by the interactive 60-second intro watchdog while the serial Ollama server was still serving A. The correction scopes the intro watchdog to interactive turns; native bot delivery retains its existing ten-minute attempt cap and normal Stop. Headless API turns keep their configured request timeout. A real-engine delayed-response regression verifies both watchdog behaviors. The fixture also uses an absolute file path (the native read tool treated ~/ literally), checks delivery failure before waiting for a sender wake, and preserves only synthetic histories/relay/named-container logs. Two diagnostic runs were intentionally interrupted after establishing their failures; neither counts as a pass. Their exact pairs and empty test networks were immediately removed.

## Evidence from this resumed run

Every test used fresh HOME, FACTR_HOME, FACTR_CONFIG_HOME and Electron userData under /private/tmp via /private/tmp/bots-test.py. Commands were prefixed with rtk. Docker info was checked first; test containers were bots-prefixed, at most one pair at a time.

| Check | Result / log |
| --- | --- |
| Full Bots UI | 96 files, 775 passed; /private/tmp/bots-resumed-ui2.log |
| Final group turn/sync and local lease regression | 2 files, 92 passed; /private/tmp/bots-resumed-groups-final.log |
| Electron computer/setup/sandbox/retire/spawn | 5 files, 105 passed; /private/tmp/bots-resumed-electron-tests.log |
| Renderer / Electron TypeScript | Passed; bots-resumed-renderer-tsc2.log, bots-resumed-electron-tsc.log and bots-resumed-real-tsc.log |
| Renderer / main bundle | Passed; bots-resumed-build.log, bots-resumed-main-build.log |
| Full gateway crate, serial | 406 passed, one ignored; /private/tmp/bots-resumed-gateway2.log |
| Test-profile engine build | Passed; /private/tmp/bots-resumed-engine-build.log |
| Real Linux sandbox + engine + Python feature | Passed; /private/tmp/bots-resumed-linux.log; own container removed |
| Final rebuilt-image probe | 57 passed, zero failed, exit 0; /private/tmp/bots-resumed-image-probe.log; its containers/network removed |
| Current watchdog/create/delivery harness | All checks passed, exit 0, including the delayed native first response; /private/tmp/bots-resumed-watchdogs.log |
| Source image rebuild | Passed; /private/tmp/bots-resumed-image-build2.log; manifest sha256:8d4d0b0eb1038101ff0df1a9c4dca7707df536f066fd666601dbe888458d86bf |
| Final rebuilt-image real two-computer suite | 8 passed, exit 0, 878.71s; /private/tmp/bots-resumed-real-final.log; exact pair/network removed |

The first resumed UI run found two stale fixture boundaries and the first renderer check found nullable error formatting; these were fixed before the green runs. The first Rust check caught a test-only replacement typo; it was corrected before the full green gateway run. The earlier 125/1 and 7/1 results remain historical failures, not evidence for the final source.

## Delivered work across the handoff

1. Setup/baseline: independent worktree dependencies, one cargo target under this worktree, gateway baseline recorded.
2. Startup/messaging/pool: bounded teammate MCP handshake with typed visible error, interactive first-response watchdog, default sessions created before the first bot gain native message_agent, three-engine LRU retirement admission/physical-exit coordination and restart deduplication. The historical packaged 13-minute root cause was not uniquely reproduced.
3. Upstream parity audit: /Users/rameelmalik/factri-ogb-notes/bot-parity-upstream.md records upstream file:line, decisions and deviations. Native message_agent keeps the audited name/schema/description, fire-and-forget queued id, roster validation, attribution and later sender wake. Hop cap 2 remains a deliberate loop-safety difference.
4. Rust seams: literal routine turns, safe saved-wrapper upgrade, owner-engine group member session pointers, endpoint/context inheritance, descendant Stop, Description loading, blank routine-name validation, delete settlement, silence/auth behavior and canonical scheduled output. Core agent loop/tools/providers/prompts were not ported. Full durable room-round execution remains partial.
5. Automated engine checklist: bot-checklist.mjs accounts for all 17 contracts and invokes real isolated engine/stub suites. It is not a claim that 17 full GUI/external-account flows passed.
6. Computer integration: existing branch changes integrated at the current engine/courier seam, scheduled retention/startup and spec recreation, explicit Stop persistence, native scheduled Bot Chat delivery, feature-link recovery after Python idle exit. Final real pair result is above.
7. Setup/fallback: platform-aware Docker status/setup plans and consent; no silent unconfined retry. Private whole-engine local sandboxes with own state/browser directories. Mac native engine/browser/relay/schedule tests and Windows launcher fakes were validated in earlier milestones (exact logs in CHECKPOINT.md).
8. Platforms (cdda98b): small platform interfaces/fakes, repeated actual non-root Linux confinement/engine/feature check inside a Linux container, and exact Windows/native-Linux/manual matrices in NEEDS-GUI-QA.md. No real Windows machine was available.
9. Documents: appended new sections to bots-virtual-computers-design.md preserving sections 1-13, including upstream deviations and corrections to earlier MVP assumptions. Final cleanup and report-copy result recorded below.

## Remaining QA and product limits

- Full gateway-owned durable group-room rounds remain partial (Rust driver=false); Desktop drives rounds. Closing Desktop is not certified to continue them.
- No real Windows confinement, ACL, private-desktop, browser, Job Object or packaged-install result. Linux evidence is nested container execution, not a native Linux desktop/Wayland result. Setup/elevation/install consent paths are fake-tested; no host installation or Docker restart was performed.
- Bare Linux without Docker and usable bubblewrap fails closed; bubblewrap is not yet bundled. This prevents claiming universal install-only-Factr-I support.
- No prebuilt image publication, official signed package or cold browser acquisition validation. The local macOS/Windows fallback has private browser storage, not a separate streamed OS desktop.
- Local OS sandboxes share IP loopback/internet and are not VMs/network boundaries. Docker's existing optional one-time OAuth seed can face refresh-rotation conflicts; local fallback does not copy host OAuth stores. Per-bot provider sign-in remains manual QA. Old shared-engine transcripts are retained without automatic import.
- Local schedules and courier require Desktop. Running Docker schedules retain their documented quit behavior; a stopped computer cannot fire until started.
- No final visible-app pass: a fresh ad-hoc-signed QA copy pinned isolation before imports, but Computer Use returned -10005 timeout selecting the window by path and bundle id. No final pointer clicks were claimed. NEEDS-GUI-QA.md names the exact create/reopen/attachment/clarify/rename and platform checks.
- The detailed supplied design-law review found no new layout, typography, palette, animation, icon, shadow or control composition in this recovery. Changes use the existing house UI and plain status/copy. Existing visible controls still need the documented real-pointer, focus, contrast, clipping and narrow-width check; no full visual certification is claimed.

## Resume commands

Keep isolated directories; never point tests at real profiles. /private/tmp/bots-test.py creates fresh directories per invocation and pins this worktree's target. If that helper is absent, recreate the environment described in CHECKPOINT.md before running.

```sh
rtk proxy python3 /private/tmp/bots-test.py engine cargo build --profile test --bin factr --locked
rtk proxy python3 /private/tmp/bots-test.py engine cargo test -p factr-gateway --locked -- --test-threads=1
rtk proxy python3 /private/tmp/bots-test.py . env CHECK_PREAMBLE=1 node engine/crates/factr-gateway/e2e/bot-chat-deliver.mjs
rtk proxy python3 /private/tmp/bots-test.py . node engine/crates/factr-gateway/e2e/bot-checklist.mjs
rtk proxy python3 /private/tmp/bots-test.py desktop/app ../node_modules/.bin/vitest run --project ui src/plugins/factr-bots
# Recreate generated desktop output before packaged/manual QA:
rtk proxy python3 /private/tmp/bots-test.py desktop/app env FACTR_QA_FUSES=1 npm run build
rtk proxy python3 /private/tmp/bots-test.py desktop/app node scripts/stage-backend-python.mjs
```

Before Docker commands, check docker info. An isolated HOME on this Mac needs DOCKER_HOST pointing at the already-running Desktop socket and a generated non-secret DOCKER_CONFIG containing only the CLI-plugin path. Do not read the user's Docker config. Build the source image, then run one real pair, clean it, then the Linux harness/image probe sequentially.

```sh
rtk proxy docker info
# With the isolated Docker environment, from the repo root:
rtk proxy docker build -f computer/Dockerfile -t bots-factr-bot-computer:dev .
# From desktop/app, still isolated:
rtk proxy env FACTR_REAL_DOCKER=1 ../node_modules/.bin/vitest run --project electron electron/bot-computers.real.test.ts
# From the repo root, after the pair is removed:
rtk proxy bash computer/tests/test_local_sandbox_linux.sh
rtk proxy bash computer/tests/test_computer_image.sh
```

## Cleanup and handoff

The final pair and image-probe traps removed all their containers and the empty test network; final bots- inventory is empty. No process from this worktree's target or disposable QA copies remains. The three disposable QA app copies and current UI stub server were removed/stopped. Generated target/, desktop/app/dist/ and desktop/app/build/ were deleted after checking they contained no tracked files and were not external symlinks. Rebuild before another host-engine test.

Worktree dependencies, synthetic test logs/diagnostics and the 2.53 GB source-built bots-factr-bot-computer:dev image are intentionally retained for manual QA. Shared Docker/BuildKit caches were not pruned. The image manifest is sha256:8d4d0b0eb1038101ff0df1a9c4dca7707df536f066fd666601dbe888458d86bf.

Validated local milestones: dac9c7e, 73c6a05d, cdda98b, 810e7ea. The final image-probe/document/cleanup commit follows those. This report is copied verbatim to /Users/rameelmalik/Documents/Sovereign AI/handoff/bots-REPORT.md. No push.
