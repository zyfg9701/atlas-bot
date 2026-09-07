# atlas-bot

Private Grok Bot stack (Bot-Relay): `bot_client` ↔ Computer Hub ↔ Box gateway.

## Status (P3)

- Monorepo + vendored `xai-tool-protocol` @ `a549186d…`
- **Hub** WebSocket JSON-RPC (`crates/atlas-bot-hub`) — cold `bot.transcript.offbox`
- **Gateway** in-memory stub (`crates/atlas-bot-gateway`) — `createAgent` / `listAgents` / `interruptAgentRun`
- **PC bot_client** Tauri 2 + TypeScript (`clients/pc/`) — multi-agent + offbox + interrupt
- License hygiene + protocol conformance CI; PC frontend CI (`pc-client.yml`)

## Layout

| Path | Role |
|------|------|
| `third_party/xai-tool-*` | Vendored Bot-Relay protocol |
| `crates/atlas-bot-hub` | Computer Hub WS binary + lib |
| `crates/atlas-bot-gateway` | In-box gateway stub |
| `clients/pc/` | PC bot_client (Tauri 2 + TS) |
| `docs/P1-runbook.md` | Hub smoke |
| `docs/P2-runbook.md` | PC closed-loop smoke |
| `docs/P3-runbook.md` | Offbox + multi-agent + interrupt |
| `docs/SOURCE_REV.md` | Upstream pin |

## Quick check

```bash
cargo test -p xai-tool-protocol --test bot_relay_conformance
cargo test -p atlas-bot-hub -p atlas-bot-gateway
cargo test -p atlas-bot-hub --test p3_smoke -- --nocapture   # SMOKE_OK
cargo run -p atlas-bot-hub
```

PC client: see [`docs/P3-runbook.md`](./docs/P3-runbook.md).

## Out of scope (P3)

mcp-adapter, real IdP, mobile, CLI, VNC, production installers, real LLM.
