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
