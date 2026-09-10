#!/usr/bin/env bash
# Mock Cursor/Atlas Agent CLI for CI / P3.5 / CS1 smoke (no real Cursor login).
# Compatible surface: accepts -p/--print and a prompt; prints a NON-ECHO reply.
# MOCK_CLI_STREAM=1 → NDJSON assistant delta(s) then result (atlas-mock-reply…).
set -euo pipefail

prompt=""
sleep_ms="${MOCK_CLI_SLEEP_MS:-0}"
agent_id="${ATLAS_AGENT_ID:-unknown}"
stream="${MOCK_CLI_STREAM:-0}"

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
    --stream-partial-output)
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

sleep_ms_ok=0
if [[ "$sleep_ms" =~ ^[0-9]+$ ]] && [[ "$sleep_ms" -gt 0 ]]; then
  sleep_ms_ok=1
fi

do_sleep() {
  if [[ "$sleep_ms_ok" -eq 1 ]]; then
    python3 - "$sleep_ms" <<'PY' 2>/dev/null || sleep $(( (sleep_ms + 999) / 1000 ))
import sys, time
time.sleep(int(sys.argv[1]) / 1000.0)
PY
  fi
}

# Non-echo, agent-scoped reply so dual-agent smoke can detect crosstalk.
plen=${#prompt}
final="atlas-mock-reply agent=${agent_id} chars=${plen} hash=$(printf '%s' "$prompt" | cksum | awk '{print $1}')"

if [[ "$stream" == "1" || "$stream" == "true" || "$stream" == "yes" ]]; then
  # Emit ≥1 mid-turn delta BEFORE sleep so gateway can fan-out while process still live.
  printf '%s\n' '{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Hello"}]},"timestamp_ms":1}'
  printf '%s\n' '{"type":"tool_call","status":"started","name":"mock_tool","summary":"mock"}'
  do_sleep
  final_json=$(python3 -c 'import json,sys; print(json.dumps(sys.argv[1]))' "$final")
  printf '{"type":"result","subtype":"success","is_error":false,"result":%s,"duration_ms":10,"session_id":"mock"}\n' "$final_json"
else
  do_sleep
  echo "$final"
fi
