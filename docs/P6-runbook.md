# P6 runbook — Bot-Relay thin CLI (β′)

> **Bot-Relay ≠ ACP.** This phase ships an in-repo CLI that connects as
> `kind=bot_client` to the existing Hub WebSocket and speaks Hub `bot.*`
> methods. It is **not** an ACP adapter, **not** a vendor of `xai-acp-lib` /
> `atlas-relay-demo`, and **not** 100% compatible with the official `atlas`
> binary (which speaks ACP). Scheme **α** (official atlas zero-change ACP
> adapter on a separate port) is **not done** in P6.

Baseline: `main` @ `00b51f4` (P5 + CI). Branch: `p6-cli-bridge`.

## Deliverable

| Path | Role |
|------|------|
| `crates/atlas-bot-cli` | Rust binary + lib (`atlas-bot-cli`) |
| `docs/P6-runbook.md` | This page |
| `crates/atlas-bot-cli/tests/p6_smoke.rs` | In-process Hub WS smoke |

## Install / run

```bash
# Build
cargo build -p atlas-bot-cli

# Start Hub (stub gateway if ATLAS_GATEWAY_URL unset)
RUST_LOG=info cargo run -p atlas-bot-hub
# listens ws://127.0.0.1:7700/ws

# Point CLI at Hub (default URL; override with --url or ATLAS_HUB_WS)
cargo run -p atlas-bot-cli -- status
cargo run -p atlas-bot-cli -- list
cargo run -p atlas-bot-cli -- list --hot
cargo run -p atlas-bot-cli -- prompt "hello from p6"
cargo run -p atlas-bot-cli -- chat "same as prompt"
cargo run -p atlas-bot-cli -- interrupt
```

Env / flags:

- `ATLAS_HUB_WS` / `--url` — default `ws://127.0.0.1:7700/ws`
- `--agent` / `-a` — default `agt_1`

## Verb mapping

| CLI | Wire |
|-----|------|
| `status` | raw hello (`kind=bot_client`) + cold `bot.status` |
| `list` | cold `bot.roster`; `--hot` also `bot.command`/`listAgents` |
| `prompt` / `chat` | `bot.subscribe` → `sendPrompt` → wait `hub:turn_finished` → `getAgentTranscriptTail` |
| `interrupt` | `bot.command` / `interruptAgentRun` |

Cold paths **never** go through `bot.command`. Unknown wire error codes degrade to an `upstream_error` display; the CLI does not invent protocol-external codes.

## Smoke

```bash
cargo test -p atlas-bot-cli --test p6_smoke -- --nocapture
# expect:
# SMOKE_OK p6 cli-bridge hello+status+list+prompt+turn_finished+transcript
```

Also in CI `hub-smoke` job (see `.github/workflows/protocol-conformance.yml`).

## Boundary gates

- Hub `hello_ack.capabilities` stay the Bot-Relay set — **no** ACP method names.
- Workspace does **not** vendor whole-tree `xai-acp-lib` or copy in `atlas-relay-demo`.
- Bot-Relay conformance fixtures unchanged by this CLI.

## Optional real gateway

Point Hub at a running gateway (`ATLAS_GATEWAY_URL=http://127.0.0.1:8787`) and use the same CLI. Stub echo replies are enough for P6 hard gate; non-echo CLI gateway is a bonus (see P3.5).

## Known limitations (§7)

1. **Binary / install:** crate binary name `atlas-bot-cli`; run via `cargo run -p atlas-bot-cli` or install with `cargo install --path crates/atlas-bot-cli`.
2. **interrupt:** implemented (`interruptAgentRun`). Against idle agent returns `hadActiveRun=false` (honest; never fakes success).
3. **vs official `atlas`:** this is a **side-path Bot-Relay CLI**, not ACP; official `atlas` will not speak to `:7700` without a future α adapter (e.g. `:8790`).
4. **α not done;** no real IdP; not a product shell replacement for PC.

## Out of scope (P6)

α ACP adapter, whole-tree ACP vendor, stuffing ACP into `bot.*`, real IdP, store packages, group channels.
