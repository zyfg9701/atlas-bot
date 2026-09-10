# I2 Login runbook -- PC + CLI + Mobile (OIDC Authorization Code + PKCE)

Baseline: I1 Hub Auth Gate already validates Bearer. I2 only obtains the ticket.
No new bot.* methods. No Hub login website. No query access_token=.
Gateway / Box unchanged for user auth.

## Ticket handed to Hub (nailed)

Prefer id_token when present; else access_token if JWT.
Hub ATLAS_AUTH_MODE=oidc verifies via JWKS (ATLAS_OIDC_JWKS_JSON).
`hello_ack.user_id = sub`.

## Flow (all surfaces)

authorize (PKCE S256) -> redirect + code -> token endpoint
  -> local store -> WS Authorization: Bearer <id_token|jwt-access>
  -> Hub I1 -> hello_ack.user_id = sub

## Env

- ATLAS_OIDC_ISSUER, ATLAS_OIDC_CLIENT_ID, ATLAS_OIDC_AUDIENCE
- ATLAS_OIDC_REDIRECT_PORT (default 0 ephemeral) — PC/CLI loopback
- ATLAS_I2_NO_BROWSER=1 for CI mock drive
- ATLAS_BOT_CONFIG_DIR override credentials dir
- ATLAS_HUB_BEARER optional CLI override
- Hub: ATLAS_AUTH_MODE=oidc + ATLAS_OIDC_JWKS_JSON
- dev (default): skip login; existing smokes stay green

## Mock OIDC

cargo run -q -p atlas-bot-auth-client --bin mock-oidc
See tools/mock-oidc/README.md

Register **both** PC loopback and mobile custom-scheme redirects on the IdP:

- PC/CLI: `http://127.0.0.1:<port>/callback`
- Mobile: `atlasbot://auth/callback`

## CLI (I2.1)

atlas-bot-cli login / logout / status
Credentials path: ~/.config/atlas-bot/credentials.json mode 0600
Loopback: http://127.0.0.1:<port>/callback

## PC (I2.1)

UI: Login / Logout + optional Bearer paste.
Browser WebSocket cannot set Authorization.
Production path: Tauri connect_ws (clients/pc/src-tauri) sends Bearer on upgrade.
HubClient.connect(url, { authorization? }) stores token; Tauri sends header.
Under Hub dev, plain browser WS remains fine.

## Mobile (I2.2 / M1)

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

### client_id (recommended separate public clients, same issuer)

| Surface | client_id |
|---------|-----------|
| Android | `atlas-bot-android` |
| iOS | `atlas-bot-ios` |
| PC (I2.1) | `atlas-bot-pc` (or ATLAS_OIDC_CLIENT_ID) |

Scopes default: `openid profile` (+ optional audience), same narrative as I2.1.

### Android

1. Set **OIDC issuer** (e.g. emulator → host mock: `http://10.0.2.2:8090`).
2. Tap **Login** → Chrome **Custom Tabs** opens authorize (PKCE S256).
3. IdP redirects to `atlasbot://auth/callback?code=…&state=…` (Manifest intent-filter).
4. App exchanges code, stores ticket in **EncryptedSharedPreferences**, prefers **id_token**.
5. Tap **Connect** → OkHttp WS upgrade sends `Authorization: Bearer <ticket>` when present.
6. **Logout** clears TokenStore and disconnects.

Implementation: `clients/android` — `auth/OidcAuth.kt`, `AuthSession.kt`, `TokenStore.kt`;
`HubClient.setAuthorization` / `buildConnectRequest` (null = today's no-token / `dev`).

Shape is AppAuth-equivalent (Custom Tabs + PKCE); no in-app WebView.

### iOS

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
| `oidc` / `static` | Fail closed (`unauthorized` / -32002 or connect failure) → Login | `hello_ack.user_id=sub` (oidc) |

### Smoke / tests

- Android JVM: `./gradlew :app:testDebugUnitTest` — PKCE challenge, callback parse, Bearer header.
- iOS: `xcodebuild … test` on macOS — same; Linux runners skip UI (document yellow).
- Live: mock-oidc → Login → Connect → Hub `oidc` → `user_id=sub`. See `docs/i22-mobile-ticket-checklist.md`.

## Smoke (PC/CLI)

See protocol-conformance workflow for i2_smoke.
Expect SMOKE_OK i2 lines.
PC unit tests cover PKCE challenge and callback parsing.

## Failure closed-set

Missing / bad / expired -> Hub unauthorized (-32002).

## Sec10 / §8 Known limitations (filled)

- Ticket to Hub: id_token preferred; else JWT access_token
- PC callback: loopback 127.0.0.1; Tauri rust WS for Bearer
- CLI: PKCE loopback (not device-code)
- **Android:** AppAuth-equivalent Custom Tabs + PKCE; scheme literal `atlasbot://auth/callback`
- **iOS:** ASWebAuthenticationSession; URL Types scheme `atlasbot`
- **mobile client_id:** separate `atlas-bot-android` / `atlas-bot-ios` (same issuer as PC)
- **refresh:** Not done — expire then re-login
- **App / Universal Link:** Not done
- **Paste Bearer advanced entry:** Not done on mobile (PC still has paste)
- **iOS CI:** macOS / xcodebuild for unit tests; Linux = logic documented, handtest yellow
- WeCom: Deferred
- Hub login site: Not built (I2.3 rejected)
- query token: Forbidden

## Boundaries

- Zero new bot.* methods
- Bearer-only inbound credential channel (I1)
- Gateway / Box do not validate user tickets
- Not an account/org/billing product
- Not mobile UI convergence
- Not WebView primary login path
