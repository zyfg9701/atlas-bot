# P3 runbook — offbox transcript + multi-agent + interrupt

## Scope

| Item | Behavior |
|------|--------|
| Cold `bot.transcript.offbox` | Hub serves cached pages `{ entries, nextCursor? `; **never** increments gateway invoke |
| Multi-agent | `createAgent` (min `name`) + `listAgents`; PC create/list/select |
| `interruptAgentRun` | Args include `agentId`; stub has interruptible running window after `sendPrompt` |
| Closed loop | New agent: subscribe   sendPrompt   `hub:turn_finished`   transcriptTail |

**Out of scope:** real IdP, mobile, VNC/attachments/groups, CLI/ACP, real LLM.

### Roster vs listAgents

- **Cold `bot.roster`**: hub in-memory cache (seed + synced on `createAgent` / turn finish / interrupt).
- **Hot `listAgents`**: live gateway inventory.
- In the in-process stub they stay aligned after create; if you point Hub at a remote HTTP gateway, refresh roster after creates or treat list as source of truth for IDs.

## Build / test

```bash
cargo test -p atlas-bot-gateway -p atlas-bot-hub
cargo test -p atlas-bot-hub --test p3_smoke -- --nocapture   # prints SMOKE_OK
cargo test -p xai-tool-protocol --test bot_relay_conformance
./tools/license-check/check.sh

cd clients/pc && npm install && npm run build && npm run test:unit
```

## Start Hub

```bash
RUST_LOG=info cargo run -p atlas-bot-hub
# ws://127.0.0.1:7700/ws
```

Stub notes:

- `sendPrompt` enters **running** for ~300ms (interruptible), then echoes and emits `hub:turn_finished`.
- Pass `"immediate": true` in sendPrompt args (PC checkbox) to skip the window.
- Offbox page size is **2** entries (stable two-page smoke).

## Automated smoke (`SMOKE_OK`)

```bash
cargo test -p atlas-bot-hub --test p3_smoke -- --nocapture
# expect: SMOKE_OK p3 dual-agent+offbox-page+interrupt
```

Covers: create second agent + list/roster, offbox two-page cold read (no gateway wake), interrupt in-flight run then sendPrompt again.

## PC UI click-path

```bash
# terminal A
cargo run -p atlas-bot-hub

# terminal B
cd clients/pc && npm run preview   # or: npm run tauri -- dev
```

1. **Connect + hello** — capabilities include `bot.transcript.offbox`.
2. **bot.status / bot.roster** — cold (Hub logs `cold ...`, no `gateway invoke`).
3. **createAgent** (name e.g. `Scout`) — appears in roster/select; auto-subscribe.
4. Switch between `agt_1` and new agent — events/transcript follow current id (no cross-talk); seq per `(connection, agent)`.
5. On new agent: **sendPrompt** -> turn_finished -> **getAgentTranscriptTail**.
6. **offbox page / offbox next** — cold offline transcript pagination for selected agent.
7. Uncheck immediate -> **sendPrompt** -> quickly **interruptAgentRun** -> send again.

## Protocol compliance

- Cold: `bot.status` / `bot.roster` / `bot.transcript.offbox` — not `bot.command`, no gateway invoke.
- Hot: only `bot.command` wakes gateway.
- Closed error set; unknown codes degrade in PC UI; `agentId` mismatch → `command_rejected` / `agent_id_mismatch` (no silent rewrite).
- Seq gaps never auto-resync; only explicit `hub:resync_required`.

## Out of scope (P3)

Android/iOS, VNC, attachments, groups/channels, real IdP, Atlas CLI, hub-mcp-adapter, production installers.
