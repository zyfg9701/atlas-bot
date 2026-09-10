# 真机联调包 · Live agent handtest checklist（L1）

> 切片 **L1 + L1b**：Win 对本机**已登录** Atlas/Cursor `agent` 的手测包  
> 基线：`main`@`589b2d5`（W1 / WS1·B / CS1 已齐）  
> 对等 mock 回归：[`windows-cli-stack-checklist.md`](./windows-cli-stack-checklist.md) W-E* / W-S*  
> 起栈 / 流式配方：[`cli-primary-runbook.md`](./cli-primary-runbook.md) §5 / §5b · [`cli-streaming-checklist.md`](./cli-streaming-checklist.md)  
> **不做：** I2.2、改 `bot.*`、假流式、自动登录、真机 CI 门禁、重写 gateway、删 mock

本页 = **真机手测叙事**；不重做 W1/WS1/CS1 代码。登录态以 **Send 成败**为准（探针只报 found / 可执行）。

---

## 0. 前置

- [ ] 本机已装 Cursor/Atlas **agent** CLI（或已知绝对路径）
- [ ] **人工已登录** agent（本包**不**自动化登录 / 不写 token / 不读 session）
- [ ] 仓库根目录；PowerShell 5.1+（7+ 亦可）
- [ ] mock 路径仍可用作对照（勿删）：`tools\mock-cli\mock-atlas-agent-cli.cmd`

可选探针（L1b）：

```powershell
.\scripts\probe-agent-cli.ps1
```

---

## L-E1 · 装 / PATH

```powershell
where.exe agent
# 或显式：
$env:ATLAS_AGENT_CLI='C:\path\to\agent.exe'   # 按本机实际路径
```

- [ ] `where.exe agent` 有输出，**或** `ATLAS_AGENT_CLI` 指向存在的文件
- [ ] （可选）`.\scripts\probe-agent-cli.ps1` → `found=true` + resolved path（**不**声称已登录）

失败 → 见下方失败对照「未装」。

---

## L-E2 · 人工登录前提（写明）

- [ ] 本机 Cursor/Atlas agent **已人工登录**（本包不打开浏览器、不塞凭证）
- [ ] 知晓：未登录时起栈可能成功，但 **Send** 会失败 / 非零 exit，且**不假成功**

---

## L-E3 · 探测（L1b）

```powershell
.\scripts\probe-agent-cli.ps1
# 可选：.\scripts\probe-agent-cli.ps1 -Cli 'C:\path\to\agent.exe'
```

- [ ] 打印 resolved path、exit code、建议下一步（`dev-cli-stack.ps1` 或 `-Stream`）
- [ ] 若 version/help 不支持 → 降级「仅 found」；**禁止**把 found 当成已登录

---

## L-E4 · text 起栈（无 `-Stream`）

```powershell
# 确保未指向 mock；清残留 MOCK（见「mock ↔ 真机切换」）
Remove-Item Env:ATLAS_AGENT_CLI -ErrorAction SilentlyContinue   # 或显式设真机路径
Remove-Item Env:MOCK_CLI_STREAM -ErrorAction SilentlyContinue
Remove-Item Env:MOCK_CLI_SLEEP_MS -ErrorAction SilentlyContinue
Remove-Item Env:ATLAS_AGENT_CLI_STREAM -ErrorAction SilentlyContinue
Remove-Item Env:ATLAS_CLI_STACK_STREAM -ErrorAction SilentlyContinue

.\scripts\dev-cli-stack.ps1
# 另开终端：
Invoke-WebRequest http://127.0.0.1:8787/healthz | Select-Object -Expand Content
```

- [ ] healthz：`backend=cli` + `agent_cli_found=true`
- [ ] healthz `agent_cli` 字符串指向**真**二进制（**非** `mock-atlas-agent-cli.cmd`）
- [ ] 横幅：`ATLAS_AGENT_CLI_STREAM=(unset=text)`；未自动 MOCK

---

## L-E5 · 流式起栈（`-Stream`）

```powershell
.\scripts\dev-cli-stack.ps1 -Stream
# 真机无 mock → **不**自动设 MOCK_CLI_STREAM
```

- [ ] 横幅含：`ATLAS_AGENT_CLI_STREAM=1`、`ATLAS_HUB_EVENT_URL=http://127.0.0.1:7701/internal/runtime-hint`
- [ ] 真机：**无** `MOCK_CLI_STREAM=1`（除非你显式设了）
- [ ] 分进程 mid-turn **只 B1**；勿再开 B2 `spawn_turn_bridge`

配方对齐 runbook §5b PowerShell「真机 agent + -Stream」。

---

## L-E6 · PC Connect / Send

- [ ] PC Connect：`ws://127.0.0.1:7700/ws`
- [ ] 选 agent → **Send**（任意短 prompt）

---

## L-E7 · Events 期望（真机证据位）

| 起法 | Events | preview |
|------|--------|---------|
| text（L-E4） | 终稿 + `hub:turn_finished` | **非** `echo:` · **非** `atlas-mock-reply` |
| `-Stream`（L-E5） | ≥1× `hub:assistant_delta`（可选 `hub:tool`）→ `hub:turn_finished` | **非** `echo:` · **非** `atlas-mock-reply` |

- [ ] **L-E1 证据（text）：** Send 终稿非 echo / 非 mock（可合后补本机截图）
- [ ] **L-E2 证据（`-Stream`）：** ≥1× `hub:assistant_delta` + `hub:turn_finished`（环境不允许则注明 agent 版本限制）

> 证据 ID 与本页步骤号同名易混：上表「L-E1/L-E2 **证据**」= 飞天验收证据位；步骤勾选仍用本页 L-E1…L-E7。

---

## 失败对照表

| 现象 | 常见根因 | 期望表现 | 处置 |
|------|----------|----------|------|
| ps1 立刻 exit 1 | **未装** / PATH 无 / 路径错 | `ATLAS_AGENT_CLI not found`；**不起** Hub-only stub | 安装 agent 或设绝对路径；临时对照用 mock（见切换表） |
| 起栈 OK，Send 失败 / 非零 exit | **未登录** / auth 过期 | gateway 错误可见；**不假成功** | 人工重新登录 agent；重试 Send（探针**不能**证明已登录） |
| 仅终稿、无 mid-turn | **无 STREAM** / 未 `-Stream` / 未设 EVENT_URL | 与 CS1 text 一致 | `.\scripts\dev-cli-stack.ps1 -Stream` 或显式 `ATLAS_AGENT_CLI_STREAM=1` + EVENT_URL |
| preview 含 `atlas-mock-reply` | **误用 mock** 仍指向 mock `.cmd` | 属 mock 路径（W-E*/W-S*） | 清 `$env:ATLAS_AGENT_CLI` 或改回真 `agent`；勿留 `MOCK_CLI_*` |
| preview 含 `echo:` | **Hub-only stub**（未挂 `ATLAS_GATEWAY_URL`） | InMemory 旁路 | **必须**经 `dev-cli-stack.ps1`；禁止只起 Hub |
| healthz `agent_cli_found=false` | 探测时有、运行时丢 / 配置漂移 | 字段诚实 | 核对 gateway 进程环境与横幅打印路径；重启栈 |

---

## mock ↔ 真机切换

| 方向 | 做法 |
|------|------|
| → mock（无真机 / CI 对位） | `$env:ATLAS_AGENT_CLI="$PWD\tools\mock-cli\mock-atlas-agent-cli.cmd"`；流式加 `-Stream`（自动 `MOCK_CLI_STREAM`） |
| → 真机 | **去掉** mock 覆盖：`Remove-Item Env:ATLAS_AGENT_CLI -ErrorAction SilentlyContinue` **或**显式设真 `agent` 路径；**不要**留 `MOCK_CLI_STREAM` / `MOCK_CLI_SLEEP_MS`；`-Stream` 时只填 STREAM + EVENT_URL（真机不自动 MOCK） |
| 验证切成功 | healthz `agent_cli` 字段指向真二进制（字符串不含 `mock-atlas-agent-cli`）；Send 回复 **无** `atlas-mock-reply` / **无** `echo:` |

环境变量名与 Unix **相同**（PowerShell `$env:ATLAS_*`）。用户已设 `ATLAS_AGENT_CLI_EXTRA_ARGS` 时不被静默覆盖。

**一句：** 清 `ATLAS_AGENT_CLI` / `MOCK_CLI_*` → 起 `dev-cli-stack.ps1`（可选 `-Stream`）→ 看 healthz `agent_cli` 字符串。

---

## W-L* 指针（与 windows checklist 对齐）

| 本页 | windows checklist | 含义 |
|------|-------------------|------|
| L-E4 text 起栈 | W-E2（mock 对位）→ **W-L1** 真机 text | 真机 healthz + Connect |
| L-E5 `-Stream` | W-S1/W-S2（mock）→ **W-L2** 真机流式 | 真机横幅无自动 MOCK + delta |
| L-E7 Events | W-E3 / W-S2 期望翻转 | preview **禁止** mock/echo |
| 失败表 | W-E1 未装 | 同「不静默 stub」 |

详见 [`windows-cli-stack-checklist.md`](./windows-cli-stack-checklist.md) **W-L*** 短节。

---

## §6 已知限制（填实）

| 项 | 填实 |
|----|------|
| 文档落盘形态 | **独立页**本文件 + windows checklist **W-L*** 短节 + 双向链（防双源漂移；大段步骤只维护本页） |
| L1b 脚本 | `scripts/probe-agent-cli.ps1`；尝试非交互 `-version` / `--version` / `-help` / `--help`；均不支持则 **仅 found**；**从不**声称已登录 |
| 真机证据 | **合后催本机**（EricComputer）：L-E1 text 终稿 + L-E2 `-Stream` ≥1× delta；缺截图**不单独挡合**（能力已在 CS1/WS1） |
| agent CLI 版本差异 | 各发行版 version/help 旗标不一；stream-json / partial 行为随 Cursor/Atlas agent 版本变化——若无 delta，先确认 `-Stream` 与登录，再注明版本限制 |
| 与 mock 回归 | W-E* / W-S* **必须保留**；unset STREAM 仍 text；Unix `p35`/`cli_stream`/`b1` 冒烟不回退 |
| 不在范围 | I2.2 · 改 `bot.*` · 假流式 · 自动登录 · 真机 CI 门禁 · 重写 `CliAgentGateway` · 删 mock |

---

## 相关

- Runbook §5 / §5b：[`cli-primary-runbook.md`](./cli-primary-runbook.md)
- 流式 checklist：[`cli-streaming-checklist.md`](./cli-streaming-checklist.md)
- Win mock 起栈 / 流式：[`windows-cli-stack-checklist.md`](./windows-cli-stack-checklist.md)
- 探针：[`../scripts/probe-agent-cli.ps1`](../scripts/probe-agent-cli.ps1)
