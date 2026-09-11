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

**Yellow (optional, not a merge gate):** see **§ 真模型黄标（黄标 · RL1）** below —
hand-record evidence against local ollama / private OpenAI-compat. Do **not**
add a required CI job that calls a real external LLM. Checklist:
[`rr1-live-llm-handtest-checklist.md`](./rr1-live-llm-handtest-checklist.md)
（**RR-P6** + **RL-C / RL-T / RL-F / RL-D** · L1–L10 · 证据短表）.

---

## § 真模型黄标（黄标 · RL1）

> **证据债文档包**（相对 mock LLM）。RR1 代码已支持 `ATLAS_BOX_LLM_*` 指真
> OpenAI-compat；本节只钉配置差、步骤、两轮证法、证据规范与失败归因。  
> **不改** `bot.*` / Hub wire / Box LLM 业务代码。**CI 仍零外网真 LLM**。  
> **默认仍 `backend=cli`**（见 [`cli-primary-runbook.md`](./cli-primary-runbook.md)）。  
> 手测勾选与失败矩阵全文：[`rr1-live-llm-handtest-checklist.md`](./rr1-live-llm-handtest-checklist.md)。  
> 叙事对齐：[`wecom-ticket-checklist.md`](./wecom-ticket-checklist.md) / i2
> 「真机企微联调」形（WL1）。  
> 可选探针：`scripts/probe-box-llm-live.env.example` +
> `scripts/probe-box-llm-live.sh`（**不**替真人两轮联调；**不**挂 CI）。

### mock ↔ 真模型配置差表

| 变量 | mock / CI | 真模型黄标 |
|------|-----------|------------|
| `ATLAS_GATEWAY_BACKEND` | 测试内 `box`；**产品默认仍 cli** | **同** `box`（手测旁路）；**禁止**改二进制默认 |
| `ATLAS_BOX_WORKSPACE` | 测试临时目录 | 本地可写目录（与 R1/R2 同） |
| `ATLAS_BOX_LLM_BASE_URL` | loopback stub | **真** base（ollama 例 `http://127.0.0.1:11434/v1`；私有网关 `https://<host>/v1`） |
| `ATLAS_BOX_LLM_MODEL` | stub 接受任意 | **真**模型 id（须已 `ollama pull` / 已部署） |
| `ATLAS_BOX_LLM_API_KEY` | 可空 / 测试值 | 本地可空；私有 API 密钥 **仅** gateway 进程环境；**禁止**贴 PR / 证据 / 日志 |
| `ATLAS_BOX_LLM_MODE` | 测试路径不强制 mock | **勿** `mock` / `off`（否则仍走 `compose_reply`） |
| `ATLAS_BOX_LLM_TIMEOUT_MS` | 测试默认即可 | 可按本地模型调大；文档记实测值 |
| `ATLAS_BOX_HISTORY_MAX_TURNS` | 默认 32 | 同；两轮证据不依赖改此值 |
| `ATLAS_OPENAI_*` | 无关 | **不**作 Box 静默 fallback（专用 `ATLAS_BOX_LLM_*`） |

**一句切换：** 真模型 = `backend=box` + 真 `BASE_URL`/`MODEL`（+ 可选本机 `API_KEY`）+ `MODE` 非 mock/off → healthz `llm_configured: true` → 同 `agentId` 两轮；做完 → **切回 mock**（unset 真 `ATLAS_BOX_LLM_*` 或改回 stub；CI 永不真 base）。

### 步骤顺序（建议）

1. **准备模型侧**：起 ollama（或私有 OpenAI-compat 网关）；确认 `GET {base}/models` 或等价可达；记下 `MODEL` id（ollama 例 base：`http://127.0.0.1:11434/v1`）。
2. **密钥**：若私有 API 要 key → 只写入本机私密 env / 密码管理器；**不**进 git、不进证据附件。
3. **起 gateway（旁路）**：`ATLAS_GATEWAY_BACKEND=box` + `ATLAS_BOX_LLM_*` 指真 base；`curl /healthz` 见 `llm_configured: true`（可选 `llm_model`）；**无** key 回显。
4. **纯聊两轮（同 agentId）**：见下方「两轮证法」。
5. **（可选）对照工具路径**：发 `LIST_DIR` / `READ_FILE` 类触发 → 仍工具先、证据可 grep；证明真模型未拆坏 R2。
6. **（可选）失败一手**：停掉 ollama / 错 `MODEL` / 错 base → 须 **Upstream 可见错误**，不假成功。
7. **留证**：按下方证据规范 + checklist 证据短表；失败对照 **L1–L10**。
8. **切回 mock**：unset 真 `ATLAS_BOX_LLM_*` 或改回 stub；确认 `private_resident_smoke` / 日常 CI **仍 mock、零出站**；产品默认仍 **cli**。

链回：[`rr1-live-llm-handtest-checklist.md`](./rr1-live-llm-handtest-checklist.md) · §7 Mock LLM · §2 Start Box · [`cli-primary-runbook.md`](./cli-primary-runbook.md)「默认仍 cli」。

### 两轮证法

| 轮 | 动作 | 期望 |
|----|------|------|
| **Turn1** | 同 `agentId` 发送可记忆短句（例：「请记住口令词 ALPHA-7」） | 回复可区分于旧 `compose_reply` / 非 `echo:`；非 stub 的 `llm-mock …` |
| **Turn2** | **同一** `agentId` 问「口令词是什么？」或「上一句我说了什么？」 | 回复能体现见过 Turn1（口头复述或可核对摘要） |

最低证据条：

1. 同 `agentId` Turn1 + Turn2 时间戳；
2. Turn2 回复体现 Turn1；**或** 脱敏「第二轮请求 `messages` 条数 ≥3 / 含 prior user+assistant」日志一行（**已剥 key**）；
3. 脱敏确认三勾（见 checklist 模板）。

### 证据规范（允许 / 禁止）

| 允许 | 禁止 |
|------|------|
| `/healthz` 摘要：`backend=box`、`llm_configured`、可选 `llm_model`（主机名级 base 即可） | `ATLAS_BOX_LLM_API_KEY` 全文；任何 Bearer / 上游 token |
| 两轮 prompt **摘要**（可打码口令词）+ 模型回复摘要；同 `agentId` | 完整 HTTP 请求体（尤其含 `Authorization` / key 头） |
| 网关日志 **时间戳** + 阶段（LLM HTTP 开始 / 2xx / 第二轮 messages 长度或「含历史」叙述） | 把 `.env` / credentials 整文件贴进 PR |
| 失败时的 Upstream / 超时 / 非 2xx 闭集表现 + 对照表行号 **L1–L10** | CI workflow 改成真 `BASE_URL` 出站 |
| 声明：本地 ollama 或「私有网关主机名」；**不**贴内网完整 URL+key | 在公开 issue 贴未打码的企业内部 endpoint+密钥 |

证据存放：私密附件 / Notion / 本机加密目录 — **勿**把 key 进 git。短表字段见 checklist「证据模板」。

### 失败对照（摘要 · 全文在 checklist）

| 行 | 现象族 |
|----|--------|
| **L1** | `/healthz` `llm_configured: false` → 未设 BASE/MODEL 或 `MODE=mock|off` |
| **L2** | 纯聊立刻 Upstream / 连接失败 → base 错、ollama 未起、端口非 `/v1` |
| **L3** | 连接成功但 404/模型错误 → `MODEL` 未 pull / 名错 |
| **L4** | 401/403 → 私有 API key 缺失/错（只改本机 env，勿贴证据） |
| **L5** | 超时 → 冷启动慢 / `TIMEOUT_MS` 过小 |
| **L6** | 第二轮「像没记忆」→ 换了 `agentId` / history 截光 / 非 box 后端 |
| **L7** | 工具也不走 / 全失败 → 误改工具栈（本刀禁止） |
| **L8** | 回复仍像旧 `compose_reply` → 实际未配 LLM 或 MODE=mock |
| **L9** | CI 变红 / 流水线出站 → 误把真 `BASE_URL` 写进 Actions → **立即改回 mock** |
| **L10** | healthz / 日志出现 key → 日志过详或探针打印 env → **禁止** |

处置细节与可勾选位：[`rr1-live-llm-handtest-checklist.md`](./rr1-live-llm-handtest-checklist.md) § 失败对照表 L1–L10。

### 切回 mock

```bash
# Unset real LLM env (or point BASE_URL back at a local stub only for handtest)
unset ATLAS_BOX_LLM_BASE_URL ATLAS_BOX_LLM_MODEL ATLAS_BOX_LLM_API_KEY
# Or: ATLAS_BOX_LLM_MODE=mock   # forces compose_reply even if base set
ATLAS_GATEWAY_BACKEND=cli       # or unset — product default
cargo test -p atlas-bot-hub --test private_resident_smoke -- --nocapture --test-threads=1
```

**CI 永不**把真 ollama / 私有 API `BASE_URL` 写进 Actions。

### 真模型证据样本状态

本环境 **无**可用真 LLM（无本机 ollama / 无私有 OpenAI-compat）→ 样本位标 **「环境待补」**。文档包（本 runbook 节 + checklist RL-* + 失败矩阵 + 证据模板 + 可选探针）齐即可合；脱敏两轮摘要在有模型后按模板补填，**不得**因此改 CI 打真网。

### 明确非本刀

改 `bot.*` / Hub wire · 默认改 `backend=box` / 删降级 cli · **RR2** 独立 resident 进程（RL4）· **CI 打真 LLM**（RL3）· 产品「证据导出」按钮（RL2 后置）· 重做 Box LLM 接线 · 删 mock · 密钥 / 完整含 key 请求体写入 PR · 企业运维 B / 商店签名 C / vendor grok。

---

## 8. Declarations

- Did **not** delete or demote cli; default remains cli.  
- Did **not** vendor grok / `xai-grok-*`.  
- Not enterprise ops B / store signing C.  
- No `bot.*` / Hub wire changes.  
- CI has **zero** hard dependency on a real external LLM.  
- **RL1:** real-model yellow-tag is a **docs/checklist evidence pack** only（见上 § 真模型黄标）；optional advisory probe; no product/protocol code; no CI live LLM; not RR2.

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
| Real-model yellow (RL1) | Docs + checklist + optional probe; sample may be「环境待补」; CI stays mock |
| ollama example base | `http://127.0.0.1:11434/v1` |
| Not done | RR2 resident process; CI live LLM (RL3); product evidence export (RL2); default→box |
