# CLI primary path · checklist / E1 mock

> 配合 [`cli-primary-runbook.md`](./cli-primary-runbook.md)。用于无真 Cursor agent 时验主链路。

## E1 mock（推荐 CI/开发证据）

```bash
# 1) 起栈（mock-cli 覆盖探测）
ATLAS_AGENT_CLI=tools/mock-cli/mock-atlas-agent-cli.sh ./scripts/dev-cli-stack.sh

# 另开终端 — 2) 健康面
curl -s http://127.0.0.1:8787/healthz
# expect: "backend":"cli" 且 "agent_cli_found":true

curl -s http://127.0.0.1:8787/stats

# 3) 既有冒烟（不依赖脚本进程）
cargo test -p atlas-bot-hub --test p35_smoke -- --nocapture --test-threads=1
# expect: SMOKE_OK p35 cli-gateway …
```

PC（栈已起）：Connect → `ws://127.0.0.1:7700/ws` → Send → 回复含 `atlas-mock-reply`（非 `echo:`）。

## 无二进制失败可见

```bash
ATLAS_AGENT_CLI=/no/such/agent-binary ./scripts/dev-cli-stack.sh
# expect: 非零退出 + 清晰 install/login / mock 提示；不启 stub 冒充
```

## 勾选

- [ ] E1 mock 栈 healthz `backend=cli` + `agent_cli_found=true`
- [ ] `p35_smoke` 绿 / `SMOKE_OK`
- [ ] 无 agent 时脚本非零退出
- [ ] pc-user-guide 主路径为 cli；stub/box/openai 标旁路
