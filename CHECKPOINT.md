# Factr-I Rust RAM task checkpoint

Run started 2026-10-18 on `feat/rust-ram`, starting commit `9b669100`.

## Rules and isolation

- Read COMMON-RULES.md, rust-handoff.md, pause notes, baseline, RAM plan, port plan and RTK.md completely.
- No pushes, remote changes, credential reads, release-checkout access, or benchmark changes.
- Tests use `/private/tmp/factri-rust-run/{home,factr,config,userdata}`. Python golden workers create their own isolated homes.
- One Cargo target directory: `/Users/rameelmalik/factri-rust-wt/target`.
- Graph: `Users-rameelmalik-factri-rust-wt`, generation `2026-10-17T21:49:51Z`, Verify tier. The new UI route files are untracked by that generation; read source directly. Golden generator is excluded from the graph.

## Milestone 1: setup (validated)

- Initial `git status --short --branch`: clean `feat/rust-ram`.
- `df -h /`: 93 GiB available, above the 20 GiB stop threshold.
- Installing dependencies with `npm ci --ignore-scripts` in desktop, using an isolated HOME and npm cache, with user npm config disabled. Did not use the pause note's copy command because its source is the forbidden release checkout.
- Running `cargo test --release -p factr-gateway` at the unchanged tip with isolated Factr dirs and the single worktree target. Log: `/private/tmp/factri-rust-run/logs/gateway-baseline.log`.
- Bundled Python golden check at unchanged tip: PASS (`dump_ui_routes.py ... --check`, exit 0).
- `npm ci --ignore-scripts --cache /private/tmp/factri-rust-run/npm-cache` in desktop: PASS, 1191 packages installed. Install lifecycle scripts intentionally disabled; Electron/native binary setup will be run explicitly if needed.
- `env HOME=/private/tmp/factri-rust-run/home FACTR_HOME=/private/tmp/factri-rust-run/factr FACTR_CONFIG_HOME=/private/tmp/factri-rust-run/config RUSTUP_HOME=/Users/rameelmalik/.rustup CARGO_HOME=/Users/rameelmalik/.cargo CARGO_TARGET_DIR=/Users/rameelmalik/factri-rust-wt/target cargo test --release -p factr-gateway` in engine: PASS, exit 0. 341 unit tests passed, 1 ignored; 30 integration tests passed; doc tests passed. Release compilation took 7m31s. Existing factr-learn dead-code warning only.

## Milestone 2: resumed parity and pet QA (validated)

- Resumed by reading this checkpoint, `git status`, `git log -8`, and the full working diff before reading both handoff files and all referenced plans/rules. The interrupted diff was partial test work; completed and validated it instead of committing it as-is.
- Graph Verify tier refreshed: project `Users-rameelmalik-factri-rust-wt`, generation `2026-10-18T11:33:33Z`. Coverage checked for every affected source; no recorded gaps. Read exact source for the fast paths, Python profile/delivery resolution and RPC dispatch.
- Expanded real-Python goldens from 2 to 22 shared cases: empty/current/default/named/missing/invalid profile queries, a named engine selecting default, managed overlays, local/root plugin dirs, top-level/nested command voice providers, empty/present/named jobs, both pairing layouts, connected delivery platform, auxiliary alias. Both command voice goldens actually contain `local_speaker` in Python's options.
- Fixed demonstrated parity bugs conservatively: explicit default on a named engine, cron `current`, root plugins, and blueprints with other profile dirs now proxy rather than return incorrect baseline data. Separate per-case Tokio runtimes prevent background gateway timers from leaking between cases.
- Pet RPC test covers enabled pets, named-profile homes, explicit default, session-bound polls and managed dirs. Found/fixed enabled pets being incorrectly stubbed while Python was stopped. Other boot probes retain their existing behavior.
- Retained regression checks for env expansion, malformed config, pinned auxiliary endpoints, synthetic ElevenLabs keys and connected-platform dotenv settings. No real credentials read.
- Bundled Python command (PATH only inherited; all worker HOME/FACTR dirs fresh): `PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=$PWD/backend:/Users/rameelmalik/factri-qa/apps/qa-variant.app/Contents/Resources/backend-python/packages /Users/rameelmalik/factri-qa/apps/qa-variant.app/Contents/Resources/backend-python/runtime/bin/python3.12 backend/scripts/dump_ui_routes.py engine/crates/factr-gateway/tests/fixtures/ui_routes --check`: PASS, all 22. Log `/private/tmp/factri-rust-run/logs/goldens-check-milestone2.log`.
- `env -i PATH="$PATH" HOME=/private/tmp/factri-rust-run/m2-clean/home FACTR_HOME=/private/tmp/factri-rust-run/m2-clean/factr FACTR_CONFIG_HOME=/private/tmp/factri-rust-run/m2-clean/config RUSTUP_HOME=/Users/rameelmalik/.rustup CARGO_HOME=/Users/rameelmalik/.cargo CARGO_TARGET_DIR=$PWD/target cargo test --release -p factr-gateway` (run in engine with absolute target path): PASS, 342 unit tests, 1 ignored, 30 integration tests, doc tests. Log `gateway-milestone2-clean.log` in the same logs dir.
- Added guard unit test afterwards: `cargo test --release -p factr-gateway --lib ui_rest::tests::uncertain_config_and_network_credentials_still_require_python`: PASS. Log `ui-guards-milestone2.log`. `git diff --check`: PASS.
- Harness correction: initial resumed full run reused the old isolated config containing a synthetic permanent `cargo publish` approval, causing approval tests to wait indefinitely. Stopped only that test PID and reran with fresh homes. An observability timing test failed under that initial run, passed alone and in the clean full run. Always use NEW test homes, not the baseline ones.
- Bundled runtime has no pytest (`python -m pytest --version` failed); golden generation itself passed. Install pytest into isolated temporary packages before Python regression tests in milestone 6.
- Disk before next build: 80 GiB free. Parent advice file read: no advice/STOP.

## Advice

- Asked Fable: “Advice only, no tools or repo access. A Rust read-only fast path falls back to Python on uncertainty. Python resolves profile=default to the root home, while the engine may run under root/profiles/scribe. Plugin discovery can use root/plugins as well as the profile plugins dir. Is the safest QA fix to proxy explicit default from a named engine and proxy schema/blueprints when either plugin dir exists, leaving profile resolution in Python? Any small boundary I should include?”
- Result: `claude` executable unavailable (also absent from ~/.local/bin, /opt/homebrew/bin and /usr/local/bin). No answer received. Chose existing conservative proxy behavior, no new architecture.

## Next

3. Native ports: manual/blueprint Scheduled-job dialogs call `/api/cron/delivery-targets`, which is still forwarded. Port the safe no-platform case, keep platform/plugin/managed cases in Python, parity-test and measure built-engine process startup. Review named-profile delivery options as a small follow-on if exact parity is practical. Do not port messaging/platforms, toolsets, or dynamic schema discovery.
4. Renderer long-chat memory, preserving the existing 600-unit paging behavior (300 was previously measured and not adopted).
5. Electron idle costs, then full QA and final docs/cleanup/report.
