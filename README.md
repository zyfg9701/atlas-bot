# atlas-bot

Private Grok Bot stack (Bot-Relay): `bot_client` ↔ Computer Hub ↔ Box gateway.

## Status (P1)

- Monorepo + vendored `xai-tool-protocol` @ `a549186d…`
- **Hub** WebSocket JSON-RPC MVP (`crates/atlas-bot-hub`)
- **Gateway** in-memory stub (`crates/atlas-bot-gateway`)
- License hygiene: root `LICENSE` (Apache-2.0), expanded `NOTICE`, CI `license-check`
- Protocol conformance CI retained

## Layout

| Path | Role |
|------|------|
| `third_party/xai-tool-*` | Vendored Bot-Relay protocol |
| `crates/atlas-bot-hub` | Computer Hub WS binary + lib |
| `crates/atlas-bot-gateway` | In-box gateway stub |
| `docs/P1-runbook.md` | Smoke steps |
| `docs/SOURCE_REV.md` | Upstream pin |

## Quick check

```bash
cargo test -p xai-tool-protocol --test bot_relay_conformance
cargo test -p atlas-bot-hub -p atlas-bot-gateway
cargo run -p atlas-bot-hub
```

See `docs/P1-runbook.md` for the closed-loop smoke script.

## Out of scope (P1)

mcp-adapter, real IdP, PC UI, CLI, VNC.
