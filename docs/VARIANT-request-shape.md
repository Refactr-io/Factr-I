# Request-shape variant (branch `perf/request-shape-variant`)

A build whose OpenAI Responses requests can take the shape the reference clients (the Codex CLI and the Python agent) send, so a streaming-speed gap can be tested with one binary. Every change has its own switch, and one master switch makes the build send byte-identical requests to `main`. Other providers (the Copilot runtime included) are not affected: they keep calling the public builders, which stay on the baseline shape.

The shape is read once per process (`RequestShape::current()` in `factr-provider-core/src/request_shape.rs`), so the prewarm payload, the first request, retries, the HTTPS fallback, model switches and every continuation use the same values.

## Switches

| Env | Field | `main` (baseline) | Variant default | What it changes |
|---|---|---|---|---|
| `FACTR_VARIANT_BASELINE=1` | all | | | Master switch: exactly the `main` request, every other switch ignored |
| `FACTR_OPENAI_STRICT_TOOLS` | `strict_tools` | `1` | `0` | Off: every tool is sent with the compatible (not strict-rewritten) schema and `strict: false` |
| `FACTR_OPENAI_CHATGPT_CACHE_KEY` | `chatgpt_cache_key` | `0` | `1` | On: the session `prompt_cache_key` is also sent in ChatGPT mode. `prompt_cache_retention` stays API-key-only. The idle prewarm payload leaves the key out so warm sockets are still adopted |
| `FACTR_OPENAI_PARALLEL_TOOLS` | `parallel_tool_calls` | `0` | `0` | On: `parallel_tool_calls: true` on the full request and its continuation. Calls from one response still run one at a time, in order |
| `FACTR_OPENAI_REPLAY_REASONING_ID` | `replay_reasoning_id` | `1` | `1` | Off: replayed reasoning items carry no `id` (full request and explicit native-compaction request) |

Each switch takes `0` or `1`; any other value keeps the default. Native compaction already has its own switch: `FACTR_OPENAI_NATIVE_COMPACTION_MODE`.

## Unverified assumptions

- That the backend accepts `strict: false` with the compatible schema is inferred from the reference clients; nothing local shows it.
- That the ChatGPT backend accepts `prompt_cache_key` is inferred from the reference clients; nothing local shows it.

## Stream timing line

Off unless `FACTR_STREAM_TIMING=1` (read once per process; any other value is off). With it on, the OpenAI runtime prints one JSON line to stderr for every model-call attempt that reaches a terminal response (`response.completed`, `response.incomplete` or `response.failed`). A retried attempt or an HTTPS fallback that reaches its own terminal response prints its own line. With it off nothing is observed and nothing is printed.

Each line carries `"factr_stream_timing":1` (grep for it); keys are in alphabetical order. Fields:

- `transport`: `websocket` (fresh socket), `ws-continuation` (reused socket with `previous_response_id`) or `https`.
- `first_event_ms`, `first_output_ms`, `last_event_ms`: milliseconds from `t0` to the first event, to the first output delta (text, refusal, tool arguments or reasoning summary), and to the last event.
- `channel_wait_ms`: total time the reader spent waiting to hand events to the consumer.
- `input_tokens`, `cached_tokens`, `output_tokens`, `reasoning_tokens`: from the terminal response's `usage` (`output_tokens` includes the hidden reasoning).
- `served_service_tier`, `response_model`: from the terminal response, `null` when absent.
- `strict_tools`, `reasoning_id_replayed`: the request-shape switches. `parallel_tool_calls`, `cache_key_sent`, `native_compaction_sent`: what the sent request carried.

The times are application-observed, not network-exact. `t0` is taken just before the request is handed to the transport (before the WebSocket send, before the HTTPS request goes out). Each event is stamped when the runtime reads its frame or chunk, before conversion and before it is passed on. The reader is not decoupled from delivery, so a slow consumer can still delay later reads; `channel_wait_ms` shows how much of that happened.

## Later test (text only, not run)

One binary, the same prompt producing about 2k output tokens at effort high, one request at a time, three calls per arm:

1. `FACTR_VARIANT_BASELINE=1`.
2. Variant defaults.
3. Single toggles: leave the master switch unset and set all four switches to their baseline values (`FACTR_OPENAI_STRICT_TOOLS=1 FACTR_OPENAI_CHATGPT_CACHE_KEY=0 FACTR_OPENAI_PARALLEL_TOOLS=0 FACTR_OPENAI_REPLAY_REASONING_ID=1`), then flip one: strict off only, cache key only, native compaction off only (`FACTR_OPENAI_NATIVE_COMPACTION_MODE=off`), parallel on only, reasoning id off only.

Two ways to compute the rate:

- From the spans (works without the flag): read each call's duration (`ended_at_ms - started_at_ms`) and `output_tokens` from the `Model call` spans (table `spans` in `factr.db`) and fit duration = a + b * output_tokens across the calls of an arm; tokens per second = 1 / b (with duration in seconds). `output_tokens` includes hidden reasoning. The baseline measured this way is about 3.3 s + 19.7 s per 1,000 output tokens (about 50 tok/s); a clear result is an arm near 10 s per 1,000 or lower (100+ tok/s). Use prompts that produce at least 1,500 output tokens so the fixed per-call part does not dominate.
- From the timing line (`FACTR_STREAM_TIMING=1`): tokens per second = `1000 * output_tokens / (last_event_ms - first_output_ms)`, and the visible rate = `1000 * (output_tokens - reasoning_tokens) / (last_event_ms - first_output_ms)`. A clear result is 100+ tok/s in an arm against about 50 in the baseline. Check `channel_wait_ms` stays small next to the streaming interval, or the consumer, not the backend, set the pace.
