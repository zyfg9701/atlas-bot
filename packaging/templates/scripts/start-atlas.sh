#!/usr/bin/env bash
# S1 one-click stack recipe: probe agent → call existing start-cli-stack.sh →
# healthz (backend=cli) → optional PC → print Connect next steps.
# Does NOT rewrite start-cli-stack; does NOT cargo run; not atlas-desktop-stack.
#
# Usage (from install / unpack / dist root):
#   ./scripts/start-atlas.sh
#   ./scripts/start-atlas.sh --skip-pc
#   ATLAS_SKIP_PC=1 ./scripts/start-atlas.sh
#   ATLAS_AGENT_CLI=/path/to/agent ./scripts/start-atlas.sh
#
# Env:
#   ATLAS_AGENT_CLI          default: agent (PATH)
#   ATLAS_SKIP_PC=1|true     skip launching pc/atlas-bot-pc
#   ATLAS_GATEWAY_HTTP_BIND  default: 127.0.0.1:8787  (must match start-cli-stack)
#   ATLAS_HUB_BIND           default: 127.0.0.1:7700
#   ATLAS_GATEWAY_BACKEND    default: cli
#
# See docs/packaging-skeleton.md §S1 and dist/README-INSTALL.md
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN_DIR="${ATLAS_BOT_BIN_DIR:-$ROOT/bin}"
START_SH="$ROOT/scripts/start-cli-stack.sh"
PC_BIN="$ROOT/pc/atlas-bot-pc"
CLI="${ATLAS_AGENT_CLI:-agent}"
GW_BIND="${ATLAS_GATEWAY_HTTP_BIND:-127.0.0.1:8787}"
HUB_BIND="${ATLAS_HUB_BIND:-127.0.0.1:7700}"
BACKEND="${ATLAS_GATEWAY_BACKEND:-cli}"
PID_DIR="${ATLAS_CLI_STACK_PID_DIR:-$ROOT/.cli-stack-pids}"
LOG_DIR="${ATLAS_CLI_STACK_LOG_DIR:-$ROOT/.cli-stack-logs}"
SKIP_PC=0
STACK_PID=""

usage() {
  cat <<'EOH'
Usage: start-atlas.sh [--skip-pc] [--help]

S1 one-click recipe (install/unpack root):
  1. Probe ATLAS_AGENT_CLI or PATH "agent"
  2. Call existing scripts/start-cli-stack.sh (binaries only — no cargo run)
  3. Wait for gateway + Hub healthz with backend=cli
  4. Launch pc/atlas-bot-pc (unless --skip-pc / ATLAS_SKIP_PC)
  5. Print Connect next steps (ws://…/ws)

Flags / env:
  --skip-pc / ATLAS_SKIP_PC=1   stack only (CI / headless)
  ATLAS_AGENT_CLI               path or name of agent CLI (required on machine)

Not this knife: signing, updater, store, C1-real, GA2/YOLO,
  atlas-desktop-stack (VNC/D1). Still need a local agent; one-click ≠ bundling it.
EOH
}

for arg in "$@"; do
  case "$arg" in
    --skip-pc|-SkipPc) SKIP_PC=1 ;;
    -h|--help) usage; exit 0 ;;
    *)
      echo "error: unknown arg: $arg (try --help)" >&2
      exit 2
      ;;
  esac
done

# env truthy
case "${ATLAS_SKIP_PC:-}" in
  1|true|TRUE|yes|YES) SKIP_PC=1 ;;
esac

die() {
  echo "error: $*" >&2
  exit 1
}

resolve_cli() {
  local c="$1"
  if [[ "$c" == */* || "$c" == ./* || "$c" == ../* ]]; then
    if [[ -x "$c" ]]; then
      if command -v realpath >/dev/null 2>&1; then realpath "$c"; else
        (cd "$(dirname "$c")" && echo "$(pwd)/$(basename "$c")")
      fi
      return 0
    fi
    if [[ -f "$c" ]]; then
      chmod +x "$c" 2>/dev/null || true
      if [[ -x "$c" ]]; then
        if command -v realpath >/dev/null 2>&1; then realpath "$c"; else
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

healthz_ok() {
  local bind="$1"
  curl -sf "http://${bind}/healthz" >/dev/null 2>&1
}

gateway_backend_is_cli() {
  local body
  body="$(curl -sf "http://${GW_BIND}/healthz" 2>/dev/null || true)"
  [[ -n "$body" ]] || return 1
  # JSON {"ok":true,"backend":"cli",…} or plain — require backend=cli when present
  if [[ "$body" == *'"backend"'* ]]; then
    [[ "$body" == *'"backend":"cli"'* || "$body" == *'"backend": "cli"'* ]]
  else
    # legacy plain "ok" — still require we set BACKEND=cli when starting
    return 0
  fi
}

echo "==> atlas-bot S1 one-click (stack+PC recipe)"
echo "    ROOT=$ROOT"
echo "    (not atlas-desktop-stack / not signing / not updater / not store)"
echo

# --- missing layout ---
if [[ ! -f "$START_SH" ]]; then
  cat >&2 <<EOM
error: missing start-cli-stack script: $START_SH

This recipe must call the existing start-cli-stack (not reinvent it).
Unpack/install an atlas-bot dist tree first, or run scripts/pack-dist.sh.
See README-INSTALL.md and docs/cli-primary-runbook.md
EOM
  exit 1
fi
if [[ ! -x "$START_SH" ]]; then
  chmod +x "$START_SH" 2>/dev/null || true
fi
if [[ ! -x "$BIN_DIR/atlas-bot-gateway" && ! -f "$BIN_DIR/atlas-bot-gateway" ]]; then
  cat >&2 <<EOM
error: missing binary: $BIN_DIR/atlas-bot-gateway

Run pack-dist / install-local, or unpack the archive so bin/ exists.
See README-INSTALL.md
EOM
  exit 1
fi
if [[ ! -x "$BIN_DIR/atlas-bot-hub" && ! -f "$BIN_DIR/atlas-bot-hub" ]]; then
  die "missing binary: $BIN_DIR/atlas-bot-hub"
fi

# --- agent probe (human fail before starting) ---
if ! CLI_RESOLVED="$(resolve_cli "$CLI")"; then
  cat >&2 <<EOM
error: ATLAS_AGENT_CLI not found: ${CLI}

Install and login the Cursor/Atlas agent CLI (default name: agent), then re-run.
One-click does NOT bundle the agent runtime.

  export ATLAS_AGENT_CLI=/path/to/agent
  $0 --skip-pc   # CI / no display

See docs/cli-primary-runbook.md and README-INSTALL.md
EOM
  exit 1
fi
export ATLAS_AGENT_CLI="$CLI_RESOLVED"
export ATLAS_GATEWAY_BACKEND="$BACKEND"
export ATLAS_GATEWAY_HTTP_BIND="$GW_BIND"
export ATLAS_HUB_BIND="$HUB_BIND"
echo "    ATLAS_AGENT_CLI=$ATLAS_AGENT_CLI"
echo "    SkipPc=$SKIP_PC"

# --- already healthy? skip re-launch ---
STACK_STARTED=0
if healthz_ok "$GW_BIND" && healthz_ok "$HUB_BIND" && gateway_backend_is_cli; then
  echo "==> stack already healthy (gateway+Hub, backend=cli); reusing"
else
  echo "==> starting stack via existing start-cli-stack.sh (background)…"
  mkdir -p "$PID_DIR" "$LOG_DIR"
  RECIPE_STACK_LOG="$LOG_DIR/start-atlas-stack.log"
  # Call existing script — do not copy its start logic. Background so we can launch PC.
  # start-cli-stack holds foreground until Ctrl-C; we keep its process alive.
  nohup bash "$START_SH" >"$RECIPE_STACK_LOG" 2>&1 &
  STACK_PID=$!
  STACK_STARTED=1
  echo "    start-cli-stack pid=$STACK_PID  log=$RECIPE_STACK_LOG"

  echo -n "    waiting for gateway /healthz (backend=cli)"
  ok=0
  for i in $(seq 1 90); do
    if ! kill -0 "$STACK_PID" 2>/dev/null; then
      echo
      echo "error: start-cli-stack exited early; last log:" >&2
      tail -n 50 "$RECIPE_STACK_LOG" >&2 || true
      echo "Hint: missing agent/bin usually prints above. See README-INSTALL.md" >&2
      exit 1
    fi
    if healthz_ok "$GW_BIND"; then
      echo " ok"
      ok=1
      break
    fi
    echo -n "."
    sleep 0.5
  done
  if [[ "$ok" -ne 1 ]]; then
    echo
    die "gateway healthz timeout; log: $RECIPE_STACK_LOG (also $LOG_DIR/gateway.log)"
  fi

  if ! gateway_backend_is_cli; then
    body="$(curl -sf "http://${GW_BIND}/healthz" 2>/dev/null || echo '(empty)')"
    echo "error: gateway healthz backend is not cli: $body" >&2
    echo "Expected ATLAS_GATEWAY_BACKEND=cli. Refusing to pretend the stack is ready." >&2
    exit 1
  fi
  echo "    gateway healthz backend=cli"

  echo -n "    waiting for Hub /healthz"
  ok=0
  for i in $(seq 1 90); do
    if ! kill -0 "$STACK_PID" 2>/dev/null; then
      echo
      echo "error: start-cli-stack exited early while waiting for Hub; last log:" >&2
      tail -n 50 "$RECIPE_STACK_LOG" >&2 || true
      exit 1
    fi
    if healthz_ok "$HUB_BIND"; then
      echo " ok"
      ok=1
      break
    fi
    echo -n "."
    sleep 0.5
  done
  if [[ "$ok" -ne 1 ]]; then
    echo
    die "Hub healthz timeout; log: $RECIPE_STACK_LOG (also $LOG_DIR/hub.log)"
  fi
fi

# --- optional PC ---
if [[ "$SKIP_PC" -eq 1 ]]; then
  echo "==> SkipPc: not launching PC"
else
  if [[ ! -x "$PC_BIN" && -f "$PC_BIN" ]]; then
    chmod +x "$PC_BIN" 2>/dev/null || true
  fi
  if [[ ! -f "$PC_BIN" ]]; then
    echo "warning: PC binary missing: $PC_BIN" >&2
    echo "         Stack is up; open PC when available, or re-run pack without --skip-pc." >&2
    echo "         Tip: $0 --skip-pc to suppress this warning in CI." >&2
  else
    echo "==> launching PC: $PC_BIN"
    if ! "$PC_BIN" >/dev/null 2>&1 & then
      echo "error: failed to launch PC: $PC_BIN" >&2
      echo "Stack is still running. Diagnose display/WebKit, or use --skip-pc." >&2
      exit 1
    fi
    echo "    PC launch requested (pid=$!)"
  fi
fi

echo
echo "Ready (S1 one-click)."
echo "  Connect:  ws://${HUB_BIND}/ws"
echo "  Gateway:  http://${GW_BIND}/healthz   (expect backend=cli)"
echo "  Hub:      http://${HUB_BIND}/healthz"
echo "  Next:     PC → Connect → select agent → Send"
echo "  Logs:     $LOG_DIR/gateway.log  $LOG_DIR/hub.log"
if [[ -n "$STACK_PID" ]]; then
  echo "  Stack pid: $STACK_PID (from start-cli-stack; Ctrl-C that job or kill PIDs under $PID_DIR)"
fi
echo "  Stop:     kill \$(cat $PID_DIR/gateway.pid) \$(cat $PID_DIR/hub.pid) 2>/dev/null"
echo
echo "Still need local agent. Unsigned / yellow OK. Not store / not updater / not atlas-desktop-stack."
exit 0
