# I2 / W1 Login runbook -- PC + CLI + Mobile (OIDC + WeCom)

Baseline: I1 Hub Auth Gate already validates Bearer. I2/W1 only obtain the ticket.
No new bot.* methods. No Hub login website. No query access_token=.
Gateway / Box unchanged for user auth.

## Ticket handed to Hub (nailed)

**OIDC:** Prefer id_token when present; else access_token if JWT.
Hub `ATLAS_AUTH_MODE=oidc` verifies via JWKS (`ATLAS_OIDC_JWKS_JSON`).
`hello_ack.user_id = sub`.

**WeCom (W1):** Hub thin exchange mints short HS256 JWT (`ATLAS_AUTH_MODE=static` +
`ATLAS_AUTH_JWT_SECRET`). `sub = wecom:<corpId>:<userid>`. Never hand opaque
WeCom access_token as Hub Bearer.

## Flow (all surfaces)

### OIDC (default)

authorize (PKCE S256) -> redirect + code -> token endpoint
  -> local store -> WS Authorization: Bearer <id_token|jwt-access>
  -> Hub I1 -> hello_ack.user_id = sub

### WeCom (W1)

authorize/扫码 (state, **no PKCE**) -> redirect + code
  -> Hub `POST /auth/wecom/exchange` (secret Hub-only)
  -> local store (provider=wecom) -> WS Bearer <hub-jwt>
  -> Hub I1 (static/oidc) -> hello_ack.user_id = wecom:<corpId>:<userid>

## Env

### Shared / OIDC

- `ATLAS_TICKET_PROVIDER=oidc|wecom` (default **oidc**)
- `ATLAS_OIDC_ISSUER`, `ATLAS_OIDC_CLIENT_ID`, `ATLAS_OIDC_AUDIENCE`
- `ATLAS_OIDC_REDIRECT_PORT` (default 0 ephemeral) — PC/CLI loopback
- `ATLAS_I2_NO_BROWSER=1` for CI mock drive (OIDC **and** WeCom)
- `ATLAS_BOT_CONFIG_DIR` override credentials dir
- `ATLAS_HUB_BEARER` optional CLI override
- Hub: `ATLAS_AUTH_MODE=oidc` + `ATLAS_OIDC_JWKS_JSON` (OIDC tickets)
- `dev` (default): skip login; existing smokes stay green

### WeCom (W1)

| Variable | Side | Notes |
|----------|------|-------|
| `ATLAS_TICKET_PROVIDER` | client | `oidc` (default) \| `wecom` |
| `ATLAS_WECOM_CORP_ID` | Hub + client authorize URL | required on wecom path |
| `ATLAS_WECOM_AGENT_ID` | same | required |
| `ATLAS_WECOM_SECRET` | **Hub only** | required; **never** in mobile/PC release packages |
| `ATLAS_WECOM_REDIRECT_URI` | client | PC loopback or `atlasbot://auth/callback` |
| `ATLAS_WECOM_API_BASE` | Hub | default `https://qyapi.weixin.qq.com`; CI → mock-wecom base |
| `ATLAS_WECOM_AUTHORIZE_BASE` | client | mock base or live open.weixin authorize host |
| `ATLAS_WECOM_JWT_TTL` | Hub | optional; default 3600s |
| `ATLAS_AUTH_JWT_SECRET` | Hub | mints/verifies exchanged JWT (`static` mode) |
| `ATLAS_HUB_HTTP` | client | Hub HTTP for exchange (else derive from `ATLAS_HUB_WS`) |

Same deploy may configure OIDC + WeCom; one login picks one provider.
Hub does **not** need `ATLAS_AUTH_MODE=wecom`.

## Mock OIDC

```bash
cargo run -q -p atlas-bot-auth-client --bin mock-oidc
```

See tools/mock-oidc/README.md

Register **both** PC loopback and mobile custom-scheme redirects on the IdP:

- PC/CLI: `http://127.0.0.1:<port>/callback`
- Mobile: `atlasbot://auth/callback`

## Mock WeCom (W1)

```bash
cargo run -q -p atlas-bot-auth-client --bin mock-wecom
```

See tools/mock-wecom/README.md

Endpoints: `/authorize`, `/cgi-bin/gettoken`, `/cgi-bin/user/getuserinfo`.
Point Hub `ATLAS_WECOM_API_BASE` + client `ATLAS_WECOM_AUTHORIZE_BASE` at the printed base.
**CI never** calls `qyapi.weixin.qq.com`.

Headless:

```bash
export ATLAS_I2_NO_BROWSER=1
export ATLAS_TICKET_PROVIDER=wecom
# … corp/agent/secret/api_base/jwt_secret/hub …
atlas-bot-cli login --provider wecom
```

## CLI (I2.1 + W1)

```bash
atlas-bot-cli login --provider oidc --issuer …
atlas-bot-cli login --provider wecom
atlas-bot-cli logout
atlas-bot-cli status
```

Credentials path: `~/.config/atlas-bot/credentials.json` mode 0600 (includes `provider`).
Loopback: `http://127.0.0.1:<port>/callback`

## PC (I2.1 + W1)

UI: Login prompts for provider (`oidc`\|`wecom`) / Logout + optional Bearer paste.
Browser WebSocket cannot set Authorization.
Production path: Tauri `connect_ws` (clients/pc/src-tauri) sends Bearer on upgrade.
`WeComProvider` helpers: `clients/pc/src/wecomClient.ts`.
Under Hub `dev`, plain browser WS remains fine.

## Mobile (I2.2 / M1 + WM1 WeCom real Login)

**Scope:** system-browser ticket pickup for Android + iOS. **Not** mobile UI
convergence, **not** I2.3 / public IdP productization, **not** auto-login desktop /
cross-device SSO, **not** Hub-hosted login site. Gateway/Box do not validate user tickets.

**Reject WebView as the primary login path** (M2 is documented as anti-pattern only).

### Redirect (exact)

| Item | Value |
|------|-------|
| Custom scheme URI | `atlasbot://auth/callback` |
| Scheme | `atlasbot` |
| Host / path | `auth` / `/callback` |
| App / Universal Link | **Not done** (optional; does not block DoD) |

### client_id (OIDC; recommended separate public clients, same issuer)

| Surface | client_id |
|---------|-----------|
| Android | `atlas-bot-android` |
| iOS | `atlas-bot-ios` |
| PC (I2.1) | `atlas-bot-pc` (or ATLAS_OIDC_CLIENT_ID) |

Scopes default: `openid profile` (+ optional audience), same narrative as I2.1.

### WeCom mobile (WM1 — real Login)

Android **and** iOS wire W1 stubs into the same I2.2 session skeleton:

1. Set **ATLAS_TICKET_PROVIDER=wecom** (advanced UI field; default remains **oidc**).
2. Fill corpId / agentId / authorize base (mock-wecom URL for CI/local).
3. Tap **Login** → system browser (**Custom Tabs** / **ASWebAuthenticationSession**), **no WebView**,
   authorize URL from `WeComAuth.buildAuthorizeUrl` (corpId/agentId/state; **no PKCE**).
4. Callback `atlasbot://auth/callback?code=…&state=…` — state mismatch fails observably.
5. App `POST {hubHttp}/auth/wecom/exchange` JSON `{code,state?}` (same shape as PC
   `exchangeWeComCode`) → Hub JWT `access_token` → TokenStore (optional `provider=wecom`).
6. Tap **Connect** → existing HubClient Bearer → `hello_ack.user_id=wecom:<corpId>:<userid>`.
7. **Logout** clears TokenStore (and provider).

**Config (mobile):**

| Item | Where |
|------|--------|
| `ATLAS_TICKET_PROVIDER` | advanced field / env (default `oidc`) |
| `ATLAS_WECOM_CORP_ID` / `AGENT_ID` | advanced fields |
| `ATLAS_WECOM_AUTHORIZE_BASE` | mock-wecom base (e.g. emulator `http://10.0.2.2:8091`) |
| Hub HTTP for exchange | derived from Hub WS URL (`ws→http`, strip `/ws`) |
| Secret | **Hub only** — never in APK/IPA |

**Mock:** `cargo run -q -p atlas-bot-auth-client --bin mock-wecom` + Hub static JWT +
`ATLAS_WECOM_API_BASE=<mock>` (see § Mock WeCom). CI must not hit `qyapi.weixin.qq.com`.

**OIDC switch:** leave provider=`oidc` — I2.2 PKCE path unchanged. Same install may hold both
configs; one Login uses the current provider only.

**Not this slice:** T2 MSI; mobile UI re-convergence; I2.3; W2 introspection; WebView primary;
new `bot.*`; Gateway/Box user-ticket validation.

Register `atlasbot://auth/callback` on the WeCom app console for live yellow-tag — see **§ 真机企微联调（黄标）** (WL1).

### Android (OIDC full)

1. Set **OIDC issuer** (e.g. emulator → host mock: `http://10.0.2.2:8090`).
2. Tap **Login** → Chrome **Custom Tabs** opens authorize (PKCE S256).
3. IdP redirects to `atlasbot://auth/callback?code=…&state=…` (Manifest intent-filter).
4. App exchanges code, stores ticket in **EncryptedSharedPreferences**, prefers **id_token**.
5. Tap **Connect** → OkHttp WS upgrade sends `Authorization: Bearer <ticket>` when present.
6. **Logout** clears TokenStore and disconnects.

Implementation: `clients/android` — `auth/OidcAuth.kt`, `AuthSession.kt`, `TokenStore.kt`;
`HubClient.setAuthorization` / `buildConnectRequest` (null = today's no-token / `dev`).

### iOS (OIDC full)

1. Set **OIDC issuer** (simulator: `http://127.0.0.1:8090`).
2. Tap **Login** → **ASWebAuthenticationSession** (callbackURLScheme `atlasbot`).
3. Callback `atlasbot://auth/callback` (Info.plist URL Types + session completion / `onOpenURL`).
4. Exchange + **Keychain** store (`afterFirstUnlockThisDeviceOnly`); prefer **id_token**.
5. Tap **Connect** → `URLRequest` WebSocket carries `Authorization: Bearer` when set.
6. **Logout** clears Keychain and disconnects.

Implementation: `clients/ios` — `OidcAuth.swift`, `AuthSession.swift`, `TokenStore.swift`.

### Auth modes

| Hub mode | No ticket | With ticket |
|----------|-----------|-------------|
| `dev` (default) | Connect works (P4 smoke unchanged) | Optional Bearer accepted |
| `oidc` / `static` | Fail closed (`unauthorized` / -32002 or connect failure) → Login | `hello_ack.user_id=sub` |

## Smoke (PC/CLI)

See protocol-conformance workflow for `i2_smoke` and `wecom_smoke`.
Expect `SMOKE_OK i2…` and `SMOKE_OK wecom…` lines.
PC unit tests cover PKCE challenge, callback parsing, and WeCom authorize URL.

## Failure closed-set

Missing / bad / expired -> Hub unauthorized (-32002).

## § 真机企微联调（黄标 · WL1）

> **证据债文档包**（相对 mock-wecom）。W1/WM1 代码已支持 env 指真 API；本节只钉配置差、步骤、redirect、证据规范与失败归因。  
> **不改** `bot.*` / Hub exchange / 四端 WeCom 业务代码。**CI 仍零外网企微**（永不打 `qyapi.weixin.qq.com`）。  
> 手测勾选与失败矩阵：[`wecom-ticket-checklist.md`](./wecom-ticket-checklist.md)（WM-P5 + **WL-C / WL-P / WL-A / WL-I**）。  
> 叙事对齐：[`live-agent-handtest-checklist.md`](./live-agent-handtest-checklist.md)（L1 mock↔真机切换形）。  
> 可选探针：`scripts/probe-wecom-live.env.example` + `scripts/probe-wecom-live.sh`（**不**替真人授权；**不**挂 CI）。

### mock ↔ 真机配置差表

| 变量 | 侧 | mock（日常 / CI） | 真机黄标 |
|------|----|-------------------|----------|
| `ATLAS_TICKET_PROVIDER` | 客户端 | `wecom` | **同** `wecom` |
| `ATLAS_WECOM_CORP_ID` | Hub + 客户端拼 URL | 测试 corp | **真**企业 ID |
| `ATLAS_WECOM_AGENT_ID` | 同 | 测试 agent | **真**应用 AgentId |
| `ATLAS_WECOM_SECRET` | **仅 Hub** | 测试 secret | **真**应用 secret；**禁止**进客户端包 / PR / 证据附件 |
| `ATLAS_WECOM_AUTHORIZE_BASE` | 客户端 | mock 打印 base | **真** authorize 主机（企微网页授权文档当前主机） |
| `ATLAS_WECOM_API_BASE` | Hub | mock base | `https://qyapi.weixin.qq.com`（或企业指定；**勿**写进 CI） |
| `ATLAS_WECOM_REDIRECT_URI` | 客户端 | 任意（mock 不验后台） | **须与企微后台登记一致** |
| `ATLAS_OIDC_REDIRECT_PORT` | CLI loopback | `0` ephemeral OK | **建议钉 `8765`**（与 PC UI 默认一致） |
| `ATLAS_AUTH_MODE` | Hub | `static`（验换发票） | **同** `static`（手测勿停在仅 `dev` 若要证 hello sub） |
| `ATLAS_AUTH_JWT_SECRET` | Hub | 测试 HMAC | 部署密钥；**不**进证据 |
| `ATLAS_HUB_HTTP` / WS | 客户端 | 本机 Hub | 指向跑 exchange 的同一 Hub |
| `ATLAS_I2_NO_BROWSER` | CLI | CI=1 驱 mock | **真机手测勿设**（需真人扫码/授权） |

**一句切换：** 真机 = 真 AUTHORIZE_BASE + 真 API_BASE + 真 corp/agent + secret 仅 Hub + 登记 redirect；做完 → **切回 mock**（AUTHORIZE_BASE/API_BASE 回 mock；CI 永不真网）。

### 步骤顺序（建议）

1. **企微后台**：建/选自建应用；抄 corpId / agentId；secret **只**写入 Hub 私密 env（密码管理器），**不进 git**。
2. **登记 redirect**（两端都要，见下表）。
3. **起 Hub**：`ATLAS_AUTH_MODE=static` + `ATLAS_AUTH_JWT_SECRET` + 真 `CORP_ID`/`AGENT_ID`/`SECRET` + 真 `API_BASE`（或默认 qyapi）。
4. **CLI**：钉 `ATLAS_OIDC_REDIRECT_PORT=8765`；`login --provider wecom`（真 AUTHORIZE_BASE；**勿**设 `ATLAS_I2_NO_BROWSER`）→ 浏览器授权 → exchange → `status` / Connect → 记 `hello_ack.user_id`。
5. **PC**：Login UI provider=`wecom` → 系统浏览器 → callback（默认 `http://127.0.0.1:8765/callback`）→ exchange → Tauri Connect → 同 hello。
6. **Android / iOS**：provider=`wecom`；系统浏览器（Custom Tabs / ASWebAuthenticationSession）；深链 `atlasbot://auth/callback`；Connect → hello。**无** WebView 主路径。
7. **留证**：按下方证据规范 + checklist 证据短表；失败对照 **R1–R9**。
8. **切回 mock**：恢复 `AUTHORIZE_BASE`/`API_BASE` 为 mock；日常与 CI 仍 mock（本刀不改 CI）。

链回：[`wecom-ticket-checklist.md`](./wecom-ticket-checklist.md) · § Mock WeCom（上）· W1 Hub exchange / WM1 移动真 Login（本页 § WeCom / § Mobile）。

### redirect 登记（钉死）

| 表面 | 登记值 | 说明 |
|------|--------|------|
| **PC / CLI loopback** | `http://127.0.0.1:8765/callback` | PC UI 默认 prompt 即 **8765**（`clients/pc/src/main.ts`）；CLI 设 `ATLAS_OIDC_REDIRECT_PORT=8765` 与企微后台一致。默认 `0` ephemeral **不可**登企微后台，真机必须钉端口。 |
| **移动深链** | `atlasbot://auth/callback` | Android Manifest + iOS URL Types；scheme `atlasbot`，host/path `auth`/`/callback` |

只登记一端 → 单端通（失败矩阵 **R8**）。

### 证据规范（允许 / 禁止）

| 允许 | 禁止 |
|------|------|
| `hello_ack.user_id` / `sub` 形 `wecom:<corpId>:<userid>`（corpId 可部分打码） | 完整 `ATLAS_WECOM_SECRET`、`ATLAS_AUTH_JWT_SECRET` |
| Hub / 客户端日志 **时间戳** + 阶段（authorize 打开 / callback 收到 / exchange 200 / WS upgrade） | 原始 `code`、完整 Bearer JWT、企微 access_token |
| 授权页 / 成功回调 **截图**（无地址栏 secret、无 token query） | 整份 `credentials.json` / `.env` 贴进 PR 或公开附件 |
| 脱敏 exchange 元数据（HTTP 状态、`expires_in`、subject 前缀） | CI 配置改成打真 `qyapi` |
| 失败闭集错误码（如 `-32002 unauthorized`）与对照表行号 **R1–R9** | 公开 issue 贴未打码 userid（若公司政策禁止） |

证据存放：私密附件 / Notion / 本机加密目录 — **勿**把 secret 进 git。短表字段见 checklist「证据模板」。

### 失败对照（摘要 · 全文在 checklist）

| 行 | 现象族 |
|----|--------|
| R1 | redirect 未登记 / 不一致 → 无法回 App |
| R2 | corp/agent/secret 不匹配或 code 过期 → exchange 4xx |
| R3 | JWT/AUTH_MODE 错 → Connect `unauthorized` |
| R4 | state mismatch |
| R5 | qyapi 网络/代理超时 |
| R6 | AUTHORIZE_BASE 仍 mock / 拼错 |
| R7 | provider 未切 wecom / 粘贴别的 Bearer → sub 形不符 |
| R8 | 只登记一端 redirect → 单端通 |
| R9 | CI 误打真企微 → **立即改回 mock** |

处置细节与可勾选位：[`wecom-ticket-checklist.md`](./wecom-ticket-checklist.md) § 失败对照表 R1–R9。

### 真机证据样本状态

本环境 **无**可用真企微账号 → 样本位标 **「环境待补」**。文档包（本 runbook 节 + checklist WL-* + 失败矩阵 + 证据模板 + 可选探针）齐即可合；端侧脱敏截图/日志在有账号后按模板补填，**不得**因此改 CI 打真网。

### 明确非本刀

T2 MSI · 改 `bot.*` / WebView / UI 再收敛 · CI 打真企微（WL3）· W2 内省 · I2.3 · 账号中台 · 删 mock · 产品「证据导出」按钮（WL2 后置）。

## § WeCom known limitations (W1 §10 filled)

- **`sub` mapping:** `wecom:<corpId>:<userid>` (unit-tested)
- **Exchange:** `POST /auth/wecom/exchange`; JWT **HS256** via Hub `ATLAS_AUTH_JWT_SECRET` (static)
- **Clients:** PC **full**; CLI **full** (mock preferred); Android + iOS **WM1 real Login** (system browser → Hub exchange)
- **PKCE:** WeCom web OAuth — **not supported**; use **state + one-time code** + Hub secret
- **扫码页:** minimal authorize redirect (mock); pretty QR page = optional / not required
- **Live WeCom yellow-tag:** see **§ 真机企微联调（黄标）** above (WL1 docs pack); CI still uses mock-wecom only
- **Not done / out of scope:** T2 MSI; I2.3 Hub login site; W2 hot-path WeCom introspection;
  WebView primary; putting `ATLAS_WECOM_SECRET` in client packages; new `bot.*`; Gateway/Box user auth

## Sec10 / §8 Known limitations (I2 filled)

- Ticket to Hub: id_token preferred; else JWT access_token
- PC callback: loopback 127.0.0.1; Tauri rust WS for Bearer
- CLI: PKCE loopback (not device-code) for OIDC; WeCom via Hub exchange
- **Android:** Custom Tabs + PKCE (OIDC) / WeCom authorize (no PKCE) → Hub exchange; scheme `atlasbot://auth/callback`
- **iOS:** ASWebAuthenticationSession; OIDC PKCE or WeCom → Hub exchange; URL Types scheme `atlasbot`
- **mobile client_id:** separate `atlas-bot-android` / `atlas-bot-ios` (same issuer as PC)
- **refresh:** Not done — expire then re-login
- **App / Universal Link:** Not done
- **Paste Bearer advanced entry:** Not done on mobile (PC still has paste)
- **iOS CI:** macOS / xcodebuild for unit tests; Linux = logic documented, handtest yellow
- ~~WeCom: Deferred~~ → **W1 landed** (see § WeCom)
- Hub login site: Not built (I2.3 rejected)
- query token: Forbidden

## Boundaries

- Zero new bot.* methods
- Bearer-only inbound credential channel (I1)
- Gateway / Box do not validate user tickets
- Not an account/org/billing product
- Not mobile UI convergence
- Not WebView primary login path
- Not T2 MSI / not W2 introspection / not public IdP productization
