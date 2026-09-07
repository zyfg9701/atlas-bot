#!/usr/bin/env bash
# Mock Cursor/Atlas Agent CLI for CI / P3.5 smoke (no real Cursor login).
# Compatible surface: accepts -p/--print and a prompt; prints a NON-ECHO reply.
set -euo pipefail

prompt=""
sleep_ms="${MOCK_CLI_SLEEP_MS:-0}"
agent_id="${ATLAS_AGENT_ID:-unknown}"

while [[ $# -gt 0 ]]; do
  case "$1" in
    -p|--print)
      shift
      ;;
    --output-format)
      shift
      [[ $# -gt 0 ]] && shift || true
      ;;
    --output-format=*)
      shift
      ;;
    -*)
      # ignore unknown flags for forward-compat
      shift
      ;;
    *)
      if [[ -z "$prompt" ]]; then
        prompt="$1"
      else
        prompt="$prompt $1"
      fi
      shift
      ;;
  esac
done

if [[ "$sleep_ms" =~ ^[0-9]+$ ]] && [[ "$sleep_ms" -gt 0 ]]; then
  # portable sleep fractional seconds
  python3 - "$sleep_ms" <<'PY' 2>/dev/null || sleep $(( (sleep_ms + 999) / 1000 ))
import sys, time
time.sleep(int(sys.argv[1]) / 1000.0)
PY
fi

# Non-echo, agent-scoped reply so dual-agent smoke can detect crosstalk.
plen=${#prompt}
echo "atlas-mock-reply agent=${agent_id} chars=${plen} hash=$(printf '%s' "$prompt" | cksum | awk '{print $1}')"
