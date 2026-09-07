# P2 runbook — PC bot_client (Tauri 2 + TypeScript)

## Scope

Minimal PC `bot_client` closed loop against the P1 Hub:

`Connect WS → hello(bot_client) → cold status/roster → subscribe → sendPrompt → hub:turn_finished → getAgentTranscriptTail`

Out of scope: mobile, VNC, IdP, CLI, attachments.

## Prerequisites

1. Hub running (see [`P1-runbook.md`](./P1-runbook.md)):

```bash
RUST_LOG=info cargo run -p atlas-bot-hub
# default: ws://127.0.0.1:7700/ws
```

2. Node 20+ and a Rust toolchain (same as workspace). Linux desktop build also needs WebKitGTK (see Manual Tauri build below).

## PC client layout

| Path | Role |
|------|------|
| `clients/pc/` | Tauri 2 app root |
| `clients/pc/src/hubClient.ts` | Bot-Relay WS client (cold/hot separation) |
| `clients/pc/src/main.ts` | Minimal UI |
| `clients/pc/src-tauri/` | Tauri 2 Rust shell (not a workspace member) |

## Frontend-only smoke (no native shell)

Useful on headless CI / servers without WebKit:

```bash
cd clients/pc
npm install
npm run build
npm run test:unit
npm run preview
```

Then in the UI:

1. Confirm Hub URL (`ws://127.0.0.1:7700/ws`).
2. **Connect + hello** — expect `hello_ack` + capabilities.
3. **bot.status** / **bot.roster** — cold path (Hub logs `cold bot.status` / `cold bot.roster`, **no** `gateway invoke`).
4. **subscribe** on `agt_1`.
5. **sendPrompt** — expect `bot.event` / `hub:turn_finished`.
6. **getAgentTranscriptTail** — show recent messages.

### Protocol notes (DoD)

- Cold path uses `bot.status` / `bot.roster` only — **never** `bot.command`.
- Seq is for **display/sort only**; gaps do **not** trigger resync. Only explicit `hub:resync_required` re-fetches transcript.
- Unknown error codes are shown as failure (degraded `upstream_error` semantics); UI does not crash.
- `sendPrompt` keeps envelope `agentId` equal to `args.agentId` (no silent rewrite on mismatch).

## Manual Tauri build (desktop window)

```bash
cd clients/pc
npm install
# Linux deps example (Debian/Ubuntu):
#   sudo apt install libwebkit2gtk-4.1-dev librsvg2-dev patchelf
npm exec tauri build
# or: npm run tauri -- build
```

Dev loop:

```bash
# terminal A: Hub
cargo run -p atlas-bot-hub

# terminal B: PC
cd clients/pc && npm run tauri -- dev
```

## CI posture

- Root **license-check** + **protocol-conformance** remain on Hub/vendor crates (PC `src-tauri` is **not** a Cargo workspace member so hub license scan stays unchanged).
- Workflow `pc-client.yml` runs install / build / unit tests on `clients/pc`.
- Full `tauri build` is **manual** on Linux agents lacking WebKitGTK; document success locally rather than requiring packaged installers in P2.

## Sample closed-loop log (expected shape)

```
→ hello { protocol_version: "1.0.0", kind: "bot_client" }
← hello_ack { connection_id, capabilities: [bot.command, bot.status, …] }
→ bot.status
← runState: hibernated
→ bot.roster
← agents: [{ agentId: "agt_1", … }]
→ bot.subscribe { agentIds: ["agt_1"] }
→ bot.command sendPrompt
← bot.event hub:turn_finished seq=1
→ bot.command getAgentTranscriptTail
← entries […]
```

## Out of scope (P2)

Android / iOS, VNC, attachments, groups/channels, real IdP, Atlas CLI, hub-mcp-adapter, production installers / auto-update.
