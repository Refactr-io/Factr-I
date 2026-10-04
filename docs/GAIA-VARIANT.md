# GAIA-only web-pinned engine variant

This branch (`gaia-variant`) is the published main of this repository plus the web-only change set that was used to build the labelled GAIA-only benchmark binary.

- Benchmark binary: `factr-gaia-0.0.0-66fab6d` (macOS arm64), sha256 `b450b43119f285c876780df6d1ee7307632f1fbaeaea49f5a4d92364d6942b80`, `__version` `{"db_schema":9,"sha":"66fab6d","version":"0.0.0"}`.
- Provenance: the binary was built, release and `--locked`, from a clean worktree of the commit the other benchmarks were pinned to (`d3b499d`, a local commit that is not part of this repository's published history) plus one commit (`66fab6d`) with the change set below. The published `main` differs from `d3b499d` only by changes outside the web code (the `factr` bridge toolset fix, a font-stack entry and documentation); the change set applies cleanly on top of it, and `engine/crates/factr-app-core/src/tool/websearch.rs`, `webfetch.rs` and `webfetch_net.rs` are identical between `d3b499d` and `main`.
- Why it exists: the engine's default `websearch` tool lets the model choose the search engine per call, which cannot be pinned by configuration alone, so a controlled benchmark needs a way to force all web traffic through one operator-chosen service. The change set adds that as opt-in switches; with none set the behaviour is unchanged. The next release folds these switches into the main engine.

## Change set (`git diff d3b499d..66fab6d`, 9 files, +441/-27)

1. **Pinned SearXNG.** When `websearch.engine = "searxng"` and a SearXNG URL is set (`websearch.searxng_url` or `FACTR_SEARXNG_URL`), the per-call `engine` argument is ignored, fallback engines are never used and the key-based backend is skipped. Requests go to `<searxng_url>/search?q=..&format=json`. The tool schema tells the model the argument is ignored.
2. **`websearch.last_resort_wikipedia`** (bool, default true; env `FACTR_WEBSEARCH_LAST_RESORT_WIKIPEDIA=0` disables). When off there is no Wikipedia request and an empty result reads plainly "No results found for: <q>".
3. **`webfetch.allowed_hosts`** (list, default empty = unrestricted; env `FACTR_WEBFETCH_ALLOWED_HOSTS`, comma list). A URL whose host is not listed is refused, and redirects are re-checked per hop. The restricted mode uses a client without proxy.
4. **`webfetch.wayback_fallback`** (bool, default true; env `FACTR_WEBFETCH_WAYBACK=0` disables). When off, archive.org is never contacted.

Configuration used for the GAIA runs (`$FACTR_HOME/config.toml`, with `<svc>` the loopback search service):

```
[websearch]
engine = "searxng"
searxng_url = "<svc>"
fallback_engines = []
last_resort_wikipedia = false

[webfetch]
allowed_hosts = ["127.0.0.1"]
wayback_fallback = false
```

## Tests

Six new unit tests (`engine/crates/factr-app-core/src/tool/gaia_switch_tests.rs`, local mock servers) cover every switch, including a redirect to a public host being blocked. A live check with a scripted fake model and a fake search service showed only loopback sockets, including for a call that names `engine: "duckduckgo"`.

## Reproduce the change set

```sh
git diff d3b499d 66fab6d      # in the original (private) working tree
# or, on this branch: the single commit on top of main
git show gaia-variant
```
