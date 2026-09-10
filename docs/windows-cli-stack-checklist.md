# Windows CLI 起栈 checklist（W1）

> 基线：`scripts/dev-cli-stack.ps1` + `tools/mock-cli/mock-atlas-agent-cli.cmd`  
> 对等：`scripts/dev-cli-stack.sh` · 验收：W1 + **WS1·B**（`feitian-win-cli-streaming-stack-acceptance.md`）

## W-E1 · 无 agent → 非零退出（禁止静默 stub）

```powershell
$env:ATLAS_AGENT_CLI='C:\nonexistent\agent-not-here.exe'
.\scripts\dev-cli-stack.ps1
# 期望：exit 1；stderr 含 "ATLAS_AGENT_CLI not found"；**不会**起 Hub-only stub
```

- [ ] 终端可见错误与 mock 一行提示  
- [ ] 未单独起起 Hub（无 stub 冒充）

**例（期望输出摘要）：**

```text
error: ATLAS_AGENT_CLI not found: C:\nonexistent\agent-not-here.exe
...
This script will NOT silently fall back to the Hub InMemory stub.
For CI / local without a real agent, override with the mock CLI:
  $env:ATLAS_AGENT_CLI='...\tools\mock-cli\mock-atlas-agent-cli.cmd'; .\scripts\dev-cli-stack.ps1
```

## W-E2 · mock .cmd 起栈 → healthz

```powershell
$env:ATLAS_AGENT_CLI="$PWD\tools\mock-cli\mock-atlas-agent-cli.cmd"
powershell -ExecutionPolicy Bypass -File .\scripts\dev-cli-stack.ps1
# 另开终端：
Invoke-WebRequest http://127.0.0.1:8787/healthz | Select-Object -Expand Content
# 期望含 backend=cli 且 agent_cli_found=true（或等价 JSON 字段）
```

- [ ] gateway `/healthz` ok  
- [ ] Hub `ws://127.0.0.1:7700/ws` 可 Connect  

**例（期望 healthz）：**

```json
{"ok":true,"backend":"cli","agent_cli":"...\\mock-atlas-agent-cli.cmd","agent_cli_found":true}
```

## W-E3（可选）· PC Send 得 mock 非 echo

- [ ] Connect → Send → 回复含 `atlas-mock-reply`（**非** `echo:`）

## W-S1 · `-Stream` 横幅可见 STREAM / EVENT_URL / MOCK

```powershell
$env:ATLAS_AGENT_CLI="$PWD\tools\mock-cli\mock-atlas-agent-cli.cmd"
.\scripts\dev-cli-stack.ps1 -Stream
# 期望横幅含：
#   ATLAS_AGENT_CLI_STREAM=1
#   ATLAS_HUB_EVENT_URL=http://127.0.0.1:7701/internal/runtime-hint
#   ATLAS_HUB_EVENT_ALLOW_INSECURE_LOOPBACK=1   # 若未自设 token
#   MOCK_CLI_STREAM=1
#   mode: Stream …
```

- [ ] 横幅打印 STREAM / EVENT_URL /（token 或 insecure）/ MOCK_CLI_STREAM / MOCK_CLI_SLEEP_MS  
- [ ] 等价：`$env:ATLAS_CLI_STACK_STREAM='1'` 无 `-Stream` 参数亦可

## W-S2 · mock + `-Stream` → PC Events mid-turn

栈起来后 PC Connect → `ws://127.0.0.1:7700/ws` → Send：

- [ ] Events ≥1× `hub:assistant_delta`（及可选 `hub:tool`）→ `hub:turn_finished`  
- [ ] sync preview 含 `atlas-mock-reply`（**非** `echo:`）  
- [ ] 分进程 **只** B1（`ATLAS_HUB_EVENT_URL`）；**勿**再开 B2 `spawn_turn_bridge`

## W-S3 · 非流式起法仍 text

```powershell
$env:ATLAS_AGENT_CLI="$PWD\tools\mock-cli\mock-atlas-agent-cli.cmd"
# 确保未设 STREAM / 无 -Stream / 无 ATLAS_CLI_STACK_STREAM
.\scripts\dev-cli-stack.ps1
# 横幅：ATLAS_AGENT_CLI_STREAM=(unset=text)；无强制 mid-turn
```

- [ ] 无 `-Stream`：不自动设 STREAM / EVENT_URL / MOCK  
- [ ] Send 仍可得终稿；无强制 `hub:assistant_delta`（与 CS1 text 一致）

## §7 已知限制（W1 + WS1·B）

| 项 | 填实 |
|----|------|
| PowerShell | **5.1+**（7+ 亦可） |
| mock | **已交** `.cmd`；Git Bash `.sh` 备选；流式分支 **零补丁**（CS1 已齐；`timeout` **秒级**粒度） |
| PID 清理 | `.cli-stack-pids/{gateway,hub}.pid` + Ctrl-C/`finally` → `Stop-Process` |
| 与 sh 差异 | healthz：`Invoke-WebRequest`（回退 `curl.exe`）；mock `.cmd` 无 `cksum` hash，仅 `chars=`；Win 有 `-Stream` 默认填充，Unix `.sh` 仅横幅/注释对齐、**不**默认 STREAM |
| `-Stream` / `ATLAS_CLI_STACK_STREAM` | 二者等价；填未设的 STREAM=1、EVENT_URL=`http://127.0.0.1:7701/internal/runtime-hint`、无 token 时 INSECURE=1、mock `.cmd` 时 MOCK_CLI_STREAM=1 |
| 显式 env 优先 | 用户已设 STREAM / EVENT_URL / MOCK / TOKEN / INSECURE 时 **不覆盖** |
| 分进程 | **只 B1**；勿 B2+B1 双开 |
| CI | **无** Win runner 门禁（WS2）；Unix `cli_stream_smoke` / `p35_smoke` / `b1_smoke` 不回退 |

## 契约对照（§2）

见 PR 描述表；脚本审查 + 本机 W-S* 即可（无 Win runner 不挡合）。
