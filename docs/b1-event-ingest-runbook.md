# B1 event ingest runbook (分进程流式)

Baseline: `main` @ `212b162` (I2.1). Stage: **分进程流式 B1**.

B1 moves `RuntimeHint` across the Hub ↔ standalone gateway process boundary so
HTTP-separated `backend=box` still fans out mid-turn `bot.event`
(`hub:tool` / `hub:assistant_delta`) plus `hub:turn_finished`. **Does not** add
`bot.*` methods, change `bot.command` shape, or expose ingest beyond loopback.

## Architecture

```text
bot_client ──WS──► Hub ──fan-out bot.event──► subscribers
                     ▲
                     │ POST loopback  ATLAS_HUB_EVENT_URL
                     │ /internal/runtime-hint
                     │
              atlas-bot-gateway (backend=box)
                     │ emit RuntimeHint
                     ├─ B2: in-process broadcast (only when Hub embeds / bridges)
                     └─ B1: HTTP POST RuntimeHint JSON (when EVENT_URL set)
```

| Path | mid-turn | turn_finished |
|------|----------|---------------|
| **B2** in-process `spawn_turn_bridge` | ✅ | ✅ (bridge owns; sync hubEmit suppressed) |
| **B1** Hub `ATLAS_GATEWAY_URL` + gateway `ATLAS_HUB_EVENT_URL` | ✅ via ingest | ✅ (B1 Finished +/or sync hubEmit; Hub dedupes) |
| HTTP split **without** EVENT_URL | ❌ | ✅ sync hubEmit only |

## B2 vs B1 mutex

- **Single-process / embed Box:** use **B2 only** (`Hub::spawn_turn_bridge`). Do **not** also set `ATLAS_HUB_EVENT_URL` on that same Box (would double mid-turn).
- **Hub + standalone gateway:** set Hub `ATLAS_GATEWAY_URL` and gateway `ATLAS_HUB_EVENT_URL` → **B1**. Do **not** also call `spawn_turn_bridge` on a co-located Box.
- Misconfig dual path: Hub dedupes identical `hub:turn_finished` (same agentId+preview within ~2s). Prefer correct mutex over relying on dedupe.

## Env

| Var | Role |
|-----|------|
| `ATLAS_HUB_EVENT_BIND` | Hub ingest listen (default `127.0.0.1:7701`). Must be **loopback**; non-loopback → fail-closed. `off` disables. |
| `ATLAS_HUB_EVENT_TOKEN` | Optional shared secret. If set, require `Authorization: Bearer …` **or** `X-Atlas-Event-Token`. |
| `ATLAS_HUB_EVENT_ALLOW_INSECURE_LOOPBACK` | When token unset, set `1` to accept unauthenticated loopback POSTs (tests / trusted localhost only). Otherwise ingest **rejects**. |
| `ATLAS_HUB_EVENT_URL` | Gateway → Hub POST URL, e.g. `http://127.0.0.1:7701/internal/runtime-hint` |
| `ATLAS_GATEWAY_URL` | Hub → gateway HTTP base (B1 split) |
| `ATLAS_GATEWAY_BACKEND` | `box` for deepen/B1 mid-turn tools |

Body size limit: **64 KiB**. POST timeout ≈ **500 ms**; failures **warn only** and never fail `sendPrompt`.

## Security

1. Ingest binds **loopback only** (default `127.0.0.1:7701`); `0.0.0.0` refused at start.
2. Prefer `ATLAS_HUB_EVENT_TOKEN` in any shared-host setup; insecure flag is for local smoke.
3. Ingest is **not** a user auth channel (users still use I1 Bearer on Hub WS).
4. Do **not** point bot_client at ingest; do **not** expose ingest publicly.

## Start (split process)

```bash
# Terminal A — standalone box gateway with B1 push
ATLAS_GATEWAY_BACKEND=box \
ATLAS_BOX_WORKSPACE=./data/box-workspace \
ATLAS_GATEWAY_HTTP_BIND=127.0.0.1:8787 \
ATLAS_HUB_EVENT_URL=http://127.0.0.1:7701/internal/runtime-hint \
ATLAS_HUB_EVENT_TOKEN=dev-shared-token \
  cargo run -p atlas-bot-gateway

# Terminal B — Hub WS + B1 ingest (no in-process Box bridge)
ATLAS_HUB_BIND=127.0.0.1:7700 \
ATLAS_HUB_EVENT_BIND=127.0.0.1:7701 \
ATLAS_HUB_EVENT_TOKEN=dev-shared-token \
ATLAS_GATEWAY_URL=http://127.0.0.1:8787 \
  cargo run -p atlas-bot-hub
```

Test-only without token:

```bash
ATLAS_HUB_EVENT_ALLOW_INSECURE_LOOPBACK=1
# and omit ATLAS_HUB_EVENT_TOKEN on both sides
```

## Smoke

```bash
cargo test -p atlas-bot-hub --test b1_smoke -- --nocapture --test-threads=1
# Expect: SMOKE_OK b1 event-ingest mid-turn+turn_finished…

# B2 regression:
cargo test -p atlas-bot-hub --test runtime_deepen_smoke -- --nocapture --test-threads=1
```

Negative (documented): Hub `ATLAS_GATEWAY_URL` + box gateway **without** `ATLAS_HUB_EVENT_URL` → no mid-turn events; sync result + `hub:turn_finished` still work.

## §8 nails

| Item | Value |
|------|-------|
| Bind | `127.0.0.1:7701` via `ATLAS_HUB_EVENT_BIND` |
| Token | Optional; without token require `ATLAS_HUB_EVENT_ALLOW_INSECURE_LOOPBACK=1` else reject |
| Dual path | Docs mutex; finished dedupe by agentId+preview (~2s) |
| POST timeout | ~500 ms; warn-only on failure |
| Finished | B1 also POSTs `kind=finished`; Hub fan-out; sync hubEmit may also fire → dedupe |

## Out of scope (B1)

I2.2 mobile login, public/non-loopback ingest, new `bot.*` methods, unloading B2, treating ingest as user auth.
