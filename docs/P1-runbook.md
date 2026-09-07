# P1 runbook — Hub + gateway stub smoke

## Build

```bash
cargo build -p atlas-bot-hub -p atlas-bot-gateway
cargo test -p atlas-bot-hub -p atlas-bot-gateway
cargo test -p xai-tool-protocol --test bot_relay_conformance
```

License gate (optional locally; required in CI):

```bash
./tools/license-check/check.sh
```

## Start Hub (in-process gateway)

```bash
RUST_LOG=info cargo run -p atlas-bot-hub
# listens on ws://127.0.0.1:7700/ws  (override with ATLAS_HUB_BIND)
```

Optional: also expose gateway HTTP for passthrough experiments:

```bash
ATLAS_GATEWAY_HTTP_BIND=127.0.0.1:8787 RUST_LOG=info cargo run -p atlas-bot-hub
```

Or point the hub at a separate HTTP gateway:

```bash
# terminal A — hub with embedded HTTP gateway
ATLAS_GATEWAY_HTTP_BIND=127.0.0.1:8787 cargo run -p atlas-bot-hub

# terminal B — another hub using HTTP passthrough (example)
ATLAS_GATEWAY_URL=http://127.0.0.1:8787 ATLAS_HUB_BIND=127.0.0.1:7701 cargo run -p atlas-bot-hub
```

## Minimal WebSocket closed loop

Use any WS client (example with `websocat` if installed):

```bash
# 1) hello (raw first frame)
printf '%s\n' '{"protocol_version":"1.0.0","kind":"bot_client"}' \
  | websocat -n1 ws://127.0.0.1:7700/ws
# expect hello_ack JSON with capabilities including bot.command, bot.status, …

# Interactive session sketch (send each line after hello_ack):
# cold — must NOT hit gateway:
{"jsonrpc":"2.0","id":1,"method":"bot.status","params":{}}
{"jsonrpc":"2.0","id":2,"method":"bot.roster","params":{}}

# subscribe:
{"jsonrpc":"2.0","id":3,"method":"bot.subscribe","params":{"agentIds":["agt_1"]}}

# hot — listAgents / sendPrompt / getAgentTranscriptTail:
{"jsonrpc":"2.0","id":4,"method":"bot.command","params":{"agentId":"agt_1","name":"listAgents","args":{}}}
{"jsonrpc":"2.0","id":5,"method":"bot.command","params":{"agentId":"agt_1","name":"sendPrompt","args":{"agentId":"agt_1","prompt":"hello P1"}}}

# expect a bot.event notification:
# {"jsonrpc":"2.0","method":"bot.event","params":{"v":1,"agentId":"agt_1","seq":1,"channel":"hub:turn_finished",…}}

{"jsonrpc":"2.0","id":6,"method":"bot.command","params":{"agentId":"agt_1","name":"getAgentTranscriptTail","args":{"id":"agt_1","limit":10}}}
```

### Python one-shot smoke (`websocket-client`)

```bash
pip install websocket-client
python3 - <<'PY'
import json, websocket
ws = websocket.create_connection("ws://127.0.0.1:7700/ws")
ws.send(json.dumps({"protocol_version":"1.0.0","kind":"bot_client"}))
print("ack", ws.recv())
def rpc(mid, method, params):
    ws.send(json.dumps({"jsonrpc":"2.0","id":mid,"method":method,"params":params}))
    print("←", ws.recv())
rpc(1, "bot.status", {})
rpc(2, "bot.roster", {})
rpc(3, "bot.subscribe", {"agentIds":["agt_1"]})
rpc(4, "bot.command", {"agentId":"agt_1","name":"listAgents","args":{}})
rpc(5, "bot.command", {"agentId":"agt_1","name":"sendPrompt","args":{"agentId":"agt_1","prompt":"hi"}})
# may receive bot.event then response — drain a couple frames
import time; time.sleep(0.2)
ws.settimeout(1)
try:
  while True:
    print("←", ws.recv())
except Exception:
  pass
ws.close()
PY
```

## Cold vs hot checks

Unit tests assert separation:

```bash
cargo test -p atlas-bot-hub cold_status_roster_do_not_invoke_gateway
cargo test -p atlas-bot-hub hot_command_invokes_gateway
```

In logs: `cold bot.status` / `cold bot.roster` lines must appear **without** `gateway invoke`; `hot bot.command -> gateway` must be paired with `gateway invoke`.

## Seq / resync notes (P1)

- `bot.event.seq` is per `(connection, agent)`, starts at **1** after each subscribe.
- Seq is an ordering reference, **not** a dedupe key; never infer resync from gaps.
- Explicit `hub:resync_required` can be injected via hub APIs/tests.

## Out of scope (P1)

mcp-adapter, real IdP, PC UI, CLI, VNC, attachment/group/channel commands.
