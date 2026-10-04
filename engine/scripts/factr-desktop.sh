#!/usr/bin/env bash
# Run the Factr desktop on the Factr-I engine (`factr`).
#
# Starts the engine (factr + factr-learn REPL + local memory) with a fresh private
# token, then launches the unmodified Factr desktop app in remote-gateway mode
# pointed at it. Quitting the app stops the engine.
#
#   scripts/factr-desktop.sh
#
# Env:
#   FACTR_PROVIDER  model provider (default: ollama). After
#                       `factr login openai-codex` (ChatGPT/Codex) use `openai`.
#   FACTR_MODEL     model id (default: qwen3.8:27b for ollama)
#   FACTR_HOME      engine state dir (default: ~/.factr/engine)
#   FACTR_APP          path to Factr.app
#   FACTR_BIN       engine binary (default: target/release/factr)
set -euo pipefail

here="$(cd "$(dirname "$0")/.." && pwd)"
bin="${FACTR_BIN:-$here/target/release/factr}"
app="${FACTR_APP:-$here/../backend/apps/desktop/release/mac-arm64/Factr.app}"
provider="${FACTR_PROVIDER:-ollama}"
model="${FACTR_MODEL:-}"
if [[ -z "$model" && "$provider" == "ollama" ]]; then model="qwen3.8:27b"; fi
export FACTR_HOME="${FACTR_HOME:-$HOME/.factr/engine}"

[[ -x "$bin" ]] || { echo "engine binary not found: $bin (run: cargo build --release --bin factr)" >&2; exit 1; }
[[ -d "$app" ]] || { echo "Factr.app not found: $app (set FACTR_APP)" >&2; exit 1; }
mkdir -p "$FACTR_HOME"
chmod 700 "$FACTR_HOME"

token="$(openssl rand -hex 32)"
log="$FACTR_HOME/engine.log"
out="$(mktemp -t factr-ready)"
trap 'kill "$engine" 2>/dev/null || true; rm -f "$out"' EXIT

args=(--provider "$provider")
[[ -n "$model" ]] && args+=(--model "$model")
FACTR_DASHBOARD_SESSION_TOKEN="$token" "$bin" "${args[@]}" serve --host 127.0.0.1 --port 0 >"$out" 2>>"$log" &
engine=$!

port=""
for _ in $(seq 1 600); do
  port="$(sed -nE 's/^FACTR_BACKEND_READY port=([0-9]+).*/\1/p' "$out" | head -1)"
  [[ -n "$port" ]] && break
  kill -0 "$engine" 2>/dev/null || { echo "engine exited; see $log" >&2; exit 1; }
  sleep 0.1
done
[[ -n "$port" ]] || { echo "engine did not become ready; see $log" >&2; exit 1; }
echo "factr engine ready on 127.0.0.1:$port (provider: $provider${model:+, model: $model}); log: $log"

FACTR_DESKTOP_REMOTE_URL="http://127.0.0.1:$port" \
FACTR_DESKTOP_REMOTE_TOKEN="$token" \
  "$app/Contents/MacOS/Factr"
