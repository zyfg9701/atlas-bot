# P1 桌面真用（D1）runbook

> 基线：`zyfg9701/atlas-bot` `main`@`f5ed461` + 本刀  
> 切片：**D1** — 把 P5r `ATLAS_VNC_MODE=proxy` 从 mock HTML 接到 **同机真 Xvfb+x11vnc**  
> 硬门槛：零新 `bot.command`；仍只用 `bot.vncDescriptor` → `{ vncUrl, expiresHint }`；无显示栈诚实 stub，**禁止谎报已连接真桌面**

对照：[`P5-runbook.md`](./P5-runbook.md) · [`P5-real-runbook.md`](./P5-real-runbook.md) · [`runtime-deepen-runbook.md`](./runtime-deepen-runbook.md)（box-vnc） · PC `doVnc`（🖥 + 「更多/调试」Open desktop）

---

## 1. 这把做什么 / 不做什么

| 做 | 不做 |
|----|------|
| 同机 Xvfb + x11vnc 起停脚本 + 探活后再 mint token URL | D2 noVNC/websockify **集群**、多区域桌面池 |
| 探活失败 / 无 upstream / 显示未起 → **degrade-to-stub** | 录屏 / 会话回放 / 键鼠审计产品化 |
| Evidence：RFB 握手（主）+ 可选截帧 | 改 `{ vncUrl, expiresHint }`；新 `bot.command` |
| 键鼠走 **VNC 客户端通道**（产品不注入） | 强制 vendor 整棵 noVNC 进仓 |
| box 仍由 Box 签发；workspace 对齐 | GA2 / YOLO；强制 CI 必有显示；强制移动端 VNC |

一句话：wire 与 PC 入口不动；缺口是把 upstream 从「纸面 host:port / mock HTML」变成可探活的真 RFB 进程。

---

## 2. 显示栈（主路径 = Xvfb + x11vnc）

脚本：[`scripts/atlas-desktop-stack.sh`](../scripts/atlas-desktop-stack.sh)

```bash
# 起（loopback-only；写出 rundir/env）
./scripts/atlas-desktop-stack.sh start
# 典型输出：
#   started Xvfb+x11vnc  DISPLAY=:97  ATLAS_VNC_UPSTREAM=127.0.0.1:NNNN
#   export ATLAS_VNC_MODE=proxy ATLAS_VNC_UPSTREAM=127.0.0.1:NNNN

source .atlas-desktop/env   # 或 ATLAS_DESKTOP_RUNDIR=…
# 然后起 Hub / gateway（见下）

./scripts/atlas-desktop-stack.sh probe       # RFB banner
./scripts/atlas-desktop-stack.sh screenshot  # 有 xwd/import 才写文件；否则 skip
./scripts/atlas-desktop-stack.sh status
./scripts/atlas-desktop-stack.sh stop        # 或 cleanup（stop + 删 rundir）
```

| 项 | 值 |
|----|-----|
| X 显示 | `ATLAS_DESKTOP_DISPLAY_NUM` 默认 `:97` |
| RFB | x11vnc `-localhost -nopw -forever -shared`（回环） |
| 失败码 | 2 缺二进制 · 3 已占用 · 4 Xvfb 未起来 · 5 x11vnc 失败 · 6 起后探活失败 |

**等价路径（不推荐作主路径）：** 已有 Wayland/其他 RFB 出口时，只要 `ATLAS_VNC_UPSTREAM=127.0.0.1:<rfb-port>` 能完成 `RFB xxx.yyy\n` 握手即可；本仓脚本不启 Wayland 合成器。

**Box 工作目录：** 设 `ATLAS_BOX_WORKSPACE` + `ATLAS_DESKTOP_AGENT_ID` 时，脚本会 `mkdir` `<workspace>/<agentId>/`；若本机有 `xterm`，会在该目录起终端。显示内默认目录 = `ATLAS_BOX_WORKSPACE/<agentId>/`（uploads 仍在 `…/uploads/`）。

---

## 3. 探活 → mint → 降级

Gateway / Box 共用 `mint_vnc_descriptor_probed`：

```
ATLAS_VNC_MODE=proxy
  + ATLAS_VNC_UPSTREAM=host:port
  + RFB 探活成功（读 12 字节 banner 以 "RFB " 开头）
      → mint http://127.0.0.1:8787/vnc/{tok_…}/   expiresHint ≈ now+5min
  否则
      → http://127.0.0.1:8787/vnc-stub?agent=…   文案明确「未连接真桌面 / not connected to a real desktop」
```

| 条件 | 行为 |
|------|------|
| `ATLAS_VNC_MODE=stub`（默认） | stub 页 |
| proxy 但无 `ATLAS_VNC_UPSTREAM` | stub（P5r degrade） |
| proxy + upstream 但 TCP 失败 / 非 RFB / 超时 | stub（**D1 新增：探活失败不 mint token**） |
| 未知 token | HTTP **403** |
| 过期 token | HTTP **410** |
| 探活成功 | token 页 `data-atlas-vnc="proxy-rfb"`（token 门，不是 framebuffer） |

探活超时：`ATLAS_VNC_PROBE_TIMEOUT_MS`（默认 800）。**禁止**把 mock HTML 单独当「真桌面」Evidence。

冷路径 `bot.status` / `bot.roster` / `bot.transcript.offbox` **不**因开桌面误醒。

---

## 4. 起服务（可用版本一行）

```bash
./scripts/atlas-desktop-stack.sh start
source .atlas-desktop/env
export ATLAS_VNC_MODE=proxy
# Hub 内嵌 stub HTTP（:8787）即可手点 PC：
RUST_LOG=info cargo run -p atlas-bot-hub

# 或 backend=box（签发在 Box；workspace 对齐）：
ATLAS_GATEWAY_BACKEND=box ATLAS_BOX_WORKSPACE=./data/box-workspace \
  ATLAS_VNC_MODE=proxy ATLAS_VNC_UPSTREAM="$ATLAS_VNC_UPSTREAM" \
  cargo run -p atlas-bot-gateway
ATLAS_GATEWAY_URL=http://127.0.0.1:8787 cargo run -p atlas-bot-hub
```

PC：Connect → 选 agent → Chat 🖥 **或** 「更多/调试」**Open desktop** → `bot.vncDescriptor` → `window.open(vncUrl)`。  
`expiresHint` 仍可见；过期后再点一次 Open desktop 刷新 descriptor（不改 IA）。

**移动（MD1）：** 同构外开 — Android Custom Tabs / iOS `UIApplication.open`；见 `docs/mobile-user-guide.md` §5。 模拟器注意：Android 常把 `127.0.0.1` 理解成模拟器自身，对照时用 `10.0.2.2` 或真机局域网；**不**改 `{ vncUrl, expiresHint }` 形状。

无显示 / 探活失败：打开的是 stub 页，**不会**写「已连接真桌面」。

---

## 5. Evidence（声明环境）

**声明支持的环境：** Linux + `Xvfb` + `x11vnc`（本仓脚本）。无显示 CI **不**跑真显示 Evidence。

| 类型 | 如何复现 |
|------|----------|
| **RFB 握手（主 Evidence）** | `./scripts/atlas-desktop-stack.sh probe` 或冒烟 `rfb_handshake_none`；日志含 `RFB 003.xxx` + ServerInit 尺寸 |
| **截帧（加分）** | `./scripts/atlas-desktop-stack.sh screenshot`（需 `xwd` 或 ImageMagick `import`）；无工具则 skip，握手仍算数 |

mock HTML / proxy-mock **不算**真桌面 Evidence（可作无显示旁路）。

冒烟：

```bash
cargo test -p atlas-bot-hub --test p1_desktop_smoke -- --nocapture --test-threads=1
# 期望：
#   SMOKE_OK p1-desktop degrade-to-stub …
#   SMOKE_OK p1-desktop cold-still-cold
#   SMOKE_OK p1-desktop proxy-rfb …          # mock RFB，无显示也可绿
#   SMOKE_OK p1-desktop box-vnc-align …
#   SMOKE_OK p1-desktop real-xvfb-x11vnc …   # 有 Xvfb+x11vnc
#   或 SMOKE_OK p1-desktop real-path SKIP no Xvfb/x11vnc (CI degrade-green; …)
```

回归（不得无故红）：

```bash
cargo test -p atlas-bot-hub --test p5_smoke -- --nocapture --test-threads=1
cargo test -p atlas-bot-hub --test p5r_smoke -- --nocapture --test-threads=1
cargo test -p atlas-bot-hub --test runtime_deepen_smoke -- --nocapture --test-threads=1
```

---

## 6. 键鼠边界

- **产品路径：** 用 VNC 客户端（TigerVNC / RealVNC / 可选 Docker noVNC）连 `ATLAS_VNC_UPSTREAM`（回环）。经 `vncUrl` 打开的 token 页只是门，不是注入器。
- **atlas-bot 不**提供输入注入 API，**不**新增 `bot.command` 传键鼠。
- **最小可见反馈：** 客户端移指针 / 敲一键；自动化等价：`DISPLAY=:N xdotool mousemove …`（打在 X 上，不是 bot 方法）或 RFB PointerEvent。
- **会话生命周期：** token TTL ≈ **5 分钟**；过期 410，未知 403；过期后不得假装仍连着 — 再点 Open desktop 重 mint。显示进程由脚本起停，不跟 token 一一撕毁（token 只挡 URL）；泄漏巡检：`./scripts/atlas-desktop-stack.sh cleanup`。

可选外置 noVNC（**不进仓**）：

```bash
# 示例，非 DoD；钉 tag；见 NOTICE
docker run --rm --network host -e VNC_SERVER=127.0.0.1:NNNN theasp/novnc:latest
```

---

## 7. 安全（回环 / 短 TTL）

- x11vnc **`-localhost`**；gateway HTTP 默认 `127.0.0.1:8787`（`ATLAS_GATEWAY_HTTP_BIND`）。
- **不写**把 RFB/桌面绑到 `0.0.0.0` 或公网暴露的步骤。
- Token 短 TTL；未知/过期 403/410。
- `-nopw` 可接受是因为绑定回环 + 产品门是 token，不是把桌面挂到网上。

---

## 8. 手测对照（D-P1…P9）

| 编号 | 期望 | 本刀落点 |
|------|------|----------|
| **D-P1** | proxy + 真 upstream 健康 → mint token URL；非 stub 叙事 | `SMOKE_OK p1-desktop proxy-rfb` / `real-xvfb-x11vnc` |
| **D-P2** | Evidence：截帧或 RFB 握手 | 握手主路径；screenshot 可选 |
| **D-P3** | 键鼠最小 | VNC 客户端；冒烟 xdotool 自动化等价 |
| **D-P4** | 无 upstream / 探活失败 → stub；不谎报 | `SMOKE_OK p1-desktop degrade-to-stub` |
| **D-P5** | token 过期/未知 → 403/410 | 同冒烟 + P5r |
| **D-P6** | PC 双入口 | 🖥 + 调试 Open desktop；文案提示 TTL 刷新（不改 IA） |
| **D-P7** | box 同源 | `SMOKE_OK p1-desktop box-vnc-align`；uploads 在 workspace |
| **D-P8** | p5 / p5r / deepen box-vnc 不回退 | CI 继续跑这三条 |
| **D-P9** | 无显示 CI degrade 绿 | `p1_desktop_smoke` 无 Xvfb 时 SKIP 真显示 |

---

## 9. 已知限制（填实）

1. **显示栈选型：** 主路径 **Xvfb + x11vnc**（loopback）。Wayland 等价仅当已有 RFB 出口时文档兼容，脚本不启。  
2. **Evidence 选型：** **RFB 握手为主**（声明 Linux+Xvfb+x11vnc，或 mock RFB 用于 CI 协议路径）。截帧为加分（`xwd`/`import`）。  
3. **外置 noVNC：** 仅文档 / Docker 可选；**不 vendor**。  
4. **真显示测：** 本机脚本 + 冒烟在有二进制时跑；CI 默认 **不** apt 装显示栈（degrade/mock 保绿）。  
5. **token TTL：** 保持 ≈ **5 min**（`DEFAULT_VNC_TOKEN_TTL_MS`）。

---

## 10. vs mock / P5 / P5r / R2.3

| 层 | P5 stub | P5r proxy | R2.3 box | **D1** |
|----|---------|-----------|----------|--------|
| wire | `bot.vncDescriptor` | 同 | 同 | **同（零新 command）** |
| URL | `/vnc-stub` | token **若**写了 upstream（旧：不探活） | Box 签发 | token **仅探活过**；否则 stub |
| 页 | 占位 | 曾用 proxy-mock 当 Evidence | 同左 | `proxy-rfb` token 门；mock ≠ 真桌面 Evidence |
| 真干活 | 否 | 否 | 否 | **同机 Xvfb+x11vnc 边界内是** |

---

## 11. Out of scope（D1）

D2 集群、录屏产品化、D3 仅文档、新 `bot.command`、改 descriptor 形状、强制 vendor noVNC、GA2、YOLO、强制移动端 VNC、CI 默认必须有显示。
