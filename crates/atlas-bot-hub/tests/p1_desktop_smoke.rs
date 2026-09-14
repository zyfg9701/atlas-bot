//! D1 / P1 desktop real-use smoke: probe-then-mint, honest stub degrade,
//! RFB Evidence when a display stack (or mock RFB) is available.
//!
//! cargo test -p atlas-bot-hub --test p1_desktop_smoke -- --nocapture --test-threads=1
//!
//! No-display CI stays green via degrade + mock-RFB paths. Real Xvfb+x11vnc
//! is optional (`SMOKE_OK p1-desktop real-path SKIP …` when binaries missing).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use atlas_bot_gateway::{
    probe_rfb_upstream, rfb_handshake_none, spawn_loopback_mock_rfb, AttachMode, BoxSidecarGateway,
    Gateway, GatewayConfig, InMemoryGateway, VncMode,
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

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn stack_script() -> PathBuf {
    repo_root().join("scripts/atlas-desktop-stack.sh")
}

fn has_display_binaries() -> bool {
    which("Xvfb") && which("x11vnc")
}

fn which(name: &str) -> bool {
    Command::new("which")
        .arg(name)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn forbidden_success_claim(html: &str) -> bool {
    // Positive success claims only. "not connected to a real desktop" is honest degrade.
    html.contains("已连接真桌面")
        || html.contains("already connected to a real desktop")
        || html.contains("already connected to real desktop")
}

#[tokio::test]
async fn p1_desktop_smoke() {
    // ── D-P4 / D-P9: proxy + dead upstream → stub, never claim live desktop ──
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let public_base = format!("http://{addr}");
    let degrade_cfg = GatewayConfig {
        turn_delay: Duration::from_millis(20),
        vnc_mode: VncMode::Proxy,
        vnc_upstream: Some("127.0.0.1:1".into()),
        vnc_public_base: public_base.clone(),
        attach_mode: AttachMode::Memory,
        ..GatewayConfig::default()
    };
    let gw_d = Arc::new(InMemoryGateway::with_config(degrade_cfg));
    let gw_http_d = gw_d.clone();
    tokio::spawn(async move {
        axum::serve(listener, atlas_bot_gateway::http_router(gw_http_d))
            .await
            .ok();
    });
    let hub_d = Hub::new(gw_d.clone());
    hub_d.spawn_turn_bridge(gw_d.subscribe_turns());
    let mut sess_d = Session::new(hub_d);
    let _ = sess_d
        .on_text(r#"{"protocol_version":"1.0.0","kind":"bot_client"}"#)
        .await;

    let before_cold = gw_d.invoke_count();
    let _ = rpc(&mut sess_d, 1, "bot.status", json!({})).await;
    let _ = rpc(&mut sess_d, 2, "bot.roster", json!({})).await;
    let _ = rpc(
        &mut sess_d,
        3,
        "bot.transcript.offbox",
        json!({ "agentId": "agt_1" }),
    )
    .await;
    assert_eq!(
        gw_d.invoke_count(),
        before_cold,
        "cold status/roster/offbox must not wake gateway"
    );

    let vnc_d = rpc(
        &mut sess_d,
        4,
        "bot.vncDescriptor",
        json!({ "agentId": "agt_1" }),
    )
    .await;
    let url_d = result(&vnc_d)["vncUrl"].as_str().unwrap().to_string();
    assert!(
        url_d.contains("/vnc-stub?agent=agt_1"),
        "unhealthy probe must degrade to stub, got {url_d}"
    );
    assert!(result(&vnc_d)["expiresHint"].as_i64().unwrap() > 0);
    let stub_html = reqwest::get(format!("http://{addr}/vnc-stub?agent=agt_1"))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(stub_html.contains("P5 VNC placeholder"), "{stub_html}");
    assert!(
        stub_html.contains("not connected to a real desktop")
            || stub_html.contains("未连接真桌面")
            || stub_html.contains("degrade-to-stub"),
        "{stub_html}"
    );
    assert!(
        !forbidden_success_claim(&stub_html),
        "stub must not claim live desktop: {stub_html}"
    );

    // D-P5: unknown token → 403/410 even in degrade world
    let bad = reqwest::get(format!("http://{addr}/vnc/tok_deadbeef/"))
        .await
        .unwrap();
    assert!(
        bad.status() == 403 || bad.status() == 410,
        "unexpected {}",
        bad.status()
    );
    println!("SMOKE_OK p1-desktop degrade-to-stub url={url_d}");

    let mid = gw_d.invoke_count();
    let _ = rpc(&mut sess_d, 5, "bot.status", json!({})).await;
    let _ = rpc(&mut sess_d, 6, "bot.roster", json!({})).await;
    assert_eq!(gw_d.invoke_count(), mid, "cold still cold after VNC mint");
    println!("SMOKE_OK p1-desktop cold-still-cold");

    // ── D-P1 / D-P2: proxy + healthy mock RFB → token URL (not stub narrative) ──
    let (_rfb_h, rfb_up) = spawn_loopback_mock_rfb().await.unwrap();
    let probe = probe_rfb_upstream(&rfb_up).await;
    assert!(probe.healthy, "mock RFB must probe: {probe:?}");
    let hs = rfb_handshake_none(&rfb_up).await.expect("handshake");
    assert!(hs.contains("RFB"), "{hs}");

    let listener2 = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr2 = listener2.local_addr().unwrap();
    let public2 = format!("http://{addr2}");
    let gw_ok = Arc::new(InMemoryGateway::with_config(GatewayConfig {
        turn_delay: Duration::from_millis(20),
        vnc_mode: VncMode::Proxy,
        vnc_upstream: Some(rfb_up.clone()),
        vnc_public_base: public2.clone(),
        ..GatewayConfig::default()
    }));
    let gw_http_ok = gw_ok.clone();
    tokio::spawn(async move {
        axum::serve(listener2, atlas_bot_gateway::http_router(gw_http_ok))
            .await
            .ok();
    });
    let hub_ok = Hub::new(gw_ok.clone());
    hub_ok.spawn_turn_bridge(gw_ok.subscribe_turns());
    let mut sess_ok = Session::new(hub_ok);
    let _ = sess_ok
        .on_text(r#"{"protocol_version":"1.0.0","kind":"bot_client"}"#)
        .await;
    let vnc_ok = rpc(
        &mut sess_ok,
        10,
        "bot.vncDescriptor",
        json!({ "agentId": "agt_1" }),
    )
    .await;
    let url_ok = result(&vnc_ok)["vncUrl"].as_str().unwrap().to_string();
    assert!(
        url_ok.contains("/vnc/tok_"),
        "healthy probe must mint token URL, got {url_ok}"
    );
    assert!(!url_ok.contains("vnc-stub"), "{url_ok}");
    let path_part = url_ok
        .rsplit("8787")
        .next()
        .and_then(|s| if s.starts_with("/vnc/") { Some(s) } else { None })
        .unwrap_or_else(|| {
            url_ok
                .split("://")
                .nth(1)
                .and_then(|s| s.find('/').map(|i| &s[i..]))
                .unwrap_or("/vnc/")
        });
    let local = format!("http://{addr2}{path_part}");
    let resp = reqwest::get(&local).await.unwrap();
    assert_eq!(resp.status(), 200);
    let html = resp.text().await.unwrap();
    assert!(
        html.contains("proxy-rfb") || html.contains("probed OK"),
        "expected real-path page not stub, got {html}"
    );
    assert!(!html.contains("P5 VNC placeholder"), "{html}");
    assert!(!forbidden_success_claim(&html), "{html}");
    assert!(html.contains(&rfb_up) || html.contains("RFB"), "{html}");
    let bad2 = reqwest::get(format!("http://{addr2}/vnc/tok_deadbeef/"))
        .await
        .unwrap();
    assert!(bad2.status() == 403 || bad2.status() == 410);
    println!(
        "SMOKE_OK p1-desktop proxy-rfb url={url_ok} upstream={rfb_up} handshake={hs}"
    );

    // ── D-P7: box mint + workspace alignment; degrade + healthy ──
    let box_root = std::env::temp_dir().join(format!(
        "atlas-p1-box-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&box_root);
    std::fs::create_dir_all(&box_root).unwrap();

    std::env::set_var("ATLAS_VNC_MODE", "proxy");
    std::env::set_var("ATLAS_VNC_UPSTREAM", "127.0.0.1:1");
    let gw_box_bad = Arc::new(BoxSidecarGateway::new(
        box_root.clone(),
        Duration::from_millis(15),
    ));
    let hub_box_bad = Hub::new(gw_box_bad.clone());
    let mut sess_bb = Session::new(hub_box_bad);
    let _ = sess_bb
        .on_text(r#"{"protocol_version":"1.0.0","kind":"bot_client"}"#)
        .await;
    let vnc_bb = rpc(
        &mut sess_bb,
        20,
        "bot.vncDescriptor",
        json!({ "agentId": "agt_1" }),
    )
    .await;
    let url_bb = result(&vnc_bb)["vncUrl"].as_str().unwrap().to_string();
    assert!(
        url_bb.contains("/vnc-stub?agent=agt_1"),
        "box unhealthy probe must stub: {url_bb}"
    );

    std::env::set_var("ATLAS_VNC_UPSTREAM", &rfb_up);
    let gw_box_ok = Arc::new(BoxSidecarGateway::new(
        box_root.clone(),
        Duration::from_millis(15),
    ));
    let hub_box_ok = Hub::new(gw_box_ok.clone());
    let mut sess_bo = Session::new(hub_box_ok);
    let _ = sess_bo
        .on_text(r#"{"protocol_version":"1.0.0","kind":"bot_client"}"#)
        .await;
    let vnc_bo = rpc(
        &mut sess_bo,
        21,
        "bot.vncDescriptor",
        json!({ "agentId": "agt_1" }),
    )
    .await;
    let url_bo = result(&vnc_bo)["vncUrl"].as_str().unwrap().to_string();
    assert!(
        url_bo.contains("/vnc/tok_"),
        "box healthy probe must mint token: {url_bo}"
    );

    let payload = b"p1-box-align";
    let b64 = base64::engine::general_purpose::STANDARD.encode(payload);
    let up = rpc(
        &mut sess_bo,
        22,
        "bot.command",
        json!({
            "agentId": "agt_1",
            "name": "uploadAttachment",
            "args": { "filename": "p1.bin", "bytesBase64": b64 }
        }),
    )
    .await;
    let up_path = result(&up)["path"].as_str().unwrap().to_string();
    assert!(
        up_path.contains("agt_1") && up_path.contains("uploads"),
        "box upload must land under workspace/<agentId>/uploads: {up_path}"
    );
    assert!(Path::new(&up_path).is_file(), "missing {up_path}");
    println!("SMOKE_OK p1-desktop box-vnc-align url={url_bo} path={up_path}");

    // ── D-P1 / D-P2 / D-P3: real Xvfb+x11vnc when binaries exist ──
    if has_display_binaries() {
        let rundir = std::env::temp_dir().join(format!(
            "atlas-p1-desk-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&rundir);
        std::fs::create_dir_all(&rundir).unwrap();
        let disp = 80 + (std::process::id() % 10) as u32;
        let script = stack_script();
        let start = Command::new("bash")
            .arg(&script)
            .arg("start")
            .env("ATLAS_DESKTOP_RUNDIR", &rundir)
            .env("ATLAS_DESKTOP_DISPLAY_NUM", disp.to_string())
            .env("ATLAS_BOX_WORKSPACE", &box_root)
            .env("ATLAS_DESKTOP_AGENT_ID", "agt_1")
            .output()
            .expect("start stack");
        if !start.status.success() {
            eprintln!(
                "display stack start failed:\n{}",
                String::from_utf8_lossy(&start.stderr)
            );
            eprintln!("{}", String::from_utf8_lossy(&start.stdout));
            let _ = Command::new("bash")
                .arg(&script)
                .arg("cleanup")
                .env("ATLAS_DESKTOP_RUNDIR", &rundir)
                .env("ATLAS_DESKTOP_DISPLAY_NUM", disp.to_string())
                .status();
            panic!("Xvfb+x11vnc present but start failed (see logs)");
        }
        let env_txt = std::fs::read_to_string(rundir.join("env")).expect("env file");
        let mut real_up = String::new();
        for line in env_txt.lines() {
            if let Some(v) = line.strip_prefix("ATLAS_VNC_UPSTREAM=") {
                real_up = v.trim().to_string();
            }
        }
        assert!(!real_up.is_empty(), "missing ATLAS_VNC_UPSTREAM in {env_txt}");
        let p_real = probe_rfb_upstream(&real_up).await;
        assert!(p_real.healthy, "real stack probe: {p_real:?}");
        let hs_real = rfb_handshake_none(&real_up)
            .await
            .expect("real RFB handshake");
        let gw_real = Arc::new(InMemoryGateway::with_config(GatewayConfig {
            turn_delay: Duration::from_millis(20),
            vnc_mode: VncMode::Proxy,
            vnc_upstream: Some(real_up.clone()),
            vnc_public_base: "http://127.0.0.1:8787".into(),
            ..GatewayConfig::default()
        }));
        let hub_real = Hub::new(gw_real.clone());
        let mut sess_r = Session::new(hub_real);
        let _ = sess_r
            .on_text(r#"{"protocol_version":"1.0.0","kind":"bot_client"}"#)
            .await;
        let vnc_r = rpc(
            &mut sess_r,
            30,
            "bot.vncDescriptor",
            json!({ "agentId": "agt_1" }),
        )
        .await;
        let url_r = result(&vnc_r)["vncUrl"].as_str().unwrap().to_string();
        assert!(url_r.contains("/vnc/tok_"), "real mint {url_r}");

        // D-P3 key/mouse automation equivalent via X server (product path is VNC client).
        let mut key_note = "xdotool-skip";
        if which("xdotool") {
            let _ = Command::new("xdotool")
                .args(["mousemove", "40", "40"])
                .env("DISPLAY", format!(":{disp}"))
                .status();
            let _ = Command::new("xdotool")
                .args(["key", "a"])
                .env("DISPLAY", format!(":{disp}"))
                .status();
            key_note = "xdotool-mousemove+key";
        }
        let shot = Command::new("bash")
            .arg(&script)
            .arg("screenshot")
            .env("ATLAS_DESKTOP_RUNDIR", &rundir)
            .env("ATLAS_DESKTOP_DISPLAY_NUM", disp.to_string())
            .output()
            .expect("screenshot");
        let shot_txt = format!(
            "{}{}",
            String::from_utf8_lossy(&shot.stdout),
            String::from_utf8_lossy(&shot.stderr)
        );
        let _ = Command::new("bash")
            .arg(&script)
            .arg("cleanup")
            .env("ATLAS_DESKTOP_RUNDIR", &rundir)
            .env("ATLAS_DESKTOP_DISPLAY_NUM", disp.to_string())
            .status();
        println!(
            "SMOKE_OK p1-desktop real-xvfb-x11vnc url={url_r} upstream={real_up} handshake={hs_real} input={key_note} shot={shot_txt}"
        );
    } else {
        println!(
            "SMOKE_OK p1-desktop real-path SKIP no Xvfb/x11vnc (CI degrade-green; RFB Evidence via mock)"
        );
    }

    let _ = std::fs::remove_dir_all(&box_root);
    // Restore env so leftover process tests (if any) stay stub.
    std::env::remove_var("ATLAS_VNC_MODE");
    std::env::remove_var("ATLAS_VNC_UPSTREAM");
}
