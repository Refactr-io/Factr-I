# Web tools: operator-controlled backends

The model never chooses where web traffic goes. The operator does, through config or environment.
The model-visible `websearch` schema is `query` and `num_results` (plus `intent`); `webfetch` takes a URL.

## websearch

Backend selection, in order:

1. `websearch.engine` / `FACTR_WEBSEARCH_ENGINE`: `duckduckgo` (default), `bing`, `searxng`.
2. `websearch.fallback_engines` / `FACTR_WEBSEARCH_FALLBACK_ENGINES`: tried when the first returns nothing.
3. A key-based backend from `web.backend` (checked first when configured; any failure falls through to the keyless chain).
4. `websearch.last_resort_wikipedia` / `FACTR_WEBSEARCH_LAST_RESORT_WIKIPEDIA` (default on): Wikipedia opensearch when everything else is empty.

Related: `websearch.searxng_url` / `FACTR_SEARXNG_URL`, `websearch.bing_api_key` / `FACTR_BING_API_KEY`
(`FACTR_BING_API_KEY_ENV` names the variable to read), `websearch.bing_market` / `FACTR_BING_MARKET` (default `en-US`).

Pinned mode: `engine = searxng` and a SearXNG URL set. Every search goes to that one instance. Fallback engines, key
backends and the per-call arguments are ignored. Wikipedia last resort still applies unless switched off.

### What the model sees

- Schema: `query`, `num_results`. No `engine`, no `bing_market`, in any mode.
- A stale model that still sends `engine` or `bing_market` is not rejected: the arguments are ignored and the result
  ends with a one-line note saying so.

## webfetch

- `webfetch.allowed_hosts` / `FACTR_WEBFETCH_ALLOWED_HOSTS` (comma or whitespace list; default empty = no restriction).
  When set, only those hosts may be fetched, and every redirect hop is re-checked. An entry `host` matches that host on
  any port; an entry `host:port` (or `[::1]:port`) matches host and port (the scheme default counts: `example.com:443`
  matches `https://example.com`).
- `webfetch.wayback_fallback` / `FACTR_WEBFETCH_WAYBACK` (default on): a 403/404/410 may fall back to an archive.org snapshot.

## Switches that exist, and defaults

| Switch | Default | Effect when changed |
|---|---|---|
| `websearch.engine` | duckduckgo | operator-chosen backend |
| `websearch.searxng_url` | unset | with `engine: searxng`: pinned mode |
| `websearch.last_resort_wikipedia` | on | off keeps all traffic on the configured engine |
| `webfetch.allowed_hosts` | empty | restricts fetches and redirects |
| `webfetch.wayback_fallback` | on | off never contacts archive.org |

Defaults reproduce the earlier behaviour except that a per-call `engine` is no longer honoured.

## Related run-policy changes (same release)

- REPL `spawn_subagent` is refused unless the run's tool policy allows `delegate`. `llm_query` and `llm_query_batch`
  stay available (plain model calls with no tools); the REPL tool description says so.
- The hosted `image_generation` tool (ChatGPT mode) is attached only when the run's tool policy allows `image_generate`
  / `image_gen`, or when the run has no enabled-toolsets policy.
- `parallel_tool_calls` stays `false` on the Responses request on purpose (serial tool calls; documented in the request builder).
