# Tool approval runbook (GA1 / P0-1)

> Gateway **local** gate for Box dangerous tools.  
> Baseline: `main` @ GA1 · **zero** new `bot.command` · product default backend remains **`cli`**.  
> Not GA2 (Hub approve method), not cli dangerous-command gate (phase 2), not product YOLO.

## 1. Gate position

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

## 2. Exempt vs gated

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

**No Allow → zero workspace side effects** (no write, no mkdir).

## 3. Allow / Deny / Timeout

| Outcome | Behavior | Grep |
|---------|----------|------|
| Allow | Execute tool; post-exec `hub:tool` with `approval=allow` | `approval=allow` |
| Deny | No side effect; closed reason | `tool_approval_denied` |
| Timeout | Same as Deny | `tool_approval_timeout` |
| `auto_deny` | Immediate Deny | `tool_approval_auto_deny` |

Hang / channel failure / unknown error → **Deny** (never silent Allow).  
Repeat or unknown `approvalId` → closed reject (`approval_closed` / HTTP 409).

## 4. Env

| Env | Default | Role |
|-----|---------|------|
| `ATLAS_TOOL_APPROVAL_MODE` | **`gate`** for box `from_env` / product | `gate` \| `auto_deny` \| `off` |
| `ATLAS_TOOL_APPROVAL_TIMEOUT_MS` | **`30000`** | Hang deadline; timeout=Deny |
| `ATLAS_TOOL_APPROVAL_TOKEN` | unset | Optional shared token for `/approve` |

- `off` is documented for deepen/legacy in-process tests and explicit bypass; **not** the product narrative for box.
- In-process `BoxSidecarGateway::new*` without env defaults to **`off`** so R2/RR1 smokes stay green; set `MODE=gate` in approval smoke / product.

## 5. `POST /approve`

```http
POST /approve
Content-Type: application/json

{ "approvalId": "appr_…", "decision": "allow" | "deny" }
```

- **Loopback-only** (`127.0.0.1` / `::1`). Non-loopback → 403.
- Optional token: `Authorization: Bearer <ATLAS_TOOL_APPROVAL_TOKEN>` or `X-Atlas-Approval-Token`.
- Failures never become Allow.
- Discoverability: `GET /healthz` includes `tool_approval_mode`, `tool_approval_gate` (boolean), `tool_approval_token_configured` (no secret echo).

## 6. GA1b pending hint (this PR: yes)

Thin reuse of existing `bot.event` / `hub:tool` (no new bot method):

- `tool`: `approval_pending`
- `exitCode`: `null`
- `summary` prefix: `[approval_pending approvalId=… tool=…] …`

PC / operators may parse this for a thin Allow/Deny card. Post-exec `hub:tool` remains the after-the-fact evidence channel (distinct from pending).

## 7. PC thin card

Chat shows a small approval card when a pending `hub:tool` arrives. Allow/Deny `POST` to the configured **gateway HTTP base** (Advanced field; default `http://127.0.0.1:8787`). No Chat/Debug IA rearrange. No new Hub wire.

## 8. Smoke / CI

```bash
cargo test -p atlas-bot-hub --test tool_approval_smoke -- --nocapture --test-threads=1
# expect: SMOKE_OK tool-approval …
```

Wired in `protocol-conformance` like `runtime_deepen_smoke` / `private_resident_smoke`.

## 9. Explicitly out of GA1

- GA2 Hub `approval_request` + `bot.command` approve  
- cli dangerous-command human gate  
- Product YOLO / session auto-allow  
- Changing default `backend=cli` → box  
- Copying OpenMausBot / Composio protocols  

## 10. One-liner for cli-primary

**Default remains `backend=cli`; Box tool approval gate (GA1) is documented in [`tool-approval-runbook.md`](./tool-approval-runbook.md).**
