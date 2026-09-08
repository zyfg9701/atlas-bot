//! R1 Box Sidecar smoke: multi-turn + workspace tool + interrupt + cold
//!
//! cargo test -p atlas-bot-hub --test runtime_smoke -- --nocapture --test-threads=1

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use atlas_bot_gateway::{BoxSidecarGateway, Gateway};
use atlas_bot_hub::{Hub, Session};
use serde_json::{json, Value};
use xai_tool_protocol::{JsonRpcId, JsonRpcRequest, JsonRpcVersion, ResponseOutcome};

fn rpc_req(id: i64, method: &str, params: Value) -> JsonRpcRequest<Value> {
    JsonRpcRequest {
        jsonrpc: JsonRpcVersion,
        id: JsonRpcId::Number(id),
        session_id: None,
        method: method.to_string(),
        params,
    }
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

#[tokio::test]
async fn runtime_smoke_box_sidecar() {
    let root = std::env::temp_dir().join(format!(
        "atlas-runtime-smoke-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();

    // Fast gateway for multi-turn + tools + cold checks.
    let fast = Arc::new(BoxSidecarGateway::new(
        PathBuf::from(&root),
        Duration::from_millis(10),
    ));
    let hub = Hub::new(fast.clone());
    hub.spawn_turn_bridge(fast.subscribe_turns());
    let mut session = Session::new(hub.clone());
    let hello = session
        .on_text(r#"{"protocol_version":"1.0.0","kind":"bot_client"}"#)
        .await;
    assert_eq!(hello.len(), 1);

    let before_cold = fast.invoke_count();
    let _ = rpc(&mut session, 10, "bot.status", json!({})).await;
    let _ = rpc(&mut session, 11, "bot.roster", json!({})).await;
    let _ = rpc(
        &mut session,
        12,
        "bot.transcript.offbox",
        json!({ "agentId": "agt_1" }),
    )
    .await;
    assert_eq!(
        fast.invoke_count(),
        before_cold,
        "cold status/roster/offbox must not wake gateway"
    );

    let send1 = rpc(
        &mut session,
        13,
        "bot.command",
        json!({
            "agentId": "agt_1",
            "name": "sendPrompt",
            "args": { "agentId": "agt_1", "prompt": "token-RUNTIME-ALPHA" }
        }),
    )
    .await;
    let r1 = result(&send1);
    assert_eq!(r1["accepted"], true);
    assert_eq!(r1["completed"], true);
    let preview1 = r1["preview"].as_str().unwrap().to_string();
    assert!(
        preview1.contains("box-sidecar"),
        "expected box reply, got {preview1}"
    );
    assert!(
        !preview1.starts_with("echo:"),
        "must not be stub echo: {preview1}"
    );
    assert!(preview1.contains("turn=1"), "{preview1}");
    assert_ne!(preview1, "token-RUNTIME-ALPHA");

    let send2 = rpc(
        &mut session,
        14,
        "bot.command",
        json!({
            "agentId": "agt_1",
            "name": "sendPrompt",
            "args": { "agentId": "agt_1", "prompt": "recall prior please" }
        }),
    )
    .await;
    let preview2 = result(&send2)["preview"].as_str().unwrap().to_string();
    assert!(preview2.contains("turn=2"), "{preview2}");
    assert!(
        preview2.contains("token-RUNTIME-ALPHA"),
        "second turn must reference first user text: {preview2}"
    );
    assert!(preview2.contains("ctx="), "{preview2}");

    let created = rpc(
        &mut session,
        15,
        "bot.command",
        json!({
            "agentId": "agt_1",
            "name": "createAgent",
            "args": { "name": "Scout" }
        }),
    )
    .await;
    let agt2 = result(&created)["agentId"].as_str().unwrap().to_string();
    let send_b = rpc(
        &mut session,
        16,
        "bot.command",
        json!({
            "agentId": agt2,
            "name": "sendPrompt",
            "args": { "agentId": agt2, "prompt": "bravo-only" }
        }),
    )
    .await;
    let pb = result(&send_b)["preview"].as_str().unwrap().to_string();
    assert!(pb.contains(&format!("agent={agt2}")), "{pb}");
    assert!(!pb.contains("token-RUNTIME-ALPHA"));

    let tail1 = rpc(
        &mut session,
        17,
        "bot.command",
        json!({
            "agentId": "agt_1",
            "name": "getAgentTranscriptTail",
            "args": { "id": "agt_1", "limit": 50 }
        }),
    )
    .await;
    let entries1 = result(&tail1)["entries"].as_array().unwrap();
    assert!(entries1
        .iter()
        .any(|e| e["text"].as_str().unwrap_or("").contains("token-RUNTIME-ALPHA")));
    assert!(!entries1
        .iter()
        .any(|e| e["text"].as_str().unwrap_or("") == "bravo-only"));

    let tool = rpc(
        &mut session,
        18,
        "bot.command",
        json!({
            "agentId": "agt_1",
            "name": "sendPrompt",
            "args": { "agentId": "agt_1", "prompt": "please LIST_DIR and RUN pwd" }
        }),
    )
    .await;
    let pt = result(&tool)["preview"].as_str().unwrap().to_string();
    assert!(pt.contains("[tool:list_dir"), "missing list_dir evidence: {pt}");
    assert!(
        pt.contains(".box-sidecar"),
        "workspace marker missing in listing: {pt}"
    );
    assert!(pt.contains("[tool:shell cmd=pwd"), "missing pwd evidence: {pt}");

    assert!(fast.invoke_count() > before_cold);

    // Interrupt path on a slow gateway.
    drop(session);
    let slow = Arc::new(BoxSidecarGateway::new(
        PathBuf::from(&root),
        Duration::from_millis(2000),
    ));
    let hub_s = Hub::new(slow.clone());
    let mut session = Session::new(hub_s.clone());
    let _ = session
        .on_text(r#"{"protocol_version":"1.0.0","kind":"bot_client"}"#)
        .await;

    let idle = rpc(
        &mut session,
        20,
        "bot.command",
        json!({
            "agentId": "agt_1",
            "name": "interruptAgentRun",
            "args": { "agentId": "agt_1" }
        }),
    )
    .await;
    assert!(
        idle.get("error").is_some(),
        "idle interrupt must error (no fake success): {idle}"
    );
    let err_msg = idle["error"]["message"].as_str().unwrap_or("");
    assert_eq!(
        err_msg, "command_rejected",
        "expected closed-set reject, got {idle}"
    );

    let hub_c = hub_s.clone();
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
    tokio::time::sleep(Duration::from_millis(80)).await;
    let interrupted = rpc(
        &mut session,
        21,
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
    assert_eq!(ir["killed"], true, "must signal cancel: {ir}");
    let send_outcome = send_task.await.expect("join");
    assert!(
        matches!(send_outcome.outcome, ResponseOutcome::Error(_)),
        "interrupted sendPrompt should error, got {send_outcome:?}"
    );

    let again_gw = Arc::new(BoxSidecarGateway::new(
        PathBuf::from(&root),
        Duration::from_millis(10),
    ));
    let hub_a = Hub::new(again_gw.clone());
    let mut session = Session::new(hub_a);
    let _ = session
        .on_text(r#"{"protocol_version":"1.0.0","kind":"bot_client"}"#)
        .await;
    let again = rpc(
        &mut session,
        22,
        "bot.command",
        json!({
            "agentId": "agt_1",
            "name": "sendPrompt",
            "args": { "agentId": "agt_1", "prompt": "after-interrupt" }
        }),
    )
    .await;
    assert_eq!(result(&again)["accepted"], true);
    assert!(result(&again)["preview"]
        .as_str()
        .unwrap()
        .contains("box-sidecar"));

    let _ = std::fs::remove_dir_all(&root);
    println!("SMOKE_OK runtime multi-turn+workspace-tool+interrupt+cold");
}
