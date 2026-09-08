# Runtime boundary runbook (R1 Box Sidecar)

Baseline: `main` @ `b94895e`. Stage: **真运行时边界** (IdP deferred).

This is **not** grok-build / `xai-grok-*`. R1 adds a long-lived Box Sidecar behind the
existing Gateway HTTP `/invoke` surface. Hub still only sets `ATLAS_GATEWAY_URL`.
CLI / stub / OpenAI backends remain available.

## Architecture

```text
bot_client ──WS──► Hub
                     ├ cold: status / roster / transcript.offbox  (no gateway wake)
                     └ hot:  bot.command ──HTTP──► atlas-bot-gateway
                                                    ATLAS_GATEWAY_BACKEND=box|cli|openai|stub
```

## Env

| Var | Role |
|-----|------|
| `ATLAS_HUB_BIND` | Hub WS bind (default `127.0.0.1:7700`) |
| `ATLAS_GATEWAY_URL` | Hub → gateway HTTP base (e.g. `http://127.0.0.1:8787`) |
| `ATLAS_GATEWAY_HTTP_BIND` | Gateway listen (default `127.0.0.1:8787`) |
| `ATLAS_GATEWAY_BACKEND` | `cli` (default) \| `openai` \| `stub` \| **`box`** |
| `ATLAS_BOX_WORKSPACE` | Box workspace root (default `./data/box-workspace`); each agent gets `<root>/<agentId>/` |
| `ATLAS_BOX_TURN_DELAY_MS` | Interruptible sendPrompt delay (default `50`) |
| `ATLAS_AGENT_CLI` / `ATLAS_OPENAI_*` | Unchanged; used when backend is `cli` / `openai` |

## Start Hub + Box Sidecar

```bash
# 1) Box sidecar (same binary, backend=box)
ATLAS_GATEWAY_BACKEND=box \
ATLAS_BOX_WORKSPACE=./data/box-workspace \
ATLAS_GATEWAY_HTTP_BIND=127.0.0.1:8787 \
  cargo run -p atlas-bot-gateway

# 2) Health
curl -s http://127.0.0.1:8787/healthz   # ok
curl -s http://127.0.0.1:8787/stats     # {"invoke_count":0}

# 3) Hub (only needs the URL — no box-specific Hub flags)
ATLAS_GATEWAY_URL=http://127.0.0.1:8787 \
  cargo run -p atlas-bot-hub
```

Coexist with CLI: set `ATLAS_GATEWAY_BACKEND=cli` (and `ATLAS_AGENT_CLI`) instead of `box`.
Stub-only: leave `ATLAS_GATEWAY_URL` unset on Hub (embedded InMemory).

## Tool whitelist (Box)

Documented triggers inside `sendPrompt` text (case-sensitive substrings):

| Trigger | Behavior |
|---------|----------|
| `LIST_DIR` | Safe read-only directory listing of the agent workspace subdir |
| `RUN ls` | Allowlisted shell: `ls` with cwd = agent workspace (no args) |
| `RUN pwd` | Allowlisted shell: `pwd` with cwd = agent workspace |

No other shell commands. Evidence is appended into the assistant transcript / `preview`
as `[tool:list_dir …]` or `[tool:shell cmd=…]` lines.

## Who owns VNC / upload

R1 **does not** move VNC or attachments onto the Box Sidecar.

- **VNC / uploadAttachment / attachUpload** stay on the existing **InMemory / embedded HTTP**
  path (P5 stub or disk+proxy). Use Hub without `ATLAS_GATEWAY_URL`, or
  `ATLAS_GATEWAY_BACKEND=stub`, for those features.
- Box backend returns `gateway/unknown-method` for upload/VNC commands.
- IdP: still deferred.

## Smoke

```bash
cargo test -p atlas-bot-hub --test runtime_smoke -- --nocapture --test-threads=1
# Expect: SMOKE_OK runtime multi-turn+workspace-tool+interrupt+cold
```

Also keep green: `p35_smoke`, `p5_smoke`, `p5r_smoke`, `p6_smoke`, `alpha_smoke`.

CI (`protocol-conformance.yml` hub-smoke) runs `runtime_smoke` after the existing smokes.

## Known limitations (§10 acceptance)

1. **Interrupt:** **True cancel** of in-flight `sendPrompt` via oneshot / Abort-style flag
   during `ATLAS_BOX_TURN_DELAY_MS`. Idle `interruptAgentRun` returns closed-set
   `command_rejected` reason `no_active_run` — **never fake success**.
2. **Tool whitelist:** only `LIST_DIR`, `RUN ls`, `RUN pwd` as above. Workspace layout:
   `$ATLAS_BOX_WORKSPACE/<agentId>/` with a `.box-sidecar` marker file.
3. **VNC/upload:** still on InMemory/HTTP stub|disk path; Box sidecar does not own them.
4. **Streaming:** sync result + existing `hub:turn_finished` (`hubEmitTurnFinished`); no
   token/tool event push in R1.
5. **Model:** deterministic local responder (history + optional tool evidence). Nails
   non-echo / multi-turn reproducibility, not IQ. **Not** grok-build.

## Out of scope (R1)

True IdP, installers, multi-box cluster, vendor grok-build / MCP full suite, changing
Bot-Relay wire, ACP inside Hub, removing P3.5 CLI.
