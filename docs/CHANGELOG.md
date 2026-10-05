# Changelog

## v0.0.1 (unreleased)

- websearch: the model-visible schema no longer has `engine` or `bing_market`; the operator picks the backend via
  `websearch.engine` / `FACTR_WEBSEARCH_ENGINE`. Stale arguments are accepted, ignored, and noted in the result.
- websearch: pinned SearXNG mode (`engine: searxng` plus a URL ignores fallbacks and key backends);
  `websearch.last_resort_wikipedia` / `FACTR_WEBSEARCH_LAST_RESORT_WIKIPEDIA`.
- webfetch: `webfetch.allowed_hosts` / `FACTR_WEBFETCH_ALLOWED_HOSTS` (redirect hops re-checked; `host:port` entries match
  the port too) and `webfetch.wayback_fallback` / `FACTR_WEBFETCH_WAYBACK`. See `docs/design/web-tools.md`.
- REPL: `spawn_subagent` now follows the run's tool policy (needs `delegate` allowed); `llm_query*` labelled as plain model calls.
- OpenAI Responses (ChatGPT mode): the hosted `image_generation` tool follows the run's tool policy; default unchanged
  when no policy is set. `parallel_tool_calls: false` is unchanged and documented.
- Large inputs: the environment line lists file sizes and line counts with a `large input` marker; the system prompt
  says to inspect size and structure first, infer non-literal properties per record, and give a best estimate rather
  than "cannot determine"; todo is for multi-step work after a first look at inputs. See `docs/design/large-input-guidance.md`.
- REPL: the tool description states the real limits; `llm_query` prompts over 200000 characters are an error (was a
  silent cut); host-call time exhaustion returns an error to the cell and keeps the worker; batches earn extra wait.
- read: a plain read of a file over ~20 KB returns an overview (size, lines, head, tail, samples) with a pointer to ranges.
- Stop nudges: self-declared `verified` todos with no inspecting tool call, a large-input answer after at most one
  analysis call, more "cannot determine" endings, and Title-case required answer labels.
- REPL: `await classify(items, labels, guidance=None, votes=1)` returns one label per item from the full label set (numbered
  chunks, JSON reply validated for every id exactly once and allowed labels, failing chunks re-asked up to twice, `votes>1`
  re-asks only disagreements); `llm_query_batch([])` is an explicit error. The REPL tool description cap is now 215 tokens.
- System prompt: whole-set judged counts are labelled per record over the full label set and counted in code; before
  answering from counts, print the table over every allowed label and check it sums to the record count. First-request
  prefix +185 tokens (4497 -> 4682).
- Stop nudges: typographic apostrophes match like plain ones; "can't/cannot calculate" and "cannot be verified / can't verify"
  count as give-ups.
