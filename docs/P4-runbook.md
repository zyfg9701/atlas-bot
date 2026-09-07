# P4 runbook — Android + iOS native bot_clients

Baseline: `main` @ `bf4038f` (P3.5 complete).  
Stack: **Kotlin + Jetpack Compose + OkHttp** / **Swift + SwiftUI + URLSessionWebSocket**.  
Generated protocol (referenced, not forked):

- `third_party/xai-tool-protocol/generated/kotlin/` → `clients/android/generated-kotlin` (symlink); app compiles via `generated-botrelay/BotRelay.kt` → that tree
- `third_party/xai-tool-protocol/generated/swift/` → `clients/ios/GeneratedProtocol` (symlink; Xcode compiles `BotRelayProtocol.swift`)

**Out of scope:** VNC, attachments, real IdP UI, store packages, full design system, groups/channels.

## Closed loop (same as PC / P2)

```
hello (kind=bot_client, protocol_version=1.0.0)
  → hello_ack (capabilities)
  → bot.status / bot.roster          # cold — never bot.command
  → bot.subscribe { agentIds }
  → bot.command sendPrompt           # hot — envelope agentId == args.agentId
  → bot.event hub:turn_finished
  → bot.command getAgentTranscriptTail
```

Seq is per `(connection, agent)` for display/sort only — **never** auto-resync on gaps; only `hub:resync_required` or reconnect (re-hello).  
`agent_id_mismatch` → show protocol error; do not rewrite.

## Start Hub (stub, no P3.5 gateway)

```bash
# from repo root
RUST_LOG=info cargo run -p atlas-bot-hub
# listens ws://127.0.0.1:7700/ws
```

Leave `ATLAS_GATEWAY_URL` unset → in-process stub (echo + `hub:turn_finished`).

## Start Hub + P3.5 real gateway

```bash
# terminal A — gateway (mock CLI = non-echo)
ATLAS_GATEWAY_BACKEND=cli \
ATLAS_AGENT_CLI="$PWD/tools/mock-cli/mock-atlas-agent-cli.sh" \
cargo run -p atlas-bot-gateway
# http://127.0.0.1:8787

# terminal B — hub pointed at gateway
ATLAS_GATEWAY_URL=http://127.0.0.1:8787 \
RUST_LOG=info cargo run -p atlas-bot-hub
```

Mobile apps are **bot_client only** — they never start a gateway. Point them at the same Hub WS URL the PC uses.

See also `docs/P3.5-runbook.md`.

## Android

### Simulator / emulator URL

| Where Hub runs | App Hub WS URL |
|----------------|----------------|
| Host machine, app in **Android Emulator** | `ws://10.0.2.2:7700/ws` (default in app) |
| Host machine, app on **physical device** (same LAN) | `ws://<host-lan-ip>:7700/ws` |
| Hub bound only to `127.0.0.1` | Emulator OK via `10.0.2.2`; device needs Hub bind `0.0.0.0` / firewall allow |

Default Hub bind is `127.0.0.1:7700`. For a physical phone, start Hub with e.g. `ATLAS_HUB_BIND=0.0.0.0:7700` (if supported) or SSH/USB reverse.

### Build & unit tests (no emulator required)

```bash
cd clients/android
./gradlew :app:assembleDebug :app:testDebugUnitTest
```

CI runs the same on `ubuntu-latest` (see `.github/workflows/android-client.yml`).

### Click path

1. Open app → confirm URL (`10.0.2.2` on emulator).
2. **Connect** → log shows `→ hello` then `← hello_ack` / capabilities.
3. **status** / **roster** → `runState` + agents (cold; Hub must not increment gateway `invoke_count`).
4. Select `agt_1` (or roster id) → **subscribe**.
5. Enter prompt → **sendPrompt** → wait for `hub:turn_finished` in log.
6. **transcriptTail** → show recent transcript (stub echo or gateway non-echo).
7. Force mismatch (optional): edit args only in a debug build — expect `command_rejected` / `agent_id_mismatch` shown in red.
8. **unsubscribe** → further events for that agent ignored.

## iOS

### Simulator URL

| Where Hub runs | App Hub WS URL |
|----------------|----------------|
| Host Mac, **iOS Simulator** | `ws://127.0.0.1:7700/ws` (default) |
| Physical iPhone, same LAN | `ws://<host-lan-ip>:7700/ws` + Hub reachable on LAN |

`10.0.2.2` is **Android emulator only** — do not use it on iOS.

### Build (requires macOS + Xcode)

No Linux/iOS CI runner in this repo for P4. On a Mac:

```bash
cd clients/ios
xcodebuild -scheme AtlasBot -destination 'platform=iOS Simulator,name=iPhone 16' build
xcodebuild -scheme AtlasBot -destination 'platform=iOS Simulator,name=iPhone 16' test
```

Open `clients/ios/AtlasBot.xcodeproj` in Xcode → Run. ATS allows local networking (`NSAllowsLocalNetworking` / arbitrary loads for P4 lab).

### Click path

Same loop as Android (§Connect → cold → subscribe → sendPrompt → turn_finished → transcriptTail).

## Smoke checklist → `SMOKE_OK`

### Automated (CI-friendly, no emulator)

```bash
# Preferred one-liner — Android HubClient closed-loop vs in-process MockWebServer
# (non-echo transcript shaped like P3.5 mock CLI)
./tools/p4-smoke.sh

# Expect:
# SMOKE_OK p4 android mock-ws non-echo+hello+cold+subscribe+sendPrompt+turn_finished+transcriptTail

# Live Hub + P3.5 gateway (mock CLI) on the same wire path mobile uses:
./tools/p4-smoke.sh --with-hub
# Expect also:
# SMOKE_OK p4 ws-hub gateway-mock non-echo+hello+cold+subscribe+sendPrompt+turn_finished+transcriptTail

# Encoding / error unit tests (included in CI)
cd clients/android && ./gradlew :app:assembleDebug :app:testDebugUnitTest

# Hub still green (optional; needs Rust toolchain)
cargo test -p atlas-bot-hub --test p3_smoke -- --nocapture
cargo test -p atlas-bot-hub --test p35_smoke -- --nocapture --test-threads=1
```

### Evidence — how non-echo is proven

| Path | What runs | Non-echo proof |
|------|-----------|----------------|
| **Android JVM** `P4ClosedLoopSmokeTest` | `HubClient` ↔ OkHttp `MockWebServer` Bot-Relay WS | `sendPrompt` preview + `getAgentTranscriptTail` contain `atlas-mock-reply …` and **must not** be `echo: …` or bare prompt |
| **Live Hub** `tools/p4-smoke.sh --with-hub` | gateway (`ATLAS_AGENT_CLI=tools/mock-cli/mock-atlas-agent-cli.sh`) + Hub + `tools/p4-ws-closed-loop.py` | Same assertions on real `hub:turn_finished` + transcriptTail |
| **Emulator hand-check** | App → `ws://10.0.2.2:7700/ws` against Hub from `--with-hub` | UI transcript shows `atlas-mock-reply` (not stub `echo:`) |

Recorded `SMOKE_OK` lines (copy from script/CI logs into PR):

```text
SMOKE_OK p4 android mock-ws non-echo+hello+cold+subscribe+sendPrompt+turn_finished+transcriptTail
SMOKE_OK p4 ws-hub gateway-mock non-echo+hello+cold+subscribe+sendPrompt+turn_finished+transcriptTail
```

Manual one-liner after a successful Android or iOS click path (stub Hub echo is OK for handshake-only; **§5 true gateway** needs mock CLI / `--with-hub`):

```text
SMOKE_OK p4 mobile hello+cold+subscribe+sendPrompt+turn_finished+transcriptTail
```

## Protocol compliance (both apps)

| Rule | Behavior |
|------|----------|
| Cold vs hot | `status` / `roster` via `bot.status` / `bot.roster` only |
| Wire camelCase | `agentId`, `agentIds`, `runState`, `fullFidelity` |
| Errors | Unknown codes → failure display (`upstream_error`); no crash |
| Seq | Discontinuity logged; no auto resync |
| Unsubscribe | Events for unsubscribed agents ignored |

## Still not in P4

IdP login UI, VNC, attachments, store listing, automation/widgets, forcing PC redesign.
