#!/usr/bin/env bash
# WL1: print which WeCom live env vars look set / whether redirect looks pinned.
# Does NOT call qyapi.weixin.qq.com, does NOT mint tokens, does NOT open a browser.
# Optional: source a private env file first, e.g.
#   set -a; source ~/.config/atlas-bot/probe-wecom-live.env; set +a
#   ./scripts/probe-wecom-live.sh
# Exit 0 always (advisory). Not a CI gate.

set -euo pipefail

echo "=== probe-wecom-live (advisory · no network · no secrets printed) ==="

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
  # strip to scheme+host for display (no query/path secrets expected)
  local host
  host=$(printf '%s' "$v" | sed -E 's#^(https?://[^/?#]+).*#\1#')
  echo "  OK  $name host → $host"
}

echo "Provider / identity:"
present ATLAS_TICKET_PROVIDER
present ATLAS_WECOM_CORP_ID
present ATLAS_WECOM_AGENT_ID

echo "Bases (hostname only):"
host_only ATLAS_WECOM_AUTHORIZE_BASE
host_only ATLAS_WECOM_API_BASE
host_only ATLAS_HUB_HTTP

echo "Redirect / loopback:"
ru="${ATLAS_WECOM_REDIRECT_URI:-}"
port="${ATLAS_OIDC_REDIRECT_PORT:-}"
if [ -z "$ru" ]; then
  echo "  --  ATLAS_WECOM_REDIRECT_URI unset (PC default prompt is http://127.0.0.1:8765/callback)"
elif printf '%s' "$ru" | grep -Eq '^http://127\.0\.0\.1:8765/callback/?$'; then
  echo "  OK  ATLAS_WECOM_REDIRECT_URI looks like pinned PC/CLI loopback ($ru)"
elif printf '%s' "$ru" | grep -Eq '^atlasbot://auth/callback/?$'; then
  echo "  OK  ATLAS_WECOM_REDIRECT_URI looks like mobile deep link ($ru)"
else
  echo "  !!  ATLAS_WECOM_REDIRECT_URI=$ru — expect http://127.0.0.1:8765/callback or atlasbot://auth/callback"
fi
if [ "$port" = "8765" ]; then
  echo "  OK  ATLAS_OIDC_REDIRECT_PORT=8765 (pinned; matches PC UI default)"
elif [ -z "$port" ] || [ "$port" = "0" ]; then
  echo "  !!  ATLAS_OIDC_REDIRECT_PORT=${port:-unset} — ephemeral; live WeCom console needs a fixed port (recommend 8765)"
else
  echo "  ..  ATLAS_OIDC_REDIRECT_PORT=$port (ensure WeCom console matches http://127.0.0.1:${port}/callback)"
fi
echo "  ..  Also register mobile: atlasbot://auth/callback"

echo "Hub auth (presence only; values never printed):"
present ATLAS_AUTH_MODE
present ATLAS_AUTH_JWT_SECRET
present ATLAS_WECOM_SECRET

echo "Browser / CI flags:"
if [ -n "${ATLAS_I2_NO_BROWSER:-}" ]; then
  echo "  !!  ATLAS_I2_NO_BROWSER is set — unset for live human authorize (OK for mock/CI only)"
else
  echo "  OK  ATLAS_I2_NO_BROWSER unset (good for live handtest)"
fi

api="${ATLAS_WECOM_API_BASE:-}"
if printf '%s' "$api" | grep -qi 'qyapi.weixin.qq.com'; then
  echo "  !!  API_BASE points at qyapi — fine for local yellow-tag Hub; NEVER put this in CI"
elif [ -n "$api" ]; then
  echo "  ..  API_BASE set (ensure CI stays on mock-wecom)"
fi

echo
echo "Next: follow docs/i2-login-runbook.md § 真机企微联调 + docs/wecom-ticket-checklist.md WL-*"
echo "This probe does not replace human WeCom authorize / evidence short-form."
