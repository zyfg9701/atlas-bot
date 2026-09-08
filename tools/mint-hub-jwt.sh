#!/usr/bin/env bash
# Mint an HS256 JWT for Hub static / oidc-mock modes.
# Usage:
#   ATLAS_AUTH_JWT_SECRET=dev-secret ./tools/mint-hub-jwt.sh user_alice
#   ./tools/mint-hub-jwt.sh user_alice 7200   # ttl seconds
set -euo pipefail
SUB="${1:?subject required}"
TTL="${2:-3600}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
exec cargo run -q -p atlas-bot-hub --bin mint-jwt -- --sub "$SUB" --ttl "$TTL" ${ISS:+--iss "$ISS"} ${AUD:+--aud "$AUD"}
