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

Not built on this branch. The planned `FACTR_STREAM_TIMING=1` per-call stderr line (`first_event_ms`, `first_output_ms`, `last_event_ms`, token counts, tier, model, `channel_wait_ms` and the shape fields) is not needed to measure rates: rates can be measured today from the `Model call` spans in `factr.db`; the timing line would only add time-to-first-output and the reasoning-token split.

## Later test (text only, not run)

One binary, the same prompt producing about 2k output tokens at effort high, one request at a time, three calls per arm:

1. `FACTR_VARIANT_BASELINE=1`.
2. Variant defaults.
3. Single toggles: leave the master switch unset and set all four switches to their baseline values (`FACTR_OPENAI_STRICT_TOOLS=1 FACTR_OPENAI_CHATGPT_CACHE_KEY=0 FACTR_OPENAI_PARALLEL_TOOLS=0 FACTR_OPENAI_REPLAY_REASONING_ID=1`), then flip one: strict off only, cache key only, native compaction off only (`FACTR_OPENAI_NATIVE_COMPACTION_MODE=off`), parallel on only, reasoning id off only.

Until the timing line exists, read each call's duration (`ended_at_ms - started_at_ms`) and `output_tokens` from the `Model call` spans (table `spans` in `factr.db`) and fit duration = a + b * output_tokens across the calls of an arm; tokens per second = 1 / b (with duration in seconds). `output_tokens` includes hidden reasoning. The baseline measured this way is about 3.3 s + 19.7 s per 1,000 output tokens (about 50 tok/s); a clear result is an arm near 10 s per 1,000 or lower (100+ tok/s). Use prompts that produce at least 1,500 output tokens so the fixed per-call part does not dominate.
