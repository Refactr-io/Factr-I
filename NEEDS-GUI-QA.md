# Factr-I bots: remaining UI and external QA

Use only this worktree, a QA/dev build and fresh HOME, FACTR_HOME, FACTR_CONFIG_HOME and FACTR_ELECTRON_USER_DATA_DIR under /private/tmp. Do not launch the installed production app or the release checkout. Use the deterministic stub contract in engine/crates/factr-gateway/e2e/bot-schedules.mjs or bot-chat-deliver.mjs. Engine suite success does not certify the controls below.

The automated entry point is bot-checklist.mjs. Its checklist.json accounts for all 17 upstream flows and identifies the narrower engine evidence.

| Item | Exact UI/external procedure and expected result |
| --- | --- |
| 1 | Fresh install with no MCP cache: New Agent > Alpha, enter Name/Title/Description, create. The intro renders once within the first-response budget. Repeat Beta while Alpha is open in a second window. On a deliberately hung MCP, a visible teammate_mcp_timeout appears; Stop and a subsequent prompt work. Historical 13-minute packaged cause remains unproven. |
| 2 | Advanced create: clone Alpha, choose model/provider and a subset of skills/toolsets/MCP, custom SOUL. Edit Profile shows stored Description. Type before a slow description fetch completes; typing survives. Duplicate retains memory/SOUL but not schedules/channels. Delete requires confirmation, stops its backend, removes its row and survives relaunch. Default cannot be deleted. |
| 3 | Click each bot: one permanent Bot Chat per profile. /new and /reset become /compact with a notice; reopen/relaunch preserves that one chat. Archive retires it and the next click creates one replacement. |
| 4 | Open a default session BEFORE creating Beta. After creation, type @beta and select its autocomplete tag. The model composes message_agent; queued acknowledgment arrives immediately, target receives @factr attribution, later reply wakes the original session. Repeat in a new default session. |
| 5 | Alpha asks Beta a question. Both transcripts show attribution. Unknown and ambiguous targets are refused with the roster. Keep the sender window closed during the reply, reopen, and verify exactly one reply. |
| 6 | Make Beta answer only [SILENT], then NO_REPLY. Neither marker appears in the rendered reply and Alpha receives no spurious wake turn. |
| 7 | Stub Beta returns HTTP 401: Alpha shows [reason: provider_auth_or_access], no retry. Stub 503 once: exactly one retry on the same session and no duplicate incoming message. |
| 8 | Create a room with 2-6 bots; send, mention one, use @user, Stop, then @all. Verify serial rounds, caps, holds and needs-you badge. Member engine session identity/turn/restart is automated. GAP: Rust advertises driver=false and rounds still live in Desktop; close-Desktop continuation is not implemented by the recovered session fix. |
| 9 | Routines: blank Name displays validation and creates no job. Enter a named weekday-9am job, then fire, pause/resume, inspect output/history, remove. Reload/relaunch preserves paused state. Run now control requires the computer branch integration. |
| 10 | Own-profile bot-chat output is automated. Also create deliver=bot-chat:Beta from Alpha; output must reach Beta's canonical engine chat exactly once. This cross-profile courier integration remains open. |
| 11 | Within a scheduled turn request another cron job. Scheduling tools stay unavailable; no nested job is persisted. Test both the model's response and a forced unavailable-tool request. |
| 12 | Spawn 2-3 parallel delegated children, press Stop while streaming. Their provider streams close, statuses leave running, partial transcripts persist and unrelated sessions remain active. Engine stream cancellation is automated; inspect rendered status/partial text. |
| 13 | Physical second machine: add a disposable connection, Create on that machine, mention its qualified tag, verify queued handoff/wake in both directions. Disconnect/reconnect without duplicates. |
| 14 | Register a disposable peer, peer dm and message_agent spark/researcher. Native HTTP contract is checked against a Python golden and stub peer; real account/secret-store integration needs user-owned setup. |
| 15 | Two disposable Telegram bot accounts: each profile answers only on its token. Attempt the same token on the second profile and verify the token-lock refusal. No real tokens were accessed by this run. |
| 16 | Computer screen: watch headed browser, Take over at a local mock login, type disposable data, Hand back, verify same browser session and enforced lease. Docker/sandbox/platform integration is still in progress. |
| 17 | Turn Bot Mode off/on in Capabilities without restart. Roster/Routines unregister/re-register live and data survives. Native protocol-tool gate is automated; live renderer lifecycle needs the click test. |

## Three-engine pool

Create Alpha/Beta/Gamma/Delta/Epsilon with computers off and cap=3. Open all five three rounds. At most three pooled engines remain resident, default primary is separate. Keep one bot streaming and repeat; the busy bot finishes. Test stale renderer busy state, hidden routine/delivery, retiring admission, immediate reopen and a crash mid-drain. Queued opens wait for physical exit; typed retiring is rerouted with the same envelope id. The production arbiter/real-engine churn is automated. Late autonomous work at the 60s drain cap needs transcript inspection.

## Platform QA

Windows and Linux setup/sandbox manual steps will be appended with their implementation. No real Windows-machine verdict is claimed.
