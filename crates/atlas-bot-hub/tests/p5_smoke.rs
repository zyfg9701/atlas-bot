//! P5 smoke: bot.vncDescriptor + uploadAttachment (+ attachUpload) → SMOKE_OK
//!
//! Run: `cargo test -p atlas-bot-hub --test p5_smoke -- --nocapture`

use std::sync::Arc;
use std::time::Duration;

use atlas_bot_gateway::{Gateway, InMemoryGateway, UPLOAD_ARGS_JSON_MAX_BYTES};
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

fn error(resp: &Value) -> &Value {
    resp.get("error")
        .unwrap_or_else(|| panic!("expected error, got {resp}"))
}

#[tokio::test]
async fn p5_smoke_vnc_upload_attach() {
    let gw = Arc::new(InMemoryGateway::with_turn_delay(Duration::from_millis(50)));
    let hub = Hub::new(gw.clone());
    hub.spawn_turn_bridge(gw.subscribe_turns());

    // Serve stub VNC page so a minted-style URL is reachable in-process.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let gw_http = gw.clone();
    tokio::spawn(async move {
        axum::serve(listener, atlas_bot_gateway::http_router(gw_http))
            .await
            .ok();
    });

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
        .any(|c| c == "bot.vncDescriptor"));

    // Cold paths must not wake gateway (prep for VNC/upload UI).
    let before_cold = gw.invoke_count();
    let _ = rpc(&mut session, 1, "bot.status", json!({})).await;
    let _ = rpc(&mut session, 2, "bot.roster", json!({})).await;
    let _ = rpc(
        &mut session,
        3,
        "bot.transcript.offbox",
        json!({ "agentId": "agt_1" }),
    )
    .await;
    assert_eq!(
        gw.invoke_count(),
        before_cold,
        "cold status/roster/offbox must not wake gateway"
    );

    // Hot: bot.vncDescriptor (Hub method, not a command name)
    let vnc = rpc(
        &mut session,
        4,
        "bot.vncDescriptor",
        json!({ "agentId": "agt_1" }),
    )
    .await;
    let vnc_res = result(&vnc);
    let vnc_url = vnc_res["vncUrl"].as_str().unwrap().to_string();
    assert!(
        vnc_url.contains("/vnc-stub?agent=agt_1"),
        "unexpected vncUrl {vnc_url}"
    );
    assert!(vnc_res["expiresHint"].as_i64().unwrap() > 0);
    assert!(gw.invoke_count() > before_cold);

    // Reachability via local stub router (same HTML the :8787 URL would serve).
    let local_url = format!("http://{addr}/vnc-stub?agent=agt_1");
    let html = reqwest::get(&local_url).await.unwrap().text().await.unwrap();
    assert!(html.contains("P5 VNC placeholder"), "{html}");
    assert!(html.contains("agt_1"), "{html}");

    // Hot: uploadAttachment success
    let b64 = "aGVsbG8gcDUgc21va2U="; // "hello p5 smoke"
    let up = rpc(
        &mut session,
        5,
        "bot.command",
        json!({
            "agentId": "agt_1",
            "name": "uploadAttachment",
            "args": { "bytesBase64": b64, "filename": "smoke.txt", "agentId": "agt_1" }
        }),
    )
    .await;
    let up_res = result(&up);
    let path = up_res["path"].as_str().unwrap().to_string();
    assert!(!path.is_empty(), "upload path empty");
    assert!(std::path::Path::new(&path).is_file(), "missing file {path}");
    let upload_id = up_res["uploadId"].as_str().unwrap().to_string();

    // attachUpload success + closed-set failure
    let att = rpc(
        &mut session,
        6,
        "bot.command",
        json!({
            "agentId": "agt_1",
            "name": "attachUpload",
            "args": { "uploadId": upload_id, "agentId": "agt_1" }
        }),
    )
    .await;
    assert_eq!(result(&att)["path"], path);

    let miss = rpc(
        &mut session,
        7,
        "bot.command",
        json!({
            "agentId": "agt_1",
            "name": "attachUpload",
            "args": { "uploadId": "upl_does_not_exist" }
        }),
    )
    .await;
    let err = error(&miss);
    assert_eq!(err["message"], "command_rejected");
    assert_eq!(err["data"]["reason"], "attachment_not_found");

    // args_too_large closed-set (JSON > 3 MiB)
    let big = "A".repeat(UPLOAD_ARGS_JSON_MAX_BYTES + 128);
    let too_big = rpc(
        &mut session,
        8,
        "bot.command",
        json!({
            "agentId": "agt_1",
            "name": "uploadAttachment",
            "args": { "bytesBase64": big, "filename": "huge.bin" }
        }),
    )
    .await;
    let err2 = error(&too_big);
    assert_eq!(err2["message"], "command_rejected");
    assert_eq!(err2["data"]["reason"], "args_too_large");

    // Cold still cold after hot work accounted.
    let mid = gw.invoke_count();
    let _ = rpc(&mut session, 9, "bot.status", json!({})).await;
    let _ = rpc(&mut session, 10, "bot.roster", json!({})).await;
    assert_eq!(gw.invoke_count(), mid);

    println!(
        "SMOKE_OK p5 vnc+upload vncUrl={vnc_url} path={path} attachOk+args_too_large+attachment_not_found"
    );
}
