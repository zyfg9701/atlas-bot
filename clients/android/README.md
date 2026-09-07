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

