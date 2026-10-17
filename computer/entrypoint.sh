#!/bin/bash
# Factr-I bot computer entrypoint (tini is PID 1 and runs this).
# Contract: FACTR_BOT = profile name, FACTR_ENGINE_TOKEN = the engine's session token (>= 32 chars),
# /data/profiles/$FACTR_BOT = the bot's profile folder (a bind mount from the host).
set -euo pipefail

bot="${FACTR_BOT:-}"
if ! [[ "$bot" =~ ^[A-Za-z0-9_-]{1,64}$ ]]; then
  echo "factr-bot-computer: FACTR_BOT must be a profile name (letters, digits, - and _)" >&2
  exit 64
fi
# The engine only accepts a Host header whose port is its own listen port, so the app picks a free host
# port P and runs the container with -e FACTR_ENGINE_PORT=P -p 127.0.0.1:P:P.
port="${FACTR_ENGINE_PORT:-8642}"
if ! [[ "$port" =~ ^[0-9]{1,5}$ ]] || [ "$port" -lt 1024 ] || [ "$port" -gt 65535 ]; then
  echo "factr-bot-computer: FACTR_ENGINE_PORT must be a port number from 1024 to 65535" >&2
  exit 64
fi
token="${FACTR_ENGINE_TOKEN:-}"
if [ "${#token}" -lt 32 ]; then
  echo "factr-bot-computer: FACTR_ENGINE_TOKEN is required (at least 32 characters)" >&2
  exit 64
fi
# The engine reads the token from stdin (--token-stdin), like the desktop app's bundled engine; keep it
# out of the environment so the agent's shell commands cannot read it.
unset FACTR_ENGINE_TOKEN

export FACTR_CONFIG_HOME="${FACTR_CONFIG_HOME:-/data}"
home="$FACTR_CONFIG_HOME/profiles/$bot"
[ "$bot" = default ] && home="$FACTR_CONFIG_HOME"
mkdir -p "$home"

# The engine's own home ($FACTR_HOME: sessions, factr.db, memory, skills and the auth store auth.json,
# openai-auth.json, google_oauth.json) is NOT inside the profile folder; left alone it would be the
# container's throwaway ~/.factr/engine. Keep it with the bot: an optional /data/engine-home mount wins,
# else a folder inside the profile mount. An explicit FACTR_HOME env wins over both.
if [ -z "${FACTR_HOME:-}" ]; then
  if [ -d /data/engine-home ]; then FACTR_HOME=/data/engine-home; else FACTR_HOME="$home/engine"; fi
fi
export FACTR_HOME
mkdir -p "$FACTR_HOME"
chmod 700 "$FACTR_HOME" 2>/dev/null || true

# Local model servers on the host (Ollama 11434, LM Studio 1234) are "localhost" in the bot's config; forward
# the same ports from the container's loopback so no config is rewritten. Needs host.docker.internal
# (automatic on Docker Desktop; `--add-host host.docker.internal:host-gateway` on Linux).
if getent hosts host.docker.internal >/dev/null 2>&1; then
  for lp in 11434 1234; do
    socat TCP-LISTEN:$lp,bind=127.0.0.1,fork,reuseaddr TCP:host.docker.internal:$lp &
  done
fi

# Bot Desktop's Xvnc must chmod its RFB unix socket, and Docker Desktop's macOS bind mounts refuse that
# ("vncExtInit: unable to set socket mode"). Probe the mount; when it refuses, keep bot-desktop/ (sockets,
# X auth, xfce config) on the container's own disk and persist only the Chromium profile in the mount.
bd="$home/bot-desktop"
probe="$home/.sock-probe"
rm -f "$probe"
sockrc=0; timeout 1 socat UNIX-LISTEN:"$probe",mode=0600 /dev/null 2>/dev/null || sockrc=$?
rm -f "$probe"
if [ "$sockrc" -ne 0 ] && [ "$sockrc" -ne 124 ] && [ ! -L "$bd" ]; then
  mkdir -p "$home/browser-profile"
  [ -d "$bd/browser-profile" ] && [ -z "$(ls -A "$home/browser-profile")" ] \
    && cp -a "$bd/browser-profile/." "$home/browser-profile/"
  rm -rf "$bd"
  mkdir -p /home/bot/.bot-desktop
  ln -s /home/bot/.bot-desktop "$bd"
fi
if [ -L "$bd" ]; then
  mkdir -p "$home/browser-profile" "$(readlink "$bd")"
  ln -sfn "$home/browser-profile" "$bd/browser-profile"
fi

# A stopped container keeps its writable layer and the bind-mounted profile: clear what a dead
# desktop, browser or X server left behind, or the next start refuses ("profile in use", "display taken").
rm -f "$bd"/launcher.pid "$bd"/env "$bd"/rfb.sock "$bd"/lease.json "$bd"/lease.lock "$bd"/activity \
      "$bd"/browser-profile/Singleton* "$bd"/browser-profile/DevToolsActivePort
rm -f /tmp/.X*-lock /tmp/.X11-unix/X* 2>/dev/null || true

# The host's IANA time zone, passed by the app as TZ (-e TZ=Europe/Paris), so `date`, cron and Chromium agree with
# the user. An unknown zone would silently mean UTC; say so and drop it instead.
if [ -n "${TZ:-}" ]; then
  if [[ "$TZ" =~ ^[A-Za-z0-9_+/.-]{1,64}$ ]] && [ -f "/usr/share/zoneinfo/$TZ" ]; then
    export TZ
  else
    echo "factr-bot-computer: unknown TZ '$TZ', using UTC" >&2
    unset TZ
  fi
fi

# Same environment the desktop app gives its bundled engine (desktop/app/electron/main.ts resolveFactrBackend).
py=/opt/factr/backend-python
export FACTR_REPL_PYTHON="$py/runtime/bin/python3.12"
export FACTR_BACKEND_PYTHON="$py/runtime/bin/python3.12"
export FACTR_BACKEND_PYTHONPATH="$py/source:$py/packages"
export FACTR_BACKEND_DEFAULTS="$py/defaults.json"
export FACTR_BACKEND_TOOLS_DIR="$py/tools"
export PYTHONDONTWRITEBYTECODE=1
export FACTR_LAZY_INSTALL_TARGET="${FACTR_LAZY_INSTALL_TARGET:-$home/lazy-packages}"
export FACTR_LAZY_INSTALL_ONLY="${FACTR_LAZY_INSTALL_ONLY:-stt.faster_whisper}"
export FACTR_OLLAMA_NUM_CTX="${FACTR_OLLAMA_NUM_CTX:-32768}"
export FACTR_RUNTIME_DIR="${FACTR_RUNTIME_DIR:-/tmp/factr-rt}"
export FACTR_DESKTOP=1
mkdir -p -m 700 "$FACTR_RUNTIME_DIR"
cd "$home"

# The bot's Screen should never be empty: bring the desktop up once the engine answers, with no RPC needed.
# (backend/tools/bot_desktop/runtime.py auto-restarts it on the next screen use because FACTR_BOT_COMPUTER=1.)
(
  for _ in $(seq 1 120); do
    curl -fsS -m 2 "http://127.0.0.1:$port/api/health" >/dev/null 2>&1 && break
    sleep 1
  done
  FACTR_CONFIG_HOME="$home" PYTHONPATH="$py/source:$py/packages" \
    "$py/runtime/bin/python3.12" -c 'from tools.bot_desktop import runtime; runtime.start()' \
    >"$FACTR_RUNTIME_DIR/desktop-autostart.log" 2>&1
) &

exec factr --profile "$bot" serve --host 0.0.0.0 --port "$port" --allow-remote --token-stdin <<<"$token"
