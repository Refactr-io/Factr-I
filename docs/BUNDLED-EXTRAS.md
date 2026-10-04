# Bundled optional extras

The packaged app has no virtualenv, so the backend's lazy installer
(`backend/tools/lazy_deps.py`) cannot add packages at run time. Anything a
user should be able to use out of the box is therefore installed at staging
time through one aggregate extra, `bundled`, in `backend/pyproject.toml`.
`desktop/app/scripts/stage-backend-python.mjs` runs
`uv pip install --target ... --requirements backend/pyproject.toml --extra bundled`
and then imports the key packages, so a missing one fails the build.

## What ships (measured on macOS arm64, CPython 3.12)

Core install alone: about 119 MB. Core plus `bundled`: about 220 MB (+101 MB).
Per-extra figures are the increase over core when installed alone, so shared
libraries (aiohttp and friends) are counted in each row.

| Extra | Gives | +MB |
| --- | --- | --- |
| `messaging` | Telegram, Discord (with PyNaCl for voice; the Opus library is a system library and is not bundled), Slack, aiohttp, brotlicffi, qrcode | 21 |
| `matrix-lite` | Matrix without end-to-end encryption (mautrix, aiosqlite, asyncpg, aiohttp-socks) | 9 |
| `dingtalk` | DingTalk | 44 |
| `teams` | Microsoft Teams | 21 |
| `wecom`, `homeassistant`, `sms` | WeCom, Home Assistant, SMS | 0 to 3 |
| `anthropic` | native Anthropic provider | 3 |
| `mcp` | MCP client | 5 |
| `acp` | agent-client-protocol server | 0 |
| `exa`, `firecrawl`, `parallel-web` | web search and extract providers (Tavily needs nothing extra) | 0 to 5 |
| `fal` | fal image generation | 0 |
| `edge-tts` | text to speech | 4 |
| `youtube` | transcript skill | 2 |
| `uvloop` | faster event loop (not installed on Windows) | 4 |

Pure Python or light prebuilt wheels on macOS, Linux and Windows. None needs a hosted
Factr-specific service and none sends telemetry.

## What is left out, and why

| Extra | +MB | Reason |
| --- | --- | --- |
| `voice`, `wake` (faster-whisper, onnxruntime, sherpa-onnx, numpy) | 170+ | ONNX/ctranslate2 runtimes, model downloads |
| `google`, `google-chat` | 105, 148 | the Google API client carries about 100 MB of discovery documents |
| `feishu` | 59 | lark-oapi is large; Feishu still works by installing it into a venv |
| `bedrock`, `daytona`, `modal`, `vercel` | 12 to 30 | cloud-specific; used by few |
| `tts-premium` (elevenlabs), `mistral` | 4, 8 | paid third-party services |
| Matrix end-to-end encryption (`mautrix[encryption]`) | n/a | needs python-olm, which has no macOS/Windows wheels and a C build step |
| `honcho`, `supermemory`, `mem0`, `otlp` | small | opt-in third-party services or telemetry export |

Without the bundled copy, the adapters report the missing package and the lazy
installer cannot satisfy it in the packaged app. Matrix is set up to run with
`MATRIX_E2EE_MODE=off` (or `optional`).

## Adding an extra

1. Make sure the extra exists in `backend/pyproject.toml` with exact pins.
2. Add `"factr-backend[<name>]"` to the `bundled` list.
3. Run `cd backend && uv lock && uv lock --check`.
4. Add the importable module names to `probeModules` in
   `desktop/app/scripts/stage-backend-python.mjs`.
5. Check the size: `uv pip install --target /tmp/x --python 3.12 -r backend/pyproject.toml --extra bundled && du -sm /tmp/x`,
   and update the table above.
