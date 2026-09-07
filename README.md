# atlas-bot

Private Grok Bot stack (Bot-Relay): `bot_client` ↔ Computer Hub ↔ Box gateway.

## Status (P2)

- Monorepo + vendored `xai-tool-protocol` @ `a549186d…`
- **Hub** WebSocket JSON-RPC MVP (`crates/atlas-bot-hub`)
- **Gateway** in-memory stub (`crates/atlas-bot-gateway`)
- **PC bot_client** Tauri 2 + TypeScript (`clients/pc/`)
- License hygiene + protocol conformance CI; PC frontend CI (`pc-client.yml`)

## Layout

| Path | Role |
|------|------|
| `third_party/xai-tool-*` | Vendored Bot-Relay protocol |
| `crates/atlas-bot-hub` | Computer Hub WS binary + lib |
| `crates/atlas-bot-gateway` | In-box gateway stub |
| `clients/pc/` | P2 PC bot_client (Tauri 2 + TS) |
| `docs/P1-runbook.md` | Hub smoke |
| `docs/P2-runbook.md` | PC closed-loop smoke |
| `docs/SOURCE_REV.md` | Upstream pin |

## Quick check

```bash
cargo test -p xai-tool-protocol --test bot_relay_conformance
cargo test -p atlas-bot-hub -p atlas-bot-gateway
cargo run -p atlas-bot-hub
```

PC client: see `docs/P2-runbook.md`.

## Out of scope (P2)

mcp-adapter, real IdP, mobile, CLI, VNC, production installers.
