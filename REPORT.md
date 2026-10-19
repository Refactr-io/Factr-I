# Factr-I flaky engine test report

Completed in `/Users/rameelmalik/factri-flaky-wt` on `fix/flaky-engine-tests`. Five test-isolation causes were fixed in separate local commits. Production engine behavior, dependencies, versions, workflow settings and test ignores are unchanged. No push, remote changes, GUI, Docker or benchmark runs. No real credential files read or copied; no other worktree modified.

## Measured failure rates

Initial baseline used unchanged commit `11e5b0ee`. The first attempt used 16 threads and Cargo's default fail-fast; the second used 16 threads with `--no-fail-fast`; the third used 64 threads with `--no-fail-fast`.

| Failure | Before its fix | Final five full runs |
| --- | --- | --- |
| Workspace attempts | 3/3 failed (100%) | 0/5 failed |
| Risky-delete test assumes HOME is outside scratch paths | 3/3 executions | 0/5 |
| Stop-nudge guard span recorder overwritten | 1/3 executions | 0/5 |
| MCP HTTP OAuth config overwritten | 1/2 executions | 0/5 |
| Hook rewrite reads another test's config | 1/2 executions | 0/5 |
| Approval risk-category test waits for an already-allowlisted command | 1/2 gateway executions hung | 0/5 |
| Undo checkpoint test sees a leaked headless mark | 1/1 intermediate full run; ordered two-test reproduction failed 1/1 | 0/5 |

Unreached tests are excluded from denominators. In baseline run 3, every other gateway unit test finished, but the risk-category test waited indefinitely. Only that child binary was stopped with SIGTERM after 114 seconds, after verifying its PID, Cargo parent and worktree path; Cargo continued the other targets. The original 16-thread baseline gateway result was 386 passed, 1 ignored, plus passing integration targets.

An intermediate MCP validation command mistakenly used crate names as libtest `--skip` filters. It reached app-core rather than base and reproduced the second stop-nudge recorder assertion (`turn_end_span_reports_how_the_turn_ended`). The corrected base validation passed. No skips were added to the code or final proof commands.

The first post-fix full attempt (`after-01`, 185.6s) found the leaked headless mark and failed. It is retained as evidence and excluded from the final streak. Running only the headless tidy test followed by the undo snapshot test with one thread then reproduced that leak deterministically; the identical ordered command passed after cleanup was added.

## Changes and local commits

| Commit | Change |
| --- | --- |
| `0bb55c61` | Record unchanged-source reproduction and isolation setup. |
| `d2666f06` | Bash safety fixture uses a unique checkout directory and the existing env lock, instead of mutable HOME. |
| `600b58ad` | MCP OAuth test takes the shared storage env lock through connect, call and restoration. |
| `6839f589` | Both stop-nudge tests and the two unlocked gateway recorder tests use existing shared test locks. |
| `9e9d6239` | Approval tests use unique config homes with restoration; all 19 channel receives are bounded at 10 seconds. Adds a permanent-grant isolation regression. |
| `5cb83a59` | Five approval scenarios release all six previously leaked process-wide headless marks. |

Changed Rust files, all test-only:

- `engine/crates/factr-app-core/src/tool/bash_tests.rs`
- `engine/crates/factr-app-core/src/agent/stop_nudge.rs`
- `engine/crates/factr-base/src/mcp/client.rs`
- `engine/crates/factr-gateway/src/learn.rs`
- `engine/crates/factr-gateway/src/rpc.rs`
- `engine/crates/factr-gateway/src/approvals.rs`

The production prefixes before the test modules are byte-identical to the starting commit. `bash_tests.rs` is included only under `cfg(all(test, not(windows)))`. No engine bug requiring production changes was found. `CHECKPOINT.md` retains the earlier integration history and this run's milestones. The prior integration report is archived in the evidence directory and preserved in Git history.

## Validation

Targeted checks passed: 47 bash tests at 64 threads; all selected workspace libraries including base's 1,366 tests at 64 threads; 10 matching span tests across app-core/base/gateway at 64 threads; 25 approval tests at 64 threads, including the new regression; and the ordered headless/undo reproduction.

The final five commands are identical, in the test profile:

```sh
cargo test --workspace --locked --no-fail-fast -- --test-threads=64
```

Each uses fresh HOME, FACTR_HOME, FACTR_CONFIG_HOME, TMPDIR, XDG_CONFIG_HOME and Electron userData beneath `/private/tmp`, an allowlisted environment, an isolated Cargo cache, and the single `engine/target` directory. Only one Cargo build/test command ran at a time.

| Run | Exit | Seconds |
| --- | --- | --- |
| `after-02` | 0 | 192.3 |
| `after-03` | 0 | 145.6 |
| `after-04` | 0 | 143.4 |
| `after-05` | 0 | 144.5 |
| `after-06` | 0 | 145.0 |

Each clean run completed all 147 test/doctest targets. Major unit targets per run: app-core 1,108 passed / 4 ignored, base 1,366 passed / 3 ignored, gateway 387 passed / 1 ignored. Integration and doc targets also passed. No new retry or skip was introduced. Whitespace and the test-only source assertions passed, and every local milestone commit has the required co-author trailer.

## Evidence and resume commands

Evidence directory: `/private/tmp/factri-flaky-l0nrzadt`. It retains `run.py`, `proof.py`, `check-test-only.py`, `results.tsv` and every named `.log`. Exact validation commands, exits and durations are in `results.tsv`. The runner source records the complete isolated environment. CI logs are `/private/tmp/factri-flaky-ci-38178832627.log` and `/private/tmp/factri-flaky-ci-38144763796.log`; these older CI failures were inspected read-only and are not counted as current-branch reproductions.

To rebuild and repeat on this Mac after cleanup, from this worktree:

```sh
rtk proxy mkdir -p /private/tmp/factri-flaky-l0nrzadt/cargo
rtk proxy cp -cR /Users/rameelmalik/.cargo/registry /private/tmp/factri-flaky-l0nrzadt/cargo/registry
rtk proxy python3 /private/tmp/factri-flaky-l0nrzadt/run.py resume-01 test --workspace --locked --no-fail-fast -- --test-threads=64
rtk proxy python3 /private/tmp/factri-flaky-l0nrzadt/check-test-only.py
rtk git diff --check
```

Use an unused run label on subsequent invocations. The cache bootstrap copies only the public Cargo registry, never Cargo config or credentials.

## Limits and cleanup

Nothing remains half-done in the requested macOS test-only scope. Windows/Linux, additional feature combinations, packaged-app and GUI QA were not run. The task explicitly excluded GUI work and changed no interface. No production port was made, so Python parity testing was not applicable. The old CI `/usage` failure and historical `DirectoryNotEmpty` cleanup race were not reproduced on this branch; no speculative production fix was made for them. Five clean runs are measured evidence, not a guarantee against every possible future schedule.

Removed only this task's `engine/target`, cloned Cargo cache/registry and per-run scratch directories after validation. Free space increased from 56.6 GiB to 70.7 GiB. No engine/target executables remained running before deletion. Logs and the reproduction runner remain available; a future run rebuilds the target.

Report copied to `/Users/rameelmalik/Documents/Sovereign AI/handoff/flaky-REPORT.md`.
