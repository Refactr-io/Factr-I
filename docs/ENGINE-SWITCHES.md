# Engine switches

Since v0.0.16 the OpenAI Responses requests take the shape the reference clients (the Codex CLI and the Python agent) send, and the agent loop has a few new defaults. Every change has its own environment switch, and one master switch, `FACTR_VARIANT_BASELINE=1`, restores the v0.0.15 behaviour (byte-identical requests, proven by golden fixtures). Other providers (the Copilot runtime included) are not affected: they keep calling the public builders, which stay on the baseline shape.

The shape is read once per process (`RequestShape::current()` in `factr-provider-core/src/request_shape.rs`), so the prewarm payload, the first request, retries, the HTTPS fallback, model switches and every continuation use the same values.

## Switches

| Env | Field | v0.0.15 (baseline) | Default | What it changes |
|---|---|---|---|---|
| `FACTR_VARIANT_BASELINE=1` | all | | | Master switch: exactly the v0.0.15 request and loop, every other switch ignored |
| `FACTR_OPENAI_STRICT_TOOLS` | `strict_tools` | `1` | `0` | Off: every tool is sent with the compatible (not strict-rewritten) schema and `strict: false` |
| `FACTR_OPENAI_CHATGPT_CACHE_KEY` | `chatgpt_cache_key` | `0` | `1` | On: the session `prompt_cache_key` is also sent in ChatGPT mode. `prompt_cache_retention` stays API-key-only. The idle prewarm payload leaves the key out so warm sockets are still adopted |
| `FACTR_OPENAI_PARALLEL_TOOLS` | `parallel_tool_calls` | `0` | `0` | On: `parallel_tool_calls: true` on the full request and its continuation. Calls from one response still run one at a time, in order |
| `FACTR_OPENAI_REPLAY_REASONING_ID` | `replay_reasoning_id` | `1` | `1` | Off: replayed reasoning items carry no `id` (full request and explicit native-compaction request) |
| `FACTR_OPENAI_WS_FAILED_ENDS` | `ws_failed_response_ends` | `0` | `1` | On: a fresh WebSocket attempt that gets `response.failed` (or an `error` event) not on the retry list forwards it once and ends, without saving the socket for reuse, as a socket continuation already did. Off, the backend sends no `response.completed` after it and keeps the socket open, so the attempt waited up to the 300s idle limit, then fell back to HTTPS and sent the request again. The default simple completion (REPL sub-calls, other background calls) then returns that error instead of empty text. This was the REPL "stall": about one sub-call in twenty, each ending in a failed response within 1-7s |
| `FACTR_OPENAI_IMAGE_TOOL_NEEDS_TOOLS` | `image_tool_needs_tools` | `0` | `1` | On: the hosted `image_generation` tool (ChatGPT mode) is attached only to a request that carries tools of its own. A REPL sub-call or any other simple completion sends none. These calls run where the session's tool policy is not visible, so they got the image tool even in runs that do not allow it; a classification sub-call that called it failed with `image_generation_user_error` ("Complexity 'medium' is not supported for this image model"), which was the failed response behind the stall |
| `FACTR_OPENAI_CACHE_KEY_MODE` | `content_cache_key` | (no key) | `session` | Opt-in, off by default. `content`: in ChatGPT mode the key is `pck_` + 24 hex chars of sha256(instructions, NUL, tools sorted by name as sorted-key JSON), with no session id, so sessions with the same prompt and tool set share it; the idle prewarm then sends and compares the key. `session`: the per-session key above. API-key mode always keeps its session key |

Each switch takes `0` or `1` (the cache-key mode takes `session` or `content`); any other value keeps the default. Native compaction already has its own switch: `FACTR_OPENAI_NATIVE_COMPACTION_MODE`.

The reference client's own key is content-addressed within one logical session (it hashes a session scope with the instructions and tools), so `content` mode is a separate design, not a copy of it.

## Loop switches

The agent loop around the requests has its own table, `LoopShape` (same file, same master switch, read once per process). These apply to every provider.

| Env | Field | v0.0.15 (baseline) | Default | What it changes |
|---|---|---|---|---|
| `FACTR_LOOP_STICKY_NUDGES` | `sticky_nudges` | `0` | `1` | Messages injected into one call's request (stop nudges, the turn's system reminder, repeat-guard, deadline and batch reminders, unsaved memory) stay at the position they were first sent at for the rest of the turn, so the next input extends the last one and a socket chain can continue. Never written to the session; dropped at the end of the turn, and on compaction or a rewritten history |
| `FACTR_LOOP_RESPECT_NO_VERIFY` | `respect_no_verify` | `0` | `1` | The newest user message that says so decides. "do not run it": no auto-verify gate run and no "verify" in the 70% deadline reminder (reading back stays allowed). "do not verify / check / review": also no verify nudge. Optional wording ("no need to test") bans nothing; quoted text and code are ignored |
| `FACTR_LOOP_NUDGE_NO_REREAD` | `nudge_no_reread` | `0` | `1` | When every unexecuted write came from write/edit/apply_patch/replace, the verify nudge asks to check against the requirements without a read-back. A shell or repl write keeps the old text |
| `FACTR_LOOP_COMPACT_WRITE_RESULT` | `compact_write_result` | `0` | `1` | The model's copy of a successful write/edit/apply_patch/replace result leaves out the appended full diff; a successful write adds "Saved as sent; no need to read it back just to confirm the save." Replace dry runs, failed results, batch results and any other text stay as they are. The session, `ToolDone` and the UI keep the full text |
| `FACTR_LOOP_TOOL_STEERING` | `tool_steering` | `0` | `1` | Tool descriptions steer file reads, searches and edits to read/agentgrep/ls and edit/write/apply_patch instead of the shell, and say a write needs no read-back just to confirm the save. Built from the final tool list; SDK-supplied definitions are left as given. About 94 tokens on the full tool set |
| `FACTR_REPL_SUBCALL_TIMEOUT_S` | `repl_subcall_timeout_s` | `0` (none) | `90` | Seconds one REPL sub-model call (`llm_query`, each `llm_query_batch` item, each `classify` chunk) may take, scaled by the sub-call's effort like the stream idle timeout (x2 high, x3 xhigh, x4 max). Past it the call is dropped (its connection closes) and fails as a timeout: `classify` asks that chunk once more and the other chunks keep their labels; `llm_query_batch` returns that item's `Error: ...` string. A whole number of seconds; `0` turns it off. Without it, one sub-call that is accepted and then goes silent holds the cell until the cell's model-call wait (120s plus 30s per wave of 8 prompts) runs out, and the whole wave is lost |
| `FACTR_REPL_KEEP_PARTIAL_WAVE` | `repl_keep_partial_wave` | `0` | `1` | A REPL batch (`llm_query_batch`, a `classify` wave) that runs out of the cell's model-call wait stops 0.25s before it, drops the calls still running (closing their connections) and returns the replies that came back; each missing item fails with "host call time budget exhausted before this item's reply came". `llm_query_batch` gives that item's `Error: ...` string. `classify` does not ask the missing chunks again in that cell and raises "classify: K of N items still without a valid label ..." with K the exact count and their positions; the labels already given are in `classify_partial` (a list aligned with the items, `None` where missing). Off: the whole host call fails and every item of the wave is reported missing |

## Unverified assumptions

- That keeping injected messages in place turns the call after a nudge into a cached continuation is shown against a loopback socket (`injected_message_continuation.rs`), not against the backend.
- That a content cache key gives cross-session cache hits on the ChatGPT backend is not shown locally.

- That the backend accepts `strict: false` with the compatible schema is inferred from the reference clients; nothing local shows it.
- That the ChatGPT backend accepts `prompt_cache_key` is inferred from the reference clients; nothing local shows it.

## Known issues

- The hosted image tool follows the run's tool policy through the logging context, which is keyed by the tokio task that started the turn. The main loop builds its requests in that task, so it sees the policy. A REPL sub-call or another background completion runs in a different task, sees no session and is treated as allowed. With `FACTR_OPENAI_IMAGE_TOOL_NEEDS_TOOLS` on (the default) those calls carry no tools and so never get the image tool; with it off, or with `FACTR_VARIANT_BASELINE=1`, they get it as in v0.0.15, even in runs whose toolsets do not allow it.
- Image calls in ChatGPT mode have been seen to fail with `image_generation_user_error` ("Complexity 'medium' is not supported for this image model"). The engine sends the hosted tool as a bare `{"type": "image_generation"}` with no options (unchanged since v0.0.0), so the rejected value is chosen by the backend or by the model's call, not by Factr-I. Which values the backend accepts now has not been checked (it needs a model call); until it is, image generation in ChatGPT mode may fail.

## Stream timing line

Off unless `FACTR_STREAM_TIMING=1` (read once per process; any other value is off). With it on, the OpenAI runtime prints one JSON line to stderr for every model-call attempt that reaches a terminal response (`response.completed`, `response.incomplete` or `response.failed`). A retried attempt or an HTTPS fallback that reaches its own terminal response prints its own line. With it off nothing is observed and nothing is printed.

Each line carries `"factr_stream_timing":1` (grep for it); keys are in alphabetical order. Fields:

- `transport`: `websocket` (fresh socket), `ws-continuation` (reused socket with `previous_response_id`) or `https`.
- `first_event_ms`, `first_output_ms`, `last_event_ms`: milliseconds from `t0` to the first event, to the first output delta (text, refusal, tool arguments or reasoning summary), and to the last event.
- `channel_wait_ms`: total time the reader spent waiting to hand events to the consumer.
- `input_tokens`, `cached_tokens`, `output_tokens`, `reasoning_tokens`: from the terminal response's `usage` (`output_tokens` includes the hidden reasoning).
- `served_service_tier`, `response_model`: from the terminal response, `null` when absent.
- `status`, `error_code`, `incomplete_reason`: from the terminal response (`completed`, `failed`, `incomplete`), so a failed response is told apart from a finished one; codes only, never the message text.
- `strict_tools`, `reasoning_id_replayed`: the request-shape switches. `parallel_tool_calls`, `cache_key_sent`, `native_compaction_sent`: what the sent request carried.

An attempt that ends with no terminal response (an error, a timeout, a dropped reader) prints one line with `"factr_stream_unfinished":1` instead, so counts that grep for `factr_stream_timing` are unchanged. Fields: `transport`, `phase`, `elapsed_ms` (from the attempt's start), `first_event_ms`, `last_event_ms`, and the request-shape fields above. `phase` is `cancelled` (the reader went away first, for example a sub-call deadline or a stopped turn), `not_sent` (fresh WebSocket only: it ended before the request was sent, during the token fetch, the connect or the send), `no_first_event` (sent, nothing came back; for HTTPS this includes waiting for the response headers and an HTTP error status) or `cut_mid_stream` (events came, then it ended). A `ws-continuation` attempt never reports `cancelled` or `not_sent`. Off with `FACTR_STREAM_TIMING` unset, like the timing line; it does not depend on `FACTR_VARIANT_BASELINE`, so both arms can be diagnosed.

The times are application-observed, not network-exact. `t0` is taken just before the request is handed to the transport (before the WebSocket send, before the HTTPS request goes out). Each event is stamped when the runtime reads its frame or chunk, before conversion and before it is passed on. The reader is not decoupled from delivery, so a slow consumer can still delay later reads; `channel_wait_ms` shows how much of that happened.

## Measuring the streaming rate

One binary, the same prompt producing about 2k output tokens at effort high, one request at a time, three calls per arm:

1. `FACTR_VARIANT_BASELINE=1`.
2. The defaults (no switch set).
3. Single toggles: leave the master switch unset and set all four switches to their baseline values (`FACTR_OPENAI_STRICT_TOOLS=1 FACTR_OPENAI_CHATGPT_CACHE_KEY=0 FACTR_OPENAI_PARALLEL_TOOLS=0 FACTR_OPENAI_REPLAY_REASONING_ID=1`), then flip one: strict off only, cache key only, native compaction off only (`FACTR_OPENAI_NATIVE_COMPACTION_MODE=off`), parallel on only, reasoning id off only.

Two ways to compute the rate:

- From the spans (works without the flag): read each call's duration (`ended_at_ms - started_at_ms`) and `output_tokens` from the `Model call` spans (table `spans` in `factr.db`) and fit duration = a + b * output_tokens across the calls of an arm; tokens per second = 1 / b (with duration in seconds). `output_tokens` includes hidden reasoning. The baseline measured this way is about 3.3 s + 19.7 s per 1,000 output tokens (about 50 tok/s); a clear result is an arm near 10 s per 1,000 or lower (100+ tok/s). Use prompts that produce at least 1,500 output tokens so the fixed per-call part does not dominate.
- From the timing line (`FACTR_STREAM_TIMING=1`): tokens per second = `1000 * output_tokens / (last_event_ms - first_output_ms)`, and the visible rate = `1000 * (output_tokens - reasoning_tokens) / (last_event_ms - first_output_ms)`. A clear result is 100+ tok/s in an arm against about 50 in the baseline. Check `channel_wait_ms` stays small next to the streaming interval, or the consumer, not the backend, set the pace.
