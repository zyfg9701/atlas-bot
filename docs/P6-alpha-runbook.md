# P6-α runbook — minimal ACP adapter

> **α ≠ β′.** β′ (`atlas-bot-cli`) speaks Hub `bot.*` directly.
> α (`atlas-acp-adapter`) exposes a **frozen ACP JSON-RPC subset** on an
> independent port and translates inward to the same Hub as a normal
> `bot_client`. Official `atlas` must **not** point at Hub `:7700`; point it
> at the adapter (`:8790`).
>
> **Subset disclaimer:** this is **not** 100% official ACP / atlas
> compatible. Filesystem, terminal, MCP, diff, permissions, and complex
> multi-session are **out of scope**. Unknown methods return a stable
> JSON-RPC `-32601` error (no crash, no silent swallow).
>
> **Hard gates:** ACP method names are **not** added to Hub
> `hello_ack.capabilities`. Workspace does **not** vendor whole-tree
> `xai-acp-lib` or `atlas-relay-demo`.

Baseline: `main` @ `906ccbd` (P6 β′). Branch: `alpha-acp-adapter`.

Pinned probe target: public ACP session/prompt shape
([agentclientprotocol.com](https://agentclientprotocol.com)) — α freezes the
whitelist below; re-validate against your local `atlas` CLI version when
hand-testing `--grok-ws-url`.

## Topology

```text
official atlas CLI  (or fake ACP client)
    │  ACP JSON-RPC / WS
    ▼
crates/atlas-acp-adapter   127.0.0.1:8790   (ATLAS_ACP_BIND)
    │  Bot-Relay bot_client
    ▼
atlas-bot-hub              127.0.0.1:7700   ← PC / mobile / β′ still direct
    │
    ▼
Gateway (stub / P3.5)
```

Adapter failure must not take down Hub (separate process / port).

## Deliverables

| Path | Role |
|------|------|
| `crates/atlas-acp-adapter` | Adapter binary + lib |
| `docs/P6-alpha-runbook.md` | This page (whitelist + run) |
| `crates/atlas-acp-adapter/tests/alpha_smoke.rs` | Fake ACP client smoke |

## Frozen ACP method whitelist

| ACP method | Inward Bot-Relay mapping | Notes |
|------------|--------------------------|-------|
| `initialize` | Hub hello (already connected as `bot_client`); return α caps | Logs `[acp→bot] initialize → hub hello` |
| `agents/list` | cold `bot.roster`; optional hot `listAgents` | List / inspect agents |
| `session/new` | Bind session ↔ `agentId` (select) | Default agent `agt_1` |
| `session/prompt` | `bot.subscribe` → `sendPrompt` → wait `hub:turn_finished` → `getAgentTranscriptTail` | Prompt text: string **or** ACP content blocks `[{type,text}]` |
| `session/cancel` | `interruptAgentRun` | Optional interrupt |
| `transcript/tail` | `getAgentTranscriptTail` | Explicit hot-tail fetch |

**Not implemented (explicit error):** `fs/*`, `terminal/*`, MCP, `session/load`,
`session/resume`, permissions, auth login, etc.

Logs: look for lines containing `[acp→bot]`.

## How to run

```bash
# Terminal A — Hub (stub gateway if ATLAS_GATEWAY_URL unset)
RUST_LOG=info cargo run -p atlas-bot-hub
# listens ws://127.0.0.1:7700/ws

# Terminal B — α adapter
RUST_LOG=info cargo run -p atlas-acp-adapter
# listens 127.0.0.1:8790  (WS on / and /ws)

# Env overrides
# ATLAS_ACP_BIND=127.0.0.1:8790
# ATLAS_HUB_WS=ws://127.0.0.1:7700/ws
```

β′ CLI still works unchanged against `:7700`:

```bash
cargo run -p atlas-bot-cli -- status
cargo run -p atlas-bot-cli -- prompt "still bot.* direct"
```

## Point official atlas at :8790

Official Atlas CLI typically takes a Grok / ACP WebSocket URL (e.g.
`--grok-ws-url` or equivalent config). Point it at the **adapter**, not Hub:

```bash
# illustrative — exact flag depends on your atlas build
atlas --grok-ws-url ws://127.0.0.1:8790/ws
# or:  ATLAS_GROK_WS_URL=ws://127.0.0.1:8790/ws atlas …
```

Hand-probe with the fake smoke client is the hard gate; official binary
click-test is optional Evidence (record CLI version if done).

## Smoke

```bash
cargo test -p atlas-acp-adapter --test alpha_smoke -- --nocapture
# expect:
# SMOKE_OK alpha acp-adapter hello+list+prompt+turn_finished+transcript
```

Also wired into CI `hub-smoke` (`.github/workflows/protocol-conformance.yml`).
The smoke asserts Hub `capabilities` contain **no** `acp` substring and that
an unknown ACP method returns JSON-RPC `-32601`.

## Boundary gates

- Hub `hello_ack.capabilities` remain Bot-Relay only — **no** ACP pollution.
- No whole-tree `xai-acp-lib` / `atlas-relay-demo` in this workspace.
- bot_relay conformance fixtures unchanged; Hub wire unchanged for ACP.
- PC / mobile / β′ keep talking to `:7700`.

## Known limitations (§7)

1. **Frozen whitelist:** `initialize`, `agents/list`, `session/new`,
   `session/prompt`, `session/cancel`, `transcript/tail` only.
2. **interrupt:** implemented via `session/cancel` → `interruptAgentRun`.
   Idle agent returns `hadActiveRun=false` (honest).
3. **Official atlas hand-probe:** smoke uses a **fake ACP client**; official
   binary pointing at `:8790` is optional Evidence, not the hard gate.
4. **Gap vs full ACP:** α covers initialize + list/select + one prompt
   closed-loop (+ cancel/tail). No fs/terminal/MCP/diff/permissions/multi-session
   fidelity. **Not** a drop-in for every atlas feature.
5. **α ≠ β′;** no real IdP.

## Out of scope (α)

100% ACP compatibility; stuffing ACP into Hub `bot.*`; whole-tree ACP vendor;
replacing β′ / PC; real IdP.
