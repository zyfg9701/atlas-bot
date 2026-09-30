# PC UI U4 · Chat Hub URL（可选 Bearer）· 手测清单

> 切片 **U4** · 客户端 `clients/pc` · `#/chat` 可改 Hub WebSocket，可选 Bearer，写入 `localStorage`  
> **零新 `bot.*`**。不改 Hub / Gateway 协议，不做签名、上架、钥匙串。  
> **不要**和 Gateway HTTP `8787`（审批）搞混。Chat 连接条里没有 Gateway 字段。审批地址仍是调试页「高级 · Gateway HTTP」，键 `atlas-pc-gw-http`（TA1）。  
> 对照：`docs/pc-user-guide.md` §1。TA1 清单仍在 `docs/pc-ui-ta1-checklist.md`。

## 前置

```powershell
# Windows 主路径
.\scripts\dev-cli-stack.ps1

cd clients/pc
npm i
npm run tauri dev    # 推荐。npm run dev 路由相同，但浏览器 WS 不能带 Authorization
npm run test:unit    # 含 hubConnection
```

Unix 对照：`./scripts/dev-cli-stack.sh`，然后 `cd clients/pc && npm run tauri dev`。

未改过地址时，Hub 仍是 `ws://127.0.0.1:7700/ws`（`DEFAULT_HUB_WS`）。  
存储键：`atlas-pc-hub-ws`、`atlas-pc-hub-bearer`。改字段后在失焦（change / blur）时写入；启动时读回同一对输入框（`#url`、`#tokenPaste`）。Chat 和调试页共用这一对，没有第二份。

Bearer 是明文 `localStorage`，只适合私人自托管。连接条上有提示：**不要在共用电脑上保存**。

## 硬门槛

| 项 | 期望 |
|----|------|
| Chat 可见 | `#/chat` 顶栏 Connect 下方有 Hub WS 和可选 Bearer。不必打开调试页，也不必展开「高级」 |
| 同一对输入 | `#/debug` 没有第二套 Hub URL / Bearer。改一处，另一页也是它 |
| 刷新仍在 | 改成自定义 `ws://` 或 `wss://` 后刷新，输入框仍是该地址 |
| Connect 用它 | Connect，以及 Tauri `connect_ws`，用这个地址，而不是写死默认值 |
| 空或非法 | 清空，或填入 `http://127.0.0.1:8787` 等非 `ws:` / `wss:`，失焦或 Connect 时回到 `ws://127.0.0.1:7700/ws` |
| Bearer | 可空。填写后刷新仍在；清空并失焦后，刷新仍为空 |
| 不是 Gateway | Chat 连接条没有 Gateway HTTP，没有 Approval token。`http://127.0.0.1:8787` 不会被当成 Hub URL |
| 共用电脑 | 界面写明 Bearer 在本机 localStorage，仅供私人自托管 |
| 协议 | 没有新的 `bot.*`。没有签名 / store |

证据：________（截图 / 录屏，需看见自定义 Hub URL 在刷新后仍在输入框里）

## P1 — Windows 冒烟：改 Hub URL，刷新，Connect

| 步 | 期望 |
|----|------|
| `cd clients/pc` 然后 `npm run tauri dev` | 窗口起来，入口或直接 `#/chat` |
| 看 `#/chat` 顶栏下方 | Hub WS 默认 `ws://127.0.0.1:7700/ws`。**没有** Gateway `8787` 输入框 |
| 改成另一个 `ws://` 或 `wss://`（本机 Hub 的实际地址），点到输入框外 | `localStorage['atlas-pc-hub-ws']` 为该值 |
| 刷新窗口 | 输入框仍是刚才的地址，不是默认值 |
| Connect | badge → ready。连接的是这条地址（Hub 需已在该 URL 上） |
| 把地址清空，或改成 `http://127.0.0.1:8787`，再失焦 | 输入框回到 `ws://127.0.0.1:7700/ws`，存储也是默认值 |

证据：________

## P2 — 可选 Bearer 同样持久化

| 步 | 期望 |
|----|------|
| Bearer 留空，Connect（`ATLAS_AUTH_MODE=dev` 的 Hub） | 能连。键 `atlas-pc-hub-bearer` 不存在 |
| 填入一枚票，点到框外，刷新 | 输入框仍是该票 |
| 清空，点到框外，刷新 | 空 |
| 调试页 Login 成功（OIDC 或 WeCom） | 同一 Bearer 框写入票，并写入 `atlas-pc-hub-bearer`。刷新后还在 |
| Logout | 框被清空；刷新后仍空 |

证据：________

私人自托管可以靠这一条在刷新后继续带 Bearer。共用电脑不要保存。

## P3 — 调试页不双份，Gateway 仍只在调试

| 步 | 期望 |
|----|------|
| 顶栏「调试」或 `#/debug` | 连接不断。Hub WS / Bearer 仍是 Chat 上那一对 |
| 展开「高级 · Gateway HTTP」 | 有 Gateway HTTP、Approval token。**没有**另一组 Hub URL / Bearer |
| 回到 `#/chat` | 仍没有 Gateway 字段。Hub 地址与调试页一致 |

证据：________

## 自动化

- [ ] `cd clients/pc && npm run test:unit` 绿（`normalizeHubWs`、`normalizeHubBearer`、load / save；Gateway HTTP 不能当 Hub URL）
- [ ] 未新增 `bot.*` 命令；未改 HubClient / Tauri `connect_ws` 协议面（只读已保存的 URL 和 Bearer）
- [ ] 未把 Gateway HTTP 或 Approval token 放进 Chat 连接条

## 实现备注（对照代码，不是协议变更）

| 项 | 行为 |
|----|------|
| 规范化 | `clients/pc/src/hubConnection.ts`。只接受带 host 的 `ws:` / `wss:`；空串和其他协议回到 `DEFAULT_HUB_WS` |
| 存储 | `atlas-pc-hub-ws`、`atlas-pc-hub-bearer`。change / blur 写入。启动时 `loadHubConnection` 填进 `#url` / `#tokenPaste` |
| 非法存量 | 若存储里已是 `http://127.0.0.1:8787` 之类，启动时改写成默认 Hub URL，避免刷新后又填回 Gateway |
| Connect | 先提交这两个字段，再 `HubClient.setUrl`，Tauri 路径把同一 URL 传给 `connect_ws` |
| Bearer 风险 | 明文 localStorage，不是钥匙串、也不是加密落盘。提示在连接条上 |
| 审批 | Gateway 仍走 TA1：`atlas-pc-gw-http` / `atlas-pc-gw-approve-token`，只在 `#advancedHub`（debug-only） |

## 明确不在 U4

新 `bot.*`、签名 / 上架、钥匙串或加密落盘、把 Gateway HTTP / Approval token 放进 Chat、改审批或 Hub 协议、改 G1 / C1 / 桌面 / 上传。
