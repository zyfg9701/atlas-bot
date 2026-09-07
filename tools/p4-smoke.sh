#!/usr/bin/env bash
# P4 smoke: Android HubClient closed-loop (JVM MockWebServer) + optional live Hub+gateway mock CLI.
#
# Usage (from repo root):
#   ./tools/p4-smoke.sh              # Android JVM smoke only (CI-friendly)
#   ./tools/p4-smoke.sh --with-hub   # also start gateway+Hub with mock CLI and run WS closed-loop
#
# Expect a line:
#   SMOKE_OK p4 android mock-ws non-echo+hello+cold+subscribe+sendPrompt+turn_finished+transcriptTail
# and with --with-hub:
#   SMOKE_OK p4 ws-hub gateway-mock non-echo+hello+cold+subscribe+sendPrompt+turn_finished+transcriptTail
#
# Emulator hand-check (same Hub stack as --with-hub):
#   point app at ws://10.0.2.2:7700/ws → Connect → cold → subscribe → sendPrompt → transcriptTail
#   confirm reply contains "atlas-mock-reply" (not "echo: …")
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

WITH_HUB=0
for arg in "$@"; do
  case "$arg" in
    --with-hub) WITH_HUB=1 ;;
    -h|--help)
      sed -n '2,20p' "$0"
      exit 0
      ;;
  esac
done

echo "==> P4 Android JVM closed-loop (MockWebServer non-echo)"
ANDROID_LOG="$(mktemp -t p4-android-smoke.XXXXXX.log)"
(
  cd clients/android
  # Ensure symlink for local/CI trees that skip checkout LFS/symlinks
  if [[ ! -e generated-kotlin/BotRelay.kt ]]; then
    ln -sfn ../../third_party/xai-tool-protocol/generated/kotlin generated-kotlin
  fi
  # --rerun-tasks so SMOKE_OK always appears in stdout (not skipped as UP-TO-DATE)
  ./gradlew :app:testDebugUnitTest \
    --tests 'com.atlasbot.client.P4ClosedLoopSmokeTest' \
    --rerun-tasks --no-daemon --console=plain
) 2>&1 | tee "$ANDROID_LOG"

XML_REPORT="clients/android/app/build/test-results/testDebugUnitTest"
if ! grep -q 'SMOKE_OK p4 android mock-ws' "$ANDROID_LOG" \
  && ! grep -Rqs 'SMOKE_OK p4 android mock-ws' "$XML_REPORT" 2>/dev/null; then
  echo "FAIL: missing SMOKE_OK p4 android line in gradle/XML output" >&2
  exit 1
fi
# Canonical evidence line (also present in P4ClosedLoopSmokeTest stdout / XML)
echo "SMOKE_OK p4 android mock-ws non-echo+hello+cold+subscribe+sendPrompt+turn_finished+transcriptTail"

if [[ "$WITH_HUB" -eq 0 ]]; then
  echo "OK — Android mock-ws smoke passed. Re-run with --with-hub for live gateway non-echo."
  echo "Emulator: start Hub+gateway (see docs/P4-runbook.md §Evidence), URL ws://10.0.2.2:7700/ws"
  exit 0
fi

echo "==> Live Hub + P3.5 gateway (mock CLI) WS closed-loop"
MOCK_CLI="$ROOT/tools/mock-cli/mock-atlas-agent-cli.sh"
chmod +x "$MOCK_CLI"

GW_LOG="$(mktemp -t p4-gw.XXXXXX.log)"
HUB_LOG="$(mktemp -t p4-hub.XXXXXX.log)"
cleanup() {
  [[ -n "${GW_PID:-}" ]] && kill "$GW_PID" 2>/dev/null || true
  [[ -n "${HUB_PID:-}" ]] && kill "$HUB_PID" 2>/dev/null || true
}
trap cleanup EXIT

ATLAS_GATEWAY_BACKEND=cli \
ATLAS_AGENT_CLI="$MOCK_CLI" \
ATLAS_GATEWAY_HTTP_BIND=127.0.0.1:8787 \
RUST_LOG=info \
  cargo run -p atlas-bot-gateway --quiet >"$GW_LOG" 2>&1 &
GW_PID=$!

# wait for gateway health
for i in $(seq 1 60); do
  if curl -sf http://127.0.0.1:8787/healthz >/dev/null 2>&1; then
    break
  fi
  if ! kill -0 "$GW_PID" 2>/dev/null; then
    echo "FAIL: gateway exited early" >&2
    tail -50 "$GW_LOG" >&2 || true
    exit 1
  fi
  sleep 0.5
done
curl -sf http://127.0.0.1:8787/healthz >/dev/null

ATLAS_GATEWAY_URL=http://127.0.0.1:8787 \
ATLAS_HUB_BIND=127.0.0.1:7700 \
RUST_LOG=info \
  cargo run -p atlas-bot-hub --quiet >"$HUB_LOG" 2>&1 &
HUB_PID=$!

for i in $(seq 1 60); do
  if python3 -c "import socket; s=socket.create_connection(('127.0.0.1',7700),1); s.close()" 2>/dev/null; then
    break
  fi
  if ! kill -0 "$HUB_PID" 2>/dev/null; then
    echo "FAIL: hub exited early" >&2
    tail -50 "$HUB_LOG" >&2 || true
    exit 1
  fi
  sleep 0.5
done

python3 "$ROOT/tools/p4-ws-closed-loop.py" --url ws://127.0.0.1:7700/ws --prompt "hello-p4-hub-smoke"

echo "OK — live Hub+gateway non-echo proven (same stack as emulator hand-check)."
echo "Emulator hand-check: leave Hub/gateway running, app URL ws://10.0.2.2:7700/ws, confirm atlas-mock-reply."
