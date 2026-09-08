//! R2 runtime deepen smoke: tools + streaming events + box VNC/attach
//!
//! cargo test -p atlas-bot-hub --test runtime_deepen_smoke -- --nocapture --test-threads=1

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use atlas_bot_gateway::{
    BoxSidecarGateway, CHANNEL_HUB_ASSISTANT_DELTA, CHANNEL_HUB_TOOL,
};
use atlas_bot_hub::{Hub, Session};
use base64::Engine;
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

fn error(resp: &Value) -> &Value {
    resp.get("error")
        .unwrap_or_else(|| panic!("expected error, got {resp}"))
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
async fn runtime_deepen_smoke() {
    let root = std::env::temp_dir().join(format!(
        "atlas-runtime-deepen-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();

    let gw = Arc::new(BoxSidecarGateway::new(
        PathBuf::from(&root),
        Duration::from_millis(15),
    ));
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

    // ── R2.1 tools: WRITE + READ + path escape + unknown shell ──────────
    let write = rpc(
        &mut session,
        2,
        "bot.command",
        json!({
            "agentId": "agt_1",
            "name": "sendPrompt",
            "args": {
                "agentId": "agt_1",
                "prompt": "WRITE_FILE deepen.txt <<< deepen-payload-42"
            }
        }),
    )
    .await;
    let pw = result(&write)["preview"].as_str().unwrap().to_string();
    assert!(pw.contains("[tool:write_file"), "write evidence missing: {pw}");
    assert!(
        root.join("agt_1/deepen.txt").is_file(),
        "file should land under workspace"
    );

    let read = rpc(
        &mut session,
        3,
        "bot.command",
        json!({
            "agentId": "agt_1",
            "name": "sendPrompt",
            "args": {
                "agentId": "agt_1",
                "prompt": "READ_FILE deepen.txt and LIST_DIR"
            }
        }),
    )
    .await;
    let pr = result(&read)["preview"].as_str().unwrap().to_string();
    assert!(pr.contains("deepen-payload-42"), "read body missing: {pr}");
    assert!(pr.contains("[tool:list_dir"), "{pr}");
    assert!(pr.contains("[tool:read_file"), "{pr}");

    let escape = rpc(
        &mut session,
        4,
        "bot.command",
        json!({
            "agentId": "agt_1",
            "name": "sendPrompt",
            "args": {
                "agentId": "agt_1",
                "prompt": "READ_FILE ../secret"
            }
        }),
    )
    .await;
    let pe = result(&escape)["preview"].as_str().unwrap().to_string();
    assert!(
        pe.contains("path escape") || pe.contains("..") || pe.contains("rejected"),
        "expected path escape reject: {pe}"
    );

    let bad_shell = rpc(
        &mut session,
        5,
        "bot.command",
        json!({
            "agentId": "agt_1",
            "name": "sendPrompt",
            "args": {
                "agentId": "agt_1",
                "prompt": "RUN curl http://example.invalid"
            }
        }),
    )
    .await;
    let ps = result(&bad_shell)["preview"].as_str().unwrap().to_string();
    assert!(
        ps.contains("command not on whitelist: curl"),
        "expected stable whitelist reject: {ps}"
    );

    println!("SMOKE_OK runtime-deepen tools…");

    // ── R2.2 streaming bot.event (tool and/or delta + turn_finished) ────
    // Drain prior events from tool turns.
    let _ = collect_events(&mut bus, &conn, Duration::from_millis(300), |_| false).await;

    let streamed = rpc(
        &mut session,
        6,
        "bot.command",
        json!({
            "agentId": "agt_1",
            "name": "sendPrompt",
            "args": {
                "agentId": "agt_1",
                "prompt": "LIST_DIR please stream me"
            }
        }),
    )
    .await;
    assert_eq!(result(&streamed)["accepted"], true);
    assert_eq!(result(&streamed)["completed"], true);

    let events = collect_events(&mut bus, &conn, Duration::from_secs(3), |evs| {
        evs.iter()
            .any(|e| e["params"]["channel"] == "hub:turn_finished")
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
        "expected ≥1 non-finished event ({CHANNEL_HUB_TOOL}/{CHANNEL_HUB_ASSISTANT_DELTA}); got {channels:?}"
    );
    println!("SMOKE_OK runtime-deepen events…");

    // Unsubscribed client still OK (sync result only).
    let mut session2 = Session::new(hub.clone());
    let _ = session2
        .on_text(r#"{"protocol_version":"1.0.0","kind":"bot_client"}"#)
        .await;
    let nosub = rpc(
        &mut session2,
        7,
        "bot.command",
        json!({
            "agentId": "agt_1",
            "name": "sendPrompt",
            "args": { "agentId": "agt_1", "prompt": "nosub-ok" }
        }),
    )
    .await;
    assert_eq!(result(&nosub)["accepted"], true);

    // ── R2.3 box VNC + upload/attach under workspace ────────────────────
    let vnc = rpc(
        &mut session,
        8,
        "bot.vncDescriptor",
        json!({ "agentId": "agt_1" }),
    )
    .await;
    let vnc_res = result(&vnc);
    let vnc_url = vnc_res["vncUrl"].as_str().unwrap().to_string();
    assert!(
        vnc_url.contains("/vnc-stub?agent=agt_1"),
        "box without upstream must mint stub (not fake desktop): {vnc_url}"
    );
    assert!(vnc_res["expiresHint"].as_i64().unwrap() > 0);

    let payload = b"box-attach-bytes";
    let b64 = base64::engine::general_purpose::STANDARD.encode(payload);
    let up = rpc(
        &mut session,
        9,
        "bot.command",
        json!({
            "agentId": "agt_1",
            "name": "uploadAttachment",
            "args": {
                "filename": "shot.bin",
                "bytesBase64": b64
            }
        }),
    )
    .await;
    let up_res = result(&up);
    let upload_id = up_res["uploadId"].as_str().unwrap().to_string();
    let up_path = up_res["path"].as_str().unwrap().to_string();
    assert!(
        up_path.contains("agt_1") && up_path.contains("uploads"),
        "upload must land under workspace/<agentId>/uploads: {up_path}"
    );
    assert!(
        PathBuf::from(&up_path).is_file(),
        "upload file missing: {up_path}"
    );
    let meta = PathBuf::from(&up_path)
        .parent()
        .unwrap()
        .join("meta.json");
    assert!(meta.is_file(), "meta.json missing beside upload");

    let attached = rpc(
        &mut session,
        10,
        "bot.command",
        json!({
            "agentId": "agt_1",
            "name": "attachUpload",
            "args": { "uploadId": upload_id }
        }),
    )
    .await;
    assert_eq!(result(&attached)["filename"], "shot.bin");

    let missing = rpc(
        &mut session,
        11,
        "bot.command",
        json!({
            "agentId": "agt_1",
            "name": "attachUpload",
            "args": { "uploadId": "upl_does_not_exist" }
        }),
    )
    .await;
    let err = error(&missing);
    assert_eq!(err["message"], "command_rejected");
    assert_eq!(err["data"]["reason"], "attachment_not_found");

    println!("SMOKE_OK runtime-deepen box-vnc-attach…");

    let _ = std::fs::remove_dir_all(&root);
}
