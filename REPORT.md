# Factr-I bots resumed run report

Status: BLOCKED at the required Fable advice step. Task 2 is partial; tasks 3-9 remain. Nothing pushed.

Worktree: `/Users/rameelmalik/factri-bots-wt-s2`, branch `feat/bots-step2`.

## Done and verified

- Recovered CHECKPOINT.md, inspected Git status/log and the sole uncommitted test diff before continuing. Validated and committed it as `a185e02`.
- The prior setup milestone `adcb5e3a` remains valid: gateway baseline 396 passed, 1 ignored; engine test-profile build passed. Dependencies are local to this worktree.
- Commit `7c5d220` enables the teammate MCP in the default profile when a Bot Mode roster exists. Installs without a managed roster and plain profiles remain unchanged. Existing model settings are preserved.
- Fixed a real creation edge case: a managed bot without config.yaml was skipped by the MCP installer. It now gets a config and teammate server.
- The real Python feature backend creates/configures a new profile in the integration harness. Its mounted first chat advertises message_agent and completes its intro.
- A default engine, without restart, creates a user chat after adding a bot and offers message_agent on the first model request. The default MCP can deliver through the relay into B's real engine and receive the reply, with the existing @factr attribution.
- No Rust source or UI changes in this resumed run. Relative to pre-Step-2 `a626cbae`, engine changes remain additive bot modules/e2e scripts plus four registration lines in rpc.rs. No new Rust port is claimed.

## Validation

Every run used fresh HOME, FACTR_HOME, FACTR_CONFIG_HOME and Electron userData under /private/tmp. The endpoint was a deterministic local OpenAI-compatible stub, not a paid provider.

| Check | Result |
| --- | --- |
| Backend `.venv/bin/python -m pytest tests/tools/test_message_agent_mcp.py -q` with PYTHONPATH set to backend | 9 passed, exit 0 |
| `node engine/crates/factr-gateway/e2e/bot-chat-deliver.mjs` with FACTR_BIN, FACTR_BACKEND_PYTHON and FACTR_BACKEND_PYTHONPATH | All checks passed, exit 0 |
| `node engine/crates/factr-gateway/e2e/bot-message-agent.mjs` with FACTR_BIN, PYTHON and BACKEND | All checks passed, exit 0 |
| `git diff --check` | Exit 0 |

The delivery suite also checks attribution, canonical-chat ownership, duplicate envelope idempotency, silence, auth classification without retry, transient retry without duplicate messages, hop cap, size cap, idle/busy mounted windows, ordering and restart persistence. The MCP-to-engine suite checks bot/default relay delivery, typed auth errors, silence, hop cap and missing-relay fallback.

Two test mistakes were corrected, then rerun: Path.resolve followed the venv interpreter symlink to a Python lacking yaml; the final runs preserve the venv interpreter path. An attribution assertion initially expected @default; source and the successful rerun establish the existing public handle is @factr. CHECKPOINT.md corrects the earlier overstatement about the initial recovered harness's Python environment.

## Half-done and unverified

- Task 2(a): the historical 13-minute creation-time intro hang remains unresolved. Neither the seeded-profile test nor the later real profiles.create/configure sequence reproduced it. The old renderer log records the observation but lacks the engine pre-model trace. The missing-config bug is not claimed as its root cause.
- Task 2(b): default config installation, new default-chat tool availability and direct default MCP delivery pass. Existing default sessions created BEFORE the roster and packaged @mention UI need QA.
- Task 2(c): no new pool fix. Existing code has a renderer activeTurn early veto and a Rust fallback based on two session-list snapshots. A snapshot does not freeze new admission. The stale-renderer hypothesis is from source inspection, not a reproduced cause of the historical five-bot timeout.
- Tasks 3-9: upstream parity audit/fire-and-forget, remaining Rust plumbing, 17-item engine harness, computer-bot branch integration, Docker setup, zero-install OS sandbox/browser integration, Linux/Windows tests and final design-document update remain undone.
- No packaged Electron, real Windows/Linux, real Ollama or Docker computer QA in this resumed run. No UI design was changed; the supplied design law has no new interface to review in this milestone.

Exact pending GUI scenarios are in NEEDS-GUI-QA.md.

## Blocker requiring the user

COMMON-RULES.md says: “At design forks, before irreversible or architectural decisions, and whenever two approaches look equally plausible, ASK Fable 5.1 a short, specific question.”

The required `claude -p --model claude-fable-5-1 ...` attempt failed with `claude: No such file or directory`. PATH and the usual executable locations have no Claude CLI. No credential or account files were accessed. A question is pending for the user to supply an executable path or waive the advice requirement.

The next material pool decision is whether to retain the existing Rust session-snapshot fallback or add a Rust retirement permit that freezes admission through the existing retirement protocol. Do not silently bypass the advice requirement to make that protocol change. The exact attempted question/result is recorded under Advice in CHECKPOINT.md. Parent advice was re-read after both milestones: none yet, no STOP.

## Resume, in order

1. Resolve the Fable executable/waiver blocker. Re-read parent advice and current Git state.
2. Finish task 2(a) diagnosis with an instrumented isolated packaged run. Complete existing-default-session QA and task 2(c) with regressions. Do not mark task 2 complete yet.
3. Continue tasks 3 through 9 in handoff order, committing validated milestones and updating CHECKPOINT.md. Complete parity/golden checks and real-app QA for every Rust port.

Build output is removed at this stopping point, so rebuild before the engine harnesses. Keep one cargo target at this worktree's target. A fresh isolated runner from the worktree root can use:

```python
import os, pathlib, subprocess, tempfile
work = pathlib.Path('/Users/rameelmalik/factri-bots-wt-s2')
root = pathlib.Path(tempfile.mkdtemp(prefix='bots-resume-', dir='/private/tmp'))
for name in ('home', 'config', 'engine', 'electron', 'cargo'):
    (root / name).mkdir()
env = os.environ.copy()
env.update(HOME=str(root/'home'), FACTR_HOME=str(root/'engine'),
    FACTR_CONFIG_HOME=str(root/'config'), FACTR_ELECTRON_USER_DATA_DIR=str(root/'electron'),
    CARGO_HOME=str(root/'cargo'), RUSTUP_HOME='/Users/rameelmalik/.rustup',
    CARGO_INCREMENTAL='0', CARGO_TARGET_DIR=str(work/'target'),
    FACTR_BIN=str(work/'target/debug/factr'), SCRATCH=str(root),
    FACTR_BACKEND_PYTHON=str(work/'backend/.venv/bin/python'),
    FACTR_BACKEND_PYTHONPATH=str(work/'backend'),
    PYTHON=str(work/'backend/.venv/bin/python'), BACKEND=str(work/'backend'),
    PYTHONPATH=str(work/'backend'))
# Check df -h / first; stop if below 20 GiB. No concurrent cargo builds.
subprocess.run(['cargo', 'build', '--profile', 'test', '--bin', 'factr', '--locked'],
    cwd=work/'engine', env=env, check=True)
subprocess.run(['cargo', 'test', '-p', 'factr-gateway', '--locked'],
    cwd=work/'engine', env=env, check=True)
subprocess.run(['.venv/bin/python', '-m', 'pytest', 'tests/tools/test_message_agent_mcp.py', '-q'],
    cwd=work/'backend', env=env, check=True)
for name in ('bot-chat-deliver.mjs', 'bot-message-agent.mjs'):
    subprocess.run(['node', str(work/'engine/crates/factr-gateway/e2e'/name)],
        cwd=work, env=env, check=True)
```

Preserve the venv path as written; do not resolve its executable symlink. A fresh CARGO_HOME downloads public build dependencies again. These are resume commands, not additional checks claimed as run here.

## Boundaries and cleanup

No pushes, remote changes, release-checkout or benchmark access, credential-file reads, Docker creation or Docker pruning. The worktree target executable has no remaining running processes (checked with process executable paths only). Generated target output is removed; dependencies are retained for resumption. Report copied to the handoff's bots-REPORT.md as required.
