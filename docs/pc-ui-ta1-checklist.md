# PC UI TA1 · Chat 工具审批 · 手测清单

> 切片 **TA1** · 客户端 `clients/pc` · Chat 里对已有 GA1/CG1 待审批做允许 / 拒绝  
> **零新 `bot.*`**。不新增 Hub 方法，不做 YOLO，不做签名上架。  
> 协议仍是 `hub:tool` 摘要前缀 `[approval_pending approvalId=… tool=…]`，以及 loopback `POST /approve`。  
> 对照：`docs/tool-approval-runbook.md` §6–§7。U3 清单仍在 `docs/pc-ui-u3-checklist.md`。

## 前置

```powershell
# Windows 主路径。CG1 要能看到中途 tool_call，需要 stream。
$env:ATLAS_TOOL_APPROVAL_MODE = "gate"
$env:ATLAS_AGENT_CLI = "$PWD\tools\mock-cli\mock-atlas-agent-cli.cmd"
.\scripts\dev-cli-stack.ps1 -Stream

cd clients/pc
npm i
npm run tauri dev    # 推荐。npm run dev 路由相同，但浏览器 WS 不能带 Authorization
npm run test:unit    # 含 toolApproval
```

Unix 对照：`ATLAS_TOOL_APPROVAL_MODE=gate ATLAS_AGENT_CLI_STREAM=1 ./scripts/dev-cli-stack.sh`，然后 `cd clients/pc && npm run tauri dev`。

Gateway HTTP 默认 `http://127.0.0.1:8787`（`localStorage` 键 `atlas-pc-gw-http`）。可选审批 token 只在调试页「高级」里填写，键 `atlas-pc-gw-approve-token`。默认 loopback、未配置 token 时，**不要打开高级**也能审批。

触发一条 gated 工具：mock CLI 在 stream 下会发出 `tool_call`（默认名 `mock_tool`，未知名即 gated）。也可以设 `MOCK_CLI_TOOL=Shell`。任意 prompt 发出去即可。Deny 走现有 kill / 超时=Deny，不会变成 Allow。

## 硬门槛

| 项 | 期望 |
|----|------|
| Chat 按钮 | 卡片上是 **允许** / **拒绝**，不是 Allow / Deny |
| 不用开调试 | `#/chat` 里卡片在输入框上方。不展开「高级」、不打开「更多 / 调试」就能点 |
| 默认地址 | 未改 Advanced 时，请求发到 `http://127.0.0.1:8787/approve`，body 为 `{ approvalId, decision: "allow" \| "deny" }` |
| 失败可见 | Gateway 没起来、HTTP 非 2xx（除 409）时，**Chat 卡片上**出现中文错误。不只写在 Raw Log |
| 多条不丢 | 同一 Agent 连续多条 pending 都留在列表里，带条数。新的一条不会盖掉旧的 approvalId |
| 其他 Agent | 非当前 Agent 的 pending 不丢。切换 Agent 后能看到并操作；当前页有「其他 Agent 还有 N 条」 |
| 调试仍可观察 | `#/debug` 高级里仍有 Gateway HTTP 与 Approval token。Events / Log 仍能看到 `approval_pending` 和 `/approve` 结果 |
| 协议 | 没有新的 `bot.*`。事件源仍是 `hub:tool` |

证据：________（截图 / 录屏，需看见 允许 / 拒绝；多条时能看见队列）

## P1 — Chat 允许 / 拒绝

| 步 | 期望 |
|----|------|
| 入口选 Chat，或打开 `#/chat` | 看不到「高级 · Hub URL / Bearer」 |
| Connect → 选中会跑工具的 agent → Send | badge → ready |
| 出现待审批卡片 | 标题含「待审批 · N」。按钮为 **允许**、**拒绝** |
| 点 **允许** | 该条从队列消失。Gateway 继续（cli：不杀进程）。Log 可有 `/approve allow`，但 Chat 不依赖 Log |
| 再触发一条，点 **拒绝** | 该条消失。行为与现有 Deny（kill / 超时=Deny）一致 |

证据：________

## P2 — 失败出现在 Chat

| 步 | 期望 |
|----|------|
| 停掉 gateway，或把 Advanced 里的 Gateway HTTP 改成一个没有服务的 loopback 端口并保存 | Chat 仍不用展开高级才能点按钮（地址来自已保存的值或默认） |
| 有待审批时点 **允许** 或 **拒绝** | 卡片内红色错误含「审批失败」和「允许」或「拒绝」。该 approvalId **仍在**队列里 |
| 打开 `#/debug` 看 Log | 同一失败也在 Log 里。Events 仍能看到原来的 `hub:tool` |

证据：________

## P3 — 多条 pending 不丢

| 步 | 期望 |
|----|------|
| 在超时前连续触发两条及以上 gated 工具（或两个 Agent 各一条） | 当前 Agent 的每一条都在列表中，标题条数等于列表条数 |
| 只处理其中一条 | 另一条还在，approvalId 不变 |
| 切到另一个有 pending 的 Agent | 看到那一条的 **允许** / **拒绝** |

证据：________

## P4 — 调试页只做观察

| 步 | 期望 |
|----|------|
| 顶栏「调试」或 `#/debug` | 连接不断 |
| 展开「高级 · Hub URL / Bearer」 | 仍有 Gateway HTTP、Approval token。改完会写入 localStorage，回到 Chat 不用再打开高级 |
| Events / Log | 能看到 pending 与审批结果。Conversation 不回退成 Raw JSON |

证据：________

## 自动化

- [ ] `cd clients/pc && npm run test:unit` 绿（`parseApprovalPending`、队列不丢 id、默认 loopback、中文失败文案、`/approve` body）
- [ ] 未新增 `bot.*` 命令；未改 Hub / Gateway 决策协议
- [ ] 未做 YOLO、签名上架、或把审批搬进新的 Hub 方法

## 实现备注（对照代码，不是协议变更）

| 项 | 行为 |
|----|------|
| 解析 | `clients/pc/src/toolApproval.ts`。只认摘要前缀 `[approval_pending approvalId=… tool=…]` |
| 队列 | `Map` 按 approvalId。同 id 只更新，不挤掉其他 id。非当前 Agent 的事件也会入队 |
| 按钮 | `decisionLabel` → **允许** / **拒绝**。POST 的 `decision` 仍是 `allow` / `deny` |
| 结束 | HTTP 2xx 或 409（已关闭）才从队列移除 |
| 失败 | `formatApprovalFailure` 写在该条卡片上，并 `appendLog` 供调试页查看 |
| 地址 | 输入框优先，否则 `atlas-pc-gw-http`，再否则 `http://127.0.0.1:8787` |
| CORS | Chat 页面和 Gateway 不同源，`application/json` 会先 `OPTIONS /approve`。Gateway 只给 `localhost` / `127.0.0.1` / `::1` / `tauri.localhost` / `ipc.localhost` 回 `Access-Control-Allow-Origin`。其他网站拿不到允许头。curl / smoke 不带 Origin，行为不变 |

## 明确不在 TA1

新 `bot.*`、GA2 Hub approve、产品 YOLO、签名 / 上架、把 Gateway HTTP 表单搬进 Chat、改 G1 / C1 / 桌面 / 上传、声称 cli 具备 box 那种「Allow 之前零副作用」。
