# PC UI U3 · Chat 气泡（去重 + Markdown）· 手测清单

> 切片 **U3** · 客户端 `clients/pc` · Conversation 只渲染 transcript 气泡  
> 零新 `bot.*`。不改 Hub / Gateway 协议，不改 G1 / C1、桌面、上传。  
> 对照：`docs/pc-user-guide.md` §1–§2。U2 清单仍在 `docs/pc-ui-u2-checklist.md`。

## 前置

```bash
# 主路径（推荐）
./scripts/dev-cli-stack.sh
# Windows:
.\scripts\dev-cli-stack.ps1

cd clients/pc && npm i
npm run tauri dev    # 推荐。npm run dev 路由相同，但浏览器 WS 不能带 Authorization
npm run test:unit    # 含 chatMessages + chatMarkdown
```

发一条会返回 Markdown 的 prompt，例如：

```text
用 Markdown 回答：加粗一个词，并给两行列表。
```

## 验收

| 项 | 期望 |
|----|------|
| 同一轮回复 | Conversation 里助手正文只出现 **一次**（transcript 气泡）。不再有 transcript JSON 段 + `hub:turn_finished` preview 各一份 |
| Chat Conversation | 没有 `hub:` 行，没有 `—— transcript ——` / `—— live events ——`，没有 Raw JSON |
| Markdown | 助手（和用户）气泡里 **粗体**、列表可见，不是字面 `**` 或 `\n` |
| 调试页 | `#/debug` 手风琴里 **Events** 仍能看到 `hub:turn_finished`；Hot transcript 仍是 JSON 原文；Log 仍是原文 |
| 两页气泡 | Chat 与 Debug 的 Conversation 都是同一套气泡，协议不回到 Conversation |

证据：________（截图 / 录屏）

## P1 — Chat 一条回复，只一个气泡

| 步 | 期望 |
|----|------|
| `#/chat` → Connect → 选 agent → 发送上面的 prompt | badge → ready |
| 看 Conversation | 一条用户气泡 + **一条**助手气泡 |
| 助手气泡 | 粗体、列表已排版；正文里看不到 `**`；看不到 `hub:turn_finished` |
| 等 turn 结束 | 气泡不变成两份相同正文 |

证据：________

## P2 — 调试页协议还在，Conversation 不回退

| 步 | 期望 |
|----|------|
| 顶栏「调试」或打开 `#/debug` | 连接不断（badge 仍 ready） |
| Conversation | 仍是气泡，没有 `▸ #n hub:` 行 |
| 展开「更多 / 调试」 | Events 含 `hub:turn_finished`；Hot transcript 是 `entries` JSON；Log 有 turn finished |

证据：________

## 自动化

- [ ] `cd clients/pc && npm run test:unit` 绿（`parseTranscriptMessages`、`dedupeMessages`、乐观发送、Markdown 消毒）
- [ ] 未新增 `bot.*` 命令；`package.json` 依赖只增加 `marked` 与 `dompurify`（测试用 `jsdom`）
- [ ] 未改 Hub / Gateway 协议、G1 / C1、桌面、上传

## 实现备注（对照代码，不是协议变更）

| 项 | 行为 |
|----|------|
| Tail 形状 | `getAgentTranscriptTail` → `{ entries: [{ id, role, text, seq }], agentId }`。只读 `text`，不读事件 `preview` |
| 去重 | 有 `id` 则按 id 折叠；没有 id 时，相邻且 `role + content` 哈希相同的气泡折叠 |
| 乐观发送 | Send 后先画本地用户气泡；tail 出现新的同文 user id 后撤掉，避免和 transcript 叠成两条 |
| Markdown | `marked` + DOMPurify，消毒后再 `innerHTML`。调试原文面板仍是 `textContent` |

## 明确不在 U3

新 `bot.*`、改 Hub/Gateway 协议、流式 delta 打进气泡、会话历史上翻产品化、签名上架、品牌重绘、换 React/Vue、改 G1/C1/桌面/上传。
