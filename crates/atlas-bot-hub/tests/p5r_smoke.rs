//! P5 real smoke: disk attachments + VNC proxy (mock) → SMOKE_OK p5r…
//!
//! Run: `cargo test -p atlas-bot-hub --test p5r_smoke -- --nocapture`
//!
//! Defaults for CI remain stub+memory via `p5_smoke`; this test drives
//! disk / proxy explicitly through [`GatewayConfig`].

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use atlas_bot_gateway::{
    AttachMode, GatewayConfig, InMemoryGateway, VncMode, UPLOAD_ARGS_JSON_MAX_BYTES,
};
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
async fn p5r_smoke_attach_disk_and_vnc_proxy() {
    let attach_root = std::env::temp_dir().join(format!(
        "atlas-p5r-smoke-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&attach_root);
    std::fs::create_dir_all(&attach_root).unwrap();

    // --- disk attach ---
    let disk_cfg = GatewayConfig {
        turn_delay: Duration::from_millis(50),
        attach_mode: AttachMode::Disk,
        attach_root: attach_root.clone(),
        attach_ttl: Duration::from_secs(24 * 3600),
        vnc_mode: VncMode::Stub,
        ..GatewayConfig::default()
    };
    let gw_disk = Arc::new(InMemoryGateway::with_config(disk_cfg.clone()));
    let hub_disk = Hub::new(gw_disk.clone());
    hub_disk.spawn_turn_bridge(gw_disk.subscribe_turns());
    let mut session = Session::new(hub_disk);
    let hello = session
        .on_text(r#"{"protocol_version":"1.0.0","kind":"bot_client"}"#)
        .await;
    assert_eq!(hello.len(), 1);

    let b64 = "cDVyIGRpc2sgc21va2U="; // "p5r disk smoke"
    let up = rpc(
        &mut session,
        1,
        "bot.command",
        json!({
            "agentId": "agt_1",
            "name": "uploadAttachment",
            "args": { "bytesBase64": b64, "filename": "p5r-disk.txt", "agentId": "agt_1" }
        }),
    )
    .await;
    let up_res = result(&up);
    let path = up_res["path"].as_str().unwrap().to_string();
    let upload_id = up_res["uploadId"].as_str().unwrap().to_string();
    assert!(
        path.starts_with(attach_root.to_string_lossy().as_ref()),
        "path not under ATTACH_ROOT: {path} root={}",
        attach_root.display()
    );
    assert!(Path::new(&path).is_file(), "missing disk file {path}");
    let meta = attach_root.join(&upload_id).join("meta.json");
    assert!(meta.is_file(), "missing meta.json at {}", meta.display());
    let meta_txt = std::fs::read_to_string(&meta).unwrap();
    assert!(meta_txt.contains("\"state\": \"ready\"") || meta_txt.contains("\"state\":\"ready\""));

    let att = rpc(
        &mut session,
        2,
        "bot.command",
        json!({
            "agentId": "agt_1",
            "name": "attachUpload",
            "args": { "uploadId": upload_id, "agentId": "agt_1" }
        }),
    )
    .await;
    assert_eq!(result(&att)["path"], path);

    // Restart gateway on same root → still attachable.
    let gw_disk2 = Arc::new(InMemoryGateway::with_config(disk_cfg));
    let hub2 = Hub::new(gw_disk2.clone());
    let mut session2 = Session::new(hub2);
    let _ = session2
        .on_text(r#"{"protocol_version":"1.0.0","kind":"bot_client"}"#)
        .await;
    let att2 = rpc(
        &mut session2,
        3,
        "bot.command",
        json!({
            "agentId": "agt_1",
            "name": "attachUpload",
            "args": { "uploadId": upload_id }
        }),
    )
    .await;
    assert_eq!(result(&att2)["path"], path);

    // Closed-set: missing id
    let miss = rpc(
        &mut session2,
        4,
        "bot.command",
        json!({
            "agentId": "agt_1",
            "name": "attachUpload",
            "args": { "uploadId": "upl_missing_p5r" }
        }),
    )
    .await;
    assert_eq!(error(&miss)["data"]["reason"], "attachment_not_found");

    println!(
        "SMOKE_OK p5r attach-disk path={path} root={} persistOk+attachment_not_found",
        attach_root.display()
    );

    // --- VNC proxy with mock upstream ---
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let public_base = format!("http://{addr}");
    let proxy_cfg = GatewayConfig {
        turn_delay: Duration::from_millis(50),
        vnc_mode: VncMode::Proxy,
        vnc_upstream: Some("127.0.0.1:5900".into()),
        vnc_public_base: public_base.clone(),
        attach_mode: AttachMode::Memory,
        ..GatewayConfig::default()
    };
    let gw_proxy = Arc::new(InMemoryGateway::with_config(proxy_cfg));
    let gw_http = gw_proxy.clone();
    tokio::spawn(async move {
        axum::serve(listener, atlas_bot_gateway::http_router(gw_http))
            .await
            .ok();
    });

    let hub_proxy = Hub::new(gw_proxy.clone());
    hub_proxy.spawn_turn_bridge(gw_proxy.subscribe_turns());
    let mut sess_p = Session::new(hub_proxy);
    let _ = sess_p
        .on_text(r#"{"protocol_version":"1.0.0","kind":"bot_client"}"#)
        .await;

    let vnc = rpc(
        &mut sess_p,
        10,
        "bot.vncDescriptor",
        json!({ "agentId": "agt_1" }),
    )
    .await;
    let vnc_res = result(&vnc);
    let vnc_url = vnc_res["vncUrl"].as_str().unwrap().to_string();
    assert!(
        vnc_url.contains("/vnc/tok_"),
        "expected token URL, got {vnc_url}"
    );
    assert!(vnc_res["expiresHint"].as_i64().unwrap() > 0);

    // Reachability: minted path on the local listener (rewrite host to bound addr).
    let path_part = vnc_url
        .split(addr.to_string().as_str())
        .nth(1)
        .unwrap_or_else(|| vnc_url.strip_prefix(public_base.as_str()).unwrap_or("/"));
    let local = format!("http://{addr}{path_part}");
    let resp = reqwest::get(&local).await.unwrap();
    assert_eq!(resp.status(), 200);
    let html = resp.text().await.unwrap();
    assert!(html.contains("proxy-mock") || html.contains("P5 VNC proxy"), "{html}");
    assert!(html.contains("127.0.0.1:5900"), "{html}");

    // Expired / unknown token → 403/410
    let bad = reqwest::get(format!("http://{addr}/vnc/tok_deadbeef/"))
        .await
        .unwrap();
    assert!(
        bad.status() == 403 || bad.status() == 410,
        "unexpected {}",
        bad.status()
    );

    println!("SMOKE_OK p5r vnc-proxy url={vnc_url} mock-upstream=127.0.0.1:5900");

    // --- proxy without upstream → stub degrade ---
    let degrade_cfg = GatewayConfig {
        turn_delay: Duration::from_millis(20),
        vnc_mode: VncMode::Proxy,
        vnc_upstream: None,
        vnc_public_base: public_base,
        ..GatewayConfig::default()
    };
    let gw_d = Arc::new(InMemoryGateway::with_config(degrade_cfg));
    let hub_d = Hub::new(gw_d.clone());
    let mut sess_d = Session::new(hub_d);
    let _ = sess_d
        .on_text(r#"{"protocol_version":"1.0.0","kind":"bot_client"}"#)
        .await;
    let vnc_d = rpc(
        &mut sess_d,
        11,
        "bot.vncDescriptor",
        json!({ "agentId": "agt_1" }),
    )
    .await;
    let url_d = result(&vnc_d)["vncUrl"].as_str().unwrap().to_string();
    assert!(
        url_d.contains("/vnc-stub?agent=agt_1"),
        "proxy without upstream must degrade to stub, got {url_d}"
    );
    println!("SMOKE_OK p5r vnc-proxy degrade-to-stub url={url_d}");

    // args_too_large still closed-set under disk path (reuse session2 / fresh)
    let big = "A".repeat(UPLOAD_ARGS_JSON_MAX_BYTES + 64);
    let too = rpc(
        &mut session2,
        5,
        "bot.command",
        json!({
            "agentId": "agt_1",
            "name": "uploadAttachment",
            "args": { "bytesBase64": big, "filename": "huge.bin" }
        }),
    )
    .await;
    assert_eq!(error(&too)["data"]["reason"], "args_too_large");

    let _ = std::fs::remove_dir_all(&attach_root);
}

#[tokio::test]
async fn p5r_smoke_disk_ttl_sweep() {
    let attach_root = std::env::temp_dir().join(format!(
        "atlas-p5r-ttl-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&attach_root);
    std::fs::create_dir_all(&attach_root).unwrap();
    let cfg = GatewayConfig {
        turn_delay: Duration::from_millis(20),
        attach_mode: AttachMode::Disk,
        attach_root: attach_root.clone(),
        attach_ttl: Duration::from_millis(40),
        ..GatewayConfig::default()
    };
    let gw = Arc::new(InMemoryGateway::with_config(cfg));
    let hub = Hub::new(gw.clone());
    let mut session = Session::new(hub);
    let _ = session
        .on_text(r#"{"protocol_version":"1.0.0","kind":"bot_client"}"#)
        .await;
    let b64 = "dHRs";
    let up = rpc(
        &mut session,
        1,
        "bot.command",
        json!({
            "agentId": "agt_1",
            "name": "uploadAttachment",
            "args": { "bytesBase64": b64, "filename": "ttl.txt" }
        }),
    )
    .await;
    let upload_id = result(&up)["uploadId"].as_str().unwrap().to_string();
    tokio::time::sleep(Duration::from_millis(80)).await;
    let miss = rpc(
        &mut session,
        2,
        "bot.command",
        json!({
            "agentId": "agt_1",
            "name": "attachUpload",
            "args": { "uploadId": upload_id }
        }),
    )
    .await;
    assert_eq!(error(&miss)["data"]["reason"], "attachment_not_found");
    println!("SMOKE_OK p5r attach-disk ttl→attachment_not_found");
    let _ = std::fs::remove_dir_all(&attach_root);
}
