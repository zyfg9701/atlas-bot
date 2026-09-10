#!/usr/bin/env bash
# Dist start skeleton: gateway(backend=cli) + Hub from dist/bin binaries (NOT cargo run).
# Contract parity with scripts/dev-cli-stack.sh — see docs/packaging-skeleton.md §start.
#
# Usage (from a packed or installed tree):
#   ./scripts/start-cli-stack.sh
#   ATLAS_AGENT_CLI=/path/to/agent ./scripts/start-cli-stack.sh
#   ATLAS_AGENT_CLI_STREAM=1 ATLAS_HUB_EVENT_URL=http://127.0.0.1:7701/internal/runtime-hint \
#     ATLAS_HUB_EVENT_ALLOW_INSECURE_LOOPBACK=1 ./scripts/start-cli-stack.sh
#
# Env (same names as dev-cli-stack):
#   ATLAS_AGENT_CLI          default: agent
#   ATLAS_GATEWAY_HTTP_BIND  default: 127.0.0.1:8787
#   ATLAS_HUB_BIND           default: 127.0.0.1:7700
#   ATLAS_GATEWAY_BACKEND    default: cli
#   ATLAS_AGENT_CLI_STREAM / ATLAS_HUB_EVENT_URL / ATLAS_HUB_EVENT_* / MOCK_CLI_*  (CS1; Unix explicit)
#
# Layout expected:
#   <root>/bin/atlas-bot-gateway
#   <root>/bin/atlas-bot-hub
#   <root>/scripts/start-cli-stack.sh   (this file)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN_DIR="${ATLAS_BOT_BIN_DIR:-$ROOT/bin}"
GW_BIN="$BIN_DIR/atlas-bot-gateway"
HUB_BIN="$BIN_DIR/atlas-bot-hub"

CLI="${ATLAS_AGENT_CLI:-agent}"
GW_BIND="${ATLAS_GATEWAY_HTTP_BIND:-127.0.0.1:8787}"
HUB_BIND="${ATLAS_HUB_BIND:-127.0.0.1:7700}"
BACKEND="${ATLAS_GATEWAY_BACKEND:-cli}"
PID_DIR="${ATLAS_CLI_STACK_PID_DIR:-$ROOT/.cli-stack-pids}"
LOG_DIR="${ATLAS_CLI_STACK_LOG_DIR:-$ROOT/.cli-stack-logs}"

if [[ ! -x "$GW_BIN" ]]; then
  echo "error: missing executable: $GW_BIN" >&2
  echo "Run scripts/pack-dist.sh (or install-local) first. See dist/README-INSTALL.md" >&2
  exit 1
fi
if [[ ! -x "$HUB_BIN" ]]; then
  echo "error: missing executable: $HUB_BIN" >&2
  exit 1
fi

resolve_cli() {
  local c="$1"
  if [[ "$c" == */* || "$c" == ./* || "$c" == ../* ]]; then
    if [[ -x "$c" ]]; then
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
  if command -v "$c" >/dev/null 2>&1; then
    command -v "$c"
    return 0
  fi
  return 1
}

if ! CLI_RESOLVED="$(resolve_cli "$CLI")"; then
  cat >&2 <<EOM
error: ATLAS_AGENT_CLI not found: ${CLI}

Install and login the Cursor/Atlas agent CLI (default binary name: agent), then re-run.
This script will NOT silently fall back to the Hub InMemory stub.

Mock bypass (dev tree only; not shipped in default dist):
  ATLAS_AGENT_CLI=<repo>/tools/mock-cli/mock-atlas-agent-cli.sh $0

See docs/cli-primary-runbook.md and dist/README-INSTALL.md
EOM
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

echo "==> CLI primary stack (dist binaries)"
echo "    ROOT=$ROOT"
echo "    gateway: $GW_BIN"
echo "    hub:     $HUB_BIN"
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

echo "==> starting atlas-bot-gateway (backend=$BACKEND) …"
"$GW_BIN" >"$GW_LOG" 2>&1 &
echo $! >"$GW_PID_FILE"

echo -n "    waiting for gateway /healthz"
for i in $(seq 1 60); do
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

if command -v curl >/dev/null 2>&1; then
  echo -n "    healthz: "
  curl -sf "http://${GW_BIND}/healthz" || true
  echo
fi

echo "==> starting atlas-bot-hub (ATLAS_GATEWAY_URL=$ATLAS_GATEWAY_URL) …"
ATLAS_GATEWAY_HTTP_BIND=off \
  "$HUB_BIN" >"$HUB_LOG" 2>&1 &
echo $! >"$HUB_PID_FILE"

echo -n "    waiting for Hub /healthz"
for i in $(seq 1 60); do
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
echo "Stack ready (dist)."
echo "  curl -s http://${GW_BIND}/healthz"
echo "  curl -s http://${GW_BIND}/stats"
echo "  PC: open $ROOT/pc/ (or installed pc/) → Connect → ws://${HUB_BIND}/ws → select agent → Send"
echo "  Logs: $GW_LOG  $HUB_LOG"
echo "  Stop: Ctrl-C (trap kills both) or kill \$(cat $GW_PID_FILE) \$(cat $HUB_PID_FILE)"
echo
echo "Foreground hold (Ctrl-C to stop)…"
while kill -0 "$(cat "$GW_PID_FILE")" 2>/dev/null \
   && kill -0 "$(cat "$HUB_PID_FILE")" 2>/dev/null; do
  sleep 2
done
echo "a child exited; shutting down" >&2
exit 1
