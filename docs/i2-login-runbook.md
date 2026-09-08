# I2.1 Login runbook -- PC + CLI (OIDC Authorization Code + PKCE)

Baseline: I1 Hub Auth Gate already validates Bearer. I2 only obtains the ticket.
No new bot.* methods. No Hub login website. No query access_token=.
Gateway / Box unchanged for user auth.

Scope: I2.1 PC + CLI only. WeCom and I2.2 mobile deferred.

## Ticket handed to Hub (nailed)

Prefer id_token when present; else access_token if JWT.
Hub ATLAS_AUTH_MODE=oidc verifies via JWKS (ATLAS_OIDC_JWKS_JSON).

## Flow

authorize (PKCE S256) -> redirect + code -> token endpoint
  -> local store -> WS Authorization: Bearer <id_token|jwt-access>
  -> Hub I1 -> hello_ack.user_id = sub

## Env

- ATLAS_OIDC_ISSUER, ATLAS_OIDC_CLIENT_ID, ATLAS_OIDC_AUDIENCE
- ATLAS_OIDC_REDIRECT_PORT (default 0 ephemeral)
- ATLAS_I2_NO_BROWSER=1 for CI mock drive
- ATLAS_BOT_CONFIG_DIR override credentials dir
- ATLAS_HUB_BEARER optional CLI override
- Hub: ATLAS_AUTH_MODE=oidc + ATLAS_OIDC_JWKS_JSON
- dev (default): skip login; existing smokes stay green

## Mock OIDC

cargo run -q -p atlas-bot-auth-client --bin mock-oidc
See tools/mock-oidc/README.md

## CLI

atlas-bot-cli login / logout / status
Credentials path: ~/.config/atlas-bot/credentials.json mode 0600
Loopback: http://127.0.0.1:<port>/callback

## PC

UI: Login / Logout + optional Bearer paste.
Browser WebSocket cannot set Authorization.
Production path: Tauri connect_ws (clients/pc/src-tauri) sends Bearer on upgrade.
HubClient.connect(url, { authorization? }) stores token; Tauri sends header.
Under Hub dev, plain browser WS remains fine.

## Smoke

See protocol-conformance workflow for i2_smoke.
Expect SMOKE_OK i2 lines.
PC unit tests cover PKCE challenge and callback parsing.

## Failure closed-set

Missing / bad / expired -> Hub unauthorized (-32002).

## Sec10 Known limitations (filled)

- Ticket to Hub: id_token preferred; else JWT access_token
- PC callback: loopback 127.0.0.1; Tauri rust WS for Bearer
- CLI: PKCE loopback (not device-code)
- refresh: Not done -- expire then re-login
- WeCom: Deferred
- Mobile I2.2: Deferred
- Hub login site: Not built (I2.3 rejected)
- query token: Forbidden

## Boundaries

- Zero new bot.* methods
- Bearer-only inbound credential channel (I1)
- Gateway / Box do not validate user tickets
- Not an account/org/billing product
