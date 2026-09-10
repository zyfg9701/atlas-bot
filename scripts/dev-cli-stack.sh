#!/usr/bin/env bash
# Start the recommended CLI primary stack: gateway(backend=cli) + Hub(GATEWAY_URL).
# Unix first. See docs/cli-primary-runbook.md for Windows / manual steps.
#
# Usage:
#   ./scripts/dev-cli-stack.sh                 # requires real `agent` on PATH
#   ATLAS_AGENT_CLI=tools/mock-cli/mock-atlas-agent-cli.sh ./scripts/dev-cli-stack.sh
#   # Streaming (explicit env; Win has -Stream — see docs / windows-cli-stack-checklist):
#   ATLAS_AGENT_CLI_STREAM=1 \
#   ATLAS_HUB_EVENT_URL=http://127.0.0.1:7701/internal/runtime-hint \
#   ATLAS_HUB_EVENT_ALLOW_INSECURE_LOOPBACK=1 \
#   MOCK_CLI_STREAM=1 \
#   ATLAS_AGENT_CLI=tools/mock-cli/mock-atlas-agent-cli.sh ./scripts/dev-cli-stack.sh
#
# Env overrides:
#   ATLAS_AGENT_CLI          default: agent
#   ATLAS_GATEWAY_HTTP_BIND  default: 127.0.0.1:8787
#   ATLAS_HUB_BIND           default: 127.0.0.1:7700
#   ATLAS_GATEWAY_BACKEND    default: cli (forced to cli by this script unless set)
#   ATLAS_AGENT_CLI_STREAM   CS1 opt-in (1/true/yes); unset = text
#   ATLAS_HUB_EVENT_URL      CS1b B1 POST (e.g. http://127.0.0.1:7701/internal/runtime-hint)
#   ATLAS_HUB_EVENT_TOKEN / ATLAS_HUB_EVENT_ALLOW_INSECURE_LOOPBACK  B1 Hub ingest auth
#   MOCK_CLI_STREAM / MOCK_CLI_SLEEP_MS  mock NDJSON deltas (inherited into gateway)
#
# This script does not default STREAM (Unix stays explicit opt-in). Win ps1 -Stream
# fills unset STREAM/EVENT_URL/MOCK — see scripts/dev-cli-stack.ps1. Multi-process
# mid-turn = B1 only; do NOT also open B2 spawn_turn_bridge.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

CLI="${ATLAS_AGENT_CLI:-agent}"
GW_BIND="${ATLAS_GATEWAY_HTTP_BIND:-127.0.0.1:8787}"
HUB_BIND="${ATLAS_HUB_BIND:-127.0.0.1:7700}"
BACKEND="${ATLAS_GATEWAY_BACKEND:-cli}"
PID_DIR="${ATLAS_CLI_STACK_PID_DIR:-$ROOT/.cli-stack-pids}"
LOG_DIR="${ATLAS_CLI_STACK_LOG_DIR:-$ROOT/.cli-stack-logs}"

resolve_cli() {
  local c="$1"
  # Absolute or relative path with a slash → must be executable file
  if [[ "$c" == */* || "$c" == ./* || "$c" == ../* ]]; then
    if [[ -x "$c" ]]; then
      # Prefer absolute for gateway spawn
      if command -v realpath >/dev/null 2>&1; then
        realpath "$c"
      else
        (cd "$(dirname "$c")" && echo "$(pwd)/$(basename "$c")")
      fi
      return 0
    fi
    if [[ -f "$c" ]]; then
      chmod +x "$c" 2>/dev/null || true
      if [[ -x "$c" ]]; then
        if command -v realpath >/dev/null 2>&1; then
          realpath "$c"
        else
          (cd "$(dirname "$c")" && echo "$(pwd)/$(basename "$c")")
        fi
        return 0
      fi
    fi
    return 1
  fi
  # Bare name → PATH
  if command -v "$c" >/dev/null 2>&1; then
    command -v "$c"
    return 0
  fi
  return 1
}

if ! CLI_RESOLVED="$(resolve_cli "$CLI")"; then
  cat >&2 <<EOF
error: ATLAS_AGENT_CLI not found: ${CLI}

Install and login the Cursor/Atlas agent CLI (default binary name: agent), then re-run.
This script will NOT silently fall back to the Hub InMemory stub.

For CI / local without a real agent, override with the mock CLI:
  ATLAS_AGENT_CLI=${ROOT}/tools/mock-cli/mock-atlas-agent-cli.sh $0

See docs/cli-primary-runbook.md
EOF
  exit 1
fi

export ATLAS_AGENT_CLI="$CLI_RESOLVED"
export ATLAS_GATEWAY_BACKEND="$BACKEND"
export ATLAS_GATEWAY_HTTP_BIND="$GW_BIND"
export ATLAS_HUB_BIND="$HUB_BIND"
export ATLAS_GATEWAY_URL="http://${GW_BIND}"

mkdir -p "$PID_DIR" "$LOG_DIR"
GW_PID_FILE="$PID_DIR/gateway.pid"
HUB_PID_FILE="$PID_DIR/hub.pid"
GW_LOG="$LOG_DIR/gateway.log"
HUB_LOG="$LOG_DIR/hub.log"

cleanup() {
  local code=$?
  if [[ -f "$GW_PID_FILE" ]]; then
    kill "$(cat "$GW_PID_FILE")" 2>/dev/null || true
    rm -f "$GW_PID_FILE"
  fi
  if [[ -f "$HUB_PID_FILE" ]]; then
    kill "$(cat "$HUB_PID_FILE")" 2>/dev/null || true
    rm -f "$HUB_PID_FILE"
  fi
  exit "$code"
}
trap cleanup EXIT INT TERM

echo "==> CLI primary stack"
echo "    ATLAS_AGENT_CLI=$ATLAS_AGENT_CLI"
echo "    ATLAS_GATEWAY_BACKEND=$ATLAS_GATEWAY_BACKEND"
echo "    gateway bind: $GW_BIND"
echo "    Hub WS:       ws://${HUB_BIND}/ws"
echo "    GATEWAY_URL:  $ATLAS_GATEWAY_URL"
echo "    ATLAS_AGENT_CLI_STREAM=${ATLAS_AGENT_CLI_STREAM:-(unset=text)}"
echo "    ATLAS_HUB_EVENT_URL=${ATLAS_HUB_EVENT_URL:-(unset)}"
if [[ -n "${ATLAS_HUB_EVENT_TOKEN:-}" ]]; then
  echo "    ATLAS_HUB_EVENT_TOKEN=(set)"
fi
if [[ -n "${ATLAS_HUB_EVENT_ALLOW_INSECURE_LOOPBACK:-}" ]]; then
  echo "    ATLAS_HUB_EVENT_ALLOW_INSECURE_LOOPBACK=$ATLAS_HUB_EVENT_ALLOW_INSECURE_LOOPBACK"
fi
echo "    MOCK_CLI_STREAM=${MOCK_CLI_STREAM:-(unset)}"
echo "    MOCK_CLI_SLEEP_MS=${MOCK_CLI_SLEEP_MS:-(unset)}"
echo

# Ensure release/debug binaries exist (cargo run is fine)
echo "==> starting atlas-bot-gateway (backend=$BACKEND) …"
cargo run -p atlas-bot-gateway --quiet >"$GW_LOG" 2>&1 &
echo $! >"$GW_PID_FILE"

# Wait for healthz
echo -n "    waiting for gateway /healthz"
for i in $(seq 1 180); do
  if curl -sf "http://${GW_BIND}/healthz" >/dev/null 2>&1; then
    echo " ok"
    break
  fi
  if ! kill -0 "$(cat "$GW_PID_FILE")" 2>/dev/null; then
    echo
    echo "error: gateway exited early; last log:" >&2
    tail -n 40 "$GW_LOG" >&2 || true
    exit 1
  fi
  echo -n "."
  sleep 0.5
  if [[ "$i" -eq 60 ]]; then
    echo
    echo "error: gateway healthz timeout; log: $GW_LOG" >&2
    tail -n 40 "$GW_LOG" >&2 || true
    exit 1
  fi
done

echo "==> starting atlas-bot-hub (ATLAS_GATEWAY_URL=$ATLAS_GATEWAY_URL) …"
# Disable embedded gateway HTTP on Hub (remote gateway owns :8787)
ATLAS_GATEWAY_HTTP_BIND=off \
  cargo run -p atlas-bot-hub --quiet >"$HUB_LOG" 2>&1 &
echo $! >"$HUB_PID_FILE"

echo -n "    waiting for Hub /healthz"
for i in $(seq 1 180); do
  if curl -sf "http://${HUB_BIND}/healthz" >/dev/null 2>&1; then
    echo " ok"
    break
  fi
  if ! kill -0 "$(cat "$HUB_PID_FILE")" 2>/dev/null; then
    echo
    echo "error: Hub exited early; last log:" >&2
    tail -n 40 "$HUB_LOG" >&2 || true
    exit 1
  fi
  echo -n "."
  sleep 0.5
  if [[ "$i" -eq 60 ]]; then
    echo
    echo "error: Hub healthz timeout; log: $HUB_LOG" >&2
    tail -n 40 "$HUB_LOG" >&2 || true
    exit 1
  fi
done

echo
echo "Stack ready."
echo "  curl -s http://${GW_BIND}/healthz"
echo "  curl -s http://${GW_BIND}/stats"
echo "  PC: Connect → ws://${HUB_BIND}/ws → select agent → Send"
echo "  Logs: $GW_LOG  $HUB_LOG"
echo "  Stop: Ctrl-C (trap kills both) or kill \$(cat $GW_PID_FILE) \$(cat $HUB_PID_FILE)"
echo
echo "Foreground hold (Ctrl-C to stop)…"
# Keep script alive so trap cleanup works; also re-check children
while kill -0 "$(cat "$GW_PID_FILE")" 2>/dev/null \
   && kill -0 "$(cat "$HUB_PID_FILE")" 2>/dev/null; do
  sleep 2
done
echo "a child exited; shutting down" >&2
exit 1
