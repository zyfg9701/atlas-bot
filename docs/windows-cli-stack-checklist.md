# Windows CLI 起栈 checklist（W1）

> 基线：`scripts/dev-cli-stack.ps1` + `tools/mock-cli/mock-atlas-agent-cli.cmd`  
> 对等：`scripts/dev-cli-stack.sh` · 验收：`feitian-windows-cli-stack-acceptance.md`

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

## §7 已知限制（W1）

| 项 | 填实 |
|----|------|
| PowerShell | **5.1+**（7+ 亦可） |
| mock | **已交** `.cmd`；Git Bash `.sh` 备选 |
| PID 清理 | `.cli-stack-pids/{gateway,hub}.pid` + Ctrl-C/`finally` → `Stop-Process` |
| 与 sh 差异 | healthz：`Invoke-WebRequest`（回退 `curl.exe`）；mock `.cmd` 无 `cksum` hash，仅 `chars=` |

## 契约对照（§2）

见 PR 描述表；脚本审查即可（无 Win runner 不挡合）。
