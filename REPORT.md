# Factr-I Rust RAM task report

2026-10-18, branch `feat/rust-ram`, resumed from `9b669100`. Authorized implementation and validation milestones are complete. Nothing pushed. Adopted code has no half-finished work. Remaining manual coverage and inherited issues are explicit below and in [NEEDS-GUI-QA.md](NEEDS-GUI-QA.md).

## Done and verified

- Expanded the native UI contract from 2 to 22 real-Python golden cases: profile values, named-engine default, managed scope, profile/root plugins, top-level/nested command voices, jobs, pairing and delivery platforms. Incorrect or uncertain native answers now proxy.
- Added native `GET /api/cron/delivery-targets` for the knowable baseline. Settings, Scheduled jobs and New job stay Python-free in the combined app. Messaging/platforms, tools/toolsets, plugin-driven schema and keyed/network voice discovery remain in Python as directed.
- Fixed enabled pets being stubbed while Python was stopped. Session/managed/ambiguous profile polls forward; canonical existing disabled profiles stay local. Real app RPCs with a synthetic valid atlas cover enabled root, named scribe/uppercase SCRIBE, managed overlay and missing-profile error 4064, including full sprite bytes.
- Bounded both Markdown parse caches to 1,048,576 UTF-16 source code units each plus their existing entry caps. Oversized documents still render fully. Closed inter-agent reply bodies/images are unmounted and released on close.
- Muted haptics skip the AudioContext warm-up, including a queued-callback race. Enabled idle timeout stays 2000 ms. Chat and Markdown file previews share a lazy memoized KaTeX runtime; optional chunk failure preserves readable source. KaTeX CSS stays eager. GPU/network settings unchanged.
- Preserved each pane's render callback across title updates, preventing React body remount and local state loss. The new mount/draft-state regression fails on the old implementation and passes on the fix.
- Engine agent loop, tools, prompts and providers were not changed. No virtualizer, paging-budget change, broad Python import rewrite or new architecture.

## Measurements

Matched local runs, fresh homes, loopback stub, one unfocused sliver window. RSS sums double-count shared pages; these are relative measurements, not physical-memory totals.

| measurement | before | after |
|---|---:|---:|
| Delivery-targets first read | 6259.72 ms | 6.57 ms |
| Delivery-targets warm read | 2.85-3.38 ms | 0.49-0.59 ms |
| Python during eight eligible dashboard reads | child up to 244.25 MB on delivery-targets path | none, no forwards; final engine peak 31.94 MB |
| Long-snapshot cache retained heap after caller release | 43.94 MB | 1.81 MB |
| Renderer at 200 settled exchanges | 1026 MB | 1005 MB |
| Renderer after forced GC | 688 MB | 645 MB |
| Peak total during that replay | 1528 MB | 1509 MB |
| Muted idle AudioService | 58 MB | absent |
| Muted idle total | 689 MB | 628 MB |
| KaTeX JS at idle/plain reply | loaded | absent; loads on first math |

The cache check is a synthetic forced-GC retention test; matched repeat elapsed was 466/457 ms, not a broad CPU claim. Both 200-exchange app runs delivered all 200 replies (202 stub POSTs) with the same 7244 DOM nodes and 151 mounted groups. The roughly 1 GB plain-chat plateau is not solved; natural settled RSS depends heavily on GC. Parent approved retaining 600 DOM / 1200 store paging budgets. Muted total also includes lazy math and run noise; enabled users still start audio by design. Startup can briefly probe a Python interpreter before the request phase; no claim that Python never executes at boot.

Raw JSON, scripts, screenshots and logs: `/private/tmp/factri-rust-run/`. Main files: `delivery-{before,after}.json`, `native-final.json`, `markdown-cache-{before,after}-repeat.json`, `renderer-before.json`, `renderer-after-m4.json`, `audio-{before,after}.json`, `math-{before,result}.json`, `pets-result.json`, `smoke-result.json` and `qa/evidence/`. Updated the authorized external `rust-ram-baseline.md` and `rust-ram-plan.md` with these results.

## Validation

All final commands used fresh isolated homes via `/private/tmp/factri-rust-run/isolated.py`; one worktree Cargo target and one Cargo build at a time.

| check | result | log under logs/ |
|---|---|---|
| `cargo test --release -p factr-gateway` | 343 unit pass, 1 ignored; 30 integration pass; doc tests pass | gateway-final.log |
| `cargo test --release -p factr-cli --bin factr` | 8 pass | cli-final.log |
| `cargo clippy --release -p factr-gateway --all-targets` | pass, existing warnings | clippy-final.log |
| `cargo build --release -p factr-cli --bin factr` | pass | build-final.log |
| Real bundled Python golden check | all 22 cases pass | goldens-final.log |
| Python UI golden/delivery/pet-toggle tests | 4 pass | python-final.log |
| Python unavailable-profile/pet RPC subset | 11 pass, 604 deselected | python-pet-final.log |
| Renderer cache/transcript/haptics/math/media | 130 pass, 13 files | renderer-final.log |
| Markdown preprocessing | 19 pass, 3 files | renderer-preprocess-final.log |
| File/pane/bar/attachment tests | 62 pass, 4 files | renderer-preview-final-clean.log |
| Pane/title and preview contribution tests | 13 pass | pane-title-final.log |
| Session/route caller tests | 9 pass | pane-callers-final.log |
| Keep-alive host and title regression | 9 pass, overlaps title suite | pane-lifecycle-final.log |
| TypeScript, touched-file ESLint, Vite production build | pass; final build 20.55 s | typecheck-pane-title.log, lint-final.log, lint-pane-title-clean.log, vite-final.log |
| Real-engine eight-route responses/process sampling | exact goldens, no request Python/forwards, no PIDs left | native-final.log |
| Combined app Settings/jobs/New job, math, disclosure images | pass, zero console errors | pages-final-combined.log, math-final-combined.log, disclosure-final-combined.log |
| Combined app attachment/@/chat/browser controls/navigation/tabs | functional assertions pass; inherited native exception below | smoke-safe-guest.log |
| Enabled/named/managed pet app RPCs | pass, synthetic atlas | pets-final.log |

Existing jsdom canvas/React act, Vite configuration and unrelated Clippy warnings remain. Tests were corrected to await lazy math and to use the actual native attachment picker/consent/accessible labels. Failed test-harness assumptions were inspected through screenshots/logs before drawing product conclusions. `git diff --check` passes. Applicable UI-law checks covered visible fallback content, clipping, alignment, contrast, gutters and actual control clicks; existing visual styling was preserved.

## Remaining QA and known limits

- Inherited Electron `WebViewImpl.createGuest` null-conversion console exception on return to a loaded Browser tab. Reproduced on the previous renderer and final build. The guest stays continuously connected and usable. The independent title-remount fix does not remove this native exception. Exact repro and capture steps are in NEEDS-GUI-QA.md.
- Each enabled-pet scenario left one Python helper after graceful app quit; the harness removed it and confirmed no processes remained. Do not claim every feature child quits gracefully.
- Physical microphone/voice prefetch, real ElevenLabs traffic, agent-driven first navigation into a blank Browser, visible-window long-chat/accessibility, real pet animation, multi-window and Windows/Linux remain untested. Earlier base error-card checks for 429/401/404/500 were not investigated. Manual steps and expected measurement bounds are in NEEDS-GUI-QA.md.
- The built binary's cached `__version` sha reports 9b669100 even after gateway/CLI rebuilds. The QA manifest uses the actual probe; build logs and native response checks prove the changed code ran. This is not a release package.
- Isolation incident: the first temporary QA copy retained the production bundle id and the guard detected a metadata change to the real preferences plist. No contents were read or restored. Changed only the QA copy to a unique bundle id; every subsequent passing guard reports real home unchanged. Runtime recovery read the installed app only; it was never launched or modified. No credentials were read, no release checkout or benchmark output was touched.

## Local milestones

`edaf8ae` parity/pet QA; `d77b732` native delivery targets; `a5b58ec` bounded renderer retention; `01e65d7` muted audio/lazy math; `a4341f5` pet boundary/final runtime QA; `a680d72` pane title lifecycle. CHECKPOINT.md contains exact history and evidence. Parent advice approved the adopted renderer/audio scope; read after milestones, no STOP.

## Cleanup and exact resume commands

Removed the 9.2 GB logical Cargo target, 924 MB workspace dependencies, unused renderer outputs, duplicate runtime and recorded scratch homes after validation. Free disk is 88 GiB (79 GiB before final builds). One ad-hoc-signed QA app and standalone Playwright harness are retained under `/private/tmp/factri-rust-run/` for the remaining GUI checks. `python-bundle` points to that app's runtime. Test logs/evidence remain; generated app homes are removed. No test/app process is left running.

Run serially in the worktree, checking at least 20 GiB free before builds. The retained wrapper sets fresh HOME, FACTR_HOME, FACTR_CONFIG_HOME, Electron userData, host locks and an environment whitelist. Do not reuse old approval/config homes.

```sh
rtk git status --short --branch
rtk git log -8 --oneline
rtk proxy df -h /
# From desktop: restore JS dependencies without install lifecycle scripts.
rtk proxy python3 /private/tmp/factri-rust-run/isolated.py npm ci --ignore-scripts --cache /private/tmp/factri-rust-run/npm-cache
# From engine, one command/build at a time:
rtk proxy python3 /private/tmp/factri-rust-run/isolated.py cargo test --release -p factr-gateway
rtk proxy python3 /private/tmp/factri-rust-run/isolated.py cargo test --release -p factr-cli --bin factr
rtk proxy python3 /private/tmp/factri-rust-run/isolated.py cargo clippy --release -p factr-gateway --all-targets
rtk proxy python3 /private/tmp/factri-rust-run/isolated.py cargo build --release -p factr-cli --bin factr
# From worktree root, using the retained QA copy's real runtime:
rtk proxy python3 /private/tmp/factri-rust-run/isolated.py /private/tmp/factri-rust-run/python-bundle/runtime/bin/python3.12 backend/scripts/dump_ui_routes.py engine/crates/factr-gateway/tests/fixtures/ui_routes --check
rtk proxy python3 /private/tmp/factri-rust-run/isolated.py python3 engine/scripts/qa_native_ui.py target/release/factr /private/tmp/factri-rust-run/python-bundle /private/tmp/factri-rust-run/native-resume.json
# From desktop/app after npm ci:
rtk proxy python3 /private/tmp/factri-rust-run/isolated.py node ../node_modules/typescript/bin/tsc -p . --noEmit
rtk proxy python3 /private/tmp/factri-rust-run/isolated.py node ../node_modules/vitest/vitest.mjs run --project ui src/lib/markdown-blocks.test.ts src/components/haptics-provider.test.tsx src/components/assistant-ui/markdown-text.lazy-math.test.tsx src/components/assistant-ui/markdown-text.lazy-failure.test.tsx src/components/assistant-ui/thread/inter-agent-collapse.test.tsx src/app/chat/right-rail/preview-file.test.tsx src/app/chat/pane-mirror.test.tsx
# No rebuild needed to replay the retained combined app, one at a time:
rtk proxy python3 /private/tmp/factri-rust-run/isolated.py node /private/tmp/factri-rust-run/qa/pages.mjs
rtk proxy python3 /private/tmp/factri-rust-run/isolated.py node /private/tmp/factri-rust-run/qa/math.mjs
rtk proxy python3 /private/tmp/factri-rust-run/isolated.py node /private/tmp/factri-rust-run/qa/disclosure.mjs
rtk proxy python3 /private/tmp/factri-rust-run/isolated.py node /private/tmp/factri-rust-run/qa/pets.mjs
rtk proxy python3 /private/tmp/factri-rust-run/isolated.py node /private/tmp/factri-rust-run/qa/smoke.mjs
```

These commands do not read user credentials or access the release worktree. The retained app/harness may be removed after manual QA. Never push from this task.
