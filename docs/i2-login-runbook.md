# I2 / W1 Login runbook -- PC + CLI + Mobile (OIDC + WeCom)

Baseline: I1 Hub Auth Gate already validates Bearer. I2/W1 only obtain the ticket.
No new bot.* methods. No Hub login website. No query access_token=.
Gateway / Box unchanged for user auth.

## Ticket handed to Hub (nailed)

**OIDC:** Prefer id_token when present; else access_token if JWT.
Hub `ATLAS_AUTH_MODE=oidc` verifies via JWKS (`ATLAS_OIDC_JWKS_JSON`).
`hello_ack.user_id = sub`.

**WeCom (W1):** Hub thin exchange mints short HS256 JWT (`ATLAS_AUTH_MODE=static` +
`ATLAS_AUTH_JWT_SECRET`). `sub = wecom:<corpId>:<userid>`. Never hand opaque
WeCom access_token as Hub Bearer.

## Flow (all surfaces)

### OIDC (default)

authorize (PKCE S256) -> redirect + code -> token endpoint
  -> local store -> WS Authorization: Bearer <id_token|jwt-access>
  -> Hub I1 -> hello_ack.user_id = sub

### WeCom (W1)

authorize/扫码 (state, **no PKCE**) -> redirect + code
  -> Hub `POST /auth/wecom/exchange` (secret Hub-only)
  -> local store (provider=wecom) -> WS Bearer <hub-jwt>
  -> Hub I1 (static/oidc) -> hello_ack.user_id = wecom:<corpId>:<userid>

## Env

### Shared / OIDC

- `ATLAS_TICKET_PROVIDER=oidc|wecom` (default **oidc**)
- `ATLAS_OIDC_ISSUER`, `ATLAS_OIDC_CLIENT_ID`, `ATLAS_OIDC_AUDIENCE`
- `ATLAS_OIDC_REDIRECT_PORT` (default 0 ephemeral) — PC/CLI loopback
- `ATLAS_I2_NO_BROWSER=1` for CI mock drive (OIDC **and** WeCom)
- `ATLAS_BOT_CONFIG_DIR` override credentials dir
- `ATLAS_HUB_BEARER` optional CLI override
- Hub: `ATLAS_AUTH_MODE=oidc` + `ATLAS_OIDC_JWKS_JSON` (OIDC tickets)
- `dev` (default): skip login; existing smokes stay green

### WeCom (W1)

| Variable | Side | Notes |
|----------|------|-------|
| `ATLAS_TICKET_PROVIDER` | client | `oidc` (default) \| `wecom` |
| `ATLAS_WECOM_CORP_ID` | Hub + client authorize URL | required on wecom path |
| `ATLAS_WECOM_AGENT_ID` | same | required |
| `ATLAS_WECOM_SECRET` | **Hub only** | required; **never** in mobile/PC release packages |
| `ATLAS_WECOM_REDIRECT_URI` | client | PC loopback or `atlasbot://auth/callback` |
| `ATLAS_WECOM_API_BASE` | Hub | default `https://qyapi.weixin.qq.com`; CI → mock-wecom base |
| `ATLAS_WECOM_AUTHORIZE_BASE` | client | mock base or live open.weixin authorize host |
| `ATLAS_WECOM_JWT_TTL` | Hub | optional; default 3600s |
| `ATLAS_AUTH_JWT_SECRET` | Hub | mints/verifies exchanged JWT (`static` mode) |
| `ATLAS_HUB_HTTP` | client | Hub HTTP for exchange (else derive from `ATLAS_HUB_WS`) |

Same deploy may configure OIDC + WeCom; one login picks one provider.
Hub does **not** need `ATLAS_AUTH_MODE=wecom`.

## Mock OIDC

```bash
cargo run -q -p atlas-bot-auth-client --bin mock-oidc
```

See tools/mock-oidc/README.md

Register **both** PC loopback and mobile custom-scheme redirects on the IdP:

- PC/CLI: `http://127.0.0.1:<port>/callback`
- Mobile: `atlasbot://auth/callback`

## Mock WeCom (W1)

```bash
cargo run -q -p atlas-bot-auth-client --bin mock-wecom
```

See tools/mock-wecom/README.md

Endpoints: `/authorize`, `/cgi-bin/gettoken`, `/cgi-bin/user/getuserinfo`.
Point Hub `ATLAS_WECOM_API_BASE` + client `ATLAS_WECOM_AUTHORIZE_BASE` at the printed base.
**CI never** calls `qyapi.weixin.qq.com`.

Headless:

```bash
export ATLAS_I2_NO_BROWSER=1
export ATLAS_TICKET_PROVIDER=wecom
# … corp/agent/secret/api_base/jwt_secret/hub …
atlas-bot-cli login --provider wecom
```

## CLI (I2.1 + W1)

```bash
atlas-bot-cli login --provider oidc --issuer …
atlas-bot-cli login --provider wecom
atlas-bot-cli logout
atlas-bot-cli status
```

Credentials path: `~/.config/atlas-bot/credentials.json` mode 0600 (includes `provider`).
Loopback: `http://127.0.0.1:<port>/callback`

## PC (I2.1 + W1)

UI: Login prompts for provider (`oidc`\|`wecom`) / Logout + optional Bearer paste.
Browser WebSocket cannot set Authorization.
Production path: Tauri `connect_ws` (clients/pc/src-tauri) sends Bearer on upgrade.
`WeComProvider` helpers: `clients/pc/src/wecomClient.ts`.
Under Hub `dev`, plain browser WS remains fine.

## Mobile (I2.2 / M1 + WM1 WeCom real Login)

**Scope:** system-browser ticket pickup for Android + iOS. **Not** mobile UI
convergence, **not** I2.3 / public IdP productization, **not** auto-login desktop /
cross-device SSO, **not** Hub-hosted login site. Gateway/Box do not validate user tickets.

**Reject WebView as the primary login path** (M2 is documented as anti-pattern only).

### Redirect (exact)

| Item | Value |
|------|-------|
| Custom scheme URI | `atlasbot://auth/callback` |
| Scheme | `atlasbot` |
| Host / path | `auth` / `/callback` |
| App / Universal Link | **Not done** (optional; does not block DoD) |

### client_id (OIDC; recommended separate public clients, same issuer)

| Surface | client_id |
|---------|-----------|
| Android | `atlas-bot-android` |
| iOS | `atlas-bot-ios` |
| PC (I2.1) | `atlas-bot-pc` (or ATLAS_OIDC_CLIENT_ID) |

Scopes default: `openid profile` (+ optional audience), same narrative as I2.1.

### WeCom mobile (WM1 — real Login)

Android **and** iOS wire W1 stubs into the same I2.2 session skeleton:

1. Set **ATLAS_TICKET_PROVIDER=wecom** (advanced UI field; default remains **oidc**).
2. Fill corpId / agentId / authorize base (mock-wecom URL for CI/local).
3. Tap **Login** → system browser (**Custom Tabs** / **ASWebAuthenticationSession**), **no WebView**,
   authorize URL from `WeComAuth.buildAuthorizeUrl` (corpId/agentId/state; **no PKCE**).
4. Callback `atlasbot://auth/callback?code=…&state=…` — state mismatch fails observably.
5. App `POST {hubHttp}/auth/wecom/exchange` JSON `{code,state?}` (same shape as PC
   `exchangeWeComCode`) → Hub JWT `access_token` → TokenStore (optional `provider=wecom`).
6. Tap **Connect** → existing HubClient Bearer → `hello_ack.user_id=wecom:<corpId>:<userid>`.
7. **Logout** clears TokenStore (and provider).

**Config (mobile):**

| Item | Where |
|------|--------|
| `ATLAS_TICKET_PROVIDER` | advanced field / env (default `oidc`) |
| `ATLAS_WECOM_CORP_ID` / `AGENT_ID` | advanced fields |
| `ATLAS_WECOM_AUTHORIZE_BASE` | mock-wecom base (e.g. emulator `http://10.0.2.2:8091`) |
| Hub HTTP for exchange | derived from Hub WS URL (`ws→http`, strip `/ws`) |
| Secret | **Hub only** — never in APK/IPA |

**Mock:** `cargo run -q -p atlas-bot-auth-client --bin mock-wecom` + Hub static JWT +
`ATLAS_WECOM_API_BASE=<mock>` (see § Mock WeCom). CI must not hit `qyapi.weixin.qq.com`.

**OIDC switch:** leave provider=`oidc` — I2.2 PKCE path unchanged. Same install may hold both
configs; one Login uses the current provider only.

**Not this slice:** T2 MSI; mobile UI re-convergence; I2.3; W2 introspection; WebView primary;
new `bot.*`; Gateway/Box user-ticket validation.

Register `atlasbot://auth/callback` on the WeCom app console for live yellow-tag (optional; not DoD).

### Android (OIDC full)

1. Set **OIDC issuer** (e.g. emulator → host mock: `http://10.0.2.2:8090`).
2. Tap **Login** → Chrome **Custom Tabs** opens authorize (PKCE S256).
3. IdP redirects to `atlasbot://auth/callback?code=…&state=…` (Manifest intent-filter).
4. App exchanges code, stores ticket in **EncryptedSharedPreferences**, prefers **id_token**.
5. Tap **Connect** → OkHttp WS upgrade sends `Authorization: Bearer <ticket>` when present.
6. **Logout** clears TokenStore and disconnects.

Implementation: `clients/android` — `auth/OidcAuth.kt`, `AuthSession.kt`, `TokenStore.kt`;
`HubClient.setAuthorization` / `buildConnectRequest` (null = today's no-token / `dev`).

### iOS (OIDC full)

1. Set **OIDC issuer** (simulator: `http://127.0.0.1:8090`).
2. Tap **Login** → **ASWebAuthenticationSession** (callbackURLScheme `atlasbot`).
3. Callback `atlasbot://auth/callback` (Info.plist URL Types + session completion / `onOpenURL`).
4. Exchange + **Keychain** store (`afterFirstUnlockThisDeviceOnly`); prefer **id_token**.
5. Tap **Connect** → `URLRequest` WebSocket carries `Authorization: Bearer` when set.
6. **Logout** clears Keychain and disconnects.

Implementation: `clients/ios` — `OidcAuth.swift`, `AuthSession.swift`, `TokenStore.swift`.

### Auth modes

| Hub mode | No ticket | With ticket |
|----------|-----------|-------------|
| `dev` (default) | Connect works (P4 smoke unchanged) | Optional Bearer accepted |
| `oidc` / `static` | Fail closed (`unauthorized` / -32002 or connect failure) → Login | `hello_ack.user_id=sub` |

## Smoke (PC/CLI)

See protocol-conformance workflow for `i2_smoke` and `wecom_smoke`.
Expect `SMOKE_OK i2…` and `SMOKE_OK wecom…` lines.
PC unit tests cover PKCE challenge, callback parsing, and WeCom authorize URL.

## Failure closed-set

Missing / bad / expired -> Hub unauthorized (-32002).

## § WeCom known limitations (W1 §10 filled)

- **`sub` mapping:** `wecom:<corpId>:<userid>` (unit-tested)
- **Exchange:** `POST /auth/wecom/exchange`; JWT **HS256** via Hub `ATLAS_AUTH_JWT_SECRET` (static)
- **Clients:** PC **full**; CLI **full** (mock preferred); Android + iOS **WM1 real Login** (system browser → Hub exchange)
- **PKCE:** WeCom web OAuth — **not supported**; use **state + one-time code** + Hub secret
- **扫码页:** minimal authorize redirect (mock); pretty QR page = optional / not required
- **Live WeCom yellow-tag:** optional; does not block DoD (CI uses mock-wecom only)
- **Not done / out of scope:** T2 MSI; I2.3 Hub login site; W2 hot-path WeCom introspection;
  WebView primary; putting `ATLAS_WECOM_SECRET` in client packages; new `bot.*`; Gateway/Box user auth

## Sec10 / §8 Known limitations (I2 filled)

- Ticket to Hub: id_token preferred; else JWT access_token
- PC callback: loopback 127.0.0.1; Tauri rust WS for Bearer
- CLI: PKCE loopback (not device-code) for OIDC; WeCom via Hub exchange
- **Android:** Custom Tabs + PKCE (OIDC) / WeCom authorize (no PKCE) → Hub exchange; scheme `atlasbot://auth/callback`
- **iOS:** ASWebAuthenticationSession; OIDC PKCE or WeCom → Hub exchange; URL Types scheme `atlasbot`
- **mobile client_id:** separate `atlas-bot-android` / `atlas-bot-ios` (same issuer as PC)
- **refresh:** Not done — expire then re-login
- **App / Universal Link:** Not done
- **Paste Bearer advanced entry:** Not done on mobile (PC still has paste)
- **iOS CI:** macOS / xcodebuild for unit tests; Linux = logic documented, handtest yellow
- ~~WeCom: Deferred~~ → **W1 landed** (see § WeCom)
- Hub login site: Not built (I2.3 rejected)
- query token: Forbidden

## Boundaries

- Zero new bot.* methods
- Bearer-only inbound credential channel (I1)
- Gateway / Box do not validate user tickets
- Not an account/org/billing product
- Not mobile UI convergence
- Not WebView primary login path
- Not T2 MSI / not W2 introspection / not public IdP productization
