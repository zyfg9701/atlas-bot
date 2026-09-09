//! B1 split-process event ingest smoke: Hub HTTP gateway + loopback RuntimeHint ingest.
//!
//! cargo test -p atlas-bot-hub --test b1_smoke -- --nocapture --test-threads=1

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use atlas_bot_gateway::{
    serve_gateway_http, BoxSidecarGateway, HttpGatewayClient, CHANNEL_HUB_ASSISTANT_DELTA,
    CHANNEL_HUB_TOOL,
};
use atlas_bot_hub::{serve_event_ingest, EventIngestConfig, Hub, Session};
use serde_json::{json, Value};

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

async fn free_loopback_port() -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral");
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    port
}

#[tokio::test]
async fn b1_event_ingest_smoke() {
    let root = std::env::temp_dir().join(format!("atlas-b1-smoke-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();

    let gw_port = free_loopback_port().await;
    let ingest_port = free_loopback_port().await;
    let gw_addr: SocketAddr = format!("127.0.0.1:{gw_port}").parse().unwrap();
    let ingest_addr: SocketAddr = format!("127.0.0.1:{ingest_port}").parse().unwrap();
    let event_url = format!("http://127.0.0.1:{ingest_port}/internal/runtime-hint");
    let token = "b1-smoke-token".to_string();

    // Standalone box gateway with B1 EVENT_URL (no in-process Hub bridge).
    let box_gw = Arc::new(BoxSidecarGateway::new_with_event(
        PathBuf::from(&root),
        Duration::from_millis(40),
        Some(event_url.clone()),
        Some(token.clone()),
    ));
    let box_for_http: Arc<dyn atlas_bot_gateway::Gateway> = box_gw.clone();
    tokio::spawn(async move {
        if let Err(e) = serve_gateway_http(box_for_http, gw_addr).await {
            eprintln!("gateway http exited: {e}");
        }
    });

    // Hub uses HTTP client only (B1 path) — do NOT spawn_turn_bridge.
    let http_client = Arc::new(HttpGatewayClient::new(format!("http://127.0.0.1:{gw_port}")));
    let hub = Hub::new(http_client);
    let ingest_cfg = EventIngestConfig::for_test(ingest_addr, Some(token.clone())).unwrap();
    let hub_ingest = hub.clone();
    tokio::spawn(async move {
        if let Err(e) = serve_event_ingest(hub_ingest, ingest_cfg).await {
            eprintln!("ingest exited: {e}");
        }
    });

    // Wait for listeners.
    for _ in 0..50 {
        let ok_gw = reqwest::Client::new()
            .get(format!("http://127.0.0.1:{gw_port}/healthz"))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false);
        let ok_ing = reqwest::Client::new()
            .get(format!("http://127.0.0.1:{ingest_port}/healthz"))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false);
        if ok_gw && ok_ing {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

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

    let streamed = rpc(
        &mut session,
        2,
        "bot.command",
        json!({
            "agentId": "agt_1",
            "name": "sendPrompt",
            "args": {
                "agentId": "agt_1",
                "prompt": "LIST_DIR please stream via B1"
            }
        }),
    )
    .await;
    assert_eq!(result(&streamed)["accepted"], true);
    assert_eq!(result(&streamed)["completed"], true);

    let events = collect_events(&mut bus, &conn, Duration::from_secs(5), |evs| {
        let has_finished = evs
            .iter()
            .any(|e| e["params"]["channel"] == "hub:turn_finished");
        let has_mid = evs.iter().any(|e| {
            let c = e["params"]["channel"].as_str().unwrap_or("");
            c == CHANNEL_HUB_TOOL || c == CHANNEL_HUB_ASSISTANT_DELTA
        });
        has_finished && has_mid
    })
    .await;

    let channels: Vec<String> = events
        .iter()
        .filter_map(|e| e["params"]["channel"].as_str().map(|s| s.to_string()))
        .collect();
    assert!(
        channels.iter().any(|c| c == "hub:turn_finished"),
        "missing turn_finished in {channels:?}"
    );
    let mid = channels
        .iter()
        .any(|c| c == CHANNEL_HUB_TOOL || c == CHANNEL_HUB_ASSISTANT_DELTA);
    assert!(
        mid,
        "expected ≥1 mid-turn event via B1 ingest ({CHANNEL_HUB_TOOL}/{CHANNEL_HUB_ASSISTANT_DELTA}); got {channels:?}"
    );

    // Optional negative (document): without EVENT_URL, mid-turn is absent across process.
    // Covered by runbook; not failing this smoke.

    println!("SMOKE_OK b1 event-ingest mid-turn+turn_finished…");
    let _ = std::fs::remove_dir_all(&root);
}
