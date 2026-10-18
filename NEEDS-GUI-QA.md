# Factr-I bots: pending GUI QA

Status: not run in this resumed session. These are pending tests, not claims of success or screen-permission failures. No UI code changed. Use this worktree and fresh HOME, FACTR_HOME, FACTR_CONFIG_HOME and Electron userData beneath /private/tmp. Use only stub model settings and generated test credentials. Do not use the release checkout or the installed production app.

## Creation-time intro hang, task 2(a)

1. Build a QA app from `feat/bots-step2` in this worktree after the advice blocker is resolved. Set FACTR_BOT_COMPUTERS=0 and FACTR_GATEWAY_TRACE=1 in the isolated QA launch. Keep the engine stderr and stub OpenAI-compatible endpoint request timeline. The existing harness model in `engine/crates/factr-gateway/e2e/bot-chat-deliver.mjs` is the reference contract; do not substitute real credentials.
2. Start with no profiles and no MCP schema cache. Create Alpha with Name, Title and Description. Keep its newly created Bot Chat mounted. Create Beta immediately, then return to Alpha.
3. Verify that Alpha's intro appears exactly once and completes. Correlate profile creation, session.create, session.title, bot.chat.claim, session.resume, prompt.submit, message acceptance and the first model request. The historical renderer log alone does not establish where the turn blocked.
4. Repeat while opening the fresh chat on a second window, and while the roster refreshes. The bot's first model request must include message_agent without restarting the app. There must be one canonical Bot Chat.
5. If the turn hangs, retain the isolated engine trace and model request timestamps before pressing Stop. Then Stop and submit a normal prompt. Report the last successful stage and actual elapsed time. Do not call the historical hang fixed merely because a later turn works.

## Default chat handoff, task 2(b)

1. On a fresh isolated install, open a default chat BEFORE creating a bot. Leave that session open.
2. Create Beta and set its description to say it answers the handoff question. Return to the already-open default chat without restarting.
3. Type `@beta Tell me the word you were given.` Verify autocomplete resolves Beta and the active agent calls message_agent. This existing-session case is not covered by the new-chat engine regression.
4. Verify Beta's own Bot Chat receives the attributed message. The existing handle for the default profile is @factr. Verify the sender receives the reply.
5. Create a NEW default chat after Beta exists. Repeat the handoff without an app restart. Engine-level schema availability and direct MCP delivery for this case passed; the actual renderer flow remains unrun.
6. Repeat on an install without any Bot Mode profiles. Its default config/tool surface must not gain the teammate MCP from this installer change.
7. Fire-and-forget is still an open task 3 deviation. The current tool blocks for a reply. Do not accept that as upstream parity.

## Pool, task 2(c), after implementation

1. Set the local engine cap to 3. Create Alpha, Beta, Gamma, Delta and Epsilon with computers off.
2. Open all five in order; wait for each intro to finish. Repeat three complete rounds quickly enough that all previous touches remain inside the keepalive window.
3. Verify every open succeeds, at most three pool engines remain resident, and the least recently used idle engine is retired. Default's primary engine is separate from the pool.
4. Keep Alpha in a deliberately slow model turn. Open the other four repeatedly. Alpha must finish uninterrupted; another idle resident must make room.
5. Switch away while a turn finishes, close/reopen the window, and repeat. Verify a stale renderer activeTurn flag cannot strand a foreground waiter.
6. Start a hidden delivery or routine in a bot. It must not be stopped to free a slot. Test a new turn arriving during retirement as well; a pair of idle snapshots alone does not freeze admission.

## Remaining handoff QA

Tasks 3-9 are not implemented by this resumed milestone. The 17-item checklist harness, full packaged checklist, computer-bot message/schedule scenarios, real Docker image test (53 checks), cross-platform setup/sandbox tests and Windows manual procedure remain to be completed in task order. No real Windows/Linux or Docker run occurred in this resumed session.
