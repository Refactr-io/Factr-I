# Remaining Factr-I QA

## Integration: sidebar and scrollbars

With an isolated development build, test the system scrollbar preferences (automatic and always visible), mouse and trackpad scrolling, hover, idle fade and reduced motion. The sidebar bar must sit at the panel edge without hiding text, shifting rows or blocking pointer/keyboard access. Check narrow panes, long labels and horizontal scrolling. Confirm Run history has a distinct history icon in the rail and command palette, selected and unselected. These pointer and pixel checks were not run during integration because GUI driving was prohibited.

## Integration: OpenCode connection

Use disposable test-account state in the QA home, never the user's existing sign-in. Check detection in onboarding and the model picker, both with and without a sign-in available. No sign-in must lead to key entry; explicit connection must refresh models and preserve the initiating profile/session. Check missing/expired sign-in, offline catalog refresh, repeated clicks, keyboard focus, readable errors and the private-deployment refusal. Confirm no secret appears in the renderer or logs. Repeat on Windows and Linux. Integration tested existing onboarding/model-picker behavior and native detection/parser tests, without exercising real credential adoption or live Zen inference.

Use only an isolated QA copy, fresh HOME/FACTR_HOME/FACTR_CONFIG_HOME/userData/host-lock directories, and a loopback stub. Never launch the installed user app or read credential files. The retained copy is `/private/tmp/factri-rust-run/Factr-RAM-QA.app`; its bundle id is `net.refactr.factr.qa.rust-ram`. The retained harness parks it as an unfocused sliver. One app at a time.

These are explicit limits of this RAM task. The automated checks below already cover ordinary chat, image attachment, @ menu, Settings, Scheduled jobs/New job, lazy math, disclosure images, user-driven browser navigation/tab switching, and pet RPCs with enabled/named/managed configurations.

## Known inherited Electron console exception

1. Run `rtk proxy python3 /private/tmp/factri-rust-run/isolated.py node /private/tmp/factri-rust-run/qa/smoke.mjs`.
2. It attaches the repo test PNG through Add context > Images, submits a stub chat, checks Reload/DevTools with no guest, opens a loopback page, declines browser-profile consent using Not now, reloads, opens a blank second tab and returns to the loaded tab.
3. The loaded document and tab restoration pass, but renderer-console.jsonl records `Error processing argument at index 0, conversion failure from null`, from Electron WebViewImpl.createGuest, during return to the loaded tab.
4. Reproduced on the renderer from before this resumed run and on the final renderer. Mutation tracing sees one continuously connected guest. Do not call this fixed. A separate title-refresh remount/state-loss issue was fixed and has an old-fails/new-passes regression.
5. Follow up with a normal on-screen window, then rapid tab switches, back/forward, reload, failed initial navigation/retry and DevTools after a guest exists. Capture native guest ids, console, and screenshot before changing lifecycle code. Expected: the document stays live, no duplicate guest, and no native exception.

## Agent-driven browser navigation

1. Start a loopback HTTP page with a unique heading. Fresh app must contain zero webview guests at resting about:blank.
2. Use a deterministic stub tool call to ask the engine to navigate its Browser to that URL, without typing the address yourself. Confirm the heading in the guest and one guest process.
3. Open a second blank tab, return to the first, ask the agent to read the page, reload and retry a failed navigation. Confirm no stale target or duplicate guest.
4. User-typed navigation, blank controls, reload and tab restoration passed here. Agent-tool dispatch into a blank tab was not exercised.

## Microphone, voice prefetch and provider network paths

1. Use an isolated app with the microphone permission flow available. The automated harness suppresses permission dialogs and external network access, so it cannot establish recording quality or physical haptics.
2. Open dictation, allow the mic, record a short known sentence, stop and submit. Repeat voice conversation start/stop. Check prefetch finishes, transcript is readable, cancellation releases capture, and the feature backend idles out once inactive.
3. For ElevenLabs, have the user configure an authorized test account through the UI. Do not inspect or copy credential files. Exercise the voices picker and first TTS request; verify it forwards to Python and handles auth/network failure visibly. Synthetic-key fallback and no-key goldens passed here; a real provider request was not made.
4. Compare first-haptic latency with haptics enabled before/after. Warm-up timing is unchanged at idle timeout 2000 ms. In a fresh persisted-muted launch, expect no AudioService after 10 s (58 MB process avoided in the measured run); enabling haptics may start it by design.

## Visible long-chat and accessibility pass

1. Replay 200 settled stub exchanges in a fully visible window, repeat with long streaming Markdown, fences, equations and loopback images. Use the retained `qa/profile-chat.mjs` pattern.
2. Check Show earlier, session switching/restored scroll position, selection, find, keyboard focus, screen-reader disclosure state and narrow split panes. Existing paging budgets remain 600 DOM / 1200 store units.
3. Open/close an inter-agent reply containing an image. Closed body/media must be absent from the DOM; open content must be complete and focus usable. Actual pointer checks passed here; a screen reader was not used.
4. Expected measurement boundary: the offscreen 200-exchange renderer was 1026 MB before / 1005 MB after, forced-GC 688 / 645 MB, same DOM. This patch does not solve the roughly 1 GB plain-chat plateau. Do not require a large visible-window RSS reduction. The separate cache stress retained 43.94 / 1.81 MB after callers released long snapshots.

## Pet visuals, shutdown and platforms

- Enabled root, named scribe/uppercase SCRIBE and managed overlays returned real metadata and sprite bytes from a synthetic atlas in this run. Manually check a real installed pet's animation, resize and overlay positioning on screen. No real pet account/assets were downloaded.
- Each enabled-pet app run left one Python helper after graceful app quit; the harness killed that owned helper and verified nothing remained. Reproduce standalone shutdown before claiming all feature children quit gracefully. Feature process ownership code was not changed.
- Run voice/cron/messaging activity followed by idle stop and cold restart together. The existing unit and native-read tests do not establish every integration combination.
- Windows/Linux, multi-window, real provider streams and subjective haptic latency were not tested on this macOS runner.
- The pause notes identify 429/401/404/500 error-card steps failing on the earlier base as well. Re-run each with a loopback provider returning that status, verify a readable retryable card, then restore a successful stub response. This inherited issue was not investigated in this RAM task.


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
| 9 | Routines: blank Name displays validation and creates no job. Enter a named weekday-9am job, then fire, pause/resume, inspect output/history, remove. Reload/relaunch preserves paused state. Click Run now and confirm one run appears without changing the schedule; its handler and visibility are automated. |
| 10 | Own-profile and cross-profile scheduled bot-chat output are automated. Create deliver=bot-chat:Beta from Alpha in the UI; verify the output renders in Beta's canonical chat exactly once, including after reopening it. |
| 11 | Within a scheduled turn request another cron job. Scheduling tools stay unavailable; no nested job is persisted. Test both the model's response and a forced unavailable-tool request. |
| 12 | Spawn 2-3 parallel delegated children, press Stop while streaming. Their provider streams close, statuses leave running, partial transcripts persist and unrelated sessions remain active. Engine stream cancellation is automated; inspect rendered status/partial text. |
| 13 | Physical second machine: add a disposable connection, Create on that machine, mention its qualified tag, verify queued handoff/wake in both directions. Disconnect/reconnect without duplicates. |
| 14 | Register a disposable peer, peer dm and message_agent spark/researcher. Native HTTP contract is checked against a Python golden and stub peer; real account/secret-store integration needs user-owned setup. |
| 15 | Two disposable Telegram bot accounts: each profile answers only on its token. Attempt the same token on the second profile and verify the token-lock refusal. No real tokens were accessed by this run. |
| 16 | Computer screen: watch headed browser, Take over at a local mock login, type disposable data, Hand back, verify same browser session and enforced lease. Docker engine/desktop protocol and local macOS browser boundaries are automated. Inspect the rendered noVNC screen, lease controls and reconnect states; local fallback is not a streamed desktop. |
| 17 | Turn Bot Mode off/on in Capabilities without restart. Roster/Routines unregister/re-register live and data survives. Native protocol-tool gate is automated; live renderer lifecycle needs the click test. |

## Three-engine pool

Create Alpha/Beta/Gamma/Delta/Epsilon with computers off and cap=3. Open all five three rounds. At most three pooled engines remain resident, default primary is separate. Keep one bot streaming and repeat; the busy bot finishes. Test stale renderer busy state, hidden routine/delivery, retiring admission, immediate reopen and a crash mid-drain. Queued opens wait for physical exit; typed retiring is rerouted with the same envelope id. The production arbiter/real-engine churn is automated. Late autonomous work at the 60s drain cap needs transcript inspection.

## Platform QA

No real Windows machine was available. The restricted-token helper has only manifest/lifecycle fakes; its WinAPI calls, ACL enforcement, browser and packaging are NOT certified. Linux runtime evidence comes from a Docker Desktop Linux container, not a native Linux desktop. No real Docker/WSL installation or elevation was performed on the host.

### Fresh Windows 11 VM: Docker setup

1. Use a disposable Windows account and a QA build from this branch with a `.qa` app identity. Do not use production Factr-I. Create a fresh directory such as `C:\bots-qa`; point HOME, FACTR_HOME, FACTR_CONFIG_HOME and FACTR_ELECTRON_USER_DATA_DIR to separate subdirectories before launching the QA exe. Disable external-login adoption in the generated test config. Use a local stub/model with synthetic credentials only.
2. With Docker absent and WSL absent, create Alpha and open Computer > Set up. Confirm the native dialog names WSL and administrator/restart requirements. Cancel: no process/install starts and the status remains retryable. Accept: Windows owns the UAC prompt; no Linux distribution is silently installed. After any requested reboot, reopen the same isolated QA state and retry.
3. With WSL older than 2.1.5, repeat. With WSL current and winget present, the exact interactive Docker.DockerDesktop installer should open. Cancel it: a visible error permits Retry. Check no terms/licence flags were accepted silently. With winget absent, the official Windows installer page opens and the app explains to return after installation.
4. Keep the app open across installation. Verify it discovers the standard Program Files Docker CLI without requiring an app restart or a refreshed PATH. Installed-but-stopped becomes Starting, then Ready after Docker's own terms/setup prompts. Deny/cancel UAC and delay startup past two minutes: an actionable error replaces Starting. Retry succeeds after correcting the condition.
5. With the source-built test image loaded, open Alpha and Beta. Confirm two `bots-` containers, loopback-only engine ports, no Docker socket/host-home mounts and different tokens. Create one file in each; neither sees the other's canary. Stop Alpha explicitly, switch chats/relaunch and wait past a schedule tick: Alpha stays stopped until Start. Remove its container externally while off: next status must discard the stale route.
6. Put a one-minute job on Alpha; verify output, pause/resume, app relaunch and Docker stop/start. Keep Desktop closed while the running scheduled container fires, then reopen to see its saved output. No cross-bot courier is expected while Desktop is closed.

### Windows local fallback, both install locations

Run this matrix once from Program Files and once from an ordinary per-user install under LocalAppData. Do not run elevated. Start with Docker unavailable and with no `.computer` marker on the test bots; a Docker-owned profile must instead refuse host fallback.

1. Set `FACTR_BOT_COMPUTERS=0`, launch the isolated packaged QA app and create Alpha/Beta with independent model configurations. Open both chats. Verify one engine and one persistent Python launcher per resident bot, no helper per message, and the normal pool limit. Both must answer using their own SOUL/profile.
2. Through Alpha's native file and shell tools, write/read `workspace/alpha.txt`. Ask Beta to read that absolute path and a host canary outside both profiles. Both accesses must fail. Ask Alpha to read a Beta canary and write the host canary: deny with original contents unchanged. Test a junction inside Alpha pointing outside; no read/write escape. Use synthetic files only.
3. Close/reopen Alpha twice. Its engine store, own files, cron state and browser cookies persist, and the protected ACL does not prevent Desktop from maintaining the profile. Inspect ACLs: the two bots have different restricted SIDs; no public write grant. Try changing Beta's ACL from Alpha, including via owner rights: it must fail.
4. Request a shell child that sleeps and starts its own child. Close the bot engine or quit the QA app, including force-ending the QA parent once. All descendants must exit with the Job Object. Assignment failure or a denied token/desktop call must report local-sandbox-unavailable before the engine executes; never an unrestricted retry. Repeat inside any enterprise job policy that is relevant.
5. Alpha must not signal/terminate Beta or the host process, read the host clipboard, switch to the interactive desktop, or drive existing host app windows. Its private desktop does not provide the Docker Screen UI. Inspect that the default chat and unrelated host apps still work.
6. Use a local mock website with a persistent cookie. Alpha signs in with disposable data; Beta starts signed out. Restart Alpha and verify its cookie remains. Ensure Chromium/agent-browser cold download uses Alpha's private AppData/cache and Python can load from both installation locations. Deny an install/download and confirm a useful failure rather than an endless wait. The actual native browser-tool path requires this check, beyond the macOS CDP fixture.
7. Alpha asks Beta to read only Beta's own canary and report it. The queued acknowledgment returns first; attribution is visible; the later reply wakes Alpha exactly once. Repeat while Beta is stopped to confirm on-demand routing, and after app relaunch to check durable courier state.
8. Add a routine to Alpha, pause it, restart the app, resume and Run now. Confirm one saved output and delivery to Alpha's canonical chat. With enabled routines on three resident bots, a fourth open must wait/fail with the existing capacity message without killing a scheduled bot. Raise the existing cap and retry. Local schedules require Desktop open.
9. Select a provider for a fresh local bot. Use a disposable provider account only if authorized: its sign-in must write the bot's private store, not adopt/copy host sign-ins. This run did not access any real provider credentials. Test old shared-store transcripts separately; automatic import is not implemented.

### Native Linux desktop

1. On Debian/Ubuntu as a non-root user whose uid/gid are not 1000, launch the same isolated QA build. Without Docker, accept/cancel the pkexec apt plan and service start. Confirm socket permission denial is actionable. Do not silently add a user to the docker group. On other distributions or Podman-only setups, verify the official-instructions state; a Docker-compatible Podman CLI/service must be explicitly configured before it can be used.
2. With Docker available and the test image loaded, create a computer. Check its uid/gid match the host user, its HOME/workspace are inside the mounted profile, files remain host-writable after restart, and desktop/browser startup still works. On native Linux a host model bound only to 127.0.0.1 is not reachable through host-gateway; use an explicitly permitted reachable listener and verify the UI's provider error when unavailable.
3. With Docker off and bubblewrap installed, test native file/shell tools, Python features, two browser profiles, queued handoff and schedules as in the Windows matrix. Check private PID and mount namespaces, no host `/tmp` files, symlink escape denial and no access to host session-bus/SSH sockets. Native display/Wayland integration is not certified by the nested container test.
4. Without bubblewrap, or with user namespaces denied (including Ubuntu/AppArmor policy), opening a local bot must fail closed with local-sandbox-unavailable. No bare host engine may remain. This is a known install-only-Factr-I distribution gap; bundle/permission strategy needs release-owner resolution before claiming universal Linux support.

### macOS packaged path still to click

The real Seatbelt engine/browser fixtures ran on this Mac. The actual packaged Electron create/open route, cold browser install, native browser tool, provider sign-in, Docker consent/error controls, noVNC/lease, minimum-memory block and app quit/relaunch still need the full visible-app pass. Use the 17 rows above plus explicit Stop with a scheduled Docker computer. Verify text contrast, focus/keyboard access, hit targets, no clipped controls or missing content, and no repeated/blank first-turn messages. Review every changed control against the supplied anti-slop law; do not call the UI review passed from protocol tests.

## Final resumed QA notes

The recovered routing/rename fixes are committed in dac9c7e. The full Bots UI suite passed 775 tests (96 files), and the final local group lease plus sync/turn suites passed 92 tests. A current renderer build and both TypeScript checks passed. These results do not replace a visible-app pass.

The resumed runner created a fresh, ad-hoc-signed disposable QA copy with HOME, engine/config/userData pinned in its entry module before any app imports. Computer Use launched it but selecting its window timed out with error -10005, both by app path and bundle id. No final pointer-based create/open/reopen verdict is claimed. The process was stopped.

Priority macOS click case: create Alpha with Docker off and the deterministic stub; one intro must render in Alpha's private engine. Open default, reopen Alpha, close/relaunch and reopen Alpha again. Exactly one canonical chat and intro must remain. Attach image/file/PDF and answer clarify/approval on Alpha; these RPCs must reach Alpha, while roster warming must not start every bot. Read the Sandbox status and provider-sharing copy at normal zoom, narrow width and increased text size; check contrast, focus, clipping, centering and keyboard access. Rename an active room while a clarify poll is held, answer it, and verify the reply follows the renamed room without restoring the old name. Disband during an in-flight reply: no room or attention card may reappear.

The real Linux sandbox harness was repeated in this resumed run: non-root mount/PID/private-temp/symlink boundaries, real engine native shell/file action and bundled Python cron endpoint all passed; the single bots- container was removed on exit. This remains nested Linux evidence, not a native Linux desktop/Wayland or real Windows result.
