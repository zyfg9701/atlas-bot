# IdP runbook — I1 Hub Auth Gate

Baseline: Hub inbound identity only. **Not** a login product, org/billing
console, or commercial plan gate. Gateway / Box Sidecar **do not** validate
user tokens.

## Modes

| `ATLAS_AUTH_MODE` | Behavior |
|-------------------|----------|
| **`dev`** (default) | No token required. `hello_ack.user_id` = `user_local_dev`. CI / existing smokes stay green. **Not** production identity. |
| **`static`** | HMAC JWT (`HS256`) with `ATLAS_AUTH_JWT_SECRET`. Validates `exp` + `sub`. |
| **`oidc`** | Validate JWT against JWKS. Prefer `ATLAS_OIDC_JWKS_JSON` (inline JSON or file path) for offline/CI; otherwise best-effort fetch from `ATLAS_OIDC_ISSUER`. |

Optional: `ATLAS_AUTH_ALLOWLIST=sub1,sub2` — subject not listed → `link_required` + reason `not_enrolled`.

Optional OIDC: `ATLAS_OIDC_AUDIENCE` — when set, audience must match.

## Credential channel (nailed)

**`Authorization: Bearer <token>` only.**

- Accepted on the HTTP WebSocket upgrade request (`GET /ws`).
- Query `access_token=` is **not** supported.
- Clients never self-announce identity; Hub sets `hello_ack.user_id` from JWT `sub` (or `user_local_dev` in `dev`).

### Tests / tungstenite

Pass the header on connect with `tokio-tungstenite::connect_async` and a
custom `http::Request`:

```rust
let req = http::Request::builder()
    .uri("ws://127.0.0.1:7700/ws")
    .header("authorization", format!("Bearer {token}"))
    .header("host", "127.0.0.1:7700")
    .header("connection", "Upgrade")
    .header("upgrade", "websocket")
    .header("sec-websocket-version", "13")
    .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
    .body(())
    .unwrap();
let (ws, _) = tokio_tungstenite::connect_async(req).await.unwrap();
```

## Failure shape (nailed)

Prefer **accept the WebSocket, then fail the first hello / first frame** with
a closed-set error. This keeps Session unit tests that connect without
headers working under `dev`, and avoids HTTP 401 plumbing for the common path.

- `dev`: missing token → still hello OK (`user_local_dev`).
- `static` / `oidc`: missing / bad / expired token → hello fails with
  **`unauthorized`** (JSON-RPC numeric `-32002`, `data.code = "unauthorized"`).
  `hello_done` stays false → later `bot.command` cannot succeed (also
  `unauthorized`). Gateway is never invoked.
- Allowlist miss → `link_required` + `reason: not_enrolled` (`retryable: false`).

## Closed-set error map

| Scenario | Wire code | Notes |
|----------|-----------|-------|
| Missing / bad / expired token | **`unauthorized`** | Primary nail; protocol ERROR_CODES `-32002` |
| Valid token, not on allowlist | `link_required` | `reason=not_enrolled` |
| (Future) enrollment revoked | `link_removed` | Do not silent-retry |
| (Future) no permission for agent | `command_rejected` | Existing forbidden-class |

`unauthorized` is emitted as a JSON-RPC error using the shared ERROR_CODES
table (not a new `BotRelayErrorCode` variant) so we do not invent wire codes.

## Mint a static token

```bash
# binary helper
ATLAS_AUTH_JWT_SECRET=dev-secret \
  cargo run -q -p atlas-bot-hub --bin mint-jwt -- --sub user_alice --ttl 3600

# or
ATLAS_AUTH_JWT_SECRET=dev-secret ./tools/mint-hub-jwt.sh user_alice

# python one-liner (PyJWT)
python3 - <<'PY'
import jwt, time, os
print(jwt.encode(
  {"sub":"user_alice","exp":int(time.time())+3600},
  os.environ["ATLAS_AUTH_JWT_SECRET"], algorithm="HS256"))
PY
```

## Start Hub

```bash
# default = dev (no token)
RUST_LOG=info cargo run -p atlas-bot-hub

# static
ATLAS_AUTH_MODE=static ATLAS_AUTH_JWT_SECRET=dev-secret \
  RUST_LOG=info cargo run -p atlas-bot-hub

# oidc mock (offline)
export ATLAS_AUTH_MODE=oidc
export ATLAS_OIDC_ISSUER=https://idp.example.test
export ATLAS_OIDC_AUDIENCE=atlas-hub
export ATLAS_OIDC_JWKS_JSON='{"keys":[{"kty":"oct","kid":"k1","alg":"HS256","k":"<base64url-secret>"}]}'
# mint with same secret bytes + iss/aud claims:
ATLAS_AUTH_JWT_SECRET='<raw-secret>' cargo run -q -p atlas-bot-hub --bin mint-jwt -- \
  --sub user_oidc --iss "$ATLAS_OIDC_ISSUER" --aud "$ATLAS_OIDC_AUDIENCE"
```

## Smoke

```bash
cargo test -p atlas-bot-hub --test idp_smoke -- --nocapture --test-threads=1
# expect lines:
#   SMOKE_OK idp dev user_local_dev
#   SMOKE_OK idp static good user_id=sub
#   SMOKE_OK idp static missing → unauthorized
#   SMOKE_OK idp static bad → unauthorized
#   SMOKE_OK idp oidc-mock user_id=sub
#   SMOKE_OK idp allowlist → link_required not_enrolled
#   SMOKE_OK idp ws-upgrade Authorization Bearer user_id=sub
#   SMOKE_OK idp gate dev+static+oidc-mock+allowlist+ws-bearer
```

CI runs `idp_smoke` with default env (`dev` for other tests). Do **not** set
`ATLAS_AUTH_MODE=static` on the shared hub-smoke job.

## Boundaries

- Auth **only** at Hub inbound. Gateway / Box never require user Bearer.
- **No** change to `bot.command` / capabilities / Bot-Relay method shapes.
- PC / mobile / CLI may paste token or set an env var; no browser login UI (I2).
- Still **not**: account registration, org/billing, commercial `no_plan` path,
  or a second auth layer on Gateway/Box.

## § Known limitations (I1 nails)

1. **Credential channel:** `Authorization: Bearer` only (WS upgrade HTTP header). No query `access_token=`.
2. **Failure shape:** Accept socket, fail hello / first frame with closed-set error (not HTTP 401). For `static`/`oidc`, no valid auth → unauthorized before entering successful `bot.command`.
3. **Primary closed-set code:** `unauthorized` (`-32002`).
4. **Allowlist:** Implemented via `ATLAS_AUTH_ALLOWLIST`; miss → `link_required` + `not_enrolled`.
5. **OIDC:** JWKS via `ATLAS_OIDC_JWKS_JSON` (mock/file) preferred for CI; live issuer fetch is best-effort. Smoke includes `SMOKE_OK idp oidc-mock…`.
6. **Out of scope:** No browser login page, no org/billing, not a commercial `no_plan` gate; Gateway/Box do not re-validate user tokens.
