# atlas-bot PC 使用说明

> 基线：`main`@`21eddb6` + **U1 PC UI 收敛** · 客户端 `clients/pc`（Tauri 2 + TS）  
> 本阶段是 **信息架构收敛**（Chat 主屏 +「更多/调试」折叠），**不是**最终品牌 UI / 框架重写 / I2.2。

---

## 1. 产品主路径（默认一屏 Chat）

默认进入 Chat 布局，主路径有意义点击 ≤3：

1. **Connect**（顶栏；Hub 地址默认 `ws://127.0.0.1:7700/ws`，改地址点「高级 · Hub URL / Bearer」）  
2. （按需）选 **Agent**；Connect 成功后会自动 `listAgents` 并选中  
3. 输入 prompt → **Send**（未订阅时自动 `subscribe`；旁路 **Interrupt**）

可选：顶栏 **Login / Logout**（Hub 非 `dev` 时）；composer 旁 📎 上传、🖥 Open desktop（亦在调试区）。

| 主屏元素 | 行为 |
|----------|------|
| 顶栏连接态 + Connect/Disconnect | `hello` 后 badge → ready |
| 高级折叠 | Hub WS、Bearer 粘贴 |
| Agent 侧栏 | 下拉 + Create |
| Conversation | 合并展示 transcript tail + 订阅到的 tool/delta 等事件 |
| Composer | Send / Interrupt / immediate |
| 错误条 | 有 Failure 才显示 |

---

## 2. 调试入口在哪

页面底部 **「更多 / 调试 ▾」** 手风琴（默认折叠）。展开后可点到全部原调试台能力（**只藏不砍**）：

| 调试区块 | 入口 |
|----------|------|
| Cold | status / roster / offbox / offbox next |
| Protocol | subscribe / unsubscribe / getTail / listAgents / createAgent |
| 连接信息 | capabilities、connection_id |
| Desktop · attachments | Open desktop、uploadAttachment、attachUpload |
| Raw panes + Log | Events、Hot transcript、完整 Log |

可选：URL `?debug=1` 或 `localStorage.setItem('atlas-pc-debug','1')` → 启动时展开调试面板（面板开关会写回 localStorage）。

手测清单见 `docs/pc-ui-u1-checklist.md`。

---

## 3. 起服务（先 Hub，再开 PC）

本地最小（开发态，**不用登录**）：

```bash
# 终端 1：Hub（默认 ATLAS_AUTH_MODE=dev）
cargo run -p atlas-bot-hub
# ws://127.0.0.1:7700/ws
```

若要真对话后端（Box）：

```bash
# 终端 A
ATLAS_GATEWAY_BACKEND=box ATLAS_BOX_WORKSPACE=./data/box-workspace \
  cargo run -p atlas-bot-gateway

# 终端 B
ATLAS_GATEWAY_URL=http://127.0.0.1:8787 cargo run -p atlas-bot-hub
```

分进程 mid-turn 事件见 `docs/b1-event-ingest-runbook.md`。

PC：

```bash
cd clients/pc && npm i && npm run tauri dev
# 或 npm run dev（浏览器 WS 不能带 Authorization；oidc/static 请用 Tauri）
```

---

## 4. 能力对照（仍可用）

| 能力 | 主屏 / 调试 |
|------|-------------|
| Connect / hello / capabilities | 主屏顶栏；caps 在调试 |
| Bearer / Login (OIDC PKCE) | 高级 + 顶栏 Login；生产靠 Tauri `connect_ws` |
| Cold status/roster/offbox | 调试 |
| list / create / subscribe / send / interrupt / tail | 主屏自动 + 调试细按钮 |
| Events / transcript | 主屏合并流；调试保留原面板 |
| VNC / upload / attach | composer 图标 + 调试 |
| Failure / Log | 错误条 + 调试 Log |

**还不是：** 品牌视觉系统、群组、上架包、I2.2 移动取票、React/Vue 重写。

---

## 5. 常见坑

| 现象 | 原因 / 处理 |
|------|-------------|
| Connect 失败 | Hub 没起，或 WS 不是 `ws://127.0.0.1:7700/ws` |
| `unauthorized` | Hub 非 `dev` 且无有效 Bearer → Login 或粘贴票 |
| 浏览器里带鉴权连不上 | 用 **Tauri**（`connect_ws`），或 CLI `login` |
| sendPrompt 无内容 / echo | Hub 仍在 stub；换成 `backend=box` 或 CLI 网关 |
| 有对话无 mid-turn 事件 | 分进程未配 B1；或未订阅（Send 会自动订） |
| 上传 `args_too_large` | 文件太大；换 &lt;1.5 MiB |
| Open desktop 空白 | stub 正常；真桌面见 P5-real runbook |

---

## 6. §7 已知限制（U1）

- **主布局形态：** 顶栏连接 + **侧栏 agent** + 主对话区（非顶栏-only）。  
- **对话流：** **单栏合并**（transcript 段 + live events 段）；调试区仍保留独立 Events / Hot transcript。  
- **调试展开：** 页底 **手风琴** `<details>`（非抽屉 / 非 `#/debug` 双路由）。  
- **浏览器-only：** `npm run dev` 可用，但浏览器 WebSocket **不能**设 `Authorization`；`oidc`/`static` Hub 需 Tauri 或 CLI。  
- Hub / `bot.*` / `hubClient` 协议面 **未改**（纯呈现）。

---

## 7. 相关文档

- U1 手测：`docs/pc-ui-u1-checklist.md`  
- 阶段 runbook：`docs/P*-runbook.md`、`docs/i2-login-runbook.md`、`docs/b1-event-ingest-runbook.md`  
- 飞天验收 / 盘古可行性：knowledge-handoff `feitian-pc-ui-convergence-acceptance.md`、`pangu-pc-ui-convergence-feasibility.md`

---

**一句话：** 默认 Chat 三步（Connect → 选 agent → Send）；Cold / VNC / upload / 协议细按钮在「更多 / 调试」。
