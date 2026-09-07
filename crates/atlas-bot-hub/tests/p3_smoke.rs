//! P3 smoke: dual-agent create/switch + offbox paging + interrupt → SMOKE_OK
//!
//! Run: `cargo test -p atlas-bot-hub --test p3_smoke -- --nocapture`

use std::sync::Arc;
use std::time::Duration;

use atlas_bot_gateway::{Gateway, InMemoryGateway};
use atlas_bot_hub::{Hub, Session};
use serde_json::{json, Value};

async fn rpc(session: &mut Session, id: i64, method: &str, params: Value) -> Value {
    let frame = json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params,
    });
    let outs = session.on_text(&frame.to_string()).await;
    assert_eq!(outs.len(), 1, "expected one response for {method}");
    serde_json::from_str(&outs[0]).expect("json")
}

fn result(resp: &Value) -> &Value {
    resp.get("result")
        .unwrap_or_else(|| panic!("expected result, got {resp}"))
}

#[tokio::test]
async fn p3_smoke_dual_agent_offbox_interrupt() {
    let gw = Arc::new(InMemoryGateway::with_turn_delay(Duration::from_secs(5)));
    let hub = Hub::new(gw.clone());
    hub.spawn_turn_bridge(gw.subscribe_turns());
    let mut session = Session::new(hub.clone());

    let hello = session
        .on_text(r#"{"protocol_version":"1.0.0","kind":"bot_client"}"#)
        .await;
    assert_eq!(hello.len(), 1);
    let ack: Value = serde_json::from_str(&hello[0]).unwrap();
    assert!(ack["capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c == "bot.transcript.offbox"));

    // Cold status / roster (no gateway wake accounted via invoke delta later).
    let before = gw.invoke_count();
    let st = rpc(&mut session, 1, "bot.status", json!({})).await;
    assert_eq!(result(&st)["runState"], "hibernated");
    let _ = rpc(&mut session, 2, "bot.roster", json!({})).await;

    // Create second agent
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
    assert_ne!(agt2, "agt_1");

    // Cold roster should include new agent (hub cache synced on create).
    let roster = rpc(&mut session, 4, "bot.roster", json!({})).await;
    let agents = result(&roster)["agents"].as_array().unwrap();
    assert!(agents.iter().any(|a| a["agentId"] == agt2));

    // listAgents hot — both visible
    let listed = rpc(
        &mut session,
        5,
        "bot.command",
        json!({
            "agentId": "agt_1",
            "name": "listAgents",
            "args": {}
        }),
    )
    .await;
    let list = result(&listed).as_array().unwrap();
    assert!(list.iter().any(|a| a["id"] == "agt_1"));
    assert!(list.iter().any(|a| a["id"] == agt2));

    // Subscribe both — seq isolated per (connection, agent)
    let _ = rpc(
        &mut session,
        6,
        "bot.subscribe",
        json!({ "agentIds": ["agt_1", agt2] }),
    )
    .await;

    // Closed loop on NEW agent
    let _ = rpc(
        &mut session,
        7,
        "bot.command",
        json!({
            "agentId": agt2,
            "name": "sendPrompt",
            "args": { "agentId": agt2, "prompt": "hello scout", "immediate": true }
        }),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    let tail = rpc(
        &mut session,
        8,
        "bot.command",
        json!({
            "agentId": agt2,
            "name": "getAgentTranscriptTail",
            "args": { "id": agt2, "limit": 20 }
        }),
    )
    .await;
    let entries = result(&tail)["entries"].as_array().unwrap();
    assert!(entries.iter().any(|e| e["text"] == "hello scout"));

    // Seed offbox with enough rows for paging, then cold-page twice.
    hub.seed_offbox(
        "agt_1",
        vec![
            json!({"id": "p1", "seq": 1, "text": "a"}),
            json!({"id": "p2", "seq": 2, "text": "b"}),
            json!({"id": "p3", "seq": 3, "text": "c"}),
        ],
    )
    .await;
    let cold_before = gw.invoke_count();
    let page1 = rpc(
        &mut session,
        9,
        "bot.transcript.offbox",
        json!({ "agentId": "agt_1" }),
    )
    .await;
    let p1 = result(&page1);
    assert_eq!(p1["entries"].as_array().unwrap().len(), 2);
    assert_eq!(p1["nextCursor"], "c2");
    let page2 = rpc(
        &mut session,
        10,
        "bot.transcript.offbox",
        json!({ "agentId": "agt_1", "cursor": "c2" }),
    )
    .await;
    let p2 = result(&page2);
    assert_eq!(p2["entries"].as_array().unwrap().len(), 1);
    assert_eq!(gw.invoke_count(), cold_before, "offbox must not wake gateway");

    // Interrupt in-flight run on agt_1, then sendPrompt again.
    let _ = rpc(
        &mut session,
        11,
        "bot.command",
        json!({
            "agentId": "agt_1",
            "name": "sendPrompt",
            "args": { "agentId": "agt_1", "prompt": "long-run" }
        }),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(40)).await;
    let interrupted = rpc(
        &mut session,
        12,
        "bot.command",
        json!({
            "agentId": "agt_1",
            "name": "interruptAgentRun",
            "args": { "agentId": "agt_1" }
        }),
    )
    .await;
    assert_eq!(result(&interrupted)["hadActiveRun"], true);
    let again = rpc(
        &mut session,
        13,
        "bot.command",
        json!({
            "agentId": "agt_1",
            "name": "sendPrompt",
            "args": { "agentId": "agt_1", "prompt": "after-interrupt", "immediate": true }
        }),
    )
    .await;
    assert_eq!(result(&again)["accepted"], true);

    // Cold paths from the start should not solely account for all invokes;
    // verify status/roster/offbox portion did not add beyond hot commands.
    assert!(gw.invoke_count() > before);

    println!("SMOKE_OK p3 dual-agent+offbox-page+interrupt");
}
