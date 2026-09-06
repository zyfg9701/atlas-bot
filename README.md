# atlas-bot

Private Grok Bot stack (Bot-Relay): `bot_client` ↔ Computer Hub ↔ Box gateway.

## Status (P0)

- Monorepo skeleton + vendored `xai-tool-protocol` @ `a549186d…`
- Fixtures + conformance test wired for CI
- **No business logic** until license audit is green
- Atlas CLI ACP `agent/relay` is **out of MVP** (separate milestone)
- `hub-mcp-adapter` not in P0/P1

## Layout

See `docs/SOURCE_REV.md`, `third_party/`, stub crates under `crates/`.

## Quick check

```bash
cargo test -p xai-tool-protocol --test bot_relay_conformance
```
