#!/usr/bin/env bash
# RL1: print which Box LLM live env vars look set / whether base looks OpenAI-compat /
# optional /healthz llm_configured check.
# Does NOT call chat/completions. Does NOT print API keys or full Authorization headers.
# Optional: source a private env file first, e.g.
#   set -a; source ~/.config/atlas-bot/probe-box-llm-live.env; set +a
#   ./scripts/probe-box-llm-live.sh
# Exit 0 always (advisory). Not a CI gate.

set -euo pipefail

echo "=== probe-box-llm-live (advisory · no chat · no secrets printed) ==="

present() {
  local name="$1"
  if [ -n "${!name:-}" ]; then
    echo "  OK  $name is set (value hidden)"
  else
    echo "  --  $name unset"
  fi
}

host_only() {
  local name="$1"
  local v="${!name:-}"
  if [ -z "$v" ]; then
    echo "  --  $name unset"
    return
  fi
  local host
  host=$(printf '%s' "$v" | sed -E 's#^(https?://[^/?#]+).*#\1#')
  echo "  OK  $name host → $host"
}

echo "Gateway / workspace:"
backend="${ATLAS_GATEWAY_BACKEND:-}"
if [ -z "$backend" ]; then
  echo "  ..  ATLAS_GATEWAY_BACKEND unset → product default cli (set box only for yellow-tag bypass)"
elif [ "$backend" = "box" ]; then
  echo "  OK  ATLAS_GATEWAY_BACKEND=box (handtest bypass; do not change binary default)"
elif [ "$backend" = "cli" ]; then
  echo "  !!  ATLAS_GATEWAY_BACKEND=cli — yellow-tag needs box for Box LLM path"
else
  echo "  ..  ATLAS_GATEWAY_BACKEND=$backend"
fi
present ATLAS_BOX_WORKSPACE

echo "Box LLM (presence / hostname only):"
host_only ATLAS_BOX_LLM_BASE_URL
if [ -n "${ATLAS_BOX_LLM_MODEL:-}" ]; then
  echo "  OK  ATLAS_BOX_LLM_MODEL is set (value hidden)"
else
  echo "  --  ATLAS_BOX_LLM_MODEL unset"
fi
present ATLAS_BOX_LLM_API_KEY
mode="${ATLAS_BOX_LLM_MODE:-}"
case "$mode" in
  mock|off)
    echo "  !!  ATLAS_BOX_LLM_MODE=$mode — forces compose_reply; not real-model yellow-tag"
    ;;
  "")
    echo "  OK  ATLAS_BOX_LLM_MODE unset (good for real-model if BASE_URL+MODEL set)"
    ;;
  *)
    echo "  ..  ATLAS_BOX_LLM_MODE=$mode"
    ;;
esac
if [ -n "${ATLAS_BOX_LLM_TIMEOUT_MS:-}" ]; then
  echo "  OK  ATLAS_BOX_LLM_TIMEOUT_MS=${ATLAS_BOX_LLM_TIMEOUT_MS}"
else
  echo "  ..  ATLAS_BOX_LLM_TIMEOUT_MS unset (gateway default OK)"
fi

base="${ATLAS_BOX_LLM_BASE_URL:-}"
if [ -z "$base" ]; then
  echo "  !!  BASE_URL missing — healthz will show llm_configured=false"
elif printf '%s' "$base" | grep -Eq '/v1/?$'; then
  echo "  OK  BASE_URL looks OpenAI-compat (ends with /v1)"
else
  echo "  !!  BASE_URL does not end with /v1 — ollama example: http://127.0.0.1:11434/v1"
fi

if printf '%s' "$base" | grep -Eq '127\.0\.0\.1:11434|localhost:11434'; then
  echo "  ..  BASE_URL looks like local ollama — fine for yellow-tag; NEVER put real BASE_URL in CI"
fi

echo "Dedicated env (no silent ATLAS_OPENAI_* fallback for Box):"
if [ -n "${ATLAS_OPENAI_BASE_URL:-}${ATLAS_OPENAI_API_KEY:-}${ATLAS_OPENAI_MODEL:-}" ]; then
  echo "  ..  ATLAS_OPENAI_* present in env — Box LLM ignores these; use ATLAS_BOX_LLM_*"
else
  echo "  OK  ATLAS_OPENAI_* not relied on for Box"
fi

healthz_url="${ATLAS_BOX_LLM_HEALTHZ_URL:-http://127.0.0.1:8787/healthz}"
echo "Optional /healthz probe → $healthz_url"
if command -v curl >/dev/null 2>&1; then
  body=""
  if body=$(curl -fsS --max-time 2 "$healthz_url" 2>/dev/null); then
    # Parse lightly without printing unexpected fields that might contain secrets.
    # Only echo backend / llm_configured / llm_model via grep of known keys.
    be=$(printf '%s' "$body" | sed -n 's/.*"backend"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -1)
    lc=$(printf '%s' "$body" | sed -n 's/.*"llm_configured"[[:space:]]*:[[:space:]]*\(true\|false\).*/\1/p' | head -1)
    lm=$(printf '%s' "$body" | sed -n 's/.*"llm_model"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -1)
    echo "  OK  healthz reachable"
    [ -n "$be" ] && echo "      backend=$be"
    if [ -n "$lc" ]; then
      echo "      llm_configured=$lc"
    else
      echo "      llm_configured=(absent — expected on backend=box when LLM env parsed)"
    fi
    [ -n "$lm" ] && echo "      llm_model=$lm"
    # Refuse to print raw body (could theoretically contain unexpected fields)
    if printf '%s' "$body" | grep -Eiq 'api[_-]?key|authorization|bearer'; then
      echo "  !!  healthz body matched key-like field names — treat as L10; do not archive raw body"
    fi
  else
    echo "  --  healthz not reachable (start gateway first, or ignore if only checking env)"
  fi
else
  echo "  --  curl not found; skip healthz"
fi

echo
echo "Next: follow docs/private-resident-runtime-runbook.md § 真模型黄标"
echo "      + docs/rr1-live-llm-handtest-checklist.md RL-C / RL-T / RL-F / RL-D"
echo "This probe does not replace human two-turn handtest / evidence short-form."
