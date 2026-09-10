# CLI 流式（CS1）手测 / 已知限制

> 切片 **CS1 / S1**：`CliAgentGateway` 行读 stream-json → `RuntimeHint` → Hub `bot.event`  
> 基线 runbook：[`cli-primary-runbook.md`](./cli-primary-runbook.md) §5b · B1：[`b1-event-ingest-runbook.md`](./b1-event-ingest-runbook.md)

## 手测位

- [ ] `ATLAS_AGENT_CLI_STREAM=1` + 真机已登录 agent：Events 见 ≥1× `hub:assistant_delta`，再 `hub:turn_finished`；sync preview 非 echo
- [ ] mock：`MOCK_CLI_STREAM=1 MOCK_CLI_SLEEP_MS=80` + `cli_stream_smoke` → `SMOKE_OK`
- [ ] text 退化（unset STREAM）：无 mid-turn；`p35_smoke` 仍绿
- [ ] interrupt → `start_kill` 仍有效（`p35_smoke`）
- [ ] 分进程 CS1b：仅 `ATLAS_HUB_EVENT_URL`，**不开** B2 bridge；或仅 B2、不设 EVENT_URL
- [ ] **未**做假流式；**未**改 `bot.*`

## 冒烟

```bash
cargo test -p atlas-bot-hub --test cli_stream_smoke -- --nocapture --test-threads=1
```

## §7 已知限制（填实）

| 项 | 决定 |
|----|------|
| 默认流式 vs 显式 env | **显式 opt-in** `ATLAS_AGENT_CLI_STREAM=1`；unset = text（与今日一致）。设 STREAM 且未设 EXTRA_ARGS 时默认 `--output-format stream-json` + `--stream-partial-output` |
| partial 过滤 | 消费 `assistant` 且 **无 `model_call_id`**（优先带 `timestamp_ms`）；带 `model_call_id` 且无 `timestamp_ms` 的行跳过。终稿优先 `type=result` 的 `result` 字段 |
| mock 行格式 | **主：NDJSON** `assistant` + `result`；gateway **兼认** `ATLAS_DELTA\\t` / `ATLAS_TOOL\\t` |
| CS1b 分进程 | **已做** gateway `publish_hint` → `ATLAS_HUB_EVENT_URL`（失败 warn）；专用分进程冒烟未单列（复用 B1 契约 / 同进程 `cli_stream_smoke`） |
| 单 hint 截断 | summary/text **64KiB** 截断；不做小块合并 |
| 双发 | **勿**同开 B2+B1（与 B1 runbook 互斥）；`dev-cli-stack` 分进程 **只** B1 |
| 假流式 | **不做**（不把终稿切成假 delta） |
| Win 起栈 | **WS1·B**：`dev-cli-stack.ps1 -Stream` / `ATLAS_CLI_STACK_STREAM=1`；显式 env 优先；见 windows checklist W-S* |
| CI | `cli_stream_smoke` = Unix；Win 手测不挡合（无 Win runner / WS2） |

## Win 手测（WS1·B）

分进程起栈流式证据以 **本机 PowerShell** 为准（无强制 Win CI runner）：

- 配方：[`cli-primary-runbook.md`](./cli-primary-runbook.md) § Windows / §5b PowerShell  
- 勾选：[`windows-cli-stack-checklist.md`](./windows-cli-stack-checklist.md) **W-S1 / W-S2 / W-S3**  
- 一键：`$env:ATLAS_AGENT_CLI="$PWD\tools\mock-cli\mock-atlas-agent-cli.cmd"; .\scripts\dev-cli-stack.ps1 -Stream`

**CI 注：** `cli_stream_smoke`（及 `p35_smoke` / `b1_smoke`）仍以 **Unix CI** 为主；不把 Win runner 当合入门禁。同进程 B2 冒烟 **不能**替代「本机 ps1 起栈」分进程 B1 手测。

## 相关

- 主路径：[`cli-primary-runbook.md`](./cli-primary-runbook.md)  
- Win 起栈 / 流式：[`windows-cli-stack-checklist.md`](./windows-cli-stack-checklist.md)  
- B1：[`b1-event-ingest-runbook.md`](./b1-event-ingest-runbook.md)  
- 回归：`p35_smoke` · `cli_stream_smoke`（Unix CI）· `b1_smoke`
