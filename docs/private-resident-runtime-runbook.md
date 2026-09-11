# Private-resident runtime runbook (RR1)

> Deepens `backend=box` with optional OpenAI-compat LLM + multi-turn
> `history`→`messages[]`. **Product default remains `backend=cli`.**  
> Baseline: R1/R2 Box (`docs/runtime-boundary-runbook.md`,
> `docs/runtime-deepen-runbook.md`) · cli primary
> (`docs/cli-primary-runbook.md`).  
> **Not in RR1:** new `bot.*` / Hub wire, `backend=resident` (RR2), vendor
> grok, enterprise ops B, store signing C, forcing CI onto a real LLM.

---

## 1. Defaults (do not change)

| Setting | Value |
|---------|-------|
| `ATLAS_GATEWAY_BACKEND` (unset) | **`cli`** |
| Recommended PC / packaging path | **cli-primary** |
| Box | Optional private-resident bypass |

**Never** change the binary default to `box` to “get a model.”

---

## 2. Start Box with optional LLM

```bash
# Optional bypass — not the product default
ATLAS_GATEWAY_BACKEND=box \
ATLAS_BOX_WORKSPACE=./data/box-workspace \
ATLAS_BOX_LLM_BASE_URL=http://127.0.0.1:11434/v1 \
ATLAS_BOX_LLM_API_KEY= \
ATLAS_BOX_LLM_MODEL=llama3.2 \
cargo run -p atlas-bot-gateway
```

| Env | Role |
|-----|------|
| `ATLAS_BOX_LLM_BASE_URL` | OpenAI-compat base (…`/v1`); required to enable LLM |
| `ATLAS_BOX_LLM_API_KEY` | May be empty for local; **never** logged / never in `/healthz` |
| `ATLAS_BOX_LLM_MODEL` | Model id; required with base URL |
| `ATLAS_BOX_LLM_MODE` | `mock` \| `off` → force today’s deterministic `compose_reply` |
| `ATLAS_BOX_LLM_TIMEOUT_MS` | HTTP timeout (default `60000`) |
| `ATLAS_BOX_HISTORY_MAX_TURNS` | Max user+assistant **pairs** in memory (default `32`; oldest dropped) |
| `ATLAS_BOX_SESSION_PERSIST` | `1`/`true` → soft append `ATLAS_BOX_WORKSPACE/<agentId>/session.jsonl` (optional; first knife does not require restore) |
| `ATLAS_BOX_WORKSPACE` / `ATLAS_BOX_TURN_DELAY_MS` | Unchanged from R1/R2 |

Dedicated `ATLAS_BOX_LLM_*` (not a silent fallback to `ATLAS_OPENAI_*`).

### Switch back to cli

```bash
ATLAS_GATEWAY_BACKEND=cli   # or unset
# existing mock-cli / p35 / Win stack paths unchanged
```

---

## 3. Tool vs LLM order (nailed)

1. **Tool triggers first** (`LIST_DIR` / `READ_FILE` / `WRITE_FILE` / allowlisted `RUN …`).  
2. If any tool evidence is produced → reply via deterministic **`compose_reply`** (tools stay greppable; path escape still rejects).  
3. **Pure chat** (no tool hit):
   - LLM configured → `POST {base}/chat/completions` with `messages[]` =
     `system` + truncated history + current `user`.
   - Else / `MODE=mock|off` → today’s `compose_reply`.

LLM HTTP / timeout / non-2xx → **visible** `Upstream` error (no silent echo).

---

## 4. `/healthz`

For `backend=box`:

```json
{ "ok": true, "backend": "box", "llm_configured": true, "llm_model": "…" }
```

- `llm_configured`: bool only.  
- Optional `llm_model`.  
- **Never** the API key.

---

## 5. Interrupt

- In-flight **delay** or **LLM HTTP** is cancellable via `interruptAgentRun` (oneshot), same closed-set narrative as openai oneshot.  
- Idle interrupt → `command_rejected` / `no_active_run` (never fake success).

---

## 6. Session / truncation

- Hot path: **in-memory** `history` per `agentId` (RR1 must).  
- Truncation: keep last `ATLAS_BOX_HISTORY_MAX_TURNS` pairs (default 32).  
- Persist: optional soft `session.jsonl` append; failures warn only. Restore-on-boot is **not** required for RR1 DoD.

---

## 7. Mock LLM (CI) vs real model (yellow)

**CI / DoD:** in-process loopback OpenAI-compat stub inside
`private_resident_smoke` (records `messages[]`, returns `llm-mock …`).  
Zero outbound real-LLM dependency.

```bash
cargo test -p atlas-bot-hub --test private_resident_smoke -- --nocapture --test-threads=1
# Expect: SMOKE_OK private-resident RR-P1… / RR-P2… / RR-P3… / RR-P5…
```

Also keep green: `runtime_smoke`, `runtime_deepen_smoke`, `p35_smoke`, …

**Yellow (optional, not a merge gate):** point `ATLAS_BOX_LLM_*` at local
ollama / private gateway; hand-record evidence. Do **not** add a required
CI job that calls the public internet for a model.

---

## 8. Declarations

- Did **not** delete or demote cli; default remains cli.  
- Did **not** vendor grok / `xai-grok-*`.  
- Not enterprise ops B / store signing C.  
- No `bot.*` / Hub wire changes.  
- CI has **zero** hard dependency on a real external LLM.

---

## 9. Known limits (filled)

| Item | RR1 choice |
|------|------------|
| Tool vs LLM order | Tools first → compose_reply; pure chat → LLM |
| Session | Memory first; optional soft jsonl append |
| Truncation | Default max **32** turn pairs |
| Mock LLM | In-test loopback stub (axum), records bodies |
| healthz | `llm_configured` (+ optional `llm_model`) |
| OpenAI env fallback | **No** — Box uses dedicated `ATLAS_BOX_LLM_*` |
