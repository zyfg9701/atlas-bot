# PC UI U2 · 纯 Chat 壳 · 手测清单

> 切片 **U2** · 客户端 `clients/pc` · hash `#/` / `#/chat` / `#/debug` · `localStorage atlas-pc-mode`  
> 能力只藏不砍。零新 `bot.*`。不是签名/上架，也不是品牌重绘。  
> 对照说明：`docs/pc-user-guide.md` §0–§2、§6。U1 清单仍在 `docs/pc-ui-u1-checklist.md`。

## 前置

```bash
# 主路径（推荐）
./scripts/dev-cli-stack.sh
# Windows:
.\scripts\dev-cli-stack.ps1

cd clients/pc && npm i
npm run tauri dev    # 推荐。npm run dev 路由相同，但浏览器 WS 不能带 Authorization
npm run test:unit    # hubClient + authClient + wecomClient + pcMode
```

清掉记忆再看入口：`localStorage.removeItem('atlas-pc-mode')`，地址改为 `#/` 或去掉 hash。

## 路由约定（实现）

| 项 | 行为 |
|----|------|
| 深链 `#/chat` / `#/debug` | 优先于已存 mode，并回写 `atlas-pc-mode` |
| 空 hash 或 `#/` + 有效 mode | 恢复该页并补 hash |
| 无深链且无有效 mode | 入口（Chat / 调试） |
| 顶栏「入口」 | 删除 `atlas-pc-mode`，回到 `#/` |
| `?debug=1` | 当次进入 `#/debug`、展开手风琴，然后去掉该 query |
| `atlas-pc-debug` | 只记住调试页手风琴（`0` 折叠；未设置或 `1` 展开），不选页面 |
| Chat ↔ Debug | 不新建、不断开 `HubClient` |

## P1 — 冷启动入口 → Chat 无调试壳 → Connect → Send

| 步 | 期望 |
|----|------|
| 无 hash、无 `atlas-pc-mode` 打开 PC | 入口两个按钮：Chat、调试 / Debug |
| 点 **Chat** | 地址 `#/chat`，`atlas-pc-mode=chat` |
| 看主屏 | 有连接徽标、Connect、agent 选择、Conversation、Send。**没有**「更多 / 调试」、Raw Log、高级 Hub、Login、Create group、Channels |
| Connect → 选 agent → Send | badge → ready；Conversation 有非空回复（cli 栈 + 方言路径） |
| 有意义点击 ≤3（不含改地址/进调试） | ✓ |

证据：________（截图/录屏/本勾选 + 后端：________）

## P2 — 入口进调试，能力还在

| 步 | 期望 |
|----|------|
| 回入口，点 **调试 / Debug** | `#/debug`，`atlas-pc-mode=debug` |
| 调试手风琴 | 默认展开（除非之前 `atlas-pc-debug=0`） |
| Cold 一条（如 status） | 有输出 |
| Raw Log 或 Events | 可见 |
| 高级 Hub、Login、G1 Create group、C1 Channels、VNC / upload | 都还在，没有删按钮 |

证据：________

## P3 — 已连接时切换 hash 不断连

| 步 | 期望 |
|----|------|
| `#/chat` Connect 到 ready | badge = ready |
| 顶栏「调试」或改成 `#/debug`，再回 `#/chat` | badge 仍是 ready；不要重新出现 disconnected（除非 Hub 自己断开） |

证据：________

## P4 — 刷新恢复 mode

| 步 | 期望 |
|----|------|
| 停在 `#/chat` 刷新 | 仍是 Chat，`atlas-pc-mode=chat` |
| 停在 `#/debug` 刷新 | 仍是 Debug |
| 直接打开 `#/debug`（即使 mode 曾是 chat） | Debug，且 mode 被写成 debug |
| `?debug=1` 打开一次 | 落到 `#/debug` 且手风琴展开；地址栏不再带 `debug=1`。之后可切到 Chat |

证据：________

## 自动化

- [ ] `cd clients/pc && npm run test:unit` 绿（含 `pcMode`：hash、mode、`?debug=1`、手风琴旗标）
- [ ] 未改 `hubClient` / `authClient` / Tauri `connect_ws` / G1 / C1 / 桌面 / 上传协议（只按 mode 显隐）

## §7 已知限制

| 项 | U2 选择 |
|----|---------|
| 深链 vs mode | 深链覆盖并回写；仅空 hash / `#/` / 无法识别的 hash 才按 mode 恢复 |
| Chat 上的 Login / 高级 Hub / G1 / C1 | **完全不在 Chat**。顶栏小字「调试」进 `#/debug`。composer 📎 / 🖥 两页都留 |
| `?debug=1` | 当次强制 `#/debug` + 展开，然后删除 query。`atlas-pc-debug` 不导航 |
| 浏览器 vs Tauri | hash 与 `localStorage` 相同；鉴权 WS 仍要 Tauri |

## 明确不在 U2

新 `bot.*`、改 Hub/Gateway 协议、签名上架、品牌重绘、换 React/Vue、C1 红条消噪、改 G1/C1/桌面/上传行为、会话历史产品化。
