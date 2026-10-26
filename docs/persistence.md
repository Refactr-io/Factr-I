# Persistence after a failed check

This is what the engine does when you give it a one-shot task (`POST /api/agent/run`) that names its own check, for example "test your file with `./eval`" or "run `pytest tests/` until it passes". It does nothing for interactive chat, for a task that names no check, or for any other kind of run.

## What it does

1. The engine finds the check command in the task text. Only a command the task itself names is used; it never guesses a check from other output.
2. When the model says it is done, the engine runs the check itself. If the model already ran exactly that command on the same files during the turn, the engine reuses that result instead of running it again.
3. If the check passes, the run ends. The engine decides this, not the model's own claim.
4. If the check fails, the engine sends the end of the failed output back as the next prompt and the model tries again. The retry prompt is short: the best score so far, the last two attempts and the last 1200 characters of the failure.
5. While the model works, if it runs the check itself and it fails with the same output as its previous run, or the files are the same as at that run, a one-line note is added to the result asking for a different approach.

A check also counts as failed when it exits 0 but prints a report that says it failed (for example `"exec_acc": false` or `"failed": 2`). Numeric scores never decide pass or fail.

## What ends a run

Only these:

- the check passes;
- no progress: the same failure, or no file change, for 3 attempts in a row (`FACTR_PERSIST_STALL`, 2 to 6);
- the retry limit: `persistence.max_retries` (default 3);
- the token ceiling: `persistence.max_tokens` (default 0 = automatic, 5 times the first turn's fresh tokens, between 400k and 2M);
- the run's own time limit: the retries stop when one more check would not fit in what is left. `persistence.max_seconds` can make this shorter (0, the default, leaves the run's own limit);
- an interrupt.

The reply to the caller says whether the check passed and, if not, why it stopped.

## Settings

| Setting | Environment | Default | Meaning |
|---|---|---|---|
| `persistence.enabled` | `FACTR_PERSIST=0\|1` | on | Turns the whole feature off. |
| `persistence.max_retries` | `FACTR_PERSIST_MAX_RETRIES` | 3 | Retry limit after the first failed check (at most 10). |
| `persistence.max_tokens` | `FACTR_PERSIST_MAX_TOKENS` | 0 (automatic) | Ceiling on fresh tokens (uncached input plus output). |
| `persistence.max_seconds` | `FACTR_PERSIST_MAX_SECONDS` | 0 (run's limit) | Stop retrying after this many seconds. |
| | `FACTR_PERSIST_STALL` | 3 | Attempts without progress before the run stops. |
| | `FACTR_PERSIST_REUSE` | on | Reuse the model's own run of the check on unchanged files. `0` runs it again. |
| | `FACTR_PERSIST_TERSE` | on | Short retry prompt. `0` gives the long one with the whole attempt log. |
| | `FACTR_PERSIST_STALL_NOTE` | on | The same-failure note. `0` turns it off. |

The engine never changes `text.verbosity` or the reasoning effort for these runs; set `agent.text_verbosity` or the model's effort yourself if you want that.

## Opt-in options

These are off unless set, and exist to trade behaviour for cost or time: `FACTR_PERSIST_AUTOCHECK=1` (the engine runs the check right after every file change and appends the verdict to that change), `FACTR_PERSIST_ORIENT=1` (the first prompt lists the working directory), `FACTR_PERSIST_TOOLS=minimal` (the run gets only the shell, read, write, edit and list tools: this removes capabilities such as search, patching and the todo list, so it is only right for tasks that need none of them), `FACTR_PERSIST_CONCISE`, `FACTR_PERSIST_SMALLEDITS`, `FACTR_PERSIST_BRIEF` (wording in the first prompt for a smaller first draft, smaller edits and shorter messages) and `FACTR_OPENAI_PARALLEL_TOOLS=1` (several tool calls per model step).

## Cost

A failing check means more model calls, so a run that never passes costs more than one that stops at the first answer. The limits above are the controls. The engine adds no cost of its own: the check runs on your machine.
