# PC UI U1 · 手测清单（Feitian 证据）

> 分支目标：Chat 主屏 +「更多/调试」折叠 · **只藏不砍**  
> 基线：`main`@`21eddb6` · 客户端 `clients/pc`  
> 后端任选 stub / cli / box（勾选时写清）。

## 前置

```bash
# Hub（dev，无登录）
cargo run -p atlas-bot-hub

cd clients/pc && npm i
npm run tauri dev    # 或 npm run dev
npm run test:unit    # hubClient + authClient
```

## P1 — Connect → sendPrompt

| 步 | 期望 |
|----|------|
| 打开 PC，确认默认是 Chat（非全控件平铺） | ✓ |
| 点 **Connect**（不必展开高级，默认 Hub URL） | badge → ready |
| Agent 已自动 list / 选中（或手选） | ✓ |
| 输入 prompt → **Send** | 非空回复出现在 Conversation（或 transcript 段）；未订时自动 subscribe |
| 有意义点击 ≤3 | Connect →（选 agent）→ Send |

证据：________（截图/录屏/本勾选 + 后端：________）

## P2 — 调试 VNC

| 步 | 期望 |
|----|------|
| 展开「更多 / 调试」或点 composer 🖥 | ✓ |
| **Open desktop** | stub/proxy `vncUrl` 可见并可打开 |

证据：________

## P3 — 调试 upload

| 步 | 期望 |
|----|------|
| 调试区选小文件 → **uploadAttachment**（或 📎） | upload path / uploadId |
| **attachUpload (last id)** | attach path 更新；过大文件 → Failure `args_too_large` |

证据：________

## P4 — dev 无 Login

| 步 | 期望 |
|----|------|
| `ATLAS_AUTH_MODE=dev` 不点 Login | Connect 成功 |
| （可选）oidc/static + Tauri Bearer | 不回归；浏览器-only 仍不能带头 |

证据：________

## 自动化

- [ ] `cd clients/pc && npm run test:unit` 绿  
- [ ] 未改 `hubClient` / `authClient` / Tauri `connect_ws` 协议面（仅 UI 胶水）

## §7 已知限制（实现填实）

| 项 | U1 选择 |
|----|---------|
| 主布局 | 顶栏 + **侧栏 agent** + 对话主区 |
| 对话流 | **单栏合并** transcript + live events；调试保留双面板 |
| 调试展开 | 页底 **手风琴**；`?debug=1` / `atlas-pc-debug=1` |
| 浏览器-only | 无 WS Authorization；鉴权 Hub 用 Tauri/CLI |

## 明确不在 U1

换 React/Vue、删能力入口、改 Hub/`bot.*`、I2.2、品牌设计系统。
