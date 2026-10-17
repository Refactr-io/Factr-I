# Factr-I bot computer image

One container per bot: the bot's own Factr-I engine (`factr --profile <bot> serve`), the Python feature backend, the Xfce desktop (Bot Desktop) and Chromium. Debian bookworm, native architecture (arm64 on Apple Silicon, amd64 elsewhere).

Image name the app uses: `bots-factr-bot-computer:<tag>` (dev builds: `bots-factr-bot-computer:dev`).

## Build

From the repo root (the context needs `engine/`, `backend/` and `computer/`; `computer/Dockerfile.dockerignore` keeps the rest out):

```sh
docker build -f computer/Dockerfile -t bots-factr-bot-computer:dev .
# optional: --build-arg FACTR_BUILD_GIT_HASH=$(git rev-parse --short HEAD)
```

The app looks for bots-factr-bot-computer:dev.

The first build compiles the Rust engine (minutes). Cargo registry and `target/` live in BuildKit cache mounts, so they never land in an image layer and later builds only recompile what changed. Stages: `engine` (cargo release build of the `factr` bin), `backend` (Python 3.12 via uv, `pyproject.toml` `--extra bundled`, laid out like the packaged app's `backend-python/`), `runtime` (Bot Desktop packages from `backend/tools/bot_desktop/runtime.py`, Chromium, fonts, tini, socat, git, curl; no compilers).

## Run contract (what the app relies on)

| Item | Value |
|---|---|
| User | `bot`, uid/gid 1000, no sudo. On a Linux host the profile folder must be writable by uid 1000 (Docker Desktop maps ownership). |
| `FACTR_BOT` (env, required) | Profile name, `[A-Za-z0-9_-]{1,64}`. The entrypoint exits 64 on anything else. |
| `FACTR_ENGINE_TOKEN` (env, required) | Engine session token, at least 32 characters (the engine refuses shorter). The entrypoint unsets it and feeds it to the engine on stdin via `--token-stdin`, the mechanism the desktop app uses for its bundled engine (`electron/engine-token.ts`). The `factr` process has it in neither environment nor argv. Caveat: PID 1's initial environ and `docker inspect` still show it; the blast radius is the bot's own engine only. |
| `FACTR_ENGINE_PORT` (env, default 8642) | The port the engine listens on (`--host 0.0.0.0 --port $FACTR_ENGINE_PORT --allow-remote`). The engine rejects any `Host` header whose port is not its listen port (403, or WebSocket close 4403), so publish it as the same number: `-e FACTR_ENGINE_PORT=P -p 127.0.0.1:P:P` with a free `P`. The image `HEALTHCHECK` follows the variable. |
| Profile mount | Host `<FACTR_CONFIG_HOME>/profiles/<bot>` to `/data/profiles/<bot>`, read-write. `FACTR_CONFIG_HOME=/data` inside; `factr --profile <bot>` needs that folder to exist. Holds `config.yaml`, `.env` (API keys the app mirrored in), `SOUL.md`, memories, the Chromium profile and the Bot Desktop state. |
| Engine home (`FACTR_HOME`) | Separate from the profile: the engine's sessions, `factr.db`, memory, skills, logs and locks, and also its **auth store** (`auth.json`, `openai-auth.json`, `google_oauth.json`, `google_credentials.json`). The engine resolves all of them from one directory, so they cannot be split. Default inside the container: `/data/profiles/<bot>/engine` (persists with the bot, per bot). Override: mount a host folder at `/data/engine-home` (picked up automatically), or set `FACTR_HOME` yourself. **It must be read-write**: tested with `:ro`, the engine exits at start (`opening factr.db: unable to open database file`). It needs write access for the SQLite databases (+wal/shm), sessions, logs, and atomic rewrites of `auth.json` when a token refreshes. Seed it with the bot's own copy of the auth files. A copy refreshed independently of the host's can invalidate the other side if the provider rotates single-use refresh tokens (not testable here); API keys through the profile `.env` avoid that. Never mount the user's real `~/.factr/engine`: it holds every session. |
| Health | `GET /api/health` returns `200 {"ok":true,...}` with no token (also `/api/status`). Every other `/api/*` route needs the token. |
| Token on requests | `X-Factr-Session-Token: <token>`, or `Authorization: Bearer <token>`, or `?token=` for WebSockets (`/api/ws`). Missing or wrong token: `401`. |
| Bot Desktop | Linux inside the container: `runtime.is_supported_host()` passes, `missing_binaries()` is empty. It starts by itself once the engine answers (the entrypoint calls `tools.bot_desktop.runtime.start()`; `FACTR_BOT_COMPUTER=1` makes the backend auto-start it on the next screen use too, so the profile's `bot_desktop.auto_start` is never touched and an idle stop does not leave the Screen empty). `display.start` over `/api/ws` stays idempotent. The RFB socket stays inside the container; the app reaches the screen through the engine's `/api/display/ws` ticket bridge. Docker Desktop for Mac refuses `chmod` on a socket in a bind mount (Xvnc: `unable to set socket mode`), so when the mount refuses, the entrypoint makes `<profile>/bot-desktop` a symlink to container-local `/home/bot/.bot-desktop` and persists only the Chromium profile at `<profile>/browser-profile` (symlinked back into `bot-desktop/browser-profile`). On a Linux host the plain folder is kept. Xfce settings and the X auth file are therefore per container life on the Mac layout. |
| Local models | The entrypoint forwards container `127.0.0.1:11434` (Ollama) and `:1234` (LM Studio) to `host.docker.internal` on the same ports with socat, so a `localhost` provider URL works unchanged. Docker Desktop resolves `host.docker.internal`; on Linux add `--add-host host.docker.internal:host-gateway` and make the host service listen on more than loopback. Set `OLLAMA_HOST=127.0.0.1:1` if a bot must not auto-pick a local Ollama model. |
| Time zone | `-e TZ=<host IANA zone>` (for example `Europe/Paris`). The image ships `tzdata`; the entrypoint exports `TZ` when `/usr/share/zoneinfo/$TZ` exists and otherwise warns and falls back to UTC. `date`, cron and Chromium then agree with the user. |
| computer_use | Not available: the image has no `cua-driver` (its upstream installer fetches a prebuilt Linux binary from GitHub at build time, which is not baked in), so the `computer_use` tool is not offered to the model; bots use the browser tool, the shell and `request_takeover`. |
| request_takeover | `request_takeover {site, why, wait_seconds?}` (Python tool, toolset `bot_desktop`, offered through the engine's `factr` tool only where a Bot Desktop can run). Starts the desktop, sets `request` on the lease (a new optional `request` field in the `display.lease` payload and `display.status` lease; the Screen pane shows it with the bell and it becomes the takeover reason when the user clicks Take over), and tells the model to wait for the hand-back instead of asking for credentials in chat. |
| Chromium | `/usr/bin/chromium`; the backend adds `--no-sandbox` itself when it detects Docker. |
| Signals | `tini` is PID 1; `docker stop -t 10` ends the engine cleanly. Stale X, Chromium singleton and desktop pid files are cleared on every start. |

## Recommended `docker run` flags (design section 6, with the coordinator's changes)

```sh
docker network create -o com.docker.network.bridge.enable_icc=false bots-factr-net   # once; bots cannot reach each other
docker run -d --name bots-<bot> --network bots-factr-net \
  --add-host host.docker.internal:host-gateway \
  --label factr.bot=<bot> --label factr.computer.v=1 \
  --cap-drop ALL --security-opt no-new-privileges \
  --pids-limit 2048 --memory 2g --cpus 2 --shm-size 512m \
  -p 127.0.0.1:P:P -e FACTR_ENGINE_PORT=P \
  -e FACTR_BOT=<bot> -e FACTR_ENGINE_TOKEN=<token> \
  -v <FACTR_CONFIG_HOME>/profiles/<bot>:/data/profiles/<bot> \
  [-v <bot engine-home dir>:/data/engine-home] \
  bots-factr-bot-computer:<tag>
```

`--pids-limit` counts threads: 512 is too low for engine, Python, Xfce and Chromium together. No Docker socket and no host home mount. Optional `--runtime runsc` (gVisor) where installed.

## Test

```sh
computer/tests/test_computer_image.sh        # builds the image if missing; BUILD=1 rebuilds; SKIP_OLLAMA=1 skips (f)
```

Starts bots on throwaway profile folders under `/Users/rameelmalik/factri-bots-scratch/img/` (override with `SCRATCH=`), as containers `bots-test-a` to `bots-test-d` on `bots-factr-net`. Checks health and token enforcement on `127.0.0.1:P`, separate filesystems, separate PID and network namespaces with no cross-bot reach, the engine-home and local-model forwards, a read-only engine home, persistence across stop/start, the Bot Desktop starting inside the container, and (when Ollama with `qwen3.8:27b` runs on the host, `OLLAMA_MODEL=` to change) one real model turn through the engine. Skips when Docker is not reachable. Removes only what it created, by exact name; the network is removed only if the test created it.

## Known limitation: dev server origin (dev builds only)

The engine refuses a WebSocket whose `Origin` is `http(s)://` with a port other than its own listen port (`origin_mismatch`, close code 4403; `engine/crates/factr-gateway/src/auth.rs`). It has no option to allow an extra origin. A packaged app loads from `file://` or `app://`, which the engine accepts, but the desktop dev server (`http://127.0.0.1:5174`) is refused, so in `npm run dev` the renderer cannot open a bot's container engine and the window sits on "Reconnecting" after a bot becomes the active profile. Developers testing bot computers should run the built renderer (`npm run build`, then `electron .` without `FACTR_DESKTOP_DEV_SERVER`), as the e2e fixtures do.

