# Atlas Bot — Android (P4)

Kotlin + Jetpack Compose + OkHttp WebSocket bot_client.

See [docs/P4-runbook.md](../../docs/P4-runbook.md).

```bash
./gradlew :app:assembleDebug :app:testDebugUnitTest
```

Generated protocol (do not fork semantics):

- `generated-kotlin` → `../../third_party/xai-tool-protocol/generated/kotlin` (full tree)
- `generated-botrelay/BotRelay.kt` → `generated-kotlin/BotRelay.kt` (compiled into the app)

## P4 smoke (`SMOKE_OK`)

```bash
# from repo root — HubClient closed-loop vs MockWebServer (non-echo)
../../tools/p4-smoke.sh
# expect: SMOKE_OK p4 android mock-ws non-echo+hello+cold+subscribe+sendPrompt+turn_finished+transcriptTail

# live Hub + P3.5 mock CLI gateway
../../tools/p4-smoke.sh --with-hub
```

See [docs/P4-runbook.md](../../docs/P4-runbook.md) §Evidence.


## I2.2 Mobile ticket (M1)

- Login uses **Chrome Custom Tabs + PKCE S256** (AppAuth-equivalent shape). **No WebView** primary path.
- Redirect: `atlasbot://auth/callback` (Manifest intent-filter).
- `client_id`: `atlas-bot-android` (register on IdP next to PC loopback).
- TokenStore: EncryptedSharedPreferences; Logout clears.
- `HubClient.connect` / `setAuthorization`: optional Bearer; `null` = today's no-token (`dev`) behavior.
- Prefer `id_token` as Hub bearer (`pickHubBearer`).

See [docs/i2-login-runbook.md](../../docs/i2-login-runbook.md) § Mobile (I2.2).


## MU1 Mobile UI convergence

- Default **Chat** primary: Connect, Login/Logout, agent picker, Conversation, Send (auto-subscribe).
- Collapsible **更多 / 调试**: Cold status/roster, explicit subscribe/unsubscribe/transcriptTail, caps/connection_id, Log, desktop/upload placeholder.
- Hub URL / OIDC issuer under **高级** fold; Login stays on the primary path (I2.2).
- See [docs/mobile-user-guide.md](../../docs/mobile-user-guide.md).

## W1 WeCom ticket (stub)

- `auth/WeComAuth.kt` — same callback `atlasbot://auth/callback`; Hub `/auth/wecom/exchange`.
- Provider switch: `ATLAS_TICKET_PROVIDER=wecom` (default oidc). **No WebView** primary.
- Full Custom Tabs wiring = follow-up; PC/CLI are full-path this PR. See `docs/i2-login-runbook.md` § WeCom.
