#!/usr/bin/env bash
set -euo pipefail
repo=$(cd "$(dirname "$0")/../.." && pwd)
scratch=$(mktemp -d "${TMPDIR:-/tmp}/bots-linux-check-XXXXXX")
name="bots-linux-sandbox-$(basename "$scratch" | tr '[:upper:]' '[:lower:]')"
cleanup() {
  case "$name" in bots-*) docker rm -f "$name" >/dev/null 2>&1 || true ;; esac
  rm -rf "$scratch"
}
trap cleanup EXIT
docker info >/dev/null
"$repo/desktop/node_modules/.bin/esbuild" "$repo/desktop/app/electron/bot-local-sandbox.ts" \
  --bundle --platform=node --format=esm --target=node18 --outfile="$scratch/launcher.mjs"
cp "$repo/computer/tests/test_local_sandbox_linux.mjs" "$scratch/check.mjs"
chmod 755 "$scratch"
docker run --rm --name "$name" --user 0 --entrypoint /bin/sh --security-opt seccomp=unconfined --security-opt apparmor=unconfined --security-opt systempaths=unconfined \
  --mount "type=bind,source=$scratch,target=/checks,readonly" \
  -e HOME=/tmp/bots-home -e FACTR_HOME=/tmp/bots-engine -e FACTR_CONFIG_HOME=/tmp/bots-config \
  -e FACTR_ELECTRON_USER_DATA_DIR=/tmp/bots-electron bots-factr-bot-computer:dev \
  -c 'apt-get update -qq && apt-get install -y -qq --no-install-recommends bubblewrap nodejs >/tmp/bots-packages.log && runuser -u bot -- node /checks/check.mjs'
