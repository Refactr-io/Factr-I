#!/bin/bash
# End-to-end test of the Factr-I bot computer image: two bots, two containers, the design's section 6 flags.
#   computer/tests/test_computer_image.sh            # reuse the image if it exists, else build it
#   BUILD=1 computer/tests/test_computer_image.sh    # always rebuild
# Skips (exit 0) when Docker is not reachable. Touches only bots-* containers/networks and the scratch dir.
set -uo pipefail

IMAGE="${IMAGE:-bots-factr-bot-computer:dev}"
SCRATCH="${SCRATCH:-${TMPDIR:-/tmp}/bots-image}/run-$$"
REPO="$(cd "$(dirname "$0")/../.." && pwd)"
A=bots-test-a-$$; B=bots-test-b-$$; C=bots-test-c-$$; D=bots-test-d-$$; NET=bots-factr-net; NET_CREATED=
BOT_A=testbota; BOT_B=testbotb; BOT_C=testbotc; BOT_D=testbotd
PASS=0; FAIL=0; FAILED=""

docker info >/dev/null 2>&1 || { echo "SKIP: Docker is not reachable"; exit 0; }

check() { # check <label> <command...>
  local label="$1"; shift
  if "$@" >/dev/null 2>&1; then echo "  ok   $label"; PASS=$((PASS+1)); else echo "  FAIL $label"; FAIL=$((FAIL+1)); FAILED="$FAILED|$label"; fi
}

cleanup() {
  docker rm -f "$A" "$B" "$C" "$D" >/dev/null 2>&1
  [ -n "$NET_CREATED" ] && docker network rm "$NET" >/dev/null 2>&1
  # Chromium and Xvnc ran as uid 1000 inside the container; on Linux hosts the files may not be ours.
  rm -rf "$SCRATCH" 2>/dev/null
}
cleanup_names_free() { for n in "$A" "$B" "$C" "$D"; do docker inspect "$n" >/dev/null 2>&1 && { echo "refusing to run: $n already exists"; exit 2; }; done; }
cleanup_names_free
trap cleanup EXIT
mkdir -p "$(dirname "$SCRATCH")"

if [ -n "${BUILD:-}" ] || ! docker image inspect "$IMAGE" >/dev/null 2>&1; then
  echo "building $IMAGE"
  docker build -f "$REPO/computer/Dockerfile" -t "$IMAGE" "$REPO" >"$SCRATCH.build.log" 2>&1 \
    || { tail -30 "$SCRATCH.build.log"; echo "FAIL: build"; exit 1; }
fi

# ---- two throwaway profile folders (an empty folder is a valid profile: no API keys, no login) ----
mkdir -p "$SCRATCH/$BOT_A" "$SCRATCH/$BOT_B"
TOKEN_A="$(openssl rand -hex 32)"; TOKEN_B="$(openssl rand -hex 32)"
# One shared bridge with inter-container traffic off (the app's network); only created, and later removed, by us.
if ! docker network inspect "$NET" >/dev/null 2>&1; then
  docker network create -o com.docker.network.bridge.enable_icc=false "$NET" >/dev/null && NET_CREATED=1 \
    || { echo "FAIL: network"; exit 1; }
fi

freeport() { python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1])'; }
# run_bot <name> <bot> <token> <hostdir> <port> [extra docker run args...]: the app's contract: the engine listens on
# FACTR_ENGINE_PORT=P and P is published as 127.0.0.1:P:P (the engine rejects a Host header with any other port).
run_bot() {
  local name="$1" bot="$2" tok="$3" dir="$4" port="$5"; shift 5
  docker run -d --name "$name" --network "$NET" --add-host host.docker.internal:host-gateway \
    --label factr.bot="$bot" --label factr.computer.v=1 \
    --cap-drop ALL --security-opt no-new-privileges --pids-limit 2048 \
    --memory 2g --cpus 2 --shm-size 512m \
    -p "127.0.0.1:$port:$port" -e FACTR_ENGINE_PORT="$port" \
    -e FACTR_BOT="$bot" -e FACTR_ENGINE_TOKEN="$tok" -e OLLAMA_HOST=127.0.0.1:1 \
    -v "$dir:/data/profiles/$bot" "$@" "$IMAGE" >/dev/null
}
# curl sends "Host: 127.0.0.1:<port>", which the engine accepts because <port> is its own listen port.
api() { # api <port> <token|-> <path>  -> http status
  local port="$1" tok="$2" path="$3" hdr=()
  [ "$tok" != "-" ] && hdr=(-H "X-Factr-Session-Token: $tok")
  curl -s -m 5 -o /dev/null -w '%{http_code}' ${hdr[@]+"${hdr[@]}"} "http://127.0.0.1:$port$path"
}
wait_health() { # wait_health <port>
  for _ in $(seq 1 60); do [ "$(api "$1" - /api/health)" = 200 ] && return 0; sleep 1; done; return 1
}

# Bot A gets the optional engine-home mount with FAKE auth (never the real ~/.factr); bot B uses the default.
PORT_A="$(freeport)"; PORT_B="$(freeport)"; PORT_C="$(freeport)"; PORT_D="$(freeport)"
mkdir -p "$SCRATCH/home-a"; echo '{"anthropic_accounts":[]}' > "$SCRATCH/home-a/auth.json"
run_bot "$A" "$BOT_A" "$TOKEN_A" "$SCRATCH/$BOT_A" "$PORT_A" -v "$SCRATCH/home-a:/data/engine-home" -e TZ=Asia/Karachi \
  && run_bot "$B" "$BOT_B" "$TOKEN_B" "$SCRATCH/$BOT_B" "$PORT_B" \
  || { echo "FAIL: docker run"; exit 1; }
echo "bot A on 127.0.0.1:$PORT_A, bot B on 127.0.0.1:$PORT_B"
check "engines come up (health 200)" bash -c "$(declare -f api wait_health); wait_health $PORT_A && wait_health $PORT_B"

echo "(e0) the Screen is never empty: the desktop starts on its own, no RPC call"
waitfor() { for _ in $(seq 1 90); do "$@" >/dev/null 2>&1 && return 0; sleep 1; done; return 1; }
check "A: Xvnc is up with no display.start call" waitfor docker exec "$A" pgrep -x Xvnc
check "A: RFB unix socket exists with no display.start call" waitfor docker exec "$A" test -S "/data/profiles/$BOT_A/bot-desktop/rfb.sock"
check "B: Xvnc and RFB socket come up on their own too" bash -c "$(declare -f waitfor); waitfor docker exec $B pgrep -x Xvnc && waitfor docker exec $B test -S /data/profiles/$BOT_B/bot-desktop/rfb.sock"
check "auto-start did not rewrite the profile (no config.yaml created)" bash -c "! test -e $SCRATCH/$BOT_A/config.yaml"

echo "(tz) host time zone"
check "A: TZ=Asia/Karachi gives +0500" bash -c "[ \"\$(docker exec $A date +%z)\" = +0500 ]"
check "B: no TZ stays UTC" bash -c "[ \"\$(docker exec $B date +%z)\" = +0000 ]"


echo "(tools) computer_use and request_takeover as the model's tool list sees them"
TOOLS="$(docker exec -e FACTR_CONFIG_HOME=/data/profiles/$BOT_A -e FACTR_INTERACTIVE=1 -e PYTHONPATH=/opt/factr/backend-python/source:/opt/factr/backend-python/packages "$A" \
  /opt/factr/backend-python/runtime/bin/python3.12 -c "
import model_tools
from tools.registry import registry
names = [d['function']['name'] for d in model_tools.get_tool_definitions(enabled_toolsets=list(registry.get_registered_toolset_names()), quiet_mode=True, skip_tool_search_assembly=True)]
print('request_takeover' in names, 'computer_use' in names)" 2>/dev/null | tail -1)"
echo "  request_takeover offered, computer_use offered: $TOOLS"
check "request_takeover is offered, computer_use is not (no cua-driver in the image)" test "$TOOLS" = "True False"

echo "(a) health route and token"
check "A: /api/health 200" test "$(api "$PORT_A" - /api/health)" = 200
check "B: /api/health 200" test "$(api "$PORT_B" - /api/health)" = 200
check "A: authed route with A token 200" test "$(api "$PORT_A" "$TOKEN_A" /api/config)" = 200
check "A: authed route without token 401" test "$(api "$PORT_A" - /api/config)" = 401
check "A: authed route with B token 401" test "$(api "$PORT_A" "$TOKEN_B" /api/config)" = 401
check "B: authed route with B token 200" test "$(api "$PORT_B" "$TOKEN_B" /api/config)" = 200
check "published on 127.0.0.1:P only" bash -c "[ \"\$(docker port $A $PORT_A/tcp)\" = 127.0.0.1:$PORT_A ]"
check "Host with a foreign port is refused (403)" test "$(curl -s -m 5 -o /dev/null -w '%{http_code}' -H 'Host: 127.0.0.1:9' "http://127.0.0.1:$PORT_A/api/health")" = 403
check "healthcheck follows FACTR_ENGINE_PORT" bash -c "docker inspect -f '{{json .Config.Healthcheck.Test}}' $A | grep -q FACTR_ENGINE_PORT"
# PID 1's initial environ (and docker inspect) still hold the token; the engine process itself must not.
check "token is not in the engine's environment or argv" bash -c "p=\$(docker exec $A pgrep -x factr | head -1); [ -n \"\$p\" ] && ! docker exec $A sh -c \"cat /proc/\$p/environ /proc/\$p/cmdline\" | tr '\\0' '\\n' | grep -q '$TOKEN_A'"
check "container runs as uid 1000, healthcheck healthy" bash -c "[ \"\$(docker exec $A id -u)\" = 1000 ] && for i in \$(seq 1 30); do [ \"\$(docker inspect -f '{{.State.Health.Status}}' $A)\" = healthy ] && exit 0; sleep 2; done; exit 1"

echo "(b) filesystems are separate"
docker exec "$A" sh -c "echo from-a > /home/bot/secret-a && echo from-a > /data/profiles/$BOT_A/secret-a"
check "A sees its own file" docker exec "$A" test -f /home/bot/secret-a
check "B does not see A's container file" bash -c "! docker exec $B test -e /home/bot/secret-a"
check "B does not see A's profile file" bash -c "! docker exec $B test -e /data/profiles/$BOT_A/secret-a"
check "B has no A profile folder at all" bash -c "! docker exec $B test -e /data/profiles/$BOT_A"
check "A's profile file is on the host folder" test -f "$SCRATCH/$BOT_A/secret-a"
check "B's host folder has no A file" bash -c "! test -e $SCRATCH/$BOT_B/secret-a"

echo "(c) separate PID and network namespaces"
nsa() { docker exec "$1" readlink "/proc/1/ns/$2"; }
check "PID namespaces differ" bash -c "$(declare -f nsa); [ \"\$(nsa $A pid)\" != \"\$(nsa $B pid)\" ]"
check "net namespaces differ" bash -c "$(declare -f nsa); [ \"\$(nsa $A net)\" != \"\$(nsa $B net)\" ]"
check "PID 1 is tini" bash -c "docker exec $A cat /proc/1/comm | grep -q tini"
IP_B="$(docker inspect -f "{{(index .NetworkSettings.Networks \"$NET\").IPAddress}}" "$B")"
check "A and B share one network ($NET)" bash -c "[ \"\$(docker inspect -f '{{.HostConfig.NetworkMode}}' $A)\" = $NET ] && [ \"\$(docker inspect -f '{{.HostConfig.NetworkMode}}' $B)\" = $NET ]"
check "A cannot resolve B by name" bash -c "! docker exec $A curl -s -m 4 -o /dev/null http://$B:$PORT_B/api/health"
check "A cannot reach B by IP ($IP_B)" bash -c "! docker exec $A curl -s -m 4 -o /dev/null http://$IP_B:$PORT_B/api/health"
check "A can still reach itself" docker exec "$A" curl -fsS -m 4 -o /dev/null http://127.0.0.1:$PORT_A/api/health

echo "(c2) engine home (FACTR_HOME), auth store, local-model forwards"
check "A: FACTR_HOME is the /data/engine-home mount; engine state and fake auth live there" bash -c "[ -f $SCRATCH/home-a/auth.json ] && [ -f $SCRATCH/home-a/factr.db ]"
check "B: FACTR_HOME defaults into its profile folder" test -f "$SCRATCH/$BOT_B/engine/factr.db"
check "A's engine home has none of B's state" bash -c "! test -e $SCRATCH/$BOT_A/engine"
check "socat forwards 11434 and 1234 on the container loopback" bash -c "docker exec $A sh -c 'pgrep -af socat' | grep -q 11434 && docker exec $A sh -c 'pgrep -af socat' | grep -q 1234"
check "pids limit is 2048" bash -c "[ \"\$(docker inspect -f '{{.HostConfig.PidsLimit}}' $A)\" = 2048 ]"

echo "(c3) optional engine-home mount read-only (what the engine needs)"
# One pair at a time on the shared daemon.
docker stop -t 10 "$B" >/dev/null
mkdir -p "$SCRATCH/home-d" "$SCRATCH/$BOT_D"; echo '{"anthropic_accounts":[]}' > "$SCRATCH/home-d/auth.json"
run_bot "$D" "$BOT_D" "$TOKEN_B" "$SCRATCH/$BOT_D" "$PORT_D" -v "$SCRATCH/home-d:/data/engine-home:ro" -e FACTR_HOME=/data/engine-home -e TZ=Nowhere/Land
sleep 20
echo "  read-only engine home: health=$(api "$PORT_D" - /api/health) state=$(docker inspect -f '{{.State.Status}}' "$D") files=$(ls "$SCRATCH/home-d" | tr '\n' ' ')"
docker logs --tail 5 "$D" 2>&1 | sed 's/^/    /' | cut -c1-200
check "an unknown TZ is dropped, not trusted" bash -c "docker logs $D 2>&1 | grep -q 'unknown TZ'"
check "a read-only engine home is refused: the engine needs it writable" test "$(docker inspect -f '{{.State.Status}}' "$D")" = exited

docker rm -f "$D" >/dev/null
docker start "$B" >/dev/null
check "B returns after the read-only mount probe" bash -c "$(declare -f api wait_health); wait_health $PORT_B"

echo "(d) persistence across docker stop/start"
BP="/data/profiles/$BOT_A/bot-desktop/browser-profile"
# A real Chromium run writes its user-data-dir; then plant stale singleton files a crash would leave behind.
docker exec "$A" chromium --headless=new --no-sandbox --disable-gpu --user-data-dir="$BP" --dump-dom about:blank >/dev/null 2>&1
# On Docker Desktop for Mac the entrypoint keeps bot-desktop/ off the mount and persists <profile>/browser-profile.
check "Chromium wrote its profile into the bot folder" bash -c "test -f '$SCRATCH/$BOT_A/browser-profile/Local State' || test -f '$SCRATCH/$BOT_A/bot-desktop/browser-profile/Local State'"
docker exec "$A" sh -c "ln -s stale-host-1234 $BP/SingletonLock; echo marker > $BP/marker"
docker stop -t 10 "$A" >/dev/null && docker start "$A" >/dev/null
check "A engine healthy again after restart" bash -c "$(declare -f api wait_health); wait_health $PORT_A"
check "profile file survived" bash -c "[ \"\$(docker exec $A cat /data/profiles/$BOT_A/secret-a)\" = from-a ]"
check "container-layer file survived" bash -c "[ \"\$(docker exec $A cat /home/bot/secret-a)\" = from-a ]"
check "Chromium profile dir survived" docker exec "$A" test -f "$BP/marker" -a -f "$BP/Local State"
check "stale Chromium SingletonLock cleared on start" bash -c "! docker exec $A test -L $BP/SingletonLock"
check "token still enforced after restart" test "$(api "$PORT_A" - /api/config)" = 401

echo "(e) Bot Desktop inside the container"
check "no missing Bot Desktop binaries" bash -c "[ \"\$(docker exec -e FACTR_CONFIG_HOME=/data/profiles/$BOT_A -e PYTHONPATH=/opt/factr/backend-python/source:/opt/factr/backend-python/packages $A /opt/factr/backend-python/runtime/bin/python3.12 -c 'from tools.bot_desktop import runtime; print(runtime.is_supported_host(), runtime.missing_binaries())')\" = 'True []' ]"
# display.start over the engine's own RPC socket, from inside the container (Host 127.0.0.1:P matches the listen port).
RPC='
import asyncio, json, sys, aiohttp
async def main(token, port):
    async with aiohttp.ClientSession() as s:
        async with s.ws_connect(f"ws://127.0.0.1:{port}/api/ws?token={token}") as ws:
            await ws.send_json({"jsonrpc": "2.0", "id": 7, "method": "display.start", "params": {}})
            while True:
                msg = await asyncio.wait_for(ws.receive(), 60)
                data = json.loads(msg.data) if msg.type == aiohttp.WSMsgType.TEXT else {}
                if data.get("id") == 7:
                    print(json.dumps(data)); return
asyncio.run(main(sys.argv[1], sys.argv[2]))
'
RPC_OUT="$(docker exec -i -e PYTHONPATH=/opt/factr/backend-python/source:/opt/factr/backend-python/packages "$A" \
  /opt/factr/backend-python/runtime/bin/python3.12 -c "$RPC" "$TOKEN_A" "$PORT_A" 2>&1 | tail -3)"
echo "  display.start -> $(echo "$RPC_OUT" | cut -c1-300)"
check "display.start answered with a result" grep -q '"result"' <<<"$RPC_OUT"
check "Xvnc process is up" docker exec "$A" pgrep -x Xvnc
check "RFB unix socket exists" docker exec "$A" test -S "/data/profiles/$BOT_A/bot-desktop/rfb.sock"
check "RFB socket is 0600" bash -c "[ \"\$(docker exec $A stat -c %a /data/profiles/$BOT_A/bot-desktop/rfb.sock)\" = 600 ]"
check "Xfce window manager is up" docker exec "$A" pgrep -x xfwm4
check "request_takeover marks the lease and the Screen sees it" bash -c "docker exec -e FACTR_CONFIG_HOME=/data/profiles/$BOT_A -e PYTHONPATH=/opt/factr/backend-python/source:/opt/factr/backend-python/packages $A /opt/factr/backend-python/runtime/bin/python3.12 -c \"
import json
from tools import bot_desktop_takeover_tool as t
from tools.bot_desktop import lease
print(json.loads(t.handle_request_takeover({'site':'example.com','why':'log in'}))['status'], lease.get().request)\" | grep -q 'requested example.com: log in'"

echo "(f) one real model turn through the engine and the socat forward (local Ollama, no cost)"
OLLAMA_MODEL="${OLLAMA_MODEL:-qwen3.8:27b}"
if [ -n "${SKIP_OLLAMA:-}" ] || ! curl -fsS -m 3 http://localhost:11434/api/tags 2>/dev/null | grep -q "\"$OLLAMA_MODEL\""; then
  echo "  skip: Ollama with $OLLAMA_MODEL is not running on the host (or SKIP_OLLAMA is set)"
else
  docker stop -t 10 "$B" >/dev/null
  mkdir -p "$SCRATCH/$BOT_C"; TOKEN_C="$(openssl rand -hex 32)"
  run_bot "$C" "$BOT_C" "$TOKEN_C" "$SCRATCH/$BOT_C" "$PORT_C" -e FACTR_PROVIDER=ollama -e FACTR_MODEL="$OLLAMA_MODEL" \
    -e FACTR_OLLAMA_NUM_CTX=4096 -e OLLAMA_HOST=127.0.0.1:11434
  check "C: engine up with the Ollama provider" bash -c "$(declare -f api wait_health); for i in 1 2 3 4 5 6; do wait_health $PORT_C && exit 0; done; exit 1"
  check "C: container loopback reaches host Ollama through socat" docker exec "$C" curl -fsS -m 5 http://127.0.0.1:11434/api/tags
  RESP="$(curl -s -m 600 -H "X-Factr-Session-Token: $TOKEN_C" -H 'content-type: application/json' \
    -d '{"prompt":"Reply with exactly one word: pong","timeout_s":540,"surface":"bot"}' "http://127.0.0.1:$PORT_C/api/agent/run")"
  echo "  /api/agent/run -> $(echo "$RESP" | cut -c1-300)"
  check "C: the model answered" grep -qi pong <<<"$RESP"
fi

echo
echo "passed $PASS, failed $FAIL"
[ "$FAIL" -eq 0 ] || { echo "failed:${FAILED//|/$'\n  '}"; docker logs --tail 40 "$A" 2>&1 | tail -20; exit 1; }
