# atlas-bot

Private Grok Bot stack (Bot-Relay): `bot_client` ↔ Computer Hub ↔ Box gateway.

## Status (P6 β′ in progress on branch)

- Monorepo + vendored `xai-tool-protocol` @ `a549186d…`
- **Hub** WebSocket JSON-RPC (`crates/atlas-bot-hub`) — cold offbox + hot `bot.vncDescriptor`
- **Gateway** stub + CLI adapter + optional OpenAI — uploadAttachment / attachUpload stubs + VNC placeholder
- **PC bot_client** Tauri 2 + TypeScript — Open desktop + file upload
- License hygiene + protocol conformance CI; PC frontend CI (`pc-client.yml`)

## Layout

| Path | Role |
|------|------|
| `third_party/xai-tool-*` | Vendored Bot-Relay protocol |
| `crates/atlas-bot-hub` | Computer Hub WS binary + lib |
| `crates/atlas-bot-gateway` | Stub + CLI/OpenAI gateway backends |
| `crates/atlas-bot-cli` | P6 β′ thin Bot-Relay CLI (`bot_client` → Hub WS; ≠ ACP) |
| `clients/pc/` | PC bot_client (Tauri 2 + TS) |
| `docs/P1-runbook.md` … `docs/P6-runbook.md` | Phase runbooks |
| `docs/SOURCE_REV.md` | Upstream pin |

## Quick check

```bash
cargo test -p xai-tool-protocol --test bot_relay_conformance
cargo test -p atlas-bot-hub -p atlas-bot-gateway
cargo test -p atlas-bot-hub --test p5_smoke -- --nocapture   # SMOKE_OK p5 …
cargo test -p atlas-bot-cli --test p6_smoke -- --nocapture  # SMOKE_OK p6 …
cargo run -p atlas-bot-hub
```

P5 VNC/attachments: see [`docs/P5-runbook.md`](./docs/P5-runbook.md).

## Out of scope (P5)

Real noVNC cluster, IdP, groups/channels, `readAttachment*`, production installers, vendor grok runtime.
