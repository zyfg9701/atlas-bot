//! CS1 CLI streaming smoke: mid-turn hub:assistant_delta + turn_finished + non-echo.
//!
//! cargo test -p atlas-bot-hub --test cli_stream_smoke -- --nocapture --test-threads=1

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use atlas_bot_gateway::{
    CliAgentGateway, CHANNEL_HUB_ASSISTANT_DELTA, CHANNEL_HUB_TOOL,
};
use atlas_bot_hub::{Hub, Session};
use serde_json::{json, Value};

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

async fn collect_events(
    bus: &mut tokio::sync::broadcast::Receiver<(String, String)>,
    conn: &str,
    wait: Duration,
    stop_when: impl Fn(&[Value]) -> bool,
) -> Vec<Value> {
    let deadline = tokio::time::Instant::now() + wait;
    let mut out = Vec::new();
    while tokio::time::Instant::now() < deadline && !stop_when(&out) {
        match tokio::time::timeout(Duration::from_millis(200), bus.recv()).await {
            Ok(Ok((cid, text))) if cid == conn => {
                if let Ok(v) = serde_json::from_str::<Value>(&text) {
                    if v["method"] == "bot.event" {
                        out.push(v);
                    }
                }
            }
            _ => {}
        }
    }
    out
}

#[tokio::test]
async fn cli_stream_smoke_mid_turn_delta() {
    let cli = mock_cli_path();

    // Mock emits NDJSON deltas; gateway streaming parses them (B2 same-process).
    std::env::set_var("MOCK_CLI_STREAM", "1");
    std::env::set_var("MOCK_CLI_SLEEP_MS", "80");

    let gw = Arc::new(CliAgentGateway::new_streaming(cli.clone(), vec![], None));
    let hub = Hub::new(gw.clone());
    hub.spawn_turn_bridge(gw.subscribe_turns());
    let mut session = Session::new(hub.clone());

    let hello = session
        .on_text(r#"{"protocol_version":"1.0.0","kind":"bot_client"}"#)
        .await;
    assert_eq!(hello.len(), 1);
    let conn = session.connection_id.clone();
    assert!(!conn.is_empty());

    let mut bus = hub.subscribe_outbound();
    let _ = rpc(
        &mut session,
        1,
        "bot.subscribe",
        json!({ "agentIds": ["agt_1"] }),
    )
    .await;

    let send = rpc(
        &mut session,
        2,
        "bot.command",
        json!({
            "agentId": "agt_1",
            "name": "sendPrompt",
            "args": { "agentId": "agt_1", "prompt": "stream-cs1-hello" }
        }),
    )
    .await;
    let r = result(&send);
    assert_eq!(r["accepted"], true);
    assert_eq!(r["completed"], true);
    let preview = r["preview"].as_str().unwrap().to_string();
    assert!(
        preview.contains("atlas-mock-reply"),
        "sync preview must be non-echo mock reply, got {preview}"
    );
    assert!(!preview.contains("echo:"), "must not be stub echo: {preview}");

    let events = collect_events(&mut bus, &conn, Duration::from_secs(5), |evs| {
        let has_finished = evs
            .iter()
            .any(|e| e["params"]["channel"] == "hub:turn_finished");
        let has_delta = evs
            .iter()
            .any(|e| e["params"]["channel"] == CHANNEL_HUB_ASSISTANT_DELTA);
        has_finished && has_delta
    })
    .await;

    let channels: Vec<String> = events
        .iter()
        .filter_map(|e| e["params"]["channel"].as_str().map(|s| s.to_string()))
        .collect();

    let delta_n = channels
        .iter()
        .filter(|c| *c == CHANNEL_HUB_ASSISTANT_DELTA)
        .count();
    assert!(
        delta_n >= 1,
        "expected ≥1 {CHANNEL_HUB_ASSISTANT_DELTA}; got {channels:?}"
    );
    assert!(
        channels.iter().any(|c| c == "hub:turn_finished"),
        "missing hub:turn_finished in {channels:?}"
    );
    // Optional: tool mid-turn from mock.
    let _tool = channels.iter().any(|c| c == CHANNEL_HUB_TOOL);

    // Text-mode regression: no mid-turn when streaming disabled + MOCK off.
    std::env::set_var("MOCK_CLI_STREAM", "0");
    std::env::set_var("MOCK_CLI_SLEEP_MS", "0");
    let gw_t = Arc::new(CliAgentGateway::new(cli, vec![], None));
    let hub_t = Hub::new(gw_t.clone());
    hub_t.spawn_turn_bridge(gw_t.subscribe_turns());
    let mut session_t = Session::new(hub_t.clone());
    let _ = session_t
        .on_text(r#"{"protocol_version":"1.0.0","kind":"bot_client"}"#)
        .await;
    let conn_t = session_t.connection_id.clone();
    let mut bus_t = hub_t.subscribe_outbound();
    let _ = rpc(
        &mut session_t,
        1,
        "bot.subscribe",
        json!({ "agentIds": ["agt_1"] }),
    )
    .await;
    let send_t = rpc(
        &mut session_t,
        2,
        "bot.command",
        json!({
            "agentId": "agt_1",
            "name": "sendPrompt",
            "args": { "agentId": "agt_1", "prompt": "text-mode-cs1" }
        }),
    )
    .await;
    let preview_t = result(&send_t)["preview"].as_str().unwrap().to_string();
    assert!(preview_t.contains("atlas-mock-reply"));
    let events_t = collect_events(&mut bus_t, &conn_t, Duration::from_secs(2), |evs| {
        evs.iter()
            .any(|e| e["params"]["channel"] == "hub:turn_finished")
    })
    .await;
    let mid_t = events_t.iter().any(|e| {
        let c = e["params"]["channel"].as_str().unwrap_or("");
        c == CHANNEL_HUB_ASSISTANT_DELTA || c == CHANNEL_HUB_TOOL
    });
    assert!(
        !mid_t,
        "text mode must not emit mid-turn deltas; got {events_t:?}"
    );
    assert!(
        events_t
            .iter()
            .any(|e| e["params"]["channel"] == "hub:turn_finished"),
        "text mode still needs turn_finished"
    );

    println!("SMOKE_OK cli_stream mid-turn delta+turn_finished+text-fallback");
}
