# atlas-bot 移动使用说明（MU1）

> 基线：`main`@`512f80d` + **MU1 移动 UI 收敛** · Android Compose / iOS SwiftUI  
> 信息架构对齐 PC U1（`docs/pc-user-guide.md`）：**一屏 Chat** +「更多/调试」；**非**品牌 UI / 跨端重写。  
> I2.2 Login **留在主路径**；企微取票未做；VNC/upload **未接线**（调试区占位）。

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
| Composer | Send；🖥📎 未接线提示 |
| 错误条 | 有 Failure 才显示 |

---

## 2. 调试入口在哪

页面底部（滚动末尾）**「更多 / 调试 ▾」** 手风琴（默认折叠；偏好可记住展开）。展开后可点到全部原 P4 能力（**只藏不砍**）：

| 调试区块 | 入口 |
|----------|------|
| Cold | status / roster |
| Protocol | 显式 subscribe / unsubscribe / transcriptTail |
| 连接信息 | capabilities、connection_id；redirect / no WebView 说明 |
| Desktop · upload | **占位「未接线」**（不砍未来约定） |
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
| VNC / upload | 调试占位「未接线」 |
| Failure / Log | 错误条 + 调试 Log |

**还不是：** 企微取票、品牌设计系统、商店上架、Flutter/RN/KMP、改 `bot.*` / Hub。

---

## 5. 常见坑

| 现象 | 原因 / 处理 |
|------|-------------|
| Connect 失败 | Hub 没起，或模拟器地址不对（`10.0.2.2` vs `127.0.0.1`） |
| `unauthorized` | Hub 非 `dev` 且无有效 Bearer → **Login** 或粘贴票 |
| Login 无回调 | 确认 IdP 注册 `atlasbot://auth/callback`；非 WebView |
| Send 无回复 | Hub stub echo 或未起 CLI 栈；展开调试看 Log / transcriptTail |
| 找不到 status/roster | 展开底部「更多 / 调试」 |

---

## 6. §8 已知限制（MU1）

- **Agent 选择：** 下拉（roster）+ **手填** agentId（非横向 chips）。  
- **对话流：** **单栏合并**（live events + transcript 刷新段）；无独立双面板。  
- **调试：** 页内 **手风琴 / DisclosureGroup**（非双 Tab / 非 sheet 路由）。  
- **桌面·上传：** **仅占位「未接线」**（PC U1 已产品化；移动不假装已有）。  
- **与 PC U1 差异：** 移动无侧栏 Create agent / Interrupt / 真 VNC·upload 图标接线；Login 因 I2.2 在主路径更显眼；信息架构同构而非像素级一致。

---

## 7. 相关文档

- MU1 手测：`docs/mu1-mobile-ui-checklist.md`  
- PC 对照：`docs/pc-user-guide.md`  
- I2.2：`docs/i2-login-runbook.md`、`docs/i22-mobile-ticket-checklist.md`  
- P4：`docs/P4-runbook.md`  
- 飞天验收 / 盘古可行性：knowledge-handoff `feitian-mobile-ui-convergence-acceptance.md`、`pangu-mobile-ui-convergence-feasibility.md`

---

**一句话：** Connect →（按需 Login）→ 选 agent → Send；Cold / 显式协议按钮 / Log / 未接线桌面上传在「更多 / 调试」。
