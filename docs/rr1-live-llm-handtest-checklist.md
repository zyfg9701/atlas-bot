# RR1 真模型黄标手测清单（RL1）

> **落点选择（RL1）：** 新建本页（**RR-P6** + **RL-C / RL-T / RL-F / RL-D**）+ 失败矩阵 L1–L10 + 证据模板；与 runbook 双向链接，避免双源长文漂移。  
> 真模型叙事长文：[`private-resident-runtime-runbook.md`](./private-resident-runtime-runbook.md) **§ 真模型黄标（黄标 · RL1）**。  
> 默认仍 cli：[`cli-primary-runbook.md`](./cli-primary-runbook.md)。  
> WL1 形对照：[`wecom-ticket-checklist.md`](./wecom-ticket-checklist.md) / [`live-agent-handtest-checklist.md`](./live-agent-handtest-checklist.md)。  
> **不做：** 改 `bot.*`、默认改 `backend=box`、RR2、CI 打真 LLM、删 mock、产品证据导出按钮。

---

## Mock 主路径（日常 / CI · 必须保留）

```bash
cargo test -p atlas-bot-hub --test private_resident_smoke -- --nocapture --test-threads=1
# Expect: SMOKE_OK private-resident RR-P1… / RR-P2… / RR-P3… / RR-P5…
```

- Loopback OpenAI-compat stub 录 `messages[]`；返回 `llm-mock …`；**零**外网真 LLM。
- 产品默认仍 **`backend=cli`**（RR-P4 守卫在 smoke 内）。
- 详见 runbook §7 Mock LLM · §2 Start Box。

### 回归位（不回退）

- [ ] `private_resident_smoke` 绿（或 docs-only PR 声明未改协议/产品代码）
- [ ] RR-P1..P5 叙事仍可用（mock 多轮 / 无 LLM 确定性 / 工具+逃逸 / 默认 cli / interrupt）
- [ ] CI / Actions **未**改成打真 ollama / 私有 API `BASE_URL`
- [ ] 默认仍 cli；未删 mock LLM stub

---

## RR-P1..P5（mock · 摘要）

| ID | 内容 | 期望 |
|----|------|------|
| **RR-P1** | mock LLM 多轮 | 同 agentId；第二轮 `messages` ≥3 / 含历史；回复 `llm-mock …` |
| **RR-P2** | 未配 LLM | 确定性 `compose_reply`；`llm_configured=false` |
| **RR-P3** | 工具先 + 路径逃逸 | LIST/READ 可 grep；逃逸仍拒；不拆 R2 |
| **RR-P4** | 默认 cli | unset → `cli`；禁止改二进制默认成 box |
| **RR-P5** | interrupt | 空闲闭集；飞行中 LLM HTTP 可取消 |

自动化：上列由 `private_resident_smoke` 覆盖。本页黄标位 **不挡** 合 main。

---

## RR-P6 · 真模型黄标可勾选子步

前置：本地 ollama **或**私有 OpenAI-compat 可达；密钥仅本机；runbook § 真模型黄标 已读。  
样本可标「环境待补」——文档包齐仍可合。

- [ ] **P6.1** 起模型侧（ollama / 私有网关）；记下 `MODEL`；例 base `http://127.0.0.1:11434/v1`
- [ ] **P6.2** 若需 key → 仅写入 gateway 进程私密 env（未进 git / PR / 证据）
- [ ] **P6.3** gateway：`ATLAS_GATEWAY_BACKEND=box` + 真 `ATLAS_BOX_LLM_BASE_URL`/`MODEL`；`MODE` 非 `mock`/`MODE=mock|off`
- [ ] **P6.4** `curl /healthz` → `backend=box`、`llm_configured: true`（可选 `llm_model`）；**无** key 回显
- [ ] **P6.5** 同 `agentId` 纯聊两轮：Turn2 体现见过 Turn1（或脱敏 messages≥3 日志一行）
- [ ] **P6.6** （可选）工具对照：LIST/READ 仍可 grep
- [ ] **P6.7** （可选）失败一手：停模型 / 错 MODEL / 错 base → Upstream 可见
- [ ] **P6.8** 证据短表已填 + 脱敏三勾；失败则记 **L#**
- [ ] **P6.9** **切回 mock**（unset 真 `ATLAS_BOX_LLM_*` 或 `MODE=mock`；backend 回 cli/unset）；确认 CI 仍零出站

细步用下方 **RL-C / RL-T / RL-F / RL-D**。

---

## RL1 真模型联调位

共性期望：

1. `backend=box` + 真 `ATLAS_BOX_LLM_*`（非 mock/MODE=mock|off）  
2. `/healthz` → `llm_configured: true`（无 key）  
3. 同 `agentId` 两轮纯聊；Turn2 见过 Turn1  
4. 证据已脱敏；**不**改 CI 打真网  
5. 做完切回 mock；默认仍 cli  

样本状态：本切片开写环境 **无真 LLM** → 证据标 **「环境待补」**；文档包齐仍可合。

### RL-C · 配置

- [ ] 真 `ATLAS_BOX_LLM_BASE_URL`（…`/v1`）+ 真 `ATLAS_BOX_LLM_MODEL`
- [ ] `ATLAS_BOX_LLM_MODE` **未**设为 `mock` / `MODE=mock|off`
- [ ] `ATLAS_GATEWAY_BACKEND=box`（手测旁路；**未**改产品默认）
- [ ] `ATLAS_OPENAI_*` **未**当作 Box 静默 fallback
- [ ] `/healthz`：`llm_configured=true`；可选 `llm_model`；**无** key / Bearer 回显
- [ ] （可选）跑 `./scripts/probe-box-llm-live.sh` 看缺哪些变量（advisory）
- [ ] 证据：□ 已填短表  □ 环境待补

### RL-T · 两轮上下文

- [ ] 固定同一 `agentId`（Turn1 → Turn2 未换）
- [ ] Turn1：可记忆短句（例口令词）；回复非旧 `compose_reply` / 非 `echo:` / 非 stub `llm-mock`
- [ ] Turn2：问及 Turn1 内容；回复能体现见过 Turn1
- [ ] **或** 脱敏日志一行：第二轮 `messages` 条数 ≥3 / 含 prior user+assistant（已剥 key）
- [ ] 证据：□ 已填短表  □ 环境待补

### RL-F · 失败一手（可选）

- [ ] 停模型 / 断 base → Upstream / 连接失败可见；**不**假成功
- [ ] 错 `MODEL` → 404 或模型错误可见
- [ ] （若私有 API）错/缺 key → 401/403 可见；**勿**把 key 贴证据
- [ ] 对照表行号已记（L2 / L3 / L4 / L5…）
- [ ] 证据：□ 已填短表  □ 跳过（可选）  □ 环境待补

### RL-D · 脱敏声明

- [ ] □ 无 `ATLAS_BOX_LLM_API_KEY` / Bearer / 上游 token 全文
- [ ] □ 无完整含 key 的 HTTP 请求体
- [ ] □ 无 `.env` / credentials 整文件
- [ ] 证据存放：私密附件 / Notion / 本机 — **未**进 git
- [ ] 结果位：□ 通过  □ 失败→L____  □ 环境待补

---

## 失败对照表 L1–L10

| 行 | 现象 | 常见根因 | 期望表现 | 处置 |
|----|------|----------|----------|------|
| **L1** | `/healthz` `llm_configured: false` | 未设 `BASE_URL`/`MODEL`；或 `MODE=mock|off` | 纯聊走 `compose_reply`，非真模型 | 补齐 `ATLAS_BOX_LLM_*`；去掉 mock/off |
| **L2** | 纯聊立刻 Upstream / 连接失败 | **base 错**、ollama 未起、端口非 `/v1` | 可见错误；**不**假成功 echo | 核对 `…/v1`；`curl` 探 models；先起 ollama |
| **L3** | 连接成功但 404/模型错误 | **`MODEL` 未 pull / 名错** | 非 2xx 可见 | `ollama pull …` 或改模型名 |
| **L4** | 401/403 | 私有 API **key 缺失/错** | Upstream 可见 | 只改本机 env；**勿**把 key 贴证据 |
| **L5** | 超时 | 本地模型冷启动慢 / `TIMEOUT_MS` 过小 | 超时错误可见 | 调大 timeout；预热模型 |
| **L6** | 第二轮「像没记忆」 | 换了 `agentId`；history 被截光（极端）；或非 Box 后端 | 无 prior | 固定同 agent；确认 `backend=box`；对照 mock RR-P1 |
| **L7** | 工具也不走 / 全失败 | 误改工具序或 workspace | R2 回退 | **禁止**本刀改工具栈；回归 RR-P3 / `private_resident` 工具位 |
| **L8** | 回复仍像旧 `compose_reply` | 实际未配 LLM 或 `MODE=mock` | 无模型腔 / 无真回复特征 | 再查 healthz；确认未走 mock |
| **L9** | CI 变红 / 流水线出站 | 误把真 `BASE_URL` 写进 Actions | 违反硬门槛 | **立即改回 mock**；RL3（CI 真 LLM）已拒 |
| **L10** | healthz / 日志出现 key | 日志过详或探针打印 env | 安全事故 | **禁止**；只报 `llm_configured`；审查探针 |

---

## 证据模板（短表）

复制一份填写。证据放私密附件 / Notion / 本机 — **勿**把 key / 完整含 key 请求体进 git。

```text
日期（Asia/Shanghai）：
基线 commit：
gateway backend：box
ATLAS_BOX_LLM_BASE_URL（主机名级，可打码路径）：
ATLAS_BOX_LLM_MODEL：
API_KEY：□ 未使用（本地） □ 已配置（仅本机 env，未入证据）
/healthz：llm_configured=  llm_model=
agentId（可打码）：
Turn1 摘要 → 回复摘要：
Turn2 摘要 → 回复摘要（须体现见过 Turn1）：
（可选）第二轮 messages 条数 /「含历史」日志一行（已剥 key）：
（可选）工具对照：□ LIST/READ 仍可 grep
（可选）失败一手：□ 停模型 → Upstream 可见
日志时间戳（healthz / turn1 / turn2）：
脱敏确认：□ 无 API key  □ 无完整含 key 请求体  □ 无 .env 整文件
结果：□ 通过  □ 失败→对照表行：L____  □ 环境待补
```

### 样本归档位（本切片）

| 项 | 状态 |
|----|------|
| 本地 ollama 两轮 | **环境待补** |
| 私有 OpenAI-compat 两轮 | **环境待补** |

有模型后按短表补脱敏摘要/日志即可；**不得**为补样本改 CI 打真网。

---

## mock ↔ 真模型切换（一句）

| 方向 | 做法 |
|------|------|
| → 真模型黄标 | `ATLAS_GATEWAY_BACKEND=box` + 真 `BASE_URL`/`MODEL`（+ 可选本机 `API_KEY`）；`MODE` 非 mock/MODE=mock|off；healthz `llm_configured=true`；同 agentId 两轮 |
| → mock（日常/CI） | unset 真 `ATLAS_BOX_LLM_*` 或 `MODE=mock`；backend 回 cli/unset；**CI 永不真 base** |
| 验证 | mock：`private_resident_smoke` / RR-P1..P5；真模型：两轮上下文 + 证据短表脱敏三勾 |

可选辅助（不替真人两轮）：`scripts/probe-box-llm-live.env.example` + `scripts/probe-box-llm-live.sh`。

---

## 链回

- Runbook 真模型节：[`private-resident-runtime-runbook.md`](./private-resident-runtime-runbook.md) § 真模型黄标（黄标 · RL1）
- Mock / CI：同 runbook §7 · `private_resident_smoke`
- 默认仍 cli：[`cli-primary-runbook.md`](./cli-primary-runbook.md)
- WL1 证据包形：[`wecom-ticket-checklist.md`](./wecom-ticket-checklist.md) · [`i2-login-runbook.md`](./i2-login-runbook.md) § 真机企微联调
- L1 手测包形：[`live-agent-handtest-checklist.md`](./live-agent-handtest-checklist.md)
