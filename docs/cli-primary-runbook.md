# CLI primary path runbook（推荐主路径）

> **唯一推荐起法**：PC → Hub → `atlas-bot-gateway` (`backend=cli`) → `ATLAS_AGENT_CLI`  
> 基线：P3.5 `CliAgentGateway` · 打磨 C1/C1b  
> 旁路（stub / box / openai）见下文对照表；协议细节仍见 [`P3.5-runbook.md`](./P3.5-runbook.md)。

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

# 无真 agent：用 mock .cmd（纯 Win，无需 Git Bash）
$env:ATLAS_AGENT_CLI="$PWD\tools\mock-cli\mock-atlas-agent-cli.cmd"; .\scripts\dev-cli-stack.ps1
```

`dev-cli-stack.ps1` 与 Unix `.sh` **语义对等**：探测 `ATLAS_AGENT_CLI`（`Get-Command` / 路径，含 `.exe`/`.cmd`）→ 找不到 **exit 1** 且不静默 stub → 起 gateway `BACKEND=cli` + Hub `ATLAS_GATEWAY_URL`（Hub 侧 `ATLAS_GATEWAY_HTTP_BIND=off`）→ 打印 `ws://127.0.0.1:7700/ws` 与 healthz → Ctrl-C 清 PID（`.cli-stack-pids/`）与日志（`.cli-stack-logs/`）。

手测勾选见 [`windows-cli-stack-checklist.md`](./windows-cli-stack-checklist.md)。

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
| `ATLAS_AGENT_CLI_EXTRA_ARGS` | `["--output-format","text"]` | JSON 字符串数组 |
| `ATLAS_AGENT_CLI_TIMEOUT_MS` | _(none)_ | 单 turn 软超时；超时错误可见 |
| `ATLAS_GATEWAY_HTTP_BIND` | `127.0.0.1:8787` | gateway listen；Hub 挂远程时建议 `off` |
| `ATLAS_HUB_BIND` | `127.0.0.1:7700` | Hub WS |

---

## 5. 真机前提 / mock 路径

- **真机：** 本机已安装 **且已登录** Cursor/Atlas agent（默认命令名 `agent`）。未登录时 spawn/exit 会失败且 **可见**，不会假成功。  
- **mock（CI/无真 agent）：**  
  - Unix：`ATLAS_AGENT_CLI=tools/mock-cli/mock-atlas-agent-cli.sh`  
  - Windows：`$env:ATLAS_AGENT_CLI="$PWD\tools\mock-cli\mock-atlas-agent-cli.cmd"`  
  回复形如 `atlas-mock-reply agent=…`（**非** `echo:` stub）。  
- 证据步骤见 [`cli-primary-checklist.md`](./cli-primary-checklist.md)。

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
2. **真机登录：** 依赖 Cursor/Atlas 本机已登录；本仓不打包安装器。  
3. **脚本平台：** Unix `scripts/dev-cli-stack.sh` + Windows `scripts/dev-cli-stack.ps1`（**语义对等**）；手动 bat 仅附录。  
4. **C1b Hub warn：** **已做**（unset `ATLAS_GATEWAY_URL` → `warn!` + 指本 runbook）。  
5. **PC loopback healthz：** **已做**（调试区探 `http://127.0.0.1:8787/healthz`，失败友好；「复制起栈命令」双行 sh + ps1）。  
6. Hub **默认仍可无 URL 起 stub**（不破坏单测）；改体验靠文档 + 脚本 + warn，**不**强制远程 gateway。  
7. 未改 `bot.*`；未做 Box LLM / I2.2。  
8. **W1 Windows：** PowerShell **5.1+**（亦支持 7+）；mock **已交** `tools/mock-cli/mock-atlas-agent-cli.cmd`（纯 cmd；Git Bash `.sh` 仍可用作备选）；PID 清理 = `.cli-stack-pids/*.pid` + `Stop-Process`（Ctrl-C / `finally`）；与 sh 的已知差异：healthz 用 `Invoke-WebRequest`（回退 `curl.exe`），mock `.cmd` 的 `chars=` 为纯长度（无 `cksum` hash）。

---

## 9. 相关

- Checklist / E1 mock：[`cli-primary-checklist.md`](./cli-primary-checklist.md)  
- Windows 起栈：[`windows-cli-stack-checklist.md`](./windows-cli-stack-checklist.md)  
- PC 使用说明：[`pc-user-guide.md`](./pc-user-guide.md)  
- P3.5 细节：[`P3.5-runbook.md`](./P3.5-runbook.md)
