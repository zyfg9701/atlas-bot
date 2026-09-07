//! P3.5 smoke: CLI gateway non-echo + dual-agent + cold + interrupt -> SMOKE_OK
//!
//! cargo test -p atlas-bot-hub --test p35_smoke -- --nocapture --test-threads=1

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use atlas_bot_gateway::{CliAgentGateway, Gateway};
use atlas_bot_hub::{Hub, Session};
use serde_json::{json, Value};
use xai_tool_protocol::{JsonRpcId, JsonRpcRequest, JsonRpcVersion, ResponseOutcome};

fn mock_cli_path() -> PathBuf {
    if let Ok(p) = std::env::var("ATLAS_AGENT_CLI") {
        let pb = PathBuf::from(&p);
        if pb.exists() {
            return pb;
        }
    }
    let candidates = [
        PathBuf::from("tools/mock-cli/mock-atlas-agent-cli.sh"),
        PathBuf::from("../tools/mock-cli/mock-atlas-agent-cli.sh"),
        PathBuf::from("../../tools/mock-cli/mock-atlas-agent-cli.sh"),
        PathBuf::from("/workspace/atlas-bot-p0/tools/mock-cli/mock-atlas-agent-cli.sh"),
    ];
    for c in candidates {
        if c.exists() {
            return std::fs::canonicalize(&c).unwrap_or(c);
        }
    }
    panic!("mock CLI not found; set ATLAS_AGENT_CLI");
}

async fn rpc(session: &mut Session, id: i64, method: &str, params: Value) -> Value {
    let frame = json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params,
    });
    let outs = session.on_text(&frame.to_string()).await;
    assert_eq!(outs.len(), 1, "expected one response for {method}: {outs:?}");
    serde_json::from_str(&outs[0]).expect("json")
}

fn result(resp: &Value) -> &Value {
    resp.get("result")
        .unwrap_or_else(|| panic!("expected result, got {resp}"))
}

fn rpc_req(id: i64, method: &str, params: Value) -> JsonRpcRequest<Value> {
    JsonRpcRequest {
        jsonrpc: JsonRpcVersion,
        id: JsonRpcId::Number(id),
        session_id: None,
        method: method.to_string(),
        params,
    }
}

#[tokio::test]
async fn p35_smoke_cli_gateway() {
    let cli = mock_cli_path();

    std::env::set_var("MOCK_CLI_SLEEP_MS", "0");
    let gw = Arc::new(CliAgentGateway::new(cli.clone(), vec![], None));
    let hub = Hub::new(gw.clone());
    hub.spawn_turn_bridge(gw.subscribe_turns());
    let mut session = Session::new(hub.clone());

    let hello = session
        .on_text(r#"{"protocol_version":"1.0.0","kind":"bot_client"}"#)
        .await;
    assert_eq!(hello.len(), 1);

    let before_cold = gw.invoke_count();
    let _ = rpc(&mut session, 1, "bot.status", json!({})).await;
    let _ = rpc(&mut session, 2, "bot.roster", json!({})).await;
    assert_eq!(gw.invoke_count(), before_cold, "cold status/roster must not wake gateway");

    let created = rpc(
        &mut session,
        3,
        "bot.command",
        json!({
            "agentId": "agt_1",
            "name": "createAgent",
            "args": { "name": "Scout" }
        }),
    )
    .await;
    let agt2 = result(&created)["agentId"].as_str().unwrap().to_string();

    let _ = rpc(
        &mut session,
        4,
        "bot.subscribe",
        json!({ "agentIds": ["agt_1", agt2] }),
    )
    .await;

    let send1 = rpc(
        &mut session,
        5,
        "bot.command",
        json!({
            "agentId": "agt_1",
            "name": "sendPrompt",
            "args": { "agentId": "agt_1", "prompt": "hello-p35-alpha" }
        }),
    )
    .await;
    let r1 = result(&send1);
    assert_eq!(r1["accepted"], true);
    assert_eq!(r1["completed"], true);
    let preview1 = r1["preview"].as_str().unwrap().to_string();
    assert!(preview1.contains("atlas-mock-reply"), "expected mock non-echo reply, got {preview1}");
    assert!(!preview1.contains("echo:"), "must not be stub echo: {preview1}");
    assert!(preview1.contains("agent=agt_1"), "reply must be scoped to agt_1: {preview1}");

    let send2 = rpc(
        &mut session,
        6,
        "bot.command",
        json!({
            "agentId": agt2,
            "name": "sendPrompt",
            "args": { "agentId": agt2, "prompt": "hello-p35-bravo" }
        }),
    )
    .await;
    let preview2 = result(&send2)["preview"].as_str().unwrap().to_string();
    assert!(preview2.contains(&format!("agent={agt2}")));
    assert!(!preview2.contains("agent=agt_1"));
    assert_ne!(preview1, preview2);

    let tail2 = rpc(
        &mut session,
        7,
        "bot.command",
        json!({
            "agentId": agt2,
            "name": "getAgentTranscriptTail",
            "args": { "id": agt2, "limit": 20 }
        }),
    )
    .await;
    let entries2 = result(&tail2)["entries"].as_array().unwrap();
    assert!(entries2.iter().any(|e| e["text"] == "hello-p35-bravo"));
    assert!(entries2.iter().any(|e| e["text"].as_str().unwrap_or("").contains("atlas-mock-reply")));
    assert!(!entries2.iter().any(|e| e["text"] == "hello-p35-alpha"));

    let cold_before = gw.invoke_count();
    let _ = rpc(&mut session, 8, "bot.transcript.offbox", json!({ "agentId": "agt_1" })).await;
    assert_eq!(gw.invoke_count(), cold_before);

    std::env::set_var("MOCK_CLI_SLEEP_MS", "2500");
    let hub_c = hub.clone();
    let send_task = tokio::spawn(async move {
        hub_c
            .handle_rpc(
                "c_interrupt",
                rpc_req(
                    90,
                    "bot.command",
                    json!({
                        "agentId": "agt_1",
                        "name": "sendPrompt",
                        "args": { "agentId": "agt_1", "prompt": "slow-run" }
                    }),
                ),
            )
            .await
    });
    tokio::time::sleep(Duration::from_millis(150)).await;
    let interrupted = rpc(
        &mut session,
        9,
        "bot.command",
        json!({
            "agentId": "agt_1",
            "name": "interruptAgentRun",
            "args": { "agentId": "agt_1" }
        }),
    )
    .await;
    let ir = result(&interrupted);
    assert_eq!(ir["hadActiveRun"], true, "interrupt must see active run: {ir}");
    assert_eq!(ir["killed"], true, "must signal kill: {ir}");
    let send_outcome = send_task.await.expect("join");
    assert!(
        matches!(send_outcome.outcome, ResponseOutcome::Error(_)),
        "interrupted sendPrompt should error, got {send_outcome:?}"
    );

    std::env::set_var("MOCK_CLI_SLEEP_MS", "0");
    let again = rpc(
        &mut session,
        10,
        "bot.command",
        json!({
            "agentId": "agt_1",
            "name": "sendPrompt",
            "args": { "agentId": "agt_1", "prompt": "after-interrupt" }
        }),
    )
    .await;
    assert_eq!(result(&again)["accepted"], true);
    assert!(result(&again)["preview"].as_str().unwrap().contains("atlas-mock-reply"));
    assert!(gw.invoke_count() > before_cold);

    let gw_r = Arc::new(CliAgentGateway::new(cli, vec![], None));
    let hub_r = Hub::new(gw_r);
    assert!(!hub_r.turn_bridge_active());
    hub_r.register_connection("c_remote").await;
    let mut bus = hub_r.subscribe_outbound();
    hub_r
        .handle_rpc("c_remote", rpc_req(0, "bot.subscribe", json!({ "agentIds": ["agt_1"] })))
        .await;
    let resp = hub_r
        .handle_rpc(
            "c_remote",
            rpc_req(
                1,
                "bot.command",
                json!({
                    "agentId": "agt_1",
                    "name": "sendPrompt",
                    "args": { "agentId": "agt_1", "prompt": "http-turn" }
                }),
            ),
        )
        .await;
    assert!(matches!(resp.outcome, ResponseOutcome::Result(_)));

    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    let mut saw = false;
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(200), bus.recv()).await {
            Ok(Ok((cid, text))) if cid == "c_remote" => {
                let v: Value = serde_json::from_str(&text).unwrap();
                if v["params"]["channel"] == "hub:turn_finished" {
                    saw = true;
                    assert!(v["params"]["event"]["preview"].as_str().unwrap_or("").contains("atlas-mock-reply"));
                    break;
                }
            }
            _ => {}
        }
    }
    assert!(saw, "Hub must emit hub:turn_finished after sync-complete sendPrompt when unbridged");

    println!("SMOKE_OK p35 cli-gateway non-echo+dual-agent+cold+interrupt+http-turn_finished");
}
