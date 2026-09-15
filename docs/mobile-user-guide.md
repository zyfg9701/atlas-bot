# atlas-bot 移动使用说明（MU1）

> 基线：`main`@`512f80d` + **MU1 移动 UI 收敛** · Android Compose / iOS SwiftUI  
> 信息架构对齐 PC U1（`docs/pc-user-guide.md`）：**一屏 Chat** +「更多/调试」；**非**品牌 UI / 跨端重写。  
> I2.2 Login **留在主路径**；企微取票未做；**MD1 Open desktop** 已接线（外开）；upload **仍未接线**。

---

## 1. 产品主路径（默认一屏 Chat）

双端默认进入 Chat 布局（非 Cold/协议按钮全平铺）。主路径有意义点击 ≤3：

1. **Connect**（顶栏；Hub / OIDC issuer 在「高级 · Hub URL / OIDC」折叠）  
2. （按需）**Login** — 系统浏览器 PKCE（Android Custom Tabs / iOS ASWebAuthenticationSession）；`dev` Hub 无票可连  
3. 选 **Agent**（下拉/手填；Connect 后自动 `roster`）→ 输入 prompt → **Send**（未订则自动 `subscribe`）

| 主屏元素 | 行为 |
|----------|------|
| 顶栏连接态 + Connect/Disconnect | `hello` 后 State → ready |
| Login / Logout | **主路径保留**（I2.2）；Logout 清安全存储 |
| 高级折叠 | Hub WS、OIDC issuer、可选粘贴 Bearer |
| Agent | 下拉（roster）+ 手填 agentId |
| Conversation | 单栏合并：订阅事件 + transcript 刷新段 |
| Composer | Send；🖥 Open desktop（外开）；📎 upload 未接线 |
| 错误条 | 有 Failure 才显示 |

---

## 2. 调试入口在哪

页面底部（滚动末尾）**「更多 / 调试 ▾」** 手风琴（默认折叠；偏好可记住展开）。展开后可点到全部原 P4 能力（**只藏不砍**）：

| 调试区块 | 入口 |
|----------|------|
| Cold | status / roster |
| Protocol | 显式 subscribe / unsubscribe / transcriptTail |
| 连接信息 | capabilities、connection_id；redirect / no WebView 说明 |
| Desktop · upload | **Open desktop** 可点（`bot.vncDescriptor` 外开）；upload 仍占位 |
| Log | 完整滚动 Log |

Android：展开状态写入 `SharedPreferences` `atlas_bot_mu1` / `debug_open`。  
iOS：`UserDefaults` `atlas_bot_mu1_debug_open`。

手测清单见 `docs/mu1-mobile-ui-checklist.md`。

---

## 3. 起服务再开 App

推荐与 PC 相同：**先起 CLI 栈 / Hub，再 Connect**。

```bash
# Unix mock
ATLAS_AGENT_CLI=tools/mock-cli/mock-atlas-agent-cli.sh ./scripts/dev-cli-stack.sh

# 或 Hub-only stub
cargo run -p atlas-bot-hub
```

- **Android 模拟器** Hub：高级里默认常为 `ws://10.0.2.2:7700/ws`（见客户端默认）  
- **iOS 模拟器**：`ws://127.0.0.1:7700/ws`  
- OIDC 手测：`cargo run -q -p atlas-bot-auth-client --bin mock-oidc`；详见 `docs/i2-login-runbook.md` / `docs/i22-mobile-ticket-checklist.md`

```bash
# Android
cd clients/android && ./gradlew :app:assembleDebug :app:testDebugUnitTest

# iOS（需 macOS + Xcode）
cd clients/ios && open AtlasBot.xcodeproj
```

---

## 4. 能力对照（仍可用）

| 能力 | 主屏 / 调试 |
|------|-------------|
| Connect / hello / capabilities | 主屏顶栏；caps / connection_id 在调试 |
| Login / Logout (OIDC PKCE) | **主路径**；issuer 在高级 |
| Cold status / roster | 调试（Connect 后主路径自动 roster） |
| subscribe / sendPrompt / transcriptTail | 主屏自动 + 调试细按钮 |
| Events / transcript | 主屏合并 Conversation |
| Open desktop（MD1） | 主屏 🖥 + 调试 Open desktop；外开 `vncUrl` |
| upload | 调试占位「未接线」（本刀不强制） |
| Failure / Log | 错误条 + 调试 Log |

**还不是：** 企微取票、品牌设计系统、商店上架、Flutter/RN/KMP、内嵌 noVNC/集群（MD2）、本刀强制 upload、改 `bot.*` 形状 / Hub。

---

## 5. Open desktop（MD1）

对齐 PC `doVnc`：选 agent → 点主屏 **🖥** 或「更多/调试」**Open desktop** → Hub 热方法 **`bot.vncDescriptor`**（**不是** `bot.command`）→ 外开返回的 `vncUrl`，并展示 `expiresHint`。

| 端 | 外开方式 |
|----|----------|
| **Android** | Custom Tabs（失败则 `ACTION_VIEW`）；**非** App 内嵌 noVNC WebView |
| **iOS** | `UIApplication.open` → 系统 Safari；**非** 内嵌 noVNC WebView |

| 项 | 说明 |
|----|------|
| 形状 | `{ vncUrl, expiresHint }` 不变；零新 `bot.command` |
| expiresHint | 成功路径可见（`null` 则明示无到期）；≈5min TTL；过期 / 410 后再点 **重 mint** |
| stub / degrade | 透传打开 stub 或 proxy token URL；App **不声称**「已连接真桌面」；页文案由 gateway 诚实降级 |
| upload | **本刀不接线**（仍占位） |
| 非本刀 | MD2 内嵌/集群、录屏、GA2、YOLO |

### 模拟器 host 可达性

gateway 签发的 `vncUrl` 常含 `127.0.0.1:8787`。手测时注意：

| 环境 | Hub WS 默认 | 打开 `vncUrl` 时 |
|------|-------------|------------------|
| **Android 模拟器** | `ws://10.0.2.2:7700/ws` | 若 `vncUrl` 是 `127.0.0.1`，模拟器内指模拟器自身 — 需把 host 换成 **`10.0.2.2`**，或用真机局域网 IP，或在宿主浏览器对照 |
| **iOS 模拟器** | `ws://127.0.0.1:7700/ws` | 模拟器与宿主共享 loopback，`127.0.0.1:8787` 通常可达 |
| **真机** | 填局域网 Hub IP | `vncUrl` host 也须对该设备可达（勿写死仅本机 loopback 给真机） |

**不**改 descriptor 形状迁就模拟器；手测/runbook 说明即可。详见 `docs/p1-desktop-runbook.md`（服务端 stub/proxy）与本节。

---

## 6. 常见坑


| 现象 | 原因 / 处理 |
|------|-------------|
| Connect 失败 | Hub 没起，或模拟器地址不对（`10.0.2.2` vs `127.0.0.1`） |
| `unauthorized` | Hub 非 `dev` 且无有效 Bearer → **Login** 或粘贴票 |
| Login 无回调 | 确认 IdP 注册 `atlasbot://auth/callback`；非 WebView |
| Send 无回复 | Hub stub echo 或未起 CLI 栈；展开调试看 Log / transcriptTail |
| 找不到 status/roster | 展开底部「更多 / 调试」 |
| Open desktop 打不开 stub/token | 模拟器 host：Android 常需 `10.0.2.2`；见 §5 |
| 以为「已连真桌面」 | stub/degrade 页诚实；App 只外开 URL，不声称真桌面 |

---

## 7. §8 已知限制（MU1 / MD1）

- **Agent 选择：** 下拉（roster）+ **手填** agentId（非横向 chips）。  
- **对话流：** **单栏合并**（live events + transcript 刷新段）；无独立双面板。  
- **调试：** 页内 **手风琴 / DisclosureGroup**（非双 Tab / 非 sheet 路由）。  
- **桌面：** **MD1 外开**已接线（见 §8）；upload **仍占位**。  
- **与 PC U1 差异：** 移动无侧栏 Create agent / Interrupt；桌面走系统浏览器外开（非内嵌 noVNC）；Login 因 I2.2 在主路径更显眼；信息架构同构而非像素级一致。

---

## 8. 相关文档

- MU1 手测：`docs/mu1-mobile-ui-checklist.md`  
- PC 对照：`docs/pc-user-guide.md`  
- I2.2：`docs/i2-login-runbook.md`、`docs/i22-mobile-ticket-checklist.md`  
- P4：`docs/P4-runbook.md`  
- 飞天验收 / 盘古可行性：knowledge-handoff `feitian-mobile-open-desktop-acceptance.md`、`pangu-mobile-open-desktop-feasibility.md`
- 桌面真用（服务端）：`docs/p1-desktop-runbook.md`

---

**一句话：** Connect →（按需 Login）→ 选 agent → Send；🖥 / Open desktop 外开 `vncUrl`；Cold / Log / upload 占位在「更多 / 调试」。
