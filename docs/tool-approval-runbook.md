# Tool approval runbook (GA1 box + CG1 cli)

> Gateway **local** gate for dangerous tools.  
> Baseline: `main` @ CG1 · **zero** new `bot.command` · product default backend remains **`cli`**.  
> Not GA2 (Hub approve method), not product YOLO, not box-style zero side-effects on cli.

## 1. Gate position (box — GA1)

```text
sendPrompt (backend=box)
        │
        ▼
  classify tool
        ├─ exempt: LIST_DIR / READ_FILE / RUN ls / RUN pwd  → execute → hub:tool (after)
        └─ gated:  WRITE_FILE / RUN mkdir / RUN cat
                │
                ├─ mode=off        → execute (documented; not product default)
                ├─ mode=auto_deny  → Deny immediately (CI)
                └─ mode=gate       → hang pending → Allow | Deny | Timeout(=Deny)
```

**Sync invoke semantics (nailed):** `sendPrompt` / HTTP `/invoke` **holds until** Allow, Deny, or timeout. No early `accepted` + async complete in GA1.

Decisions return to the gateway only (`POST /approve` or in-process hook). Hub may **display** pending; it does **not** own the decision (no new `bot.command`).

## 2. Exempt vs gated (box — GA1)

| Tool | Policy |
|------|--------|
| `WRITE_FILE` | **Gated** (default deny pending) |
| `RUN mkdir` | **Gated** |
| `RUN cat` | **Gated** |
| Other non-whitelist `RUN` | Closed reject (unchanged) |
| `LIST_DIR` | **Exempt** |
| `READ_FILE` | **Exempt** |
| `RUN ls` / `RUN pwd` | **Exempt** |

Unlisted write/dangerous → treat as gated when added later.

**No Allow → zero workspace side effects** (no write, no mkdir) — **box only**.

## 3. Allow / Deny / Timeout (shared closed-set)

| Outcome | Behavior | Grep |
|---------|----------|------|
| Allow | Continue (box: execute; cli: do not kill, keep reading stream) | `approval=allow` |
| Deny | Closed reason; **cli: interrupt/kill child** | `tool_approval_denied` |
| Timeout | Same as Deny | `tool_approval_timeout` |
| `auto_deny` | Immediate Deny | `tool_approval_auto_deny` |

Hang / channel failure / kill failure / unknown error → **Deny** (never silent Allow).  
Repeat or unknown `approvalId` → closed reject (`approval_closed` / HTTP 409).

## 4. Env

| Env | Default | Role |
|-----|---------|------|
| `ATLAS_TOOL_APPROVAL_MODE` | **`gate`** for box **and cli** `from_env` / product | `gate` \| `auto_deny` \| `off` |
| `ATLAS_TOOL_APPROVAL_TIMEOUT_MS` | **`30000`** | Hang deadline; timeout=Deny |
| `ATLAS_TOOL_APPROVAL_TOKEN` | unset | Optional shared token for `/approve` |
| `ATLAS_AGENT_CLI_STREAM` | unset (text) | **CG1 coverage prerequisite** — must be `1`/`true`/`yes` to gate mid-turn `tool_call` |

- `off` is documented for deepen/legacy in-process tests and explicit bypass; **not** the product narrative.
- In-process `BoxSidecarGateway::new*` / `CliAgentGateway::new*` without env defaults to **`off`** so R2/RR1/cli smokes stay green; set `MODE=gate` in approval smoke / product.
- **§7 filled:** cli `from_env` default `ATLAS_TOOL_APPROVAL_MODE` = **`gate`** (feasibility alignment).

## 5. `POST /approve`

```http
POST /approve
Content-Type: application/json

{ "approvalId": "appr_…", "decision": "allow" | "deny" }
```

- **Loopback-only** (`127.0.0.1` / `::1`). Non-loopback → 403.
- Optional token: `Authorization: Bearer <ATLAS_TOOL_APPROVAL_TOKEN>` or `X-Atlas-Approval-Token`.
- Failures never become Allow.
- Discoverability: `GET /healthz` includes `tool_approval_mode`, `tool_approval_gate` (boolean), `tool_approval_token_configured` (no secret echo) for **both** `backend=box` and `backend=cli`.

## 6. GA1b pending hint

Thin reuse of existing `bot.event` / `hub:tool` (no new bot method):

- `tool`: `approval_pending`
- `exitCode`: `null`
- `summary` prefix: `[approval_pending approvalId=… tool=…] …`

PC / operators may parse this for a thin Allow/Deny card. Post-exec `hub:tool` remains the after-the-fact evidence channel (distinct from pending).

## 7. PC thin card

Chat shows a small approval card when a pending `hub:tool` arrives. Allow/Deny `POST` to the configured **gateway HTTP base** (Advanced field; default `http://127.0.0.1:8787`). No Chat/Debug IA rearrange. No new Hub wire.

**CG1:** **reuse** the same card (zero or copy-only) — cli pending uses the identical `[approval_pending …]` prefix.

## 8. Smoke / CI

```bash
# GA1 box
cargo test -p atlas-bot-hub --test tool_approval_smoke -- --nocapture --test-threads=1
# expect: SMOKE_OK tool-approval …

# CG1 cli
cargo test -p atlas-bot-hub --test tool_approval_cli_smoke -- --nocapture --test-threads=1
# expect: SMOKE_OK tool-approval-cli …
```

Wired in `protocol-conformance` like `runtime_deepen_smoke` / `private_resident_smoke`.

---

## 9. CG1 — cli dangerous-command gate

### 9.1 Position

```text
sendPrompt (backend=cli, product default)
        │
        ▼
  spawn ATLAS_AGENT_CLI (-p [, stream-json])
        │
        ├─ text mode → no mid-turn tool_call → CG1 does NOT claim coverage
        │
        └─ stream (ATLAS_AGENT_CLI_STREAM=1): line-read stdout
                │
                └─ tool_call started / ATLAS_TOOL (no exit code)
                        ├─ exempt name → emit hub:tool only
                        └─ gated / unknown → hang PendingApproval
                                ├─ emit [approval_pending …] (GA1b)
                                ├─ Allow → do not kill; continue stream
                                └─ Deny / Timeout / auto_deny → interrupt/kill child
                                                   + closed-set reason
```

**Hard boundary:** CG1 = **stream observation gate + Deny kills process**. It does **not** claim box-style “zero side-effects before Allow”. If CLI already started a tool before `started` reaches stdout, gateway best-effort kills the child.

### 9.2 Stream prerequisite

| Mode | CG1 coverage |
|------|----------------|
| `ATLAS_AGENT_CLI_STREAM=1` + stream-json / ATLAS_TOOL | **Yes** — dangerous `started` gated |
| Text / stream off | **No** — cannot see mid-turn tools; do not claim coverage |
| CLI tools that never emit to stdout | **No** |

### 9.3 Exempt vs gated (cli — case-insensitive on `name`)

| Policy | Names / heuristic |
|--------|-------------------|
| **Exempt** | `Read` / `ReadFile` / `Grep` / `Glob` / `LS` / `ListDir` / `SemanticSearch` / `WebSearch` / `fetch` / `WebFetch` |
| **Gated** | `Shell` / `Bash` / `Run` / `Write` / `WriteFile` / `Edit` / `Delete` / `Remove` / `ApplyPatch` / `NotebookEdit` / write-ish MCP (`mcp` + write/exec keywords in name/summary) |
| **Gated (default)** | **Unknown** `tool_call` name |
| **Observe only** | `tool_call` `status=completed` (already finished — do not re-gate) |

### 9.4 Allow / Deny / Timeout (cli)

| Decision | cli behavior |
|----------|----------------|
| Allow | Do not kill; continue reading stream; may emit `approval=allow` on hub:tool |
| Deny | **interrupt/kill** child; closed `tool_approval_denied`; turn ends with greppable reason |
| Timeout | Same as Deny; `tool_approval_timeout` |
| Hang / channel / kill failure | **Deny** (never silent Allow) |

Deny kills the **whole turn** (CG1 boundary — accepted).

### 9.5 vs box differences

| Dimension | box (GA1) | cli (CG1) |
|-----------|-----------|-----------|
| Product default backend | No (bypass) | **Yes (primary)** |
| Execution ownership | gateway/box runs WRITE/RUN | External `ATLAS_AGENT_CLI` black box |
| Trigger | prompt markers `WRITE_FILE` / `RUN …` | stdout stream-json `tool_call` / `ATLAS_TOOL` |
| Hang semantics | Allow before **zero workspace side effects** | Allow = don’t kill; Deny = **kill**; **no** zero-first-side-effect guarantee |
| Stream dependency | None | **Required** for coverage |
| Exempt | LIST/READ/ls/pwd | Read/Grep/Glob/LS/… (§9.3) |
| Gated | WRITE / RUN mkdir\|cat | Shell/Write/Edit/Delete/unknown (§9.3) |
| Decision channel | `POST /approve` + PC card | **Same** (reuse) |
| `bot.command` | Zero new | Zero new |
| healthz | `tool_approval_*` | **Same** (CG1 wired) |
| YOLO / GA2 | No | No |

### 9.6 Known limits (§7)

- `started` may arrive after CLI already wrote disk → DoD = best-effort kill.
- Real Cursor/Atlas `name` set: closed table + **unknown default gated**.
- Deny kills whole turn (documented).
- cli `from_env` default MODE = **`gate`**.
- When stream is off but MODE≠off, gateway logs a strong warn (text cannot gate).

## 10. Explicitly out of GA1/CG1

- GA2 Hub `approval_request` + `bot.command` approve  
- Product YOLO / session auto-allow  
- Changing default `backend=cli` → box  
- Claiming text mode can gate tools  
- Claiming cli has box-style zero side-effects before Allow  
- CLI-native hook / ACP approval (separate page)  
- Copying OpenMausBot / Composio protocols  
- P1 desktop / IA rearrange  

## 11. One-liner for cli-primary

**Default remains `backend=cli`; dangerous-command human gate (CG1) requires `ATLAS_AGENT_CLI_STREAM=1` — see [`tool-approval-runbook.md`](./tool-approval-runbook.md) § cli/CG1 (observation + kill; not box zero side-effects). Box GA1 gate documented above.**
