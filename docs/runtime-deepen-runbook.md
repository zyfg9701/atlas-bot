# Runtime deepen runbook (R2)

Baseline: `main` @ `ba10794` (IdP I1 + R1 Box Sidecar). Stage: **运行时加深**.

R2 deepens the Box Sidecar (tools + mid-turn `bot.event` + VNC/attach on the same
workspace). **Does not** change `bot.command` / capabilities wire shapes, vendor
grok-build, or move IdP off Hub.

## Architecture

```text
bot_client ──WS──► Hub
                     ├ cold: status / roster / transcript.offbox
                     └ hot:  bot.command / bot.vncDescriptor
                              │
                              ▼
                     Gateway backend = box | cli | openai | stub
                              │
                     BoxSidecar (when backend=box)
                       workspace: ATLAS_BOX_WORKSPACE/<agentId>/
                       RuntimeHint ──B2──► Hub fan-out bot.event
```

**B2 (default / smoke):** in-process `RuntimeHint` broadcast bridged by
`Hub::spawn_turn_bridge`. Deepen smoke uses Hub+Box in-process.

**B1 (HTTP-separated):** when Hub uses `ATLAS_GATEWAY_URL` against a standalone
`atlas-bot-gateway` (`backend=box`), set gateway `ATLAS_HUB_EVENT_URL` to Hub
loopback ingest (`POST /internal/runtime-hint`, default bind `127.0.0.1:7701`)
so mid-turn `hub:tool` / `hub:assistant_delta` reach subscribers. See
`docs/b1-event-ingest-runbook.md`. Without `ATLAS_HUB_EVENT_URL`, sync
`sendPrompt` + `hubEmitTurnFinished` still work (no mid-turn). Deepen smoke
(`runtime_deepen_smoke`) remains the in-process **B2** path; B1 has `b1_smoke`.

## Env

| Var | Role |
|-----|------|
| `ATLAS_GATEWAY_BACKEND` | `box` for deepen path; `cli` / `openai` / `stub` coexist |
| `ATLAS_BOX_WORKSPACE` | Box root (default `./data/box-workspace`); agent dir `<root>/<agentId>/` |
| `ATLAS_BOX_TURN_DELAY_MS` | Interruptible sendPrompt delay (default `50`) |
| `ATLAS_VNC_MODE` | `stub` (default) \| `proxy` |
| `ATLAS_VNC_UPSTREAM` | Optional `host:port`; absent → stub page (never fake desktop) |
| `ATLAS_VNC_STUB_BASE` | Public base for minted URLs (default `http://127.0.0.1:8787`) |
| `ATLAS_ATTACH_TTL_SECS` | Attachment TTL (default 24h); Box stores under workspace |
| `ATLAS_GATEWAY_URL` | Hub → remote gateway (pair with B1 `ATLAS_HUB_EVENT_URL` for mid-turn) |
| `ATLAS_HUB_EVENT_URL` | Gateway → Hub ingest (B1); see b1-event-ingest-runbook |
| `ATLAS_AUTH_MODE` | Unchanged (Hub-only IdP) |

## Tool whitelist (Box)

Case-sensitive substrings inside `sendPrompt` text:

| Trigger | Behavior |
|---------|----------|
| `LIST_DIR` | List agent workspace (no escape) |
| `READ_FILE <rel>` | Read UTF-8 text ≤64KiB under agent root |
| `WRITE_FILE <rel> <<< body` or `WRITE_FILE <rel> body` | Write/overwrite ≤64KiB |
| `RUN ls` / `RUN pwd` | Allowlisted shell, cwd = agent root |
| `RUN cat <rel>` | Read via allowlisted cat semantics (same sandbox) |
| `RUN mkdir <rel>` | Create subdir under agent root |

**Rejects (stable error in transcript + optional `hub:tool`):**

- Path with `..` or absolute escape outside `ATLAS_BOX_WORKSPACE/<agentId>/`
- Text over 64KiB (`text exceeds … byte cap`)
- Any other `RUN <cmd>` (e.g. `RUN curl`) → `command not on whitelist: <cmd>`

Evidence lines: `[tool:list_dir …]`, `[tool:read_file …]`, `[tool:write_file …]`,
`[tool:shell …]`.

## Event channels

| Channel | When |
|---------|------|
| `hub:tool` | Each tool invocation (name + summary + exitCode) |
| `hub:assistant_delta` | Small text chunk before turn completes |
| `hub:turn_finished` | End of turn (P3.5/R1 compatible) |

Unsubscribed clients: sync `sendPrompt` result only (R1-compatible).

## VNC / attachments when `backend=box`

- `bot.vncDescriptor` → **Box** mints URL (shared helper with InMemory P5).
- No `ATLAS_VNC_UPSTREAM` → `/vnc-stub?agent=…` stub page.
- `uploadAttachment` → `ATLAS_BOX_WORKSPACE/<agentId>/uploads/<uploadId>/` + `meta.json`.
- Missing / expired → closed-set `attachment_not_found`.
- TTL: same default as P5 (`ATLAS_ATTACH_TTL_SECS`); files live under workspace.

When `backend!=box`, InMemory/HTTP P5 paths are unchanged (`p5_smoke` / `p5r_smoke`).

## Start (box)

```bash
ATLAS_GATEWAY_BACKEND=box \
ATLAS_BOX_WORKSPACE=./data/box-workspace \
ATLAS_GATEWAY_HTTP_BIND=127.0.0.1:8787 \
  cargo run -p atlas-bot-gateway

ATLAS_GATEWAY_URL=http://127.0.0.1:8787 \
  cargo run -p atlas-bot-hub
```

For B2 streaming locally without HTTP split, embed Box in tests via
`Hub::new(BoxSidecarGateway)` + `spawn_turn_bridge` (see smoke).

## Smoke

```bash
cargo test -p atlas-bot-hub --test runtime_deepen_smoke -- --nocapture --test-threads=1
# Expect:
#   SMOKE_OK runtime-deepen tools…
#   SMOKE_OK runtime-deepen events…
#   SMOKE_OK runtime-deepen box-vnc-attach…

# B1 split-process mid-turn (see docs/b1-event-ingest-runbook.md):
cargo test -p atlas-bot-hub --test b1_smoke -- --nocapture --test-threads=1
```

Also keep green: `runtime_smoke`, `p35_smoke`, `p5_smoke`, `p5r_smoke`,
`idp_smoke`, `p6_smoke`, `alpha_smoke`.

## Known limitations (§7)

1. **Tool table:** LIST_DIR, READ_FILE, WRITE_FILE, RUN ls/pwd/cat/mkdir.
   Not done: arbitrary shell, curl, package managers, MCP, LSP, browser automation.
2. **Event channels:** `hub:tool`, `hub:assistant_delta`, `hub:turn_finished`.
   Bridging: **B2 in-process** (`spawn_turn_bridge`) or **B1** loopback ingest
   (`ATLAS_HUB_EVENT_URL`). Deepen smoke uses B2; split-process smoke is `b1_smoke`.
3. **Attachment TTL:** Aligns with P5 default TTL (`ATLAS_ATTACH_TTL_SECS`, 24h)
   with files rooted under the agent workspace (`…/uploads/`).
4. **Model:** Deterministic local responder (history + tool evidence). Not an LLM;
   not grok-build.
5. **VNC:** Stub-only without `ATLAS_VNC_UPSTREAM`; proxy token URL when
   `ATLAS_VNC_MODE=proxy` + upstream set (same P5 mock page semantics).

## Out of scope (R2)

I2 login UI, arbitrary shell / MCP suite, multi-box VNC cluster, vendor grok-build,
changing `bot.command` shape, user auth on Gateway.
