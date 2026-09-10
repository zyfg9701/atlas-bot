# atlas-bot

Private Grok Bot stack (Bot-Relay): `bot_client` ↔ Computer Hub ↔ Box gateway.

## Status (α ACP adapter on branch; P6 β′ on main)

- Monorepo + vendored `xai-tool-protocol` @ `a549186d…`
- **Hub** WebSocket JSON-RPC (`crates/atlas-bot-hub`) — cold offbox + hot `bot.vncDescriptor`
- **Gateway** stub + CLI adapter + optional OpenAI + **R1 Box Sidecar** — uploadAttachment / attachUpload stubs + VNC placeholder
- **PC bot_client** Tauri 2 + TypeScript — Open desktop + file upload
- License hygiene + protocol conformance CI; PC frontend CI (`pc-client.yml`)

## Layout

| Path | Role |
|------|------|
| `third_party/xai-tool-*` | Vendored Bot-Relay protocol |
| `crates/atlas-bot-hub` | Computer Hub WS binary + lib |
| `crates/atlas-bot-gateway` | Stub + CLI/OpenAI + R1 Box Sidecar backends |
| `crates/atlas-bot-cli` | P6 β′ thin Bot-Relay CLI (`bot_client` → Hub WS; ≠ ACP) |
| `crates/atlas-acp-adapter` | α minimal ACP subset adapter (`:8790` → Hub bot_client) |
| `clients/pc/` | PC bot_client (Tauri 2 + TS) |
| `docs/cli-primary-runbook.md` | **Recommended CLI primary path** (dev) |
| `docs/packaging-skeleton.md` | P1 dist/install/start skeleton (not store) |
| `docs/P1-runbook.md` … `docs/P6-*.md` / `runtime-boundary-runbook.md` | Phase + R1 runbooks |
| `docs/SOURCE_REV.md` | Upstream pin |

## Primary path (CLI)

Recommended stack: **Hub ↔ gateway(`backend=cli`) ↔ `ATLAS_AGENT_CLI`**.

```bash
./scripts/dev-cli-stack.sh
# or without a real agent:
ATLAS_AGENT_CLI=tools/mock-cli/mock-atlas-agent-cli.sh ./scripts/dev-cli-stack.sh
```

Then PC Connect → Send. Full guide: [`docs/cli-primary-runbook.md`](./docs/cli-primary-runbook.md).
Stub / box / openai are **side paths** (see runbook table).

## Packaging skeleton (P1 · not a store release)

Local `dist/` layout + install/start scripts: [`docs/packaging-skeleton.md`](./docs/packaging-skeleton.md).

```bash
./scripts/pack-dist.sh && ./scripts/install-local.sh
# then: ~/atlas-bot/scripts/start-cli-stack.sh  +  open ~/atlas-bot/pc/atlas-bot-pc
```

**Not** store listing / notarization / WeCom tickets / MSI. Unsigned yellow prompts OK. Does **not** change `bot.*`. Does **not** ship external `agent`. Dev path remains `scripts/dev-cli-stack.*` + `cargo run`.

## Quick check

```bash
cargo test -p xai-tool-protocol --test bot_relay_conformance
cargo test -p atlas-bot-hub -p atlas-bot-gateway
cargo test -p atlas-bot-hub --test p5_smoke -- --nocapture   # SMOKE_OK p5 …
cargo test -p atlas-bot-cli --test p6_smoke -- --nocapture  # SMOKE_OK p6 …
cargo test -p atlas-acp-adapter --test alpha_smoke -- --nocapture  # SMOKE_OK alpha …
cargo test -p atlas-bot-hub --test runtime_smoke -- --nocapture  # SMOKE_OK runtime …
cargo run -p atlas-bot-hub
```

P5 VNC/attachments: see [`docs/P5-runbook.md`](./docs/P5-runbook.md).

## Out of scope (P5)

Real noVNC cluster, IdP, groups/channels, `readAttachment*`, production/store installers (P1 is dist+scripts skeleton only — see packaging-skeleton), vendor grok runtime.
