# atlas-bot PC 使用说明

> 基线：`main`@`e6f9e66` + **U2 纯 Chat 壳** · 客户端 `clients/pc`（Tauri 2 + TS）  
> 入口二选一：`#/chat` 对话壳，`#/debug` 保留原调试台。**推荐后端 = cli gateway**（见 cli-primary-runbook）。本阶段是壳分流，不是最终品牌 UI / 签名上架 / Box LLM / I2.2。

---

## 0. 入口、深链、`atlas-pc-mode`

冷启动（地址没有 `#/chat` / `#/debug`，且 `localStorage atlas-pc-mode` 不是 `chat` 或 `debug`）落在 **入口页**：两个按钮 **Chat** 和 **调试 / Debug**。点选写入 `atlas-pc-mode`，再进入对应 hash。

| 规则 | 行为 |
|------|------|
| 深链 `#/chat`、`#/debug` | 直接进该页，并**覆盖**已存 `atlas-pc-mode` |
| 空 hash 或 `#/`，且 mode 有效 | 恢复上次模式，并把 hash 补成 `#/chat` 或 `#/debug` |
| 空 hash 或 `#/`，mode 缺失或无效 | 入口页 |
| 顶栏 **入口** | 清除 `atlas-pc-mode`，回到 `#/` |
| 同一会话 Chat ↔ Debug | **不断开**已连接的 WebSocket（页面内同一个 `HubClient`） |

`?debug=1`：这一次加载进入 `#/debug` 并强制展开调试手风琴，然后从地址栏去掉该参数，避免之后点 Chat 又被弹回。`atlas-pc-debug` **不选页面**，只记住调试页手风琴：`0` 折叠，未设置或 `1` 展开。

浏览器 `npm run dev` 与 Tauri 共用这套 hash；差别仍是浏览器 WebSocket 不能设 `Authorization`。

---

## 1. 产品主路径（`#/chat`）

入口选 Chat（或直接打开 `#/chat`）后，主路径有意义点击 ≤3：

1. **Connect**（顶栏；Hub 地址默认 `ws://127.0.0.1:7700/ws`。改地址、Bearer、Login 在 **调试页** 的「高级 · Hub URL / Bearer」）  
2. （按需）选 **Agent**；Connect 成功后会自动 `listAgents` 并选中  
3. 输入 prompt → **Send**（未订阅时自动 `subscribe`；旁路 **Interrupt**）

Chat **不出现**：「更多 / 调试」手风琴、Raw Log、协议细按钮、高级 Hub、顶栏 Login、侧栏 Create group（G1）和 Channels（C1）。顶栏小字 **调试** 进入 `#/debug`。composer 旁 📎 / 🖥 仍在（调试页里也有完整 upload / Open desktop）。

| 主屏元素 | 行为 |
|----------|------|
| 顶栏连接态 + Connect/Disconnect | `hello` 后 badge → ready |
| 顶栏「调试」「入口」 | 去 `#/debug`，或清除 mode 回入口 |
| Agent 侧栏 | 下拉 + Create agent（G1/C1 仅调试页） |
| Conversation | 只显示 transcript 消息气泡（Markdown）。不拼 `hub:*` / Raw JSON |
| Composer | Send / Interrupt / immediate |
| 错误条 | 有 Failure 才显示 |

---

## 2. 调试页 `#/debug`

入口选调试，或 Chat 顶栏 **调试**，或直接打开 `#/debug`。这是原先整页调试台（**只藏不砍**，没有删入口）：高级 Hub、Login / Logout、侧栏 G1 / C1、页底 **「更多 / 调试 ▾」**。手风琴在调试页**默认展开**（`atlas-pc-debug=0` 时保持折叠）。展开区内：

| 调试区块 | 入口 |
|----------|------|
| Cold | status / roster / offbox / offbox next |
| Protocol | subscribe / unsubscribe / getTail / listAgents / createAgent；群组走侧栏 createGroup / setGroupMembers |
| 连接信息 | capabilities、connection_id |
| Desktop · attachments | Open desktop、uploadAttachment、attachUpload |
| Raw panes + Log | Events、Hot transcript、完整 Log |

手风琴开关仍写回 `localStorage atlas-pc-debug`。`?debug=1` 见 §0。

手测清单见 `docs/pc-ui-u2-checklist.md`（U1 对照：`docs/pc-ui-u1-checklist.md`；对话气泡：`docs/pc-ui-u3-checklist.md`）。

---

## 3. 起服务（**主路径 = CLI 栈**，再开 PC）

推荐：**先起 cli 栈，再 Connect**（PC 只连 Hub，不起 agent）。

**Unix：**

```bash
# 真机（PATH 上已登录的 agent）
./scripts/dev-cli-stack.sh

# 无真 agent / CI
ATLAS_AGENT_CLI=tools/mock-cli/mock-atlas-agent-cli.sh ./scripts/dev-cli-stack.sh
```

**Windows：先跑 ps1，再 Connect**

```powershell
.\scripts\dev-cli-stack.ps1
# 或 Bypass：
powershell -ExecutionPolicy Bypass -File .\scripts\dev-cli-stack.ps1
# mock：
$env:ATLAS_AGENT_CLI="$PWD\tools\mock-cli\mock-atlas-agent-cli.cmd"; .\scripts\dev-cli-stack.ps1
```

详情与环境变量见 [`docs/cli-primary-runbook.md`](./cli-primary-runbook.md)。

PC：

```bash
cd clients/pc && npm i && npm run tauri dev
# Connect → ws://127.0.0.1:7700/ws → 选 agent → Send
# 或 npm run dev（浏览器 WS 不能带 Authorization；oidc/static 请用 Tauri）
```

调试区可 **复制起栈命令**，并 loopback 探 `http://127.0.0.1:8787/healthz`（`backend` / `agent_cli_found`）。

### 旁路起法（非主路径）

| 旁路 | 何时用 | 起法摘要 |
|------|--------|----------|
| **Hub-only / stub** | 协议 / UI 自测（echo） | `cargo run -p atlas-bot-hub`（**不**设 `ATLAS_GATEWAY_URL`） |
| **box** | 私有运行时实验 | gateway `ATLAS_GATEWAY_BACKEND=box` + Hub `ATLAS_GATEWAY_URL=…` |
| **openai** | 兼容 API | gateway `ATLAS_GATEWAY_BACKEND=openai` + 相关 `ATLAS_OPENAI_*` |

分进程 mid-turn 事件见 `docs/b1-event-ingest-runbook.md`。

---

## 4. 能力对照（仍可用）

| 能力 | Chat `#/chat` / 调试 `#/debug` |
|------|-------------|
| Connect / hello / capabilities | 两页顶栏都能 Connect；caps 在调试页 |
| Bearer / Login (OIDC PKCE) | 仅调试页（高级 Hub + Login）；生产靠 Tauri `connect_ws` |
| Cold status/roster/offbox | 调试页 |
| list / create / **createGroup / setGroupMembers** / **Channels 四命令** / subscribe / send / interrupt / tail | Chat：自动 list + 侧栏 Create agent + Send；G1/C1 与协议细按钮在调试页 |
| Events / transcript | 两页 Conversation 都是气泡；`hub:*` 只在调试页 Events / Hot transcript / Log |
| VNC / upload / attach | composer 图标（两页）+ 调试页完整控件 |
| Failure / Log | 错误条 + 调试页 Log |

**还不是：** 品牌视觉系统、**真连 Slack / 多平台频道**（C1-real / C1-multi 另票）、上架包、强制移动建群/频道 UI、群套群 / fan-out 编排、删群产品化、I2.2 移动取票、React/Vue 重写、GA2/YOLO/desktop 集群、token 钥匙串/加密落盘。

**G1 已交付（PC）：** 侧栏 **Create group**（name + 多选已有非群 agent）→ 列表带 `[group]` 标记并可自动选中；选中群可见成员；**Change members** 改成员；对群 agentId 既有 Subscribe/Send/transcript 不回归。零新 `bot.*`（仅 catalog `createGroup` / `setGroupMembers` 经 `bot.command`）。Gateway：**InMemory + Box** 同形；**CLI / OpenAI** 本刀未跟（对该后端群命令仍 `UnknownMethod` 闭集拒绝，不装成功）。
**C1 已交付（PC）：** 侧栏 **Channels (C1 · local stub)** — 单平台 `slack` manifest；password 粘贴 token → Connect / Disconnect / Refresh；文案标明本地 stub、未出网。零新 `bot.*`（仅 catalog 四命令经 `bot.command`）。Gateway：**InMemory + Box** 同形；**CLI / OpenAI** 未跟（闭集 `UnknownMethod`）。Token 仅进程内存，reply/日志不回显，Disconnect 清除；不写 localStorage/钥匙串。

---

## 4.5 G1 建群三步（PC · `#/debug` 侧栏 · Hub-only / stub 或 Box）

1. **Connect** → 确保已有 ≥1 非群 agent（默认 `agt_1`，或侧栏 Create）。  
2. 侧栏填 **group name**，在 Members 多选框勾选成员 → **Create group** → 列表出现带 `[group]` 的项并选中。  
3. 改成员：选中群 → 调整 Members 多选 → **Change members**；`group members:` 行与再 `listAgents` 一致。

群上对话与单 agent 相同（单 transcript stub/echo；**非**多成员 fan-out）。手测见 [`docs/g1-group-handtest.md`](./g1-group-handtest.md)。

---

## 4.6 C1 Channels 三步（PC · `#/debug` 侧栏 · 本地 Slack stub · 未出网）

1. **Connect** → 选中任意 agent（含群；默认允许挂频道）。  
2. 侧栏 **Channels (C1 · local stub)**：可见 `slack` manifest（文案含「local stub / no egress」）→ password 框粘贴 token → **Connect** → connections 显示 `connected`（**非**真连 Slack）。  
3. **Disconnect** 清除 gateway 内存 token；**Refresh** 仅重读/规范化本地 status（不探外网）。

**Token：** 只经 `connectChannel` args 进入；gateway **进程内存**；reply / listAgents / 日志不回显；Disconnect 清除；**不**写入 localStorage / 钥匙串。进程重启丢失。

零新 `bot.*`（仅 catalog `connectChannel` / `disconnectChannel` / `refreshChannel` / `getAgentChannels` 经 `bot.command`）。Gateway：**InMemory + Box** 同形；**CLI / OpenAI** 本刀未跟（闭集 `UnknownMethod`）。

手测见 [`docs/c1-channel-handtest.md`](./c1-channel-handtest.md)。**非 C1-real / 非 C1-multi / 非上架签名 / 非 GA2·YOLO·集群。**

---

## 5. 常见坑

| 现象 | 原因 / 处理 |
|------|-------------|
| Connect 失败 | Hub 没起，或 WS 不是 `ws://127.0.0.1:7700/ws` |
| `unauthorized` | Hub 非 `dev` 且无有效 Bearer → Login 或粘贴票 |
| 浏览器里带鉴权连不上 | 用 **Tauri**（`connect_ws`），或 CLI `login` |
| sendPrompt 无内容 / echo | Hub 仍在 stub（未设 `ATLAS_GATEWAY_URL`）；请走 **cli 主路径** `./scripts/dev-cli-stack.sh` 或 `.\scripts\dev-cli-stack.ps1` |
| 有对话无 mid-turn 事件 | 分进程未配 B1；或未订阅（Send 会自动订） |
| 上传 `args_too_large` | 文件太大；换 &lt;1.5 MiB |
| Open desktop 空白 / stub | 无显示栈或探活失败（诚实降级）；起 `./scripts/atlas-desktop-stack.sh start` 后见 [p1-desktop-runbook.md](./p1-desktop-runbook.md) |

---

## 6. §7 已知限制（U2）

- **深链 vs `atlas-pc-mode`：** 显式 `#/chat` / `#/debug` 优先并回写 mode。仅当 hash 为空、`#/` 或无法识别时，才用 mode 恢复。点「入口」会清掉 mode。  
- **Chat 上没有的东西：** 高级 Hub、Login、G1 Create group、C1 Channels、调试手风琴 / Raw Log / 协议细按钮。改 Hub 地址或登录走顶栏 **调试**。📎 / 🖥 仍留在 composer。  
- **`?debug=1` 与 `#/debug`：** 查询参数只在当次加载强制进入调试页并展开手风琴，随后从地址栏删除。之后以 hash 和 `atlas-pc-mode` 为准。`atlas-pc-debug` 只控制手风琴，不决定路由。  
- **主布局形态：** 顶栏连接 + **侧栏 agent** + 主对话区（非顶栏-only）。调试页是同一套 DOM，不是第二份连接。  
- **对话流（U3）：** Conversation 只渲染 transcript 气泡（用户 / 助手 / 系统），正文走 Markdown。同一条回复不会再被 `hub:turn_finished` 的 preview 贴第二次。调试页 Events / Hot transcript / Log 仍是原文。见 `docs/pc-ui-u3-checklist.md`。  
- **浏览器-only vs Tauri：** 路由相同（hash + `localStorage`）。`npm run dev` 的浏览器 WebSocket **不能**设 `Authorization`；`oidc`/`static` Hub 需 Tauri 或 CLI。  
- Hub / `bot.*` / `hubClient` 协议面 **未改**（纯呈现）。零新 `bot.*`。不是签名/上架，也不是品牌重绘。

---

## 7. 相关文档

- U3 手测：`docs/pc-ui-u3-checklist.md`  
- U2 手测：`docs/pc-ui-u2-checklist.md`  
- U1 手测：`docs/pc-ui-u1-checklist.md`  
- G1 群组手测：[`docs/g1-group-handtest.md`](./g1-group-handtest.md)  
- C1 频道手测：[`docs/c1-channel-handtest.md`](./c1-channel-handtest.md)  
- **主路径：** [`docs/cli-primary-runbook.md`](./cli-primary-runbook.md)  
- Windows 起栈：[`docs/windows-cli-stack-checklist.md`](./windows-cli-stack-checklist.md)  
- 阶段 runbook：`docs/P*-runbook.md`、`docs/p1-desktop-runbook.md`、`docs/i2-login-runbook.md`、`docs/b1-event-ingest-runbook.md`  
- 飞天验收 / 盘古可行性：knowledge-handoff `feitian-pc-ui-convergence-acceptance.md`、`pangu-pc-ui-convergence-feasibility.md`

---

**一句话：** 先起 CLI 栈（Unix `./scripts/dev-cli-stack.sh` / Win `.\scripts\dev-cli-stack.ps1`），入口选 Chat（或打开 `#/chat`）再三步 Connect → 选 agent → Send；调试能力在 `#/debug`；stub/box/openai 是旁路。
