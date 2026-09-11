# CLI primary path runbook（推荐主路径）

> **唯一推荐起法**：PC → Hub → `atlas-bot-gateway` (`backend=cli`) → `ATLAS_AGENT_CLI`  
> 基线：P3.5 `CliAgentGateway` · 打磨 C1/C1b  
> 旁路（stub / box / openai）见下文对照表；协议细节仍见 [`P3.5-runbook.md`](./P3.5-runbook.md)。

> **默认仍 cli；box 旁路（可选 LLM）见 [`private-resident-runtime-runbook.md`](./private-resident-runtime-runbook.md)。**


> **Dev vs dist:** 开发用仓库内 `cargo run` / `scripts/dev-cli-stack.*`；分发用 `dist/` 二进制 + `dist/scripts/start-cli-stack.*`（见 [`packaging-skeleton.md`](./packaging-skeleton.md)）。本页默认描述 **开发主路径**。

---

## 1. 拓扑

```text
PC (bot_client) ──WS──► Hub (:7700) ──HTTP──► atlas-bot-gateway (:8787, backend=cli)
                                                    │
                                                    └─ spawn ATLAS_AGENT_CLI (-p …)
```

产品叙事：**对话 = 本机 Atlas/Cursor agent**。PC **只连 Hub**；不在 PC 内起 gateway。

---


## 2. 一键脚本

```bash
# 真机（PATH 上已有已登录的 agent）
./scripts/dev-cli-stack.sh

# CI / 无真 agent：用 mock-cli 覆盖探测（同一脚本，不静默改 stub）
ATLAS_AGENT_CLI=tools/mock-cli/mock-atlas-agent-cli.sh ./scripts/dev-cli-stack.sh
```

脚本会：

1. 探测 `ATLAS_AGENT_CLI`（默认 `agent`，`command -v` / 可执行路径）  
2. **找不到则非零退出**并提示安装/登录；**不会**静默起 Hub InMemory stub  
3. 后台起 gateway `BACKEND=cli`（`:8787`）+ Hub `ATLAS_GATEWAY_URL=http://127.0.0.1:8787`（WS `:7700`）  
4. Ctrl-C 清理两个子进程；打印 `/healthz` 与 PC Connect 提示  

日志目录默认 `.cli-stack-logs/`（gitignore 外可本地删）。

### Windows（一等主路径）

```powershell
# 真机（PATH 上已有已登录的 agent）
.\scripts\dev-cli-stack.ps1

# 若 ExecutionPolicy 拦截：
powershell -ExecutionPolicy Bypass -File .\scripts\dev-cli-stack.ps1

# 无真 agent：用 mock .cmd（纯 Win，无需 Git Bash）— text 起栈（W1）
$env:ATLAS_AGENT_CLI="$PWD\tools\mock-cli\mock-atlas-agent-cli.cmd"; .\scripts\dev-cli-stack.ps1

# WS1·B 流式一键（-Stream）：默认 STREAM=1 + 分进程 EVENT_URL + mock 时 MOCK_CLI_STREAM=1
# Hub 侧默认 insecure loopback（或你已设的共享 token）；显式 env 不被覆盖
$env:ATLAS_AGENT_CLI="$PWD\tools\mock-cli\mock-atlas-agent-cli.cmd"; .\scripts\dev-cli-stack.ps1 -Stream

# 等价：环境开关（无 -Stream 参数时）
$env:ATLAS_CLI_STACK_STREAM='1'
$env:ATLAS_AGENT_CLI="$PWD\tools\mock-cli\mock-atlas-agent-cli.cmd"
.\scripts\dev-cli-stack.ps1
```

`dev-cli-stack.ps1` 与 Unix `.sh` **语义对等**：探测 `ATLAS_AGENT_CLI`（`Get-Command` / 路径，含 `.exe`/`.cmd`）→ 找不到 **exit 1** 且不静默 stub → 起 gateway `BACKEND=cli` + Hub `ATLAS_GATEWAY_URL`（Hub 侧 `ATLAS_GATEWAY_HTTP_BIND=off`）→ 打印 `ws://127.0.0.1:7700/ws` 与 healthz → Ctrl-C 清 PID（`.cli-stack-pids/`）与日志（`.cli-stack-logs/`）。

**`-Stream` / `ATLAS_CLI_STACK_STREAM=1`（WS1·B）：** 仅填 **未设** 的默认——`ATLAS_AGENT_CLI_STREAM=1`、分进程 `ATLAS_HUB_EVENT_URL=http://127.0.0.1:7701/internal/runtime-hint`、无 token 时 `ATLAS_HUB_EVENT_ALLOW_INSECURE_LOOPBACK=1`、若 CLI 为 mock `.cmd` 则 `MOCK_CLI_STREAM=1`。已设的 STREAM / EVENT_URL / MOCK / TOKEN / INSECURE **不被无声覆盖**。无 `-Stream`：仍为 text 起栈（与 W1 一致）。分进程 mid-turn **只走 B1**，勿与 B2 双开。横幅会打印 STREAM / EVENT_URL / token|insecure / MOCK_CLI_*。

手测勾选见 [`windows-cli-stack-checklist.md`](./windows-cli-stack-checklist.md)（含 W-S1..W-S3 流式位；真机 W-L* → [`live-agent-handtest-checklist.md`](./live-agent-handtest-checklist.md)）。

#### 附录：手动双终端（bat）

仅在无法跑 ps1 时使用：

```bat
where agent
REM 或 set ATLAS_AGENT_CLI=C:\path\to\agent.exe
set ATLAS_GATEWAY_BACKEND=cli
set ATLAS_GATEWAY_HTTP_BIND=127.0.0.1:8787
cargo run -p atlas-bot-gateway

REM 另一终端
set ATLAS_GATEWAY_URL=http://127.0.0.1:8787
set ATLAS_GATEWAY_HTTP_BIND=off
cargo run -p atlas-bot-hub
```

---

## 3. 三终端手动起法

```bash
# 终端 1 — gateway (cli)
ATLAS_GATEWAY_BACKEND=cli \
ATLAS_AGENT_CLI=agent \
  cargo run -p atlas-bot-gateway
# GET http://127.0.0.1:8787/healthz  → JSON: backend, agent_cli, agent_cli_found

# 终端 2 — Hub 挂远程 gateway
ATLAS_GATEWAY_URL=http://127.0.0.1:8787 \
ATLAS_GATEWAY_HTTP_BIND=off \
  cargo run -p atlas-bot-hub
# ws://127.0.0.1:7700/ws

# 终端 3 — PC
cd clients/pc && npm i && npm run tauri dev
# Connect → 选 agent → Send
```

---

## 4. 环境变量表

| 变量 | 默认 | 说明 |
|------|------|------|
| `ATLAS_GATEWAY_URL` | _(unset)_ | Hub：设为 `http://127.0.0.1:8787` 才走真 gateway；**unset = 嵌入 InMemory stub**（旁路） |
| `ATLAS_GATEWAY_BACKEND` | `cli` | gateway 二进制默认 **cli** |
| `ATLAS_AGENT_CLI` | `agent` | 本机 Cursor/Atlas agent 二进制；CI/Win 可指 `tools/mock-cli/mock-atlas-agent-cli.sh` 或 `.cmd` |
| `ATLAS_AGENT_CLI_EXTRA_ARGS` | text 或 stream-json（见下） | JSON 字符串数组；**未设时**随 `ATLAS_AGENT_CLI_STREAM` 选默认 |
| `ATLAS_AGENT_CLI_STREAM` | _(unset)_ | **CS1 显式 opt-in**：`1`/`true`/`yes` → 默认 EXTRA_ARGS 切 `--output-format stream-json` + `--stream-partial-output`，stdout **按行**解析 mid-turn |
| `ATLAS_AGENT_CLI_TIMEOUT_MS` | _(none)_ | 单 turn 软超时；超时错误可见 |
| `ATLAS_HUB_EVENT_URL` | _(unset)_ | **CS1b**：分进程时 POST RuntimeHint（与 Box B1 同）；失败只 warn。**勿与同进程 B2 bridge 双开** |
| `ATLAS_HUB_EVENT_TOKEN` | _(unset)_ | B1 ingest Bearer / X-Atlas-Event-Token |
| `ATLAS_GATEWAY_HTTP_BIND` | `127.0.0.1:8787` | gateway listen；Hub 挂远程时建议 `off` |
| `ATLAS_HUB_BIND` | `127.0.0.1:7700` | Hub WS |

---

## 5. 真机前提 / mock 路径

- **真机：** 本机已安装 **且已登录** Cursor/Atlas agent（默认命令名 `agent`）。未登录时 spawn/exit 会失败且 **可见**，不会假成功。  
- **mock（CI/无真 agent）：**  
  - Unix：`ATLAS_AGENT_CLI=tools/mock-cli/mock-atlas-agent-cli.sh`  
  - Windows：`$env:ATLAS_AGENT_CLI="$PWD\tools\mock-cli\mock-atlas-agent-cli.cmd"`  
  回复形如 `atlas-mock-reply agent=…`（**非** `echo:` stub）。  
  - 流式：`MOCK_CLI_STREAM=1`（可选 `MOCK_CLI_SLEEP_MS`）打 NDJSON delta + `result`。  
- 证据步骤见 [`cli-primary-checklist.md`](./cli-primary-checklist.md)；流式见 [`cli-streaming-checklist.md`](./cli-streaming-checklist.md)。  
- **真机联调包（L1）：** Win 已登录 agent 手测 + 失败对照 + mock↔真机切换 → [`live-agent-handtest-checklist.md`](./live-agent-handtest-checklist.md)；探针 `scripts/probe-agent-cli.ps1`（L1b）。

---

## 5b. CLI 流式（CS1）

> 默认仍是 **text**（无 mid-turn）。设 `ATLAS_AGENT_CLI_STREAM=1` 才走真 stream-json 行协议。  
> **禁止**假流式（把终稿切成假 delta）。不改 `bot.*`。

### 真机

```bash
ATLAS_GATEWAY_BACKEND=cli \
ATLAS_AGENT_CLI=agent \
ATLAS_AGENT_CLI_STREAM=1 \
  cargo run -p atlas-bot-gateway
```

需已登录 agent。Events 订阅可见 `hub:assistant_delta` / 可选 `hub:tool`，终稿仍走 sync `sendPrompt` result + `hub:turn_finished`。

### mock 流式（CI / 无真 agent）

```bash
# 一条起 mock 流式输出（delta → sleep → result）
MOCK_CLI_STREAM=1 MOCK_CLI_SLEEP_MS=80 \
  ATLAS_AGENT_CLI=tools/mock-cli/mock-atlas-agent-cli.sh \
  tools/mock-cli/mock-atlas-agent-cli.sh -p --output-format stream-json "hi"

# 冒烟（同进程 B2）
MOCK_CLI_STREAM=1 cargo test -p atlas-bot-hub --test cli_stream_smoke -- --nocapture --test-threads=1
```

mock 行格式：**Cursor 形 NDJSON**（`assistant` + `result`）；gateway **兼认**简易 `ATLAS_DELTA\t…` / `ATLAS_TOOL\t…`。

### 分进程 B1（CS1b）

设 `ATLAS_HUB_EVENT_URL=http://127.0.0.1:7701/internal/runtime-hint`（见 [`b1-event-ingest-runbook.md`](./b1-event-ingest-runbook.md)）。  
**规则与 B1 相同：勿同开 B2 `spawn_turn_bridge` + B1 POST**（Hub 对 `turn_finished` 有去重，但 mid-turn 会双发）。  
分进程栈（`dev-cli-stack.*`）**只** B1；**不要** B2+B1 双开。

手测勾选：[`cli-streaming-checklist.md`](./cli-streaming-checklist.md) · Win mock：[`windows-cli-stack-checklist.md`](./windows-cli-stack-checklist.md) W-S* · **真机：** [`live-agent-handtest-checklist.md`](./live-agent-handtest-checklist.md)（W-L*）。

### PowerShell 流式起栈（WS1·B）

```powershell
# 一键 -Stream + mock .cmd（推荐手测）
$env:ATLAS_AGENT_CLI="$PWD\tools\mock-cli\mock-atlas-agent-cli.cmd"
.\scripts\dev-cli-stack.ps1 -Stream
# 横幅应见 STREAM=1、EVENT_URL=…/internal/runtime-hint、MOCK_CLI_STREAM=1、INSECURE=1（或你设的 token）

# 显式 env 备选（不依赖 -Stream；显式值优先，-Stream 不会覆盖）
$env:ATLAS_AGENT_CLI="$PWD\tools\mock-cli\mock-atlas-agent-cli.cmd"
$env:ATLAS_AGENT_CLI_STREAM='1'
$env:ATLAS_HUB_EVENT_URL='http://127.0.0.1:7701/internal/runtime-hint'
$env:ATLAS_HUB_EVENT_ALLOW_INSECURE_LOOPBACK='1'   # 或改用共享 ATLAS_HUB_EVENT_TOKEN
$env:MOCK_CLI_STREAM='1'
.\scripts\dev-cli-stack.ps1

# 真机 agent + -Stream（无 mock → 不自动 MOCK_CLI_STREAM）
.\scripts\dev-cli-stack.ps1 -Stream
```

PC（**mock**）：Connect → `ws://127.0.0.1:7700/ws` → Send → Events ≥1× `hub:assistant_delta` → `hub:turn_finished`；preview 含 `atlas-mock-reply`（非 `echo:`）。  
PC（**真机**）：同 Connect/Send；preview **非** `echo:` / **非** `atlas-mock-reply`；步骤与失败表见 [`live-agent-handtest-checklist.md`](./live-agent-handtest-checklist.md)。  
mock `.cmd` 的 `MOCK_CLI_SLEEP_MS` 经 `timeout` 近似，**秒级粒度**（见 windows checklist §7）。


---

## 6. 可观测性

```bash
curl -s http://127.0.0.1:8787/healthz
# {"ok":true,"backend":"cli","agent_cli":"…","agent_cli_found":true|false}

curl -s http://127.0.0.1:8787/stats
# {"invoke_count":N,"backend":"cli","agent_cli":"…","agent_cli_found":true|false}
```

`sendPrompt` / invoke 失败 upstream 文案区分：

| 情况 | 文案关键字 |
|------|------------|
| 二进制不存在 | `CLI binary not found` |
| spawn 其他 IO 失败 | `CLI spawn failed` |
| 子进程非零退出 | `CLI non-zero exit` |
| 软超时 | `CLI timeout` |

**禁止**在 CLI 缺失时静默 echo 成功。

Hub **C1b**：未设 `ATLAS_GATEWAY_URL` 启动时 `warn!` 提示当前为 InMemory stub，并指向本 runbook。

---

## 7. 旁路对照表

| 后端 | 标签 | 用途 |
|------|------|------|
| **cli** | **主路径** | 真 Atlas/Cursor agent（或 mock-cli 验链路） |
| Hub 无 `ATLAS_GATEWAY_URL` / stub | 旁路 | 协议 / UI 自测（echo） |
| box | 旁路 | 私有运行时实验（确定性应答 + tools） |
| openai | 旁路 | 兼容 API |

---

## 8. 已知限制（C1 + W1 填实）

1. **默认 `ATLAS_AGENT_CLI`：** 名 `agent`；探测 = `command -v` / `Get-Command` / 可执行路径存在（脚本）+ gateway `resolve_agent_cli_found`（PATH 或文件）。  
2. **真机登录：** 依赖 Cursor/Atlas 本机已登录；本仓不打包外置 `agent`。P1 有 `dist/`+install 骨架（非商店/非签名）见 [`packaging-skeleton.md`](./packaging-skeleton.md)。  
3. **脚本平台：** Unix `scripts/dev-cli-stack.sh` + Windows `scripts/dev-cli-stack.ps1`（**语义对等**）；手动 bat 仅附录。  
4. **C1b Hub warn：** **已做**（unset `ATLAS_GATEWAY_URL` → `warn!` + 指本 runbook）。  
5. **PC loopback healthz：** **已做**（调试区探 `http://127.0.0.1:8787/healthz`，失败友好；「复制起栈命令」双行 sh + ps1）。  
6. Hub **默认仍可无 URL 起 stub**（不破坏单测）；改体验靠文档 + 脚本 + warn，**不**强制远程 gateway。  
7. 未改 `bot.*`；未做 Box LLM / I2.2。  
8. **W1 Windows：** PowerShell **5.1+**（亦支持 7+）；mock **已交** `tools/mock-cli/mock-atlas-agent-cli.cmd`（纯 cmd；Git Bash `.sh` 仍可用作备选）；PID 清理 = `.cli-stack-pids/*.pid` + `Stop-Process`（Ctrl-C / `finally`）；与 sh 的已知差异：healthz 用 `Invoke-WebRequest`（回退 `curl.exe`），mock `.cmd` 的 `chars=` 为纯长度（无 `cksum` hash）。  
9. **CS1 流式：** 默认 **text**；`ATLAS_AGENT_CLI_STREAM=1` opt-in（见 §5b / [`cli-streaming-checklist.md`](./cli-streaming-checklist.md) §7）。未做假流式；未改 `bot.*`。  
10. **WS1·B Win 流式起法：** `-Stream` 或 `ATLAS_CLI_STACK_STREAM=1` 填未设默认（STREAM / EVENT_URL / insecure-or-token / mock 时 MOCK）；显式 env 优先；无开关 = text。Unix `.sh` **不**默认 STREAM（仅横幅/注释对齐）。分进程 mid-turn **仅 B1**。无 Win CI runner（WS2）。

---

## 9. 相关

- Checklist / E1 mock：[`cli-primary-checklist.md`](./cli-primary-checklist.md)  
- Windows 起栈：[`windows-cli-stack-checklist.md`](./windows-cli-stack-checklist.md)  
- 真机联调包（L1）：[`live-agent-handtest-checklist.md`](./live-agent-handtest-checklist.md)  
- PC 使用说明：[`pc-user-guide.md`](./pc-user-guide.md)  
- P3.5 细节：[`P3.5-runbook.md`](./P3.5-runbook.md)  
- CLI 流式：[`cli-streaming-checklist.md`](./cli-streaming-checklist.md)  
- 分发雏形（dist/install）：[`packaging-skeleton.md`](./packaging-skeleton.md)  
- B1 ingest：[`b1-event-ingest-runbook.md`](./b1-event-ingest-runbook.md)
