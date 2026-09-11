# WeCom ticket checklist (W1 / WM1 / WL1 handtest)

> **落点选择（WL1）：** 扩写本页（WM-P5 + **WL-C / WL-P / WL-A / WL-I**）+ 失败矩阵 + 证据模板；**不**另建 `wecom-live-handtest-checklist.md`，避免与 runbook 双源漂移。  
> 真机叙事长文：[`i2-login-runbook.md`](./i2-login-runbook.md) **§ 真机企微联调（黄标）**。  
> L1 风格对照：[`live-agent-handtest-checklist.md`](./live-agent-handtest-checklist.md)。  
> **不做：** T2 MSI、改 `bot.*`、WebView 主路径、CI 打真企微、删 mock、产品证据导出按钮。

---

## Mock 主路径（日常 / CI · 必须保留）

Offline mock path (preferred for CI / local):

1. `cargo run -q -p atlas-bot-auth-client --bin mock-wecom` — note base URL
2. Hub: `ATLAS_AUTH_MODE=static` + `ATLAS_AUTH_JWT_SECRET=…` + `ATLAS_WECOM_CORP_ID/AGENT_ID/SECRET` + `ATLAS_WECOM_API_BASE=<mock>`
3. CLI: `ATLAS_I2_NO_BROWSER=1 ATLAS_TICKET_PROVIDER=wecom ATLAS_WECOM_AUTHORIZE_BASE=<mock> ATLAS_HUB_HTTP=http://127.0.0.1:7700 atlas-bot-cli login --provider wecom`
4. `atlas-bot-cli status` → `hello_ack.user_id=wecom:<corpId>:<userid>`
5. Switch back: `ATLAS_TICKET_PROVIDER=oidc` / `login --provider oidc` — I2 path unchanged
6. `ATLAS_AUTH_MODE=dev` without ticket still connects

Automated: `cargo test -p atlas-bot-cli --test wecom_smoke -- --nocapture --test-threads=1` → `SMOKE_OK wecom…`

See `docs/i2-login-runbook.md` § WeCom / § Mock WeCom.

### 回归位（不回退）

- [ ] mock-wecom 仍可起；Hub `API_BASE` 指 mock
- [ ] `wecom_smoke` 绿（或 docs-only PR 声明未改协议代码）
- [ ] WM-P1..P4 仍可勾（下表）
- [ ] CI / Actions **未**改成打 `qyapi.weixin.qq.com`

---

## Mobile WM1 handtest (WM-P*)

Offline mock preferred (same mock-wecom + Hub static as above).

| ID | Steps | Expect |
|----|-------|--------|
| **WM-P1** | Android: provider=wecom → Login (Custom Tabs) → exchange → Connect | hello `sub=wecom:…` (emulator/logs **or** unit tests + this doc) |
| **WM-P2** | iOS: same with ASWebAuthenticationSession | same; Linux CI = logic unit tests + doc (macOS/handtest yellow) |
| **WM-P3** | provider=oidc | I2.2 OIDC Login unchanged |
| **WM-P4** | `ATLAS_AUTH_MODE=dev`, no ticket | Connect still works |
| **WM-P5** | live WeCom yellow-tag（见下可勾选子步 + **WL-***） | register redirect; hello `sub`; yellow-tag only；样本可「环境待补」 |

Hard bans: no WebView primary; no secret in mobile package; no new `bot.*`; Bearer-only; CI never hits `qyapi.weixin.qq.com`.
Not in WM1/WL1: T2 MSI, UI re-convergence, I2.3, W2.

### WM-P5 · 真机黄标可勾选子步

前置：企微自建应用已建；secret **仅** Hub；runbook § 真机企微联调 已读。

- [ ] **P5.1** 企微后台已抄 corpId / agentId；secret 写入 Hub 私密 env（未进 git / PR）
- [ ] **P5.2** 登记 redirect：`http://127.0.0.1:8765/callback` **且** `atlasbot://auth/callback`
- [ ] **P5.3** Hub：`static` + JWT secret + 真三件套 + 真 `API_BASE`（或默认 qyapi）
- [ ] **P5.4** 客户端：真 `AUTHORIZE_BASE`；`ATLAS_TICKET_PROVIDER=wecom`；CLI **未**设 `ATLAS_I2_NO_BROWSER`；CLI 钉 `ATLAS_OIDC_REDIRECT_PORT=8765`
- [ ] **P5.5** 至少一端完成 Login → exchange → Connect → `hello_ack.user_id=wecom:<corpId>:<userid>`（Bearer-only）
- [ ] **P5.6** 证据短表已填 + 脱敏三勾；失败则记 **R#**
- [ ] **P5.7** **切回 mock**（AUTHORIZE_BASE / API_BASE）；确认 CI 仍零外网

各端细步用下方 **WL-C / WL-P / WL-A / WL-I**。

---

## WL1 真机联调位（CLI / PC / Android / iOS）

每端期望共性：

1. Login（provider=`wecom`）→ 系统浏览器授权 → callback  
2. Hub `POST /auth/wecom/exchange` → 本地存票（`provider=wecom`）  
3. Connect → WS `Authorization: Bearer` only  
4. `hello_ack.user_id` / sub = `wecom:<corpId>:<userid>`  
5. 移动：**无** WebView 主路径；secret **不**在 APK/IPA  

样本状态：本切片开写环境 **无真企微账号** → 各端证据标 **「环境待补」**；文档包齐仍可合。

### WL-C · CLI

- [ ] `ATLAS_OIDC_REDIRECT_PORT=8765`；`ATLAS_WECOM_AUTHORIZE_BASE=<真>`；Hub HTTP 指向本机 exchange Hub
- [ ] **勿**设 `ATLAS_I2_NO_BROWSER`
- [ ] `atlas-bot-cli login --provider wecom` → 浏览器授权 → exchange 成功
- [ ] `atlas-bot-cli status` / Connect → `hello_ack.user_id=wecom:<corpId>:<userid>`
- [ ] 凭证含 `provider=wecom`（勿把整份 `credentials.json` 贴进证据）
- [ ] 证据：□ 已填短表  □ 环境待补

### WL-P · PC

- [ ] Login UI：provider=`wecom`；corp/agent/authorize base 为真值
- [ ] redirect 与后台一致：`http://127.0.0.1:8765/callback`（PC 默认 prompt）
- [ ] 系统浏览器 → callback → exchange → **Tauri** Connect（Bearer on upgrade）
- [ ] `hello_ack.user_id=wecom:<corpId>:<userid>`；Bearer-only（无 query token）
- [ ] 证据：□ 已填短表  □ 环境待补

### WL-A · Android

- [ ] provider=`wecom`；Custom Tabs（**非** WebView）
- [ ] 深链 `atlasbot://auth/callback` 已在企微后台 + Manifest
- [ ] exchange → TokenStore → Connect → hello `wecom:…`
- [ ] 确认 APK **无** `ATLAS_WECOM_SECRET`
- [ ] 证据：□ 已填短表  □ 环境待补

### WL-I · iOS

- [ ] provider=`wecom`；ASWebAuthenticationSession（**非** WebView）
- [ ] 同深链 `atlasbot://auth/callback`（Info.plist URL Types）
- [ ] exchange → Keychain TokenStore → Connect → hello `wecom:…`
- [ ] 确认 IPA **无** secret
- [ ] 证据：□ 已填短表  □ 环境待补

---

## 失败对照表 R1–R9

| 行 | 现象 | 常见根因 | 期望表现 | 处置 |
|----|------|----------|----------|------|
| **R1** | 授权后卡死 / 无法回 App | **redirect 未登记** 或与 `ATLAS_WECOM_REDIRECT_URI` / 实际 loopback 不一致 | 浏览器停错误页或打不开深链 | 企微后台补登记；PC/CLI 核对 **8765**；移动核对 `atlasbot://auth/callback` |
| **R2** | 授权成功但 exchange 4xx | **corp/agent 与 secret 不匹配**；或 code 已用/过期 | Hub 可观测错误；客户端不假成功 | 核对 Hub 三件套；重走 authorize 取新 code |
| **R3** | exchange 200 但 Connect `unauthorized` | **JWT secret 错**；或 `ATLAS_AUTH_MODE` 非可验换发票（手测应用 `static`） | `-32002` 或升级失败 | mint 与 verify 同 `ATLAS_AUTH_JWT_SECRET`；手测用 `static` |
| **R4** | 回调即失败「state」 | **state mismatch**（二次 Login / 冷启丢 pending） | 客户端可观测失败 | 重开 Login；勿并行多会话 |
| **R5** | Hub 调企微超时/连接失败 | **网络到 qyapi** / 代理 / 防火墙 | exchange 失败；日志含上游错误 | 出网策略；临时回 mock 证本机栈 |
| **R6** | authorize URL 打不开 | **AUTHORIZE_BASE 仍指向 mock** 或拼错 | 浏览器 DNS/404 | 切真 authorize 主机；与 API_BASE **成对**切换 |
| **R7** | hello `user_id` 非 `wecom:…` | 走了 OIDC / 粘贴了别的 Bearer / provider 未切 | sub 形不符 | 确认 `ATLAS_TICKET_PROVIDER=wecom` 与本次 Login |
| **R8** | 移动能登 PC 不能（或反向） | **只登记了一端 redirect** | 单端成功 | loopback **与** 深链都登记 |
| **R9** | CI 变红 / 流水线打外网 | 误把真 `API_BASE` 写进 CI | 违反硬门槛 | **立即改回 mock**；WL3（CI 真企微）已拒 |

---

## 证据模板（短表 · §2.4）

复制一份按端填写。证据放私密附件 / Notion / 本机 — **勿**把 secret / 完整 token 进 git。

```text
日期（Asia/Shanghai）：
基线 commit / 端（CLI|PC|Android|iOS）：
corpId（可打码）：agentId（可打码）：
AUTHORIZE_BASE / API_BASE（主机名即可）：
redirect 已登记：□ loopback (127.0.0.1:8765/callback)  □ atlasbot://auth/callback
Hub ATLAS_AUTH_MODE：
hello_ack.user_id / sub：wecom:<…>:<…>
日志时间戳（authorize / callback / exchange / hello）：
截图附件名（已确认无 secret/token）：
脱敏确认：□ 无 secret  □ 无完整 JWT  □ 无 credentials.json / .env
结果：□ 通过  □ 失败→对照表行：R____  □ 环境待补
```

### 样本归档位（本切片）

| 端 | 状态 |
|----|------|
| CLI（WL-C） | **环境待补** |
| PC（WL-P） | **环境待补** |
| Android（WL-A） | **环境待补** |
| iOS（WL-I） | **环境待补** |

有账号后按短表补脱敏截图/日志即可；**不得**为补样本改 CI 打真网。

---

## mock ↔ 真机切换（一句）

| 方向 | 做法 |
|------|------|
| → 真机黄标 | 真 `AUTHORIZE_BASE` + 真 `API_BASE` + 真 corp/agent；secret **仅 Hub**；登记 **8765** loopback + `atlasbot://auth/callback`；`static`+JWT；CLI 勿 `I2_NO_BROWSER` |
| → mock（日常/CI） | `AUTHORIZE_BASE`/`API_BASE` 回 mock 打印 base；测试 corp/agent/secret；可 `ATLAS_I2_NO_BROWSER=1`；**CI 永不 qyapi** |
| 验证 | mock：`wecom_smoke` / WM-P1..P4；真机：hello `wecom:…` + 证据短表脱敏三勾 |

可选辅助（不替真人授权）：`scripts/probe-wecom-live.env.example` + `scripts/probe-wecom-live.sh`。

---

## 链回

- Runbook 真机节：[`i2-login-runbook.md`](./i2-login-runbook.md) § 真机企微联调（黄标）
- Mock WeCom：`tools/mock-wecom/README.md`
- W1 / WM1：同 runbook § WeCom / § Mobile
- L1 手测包形：[`live-agent-handtest-checklist.md`](./live-agent-handtest-checklist.md)
