# MU1 移动 UI 收敛 · 手测清单（Feitian 证据）

> 分支目标：双端 Chat 主屏 +「更多/调试」折叠 · **只藏不砍** · Login 主路径  
> 基线：`main`@`512f80d` · `clients/android` + `clients/ios`  
> 后端任选 stub / cli（勾选时写清）。

## 前置

```bash
cargo run -p atlas-bot-hub
# 或 CLI 栈：ATLAS_AGENT_CLI=tools/mock-cli/mock-atlas-agent-cli.sh ./scripts/dev-cli-stack.sh

cd clients/android && ./gradlew :app:assembleDebug :app:testDebugUnitTest
# iOS: open clients/ios/AtlasBot.xcodeproj（macOS）
```

## M-P1 — Connect → sendPrompt（dev）

| 步 | 期望 |
|----|------|
| 打开 App，确认默认是 Chat（非全控件平铺） | ✓ |
| 点 **Connect**（不必展开高级，默认 Hub URL） | State → ready |
| Agent 已自动 roster / 可选手填 | ✓ |
| 输入 prompt → **Send** | Conversation 出现事件/transcript；未订时自动 subscribe |
| 有意义点击 ≤3 | Connect →（选 agent）→ Send |

证据：________（后端：________）

## M-P2 — 折叠区 Cold / 显式协议

| 步 | 期望 |
|----|------|
| 展开「更多 / 调试」 | ✓ |
| **status** / **roster** | runState / 列表更新 |
| 显式 **subscribe** / **unsubscribe** / **transcriptTail** | Log 有对应行 |

证据：________

## M-P3 —（可选）有票 Login

| 步 | 期望 |
|----|------|
| 高级填 issuer → 主路径 **Login** | 系统浏览器 PKCE；非 WebView |
| 回调后 ticket stored → **Connect** | `hello_ack.user_id=sub` |
| **Logout** | 安全存储清空 |

证据：________

## 自动化

- [ ] Android：`./gradlew :app:testDebugUnitTest` 绿  
- [ ] Android：`./tools/p4-smoke.sh` → `SMOKE_OK`  
- [ ] 未改 `HubClient` / `AuthSession` / 深链 / `bot.*` 协议面（仅 UI 呈现）

## §8 已知限制（实现填实）

| 项 | MU1 选择 |
|----|----------|
| Agent 选择 | 下拉 + 手填 |
| 对话流 | 单栏合并 events + transcript |
| 调试展开 | 手风琴 / DisclosureGroup；偏好记住 |
| 桌面（MD1） | 🖥 + Open desktop 外开；expiresHint 可见 |
| upload | 仍占位未接线 |
| 与 PC U1 | 无侧栏 Create/Interrupt/真 VNC·upload；Login 主路径更显眼 |

## 明确不在 MU1

企微取票、删 Login/P4 入口、改 Hub/`bot.*`、Flutter/RN/KMP、品牌上架。


---

## MD1 — Open desktop 手测（增补）

> 对齐 `feitian-mobile-open-desktop-acceptance.md` MD-P1…P9 · 零新 `bot.command` · 外开非内嵌

| 编号 | 内容 | 期望 |
|------|------|------|
| **MD-P1** | Android HubClient + RPC | `bot.vncDescriptor` 帧单测绿；手测 Log 有方法名 |
| **MD-P2** | Android 外开 | 点 🖥 / Open desktop → Custom Tabs 打开 `vncUrl`；hint 可见 |
| **MD-P3** | iOS 外开 | 同上（`UIApplication.open` / Safari） |
| **MD-P4** | stub 路径 | 打开 stub；App/页 **不谎报**真桌面 |
| **MD-P5** | （可选）proxy | 有 D1 栈时外开 token URL |
| **MD-P6** | 过期刷新 | 再点重 mint（≈5min TTL） |
| **MD-P7** | 失败路径 | 无 agent / RPC 失败 → 错误条；不假装已开 |
| **MD-P8** | 回归 | MU1 Chat；I2.2 Login 外开不回归 |
| **MD-P9** | 文档 / 模拟器 host | `mobile-user-guide` §5；Android `10.0.2.2` / iOS `127.0.0.1` |

明确不做：MD2 内嵌 noVNC/集群、MD3 仅文档、GA2、YOLO、本刀强制 upload。
