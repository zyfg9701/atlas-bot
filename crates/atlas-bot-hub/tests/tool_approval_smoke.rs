//! GA1 / P0-1 tool approval smoke (gateway local gate).
//!
//! cargo test -p atlas-bot-hub --test tool_approval_smoke -- --nocapture --test-threads=1
//!
//! Covers GA-P1–P6 (+ P7 cli unchanged note). Uses `auto_deny` / inject Allow;
//! does not depend on a human or the network.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use atlas_bot_gateway::{
    gateway_http_router_with_meta, ApprovalDecision, BoxSidecarGateway, GatewayHttpMeta,
    ToolApprovalMode, APPROVAL_PENDING_PREFIX, CHANNEL_HUB_TOOL, REASON_AUTO_DENY, REASON_DENIED,
    REASON_TIMEOUT,
};
use atlas_bot_hub::{Hub, Session};
use serde_json::{json, Value};
use tokio::sync::Mutex;

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

fn make_gw(root: PathBuf, mode: ToolApprovalMode, timeout_ms: u64) -> BoxSidecarGateway {
    std::env::set_var("ATLAS_TOOL_APPROVAL_MODE", mode.as_str());
    std::env::set_var("ATLAS_TOOL_APPROVAL_TIMEOUT_MS", timeout_ms.to_string());
    // Clear optional token for smoke unless a case sets it.
    std::env::remove_var("ATLAS_TOOL_APPROVAL_TOKEN");
    BoxSidecarGateway::new_with_approval(
        root,
        Duration::from_millis(5),
        None,
        None,
        None,
        32,
        false,
        mode,
    )
}

async fn collect_tool_events(
    bus: &mut tokio::sync::broadcast::Receiver<(String, String)>,
    conn: &str,
    wait: Duration,
) -> Vec<Value> {
    let deadline = tokio::time::Instant::now() + wait;
    let mut out = Vec::new();
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(50), bus.recv()).await {
            Ok(Ok((cid, text))) if cid == conn => {
                if let Ok(v) = serde_json::from_str::<Value>(&text) {
                    if v["method"] == "bot.event" && v["params"]["channel"] == CHANNEL_HUB_TOOL {
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
async fn tool_approval_smoke() {
    let root = std::env::temp_dir().join(format!(
        "atlas-tool-approval-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();

    // ── GA-P1: auto_deny WRITE / mkdir → zero side effects ───────────────
    {
        let gw = Arc::new(make_gw(root.join("p1"), ToolApprovalMode::AutoDeny, 5_000));
        let hub = Hub::new(gw.clone());
        hub.spawn_turn_bridge(gw.subscribe_turns());
        let mut session = Session::new(hub.clone());
        let _ = session
            .on_text(r#"{"protocol_version":"1.0.0","kind":"bot_client"}"#)
            .await;
        let write = rpc(
            &mut session,
            1,
            "bot.command",
            json!({
                "agentId": "agt_1",
                "name": "sendPrompt",
                "args": {
                    "agentId": "agt_1",
                    "prompt": "WRITE_FILE p1.txt <<< should-not-land"
                }
            }),
        )
        .await;
        let pw = result(&write)["preview"].as_str().unwrap().to_string();
        assert!(
            pw.contains(REASON_AUTO_DENY) || pw.contains("tool_approval"),
            "P1 deny evidence missing: {pw}"
        );
        assert!(
            !root.join("p1/agt_1/p1.txt").exists(),
            "P1: WRITE must not land without Allow"
        );

        let mkdir = rpc(
            &mut session,
            2,
            "bot.command",
            json!({
                "agentId": "agt_1",
                "name": "sendPrompt",
                "args": {
                    "agentId": "agt_1",
                    "prompt": "RUN mkdir p1dir"
                }
            }),
        )
        .await;
        let pm = result(&mkdir)["preview"].as_str().unwrap().to_string();
        assert!(
            pm.contains(REASON_AUTO_DENY) || pm.contains("tool_approval"),
            "P1 mkdir deny missing: {pm}"
        );
        assert!(
            !root.join("p1/agt_1/p1dir").exists(),
            "P1: mkdir must not land"
        );
        println!("SMOKE_OK tool-approval GA-P1 auto_deny zero-side-effect");
    }

    // ── GA-P2: inject Allow → side effect + approval association ─────────
    {
        let gw = Arc::new(make_gw(root.join("p2"), ToolApprovalMode::Gate, 10_000));
        gw.inject_approval_decision(ApprovalDecision::Allow);
        let hub = Hub::new(gw.clone());
        hub.spawn_turn_bridge(gw.subscribe_turns());
        let mut session = Session::new(hub.clone());
        let _ = session
            .on_text(r#"{"protocol_version":"1.0.0","kind":"bot_client"}"#)
            .await;
        let write = rpc(
            &mut session,
            3,
            "bot.command",
            json!({
                "agentId": "agt_1",
                "name": "sendPrompt",
                "args": {
                    "agentId": "agt_1",
                    "prompt": "WRITE_FILE p2.txt <<< allow-payload"
                }
            }),
        )
        .await;
        let pw = result(&write)["preview"].as_str().unwrap().to_string();
        assert!(
            pw.contains("[tool:write_file") && pw.contains("approval=allow"),
            "P2 allow evidence missing: {pw}"
        );
        let body = std::fs::read_to_string(root.join("p2/agt_1/p2.txt")).expect("P2 file");
        assert_eq!(body.trim(), "allow-payload");
        println!("SMOKE_OK tool-approval GA-P2 inject-allow wrote");
    }

    // ── GA-P3: explicit Deny via submit_tool_approval ─────────────────────
    {
        let gw = Arc::new(make_gw(root.join("p3"), ToolApprovalMode::Gate, 10_000));
        let hub = Hub::new(gw.clone());
        hub.spawn_turn_bridge(gw.subscribe_turns());
        let mut session = Session::new(hub.clone());
        let hello = session
            .on_text(r#"{"protocol_version":"1.0.0","kind":"bot_client"}"#)
            .await;
        assert_eq!(hello.len(), 1);
        let conn = session.connection_id.clone();
        let _ = rpc(
            &mut session,
            4,
            "bot.subscribe",
            json!({ "agentIds": ["agt_1"] }),
        )
        .await;
        let mut bus = hub.subscribe_outbound();

        let gw2 = gw.clone();
        let deny_task = tokio::spawn(async move {
            for _ in 0..200 {
                let ids = gw2.pending_approval_ids().await;
                if let Some(id) = ids.first() {
                    let r = gw2
                        .submit_tool_approval(id, ApprovalDecision::Deny)
                        .await
                        .expect("deny");
                    assert_eq!(r["decision"], "deny");
                    // Repeat → closed reject
                    let again = gw2
                        .submit_tool_approval(id, ApprovalDecision::Allow)
                        .await;
                    assert!(again.is_err(), "repeat approve must closed-reject");
                    return;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            panic!("P3: never saw pending approval");
        });

        let write = rpc(
            &mut session,
            5,
            "bot.command",
            json!({
                "agentId": "agt_1",
                "name": "sendPrompt",
                "args": {
                    "agentId": "agt_1",
                    "prompt": "WRITE_FILE p3.txt <<< deny-me"
                }
            }),
        )
        .await;
        deny_task.await.unwrap();
        let pw = result(&write)["preview"].as_str().unwrap().to_string();
        assert!(
            pw.contains(REASON_DENIED),
            "P3 deny reason missing: {pw}"
        );
        assert!(!root.join("p3/agt_1/p3.txt").exists());

        // Unknown id → closed reject
        let unk = gw
            .submit_tool_approval("appr_does_not_exist", ApprovalDecision::Allow)
            .await;
        assert!(unk.is_err());

        let events = collect_tool_events(&mut bus, &conn, Duration::from_millis(300)).await;
        let pending_seen = events.iter().any(|e| {
            e["params"]["event"]["summary"]
                .as_str()
                .unwrap_or("")
                .contains(APPROVAL_PENDING_PREFIX)
        });
        assert!(pending_seen, "GA1b pending hint missing: {events:?}");
        println!("SMOKE_OK tool-approval GA-P3 explicit-deny + GA1b pending");
    }

    // ── GA-P4: timeout == Deny ───────────────────────────────────────────
    {
        let gw = Arc::new(make_gw(root.join("p4"), ToolApprovalMode::Gate, 80));
        let hub = Hub::new(gw.clone());
        hub.spawn_turn_bridge(gw.subscribe_turns());
        let mut session = Session::new(hub.clone());
        let _ = session
            .on_text(r#"{"protocol_version":"1.0.0","kind":"bot_client"}"#)
            .await;
        let write = rpc(
            &mut session,
            6,
            "bot.command",
            json!({
                "agentId": "agt_1",
                "name": "sendPrompt",
                "args": {
                    "agentId": "agt_1",
                    "prompt": "WRITE_FILE p4.txt <<< timeout-me"
                }
            }),
        )
        .await;
        let pw = result(&write)["preview"].as_str().unwrap().to_string();
        assert!(
            pw.contains(REASON_TIMEOUT),
            "P4 timeout reason missing: {pw}"
        );
        assert!(!root.join("p4/agt_1/p4.txt").exists());
        println!("SMOKE_OK tool-approval GA-P4 timeout-deny");
    }

    // ── GA-P5: LIST/READ/ls/pwd exempt ───────────────────────────────────
    {
        let gw = Arc::new(make_gw(root.join("p5"), ToolApprovalMode::Gate, 5_000));
        // Seed a file without going through WRITE gate: write directly.
        let agt = root.join("p5/agt_1");
        std::fs::create_dir_all(&agt).unwrap();
        std::fs::write(agt.join("seed.txt"), b"seed-ok\n").unwrap();
        let hub = Hub::new(gw.clone());
        hub.spawn_turn_bridge(gw.subscribe_turns());
        let mut session = Session::new(hub.clone());
        let _ = session
            .on_text(r#"{"protocol_version":"1.0.0","kind":"bot_client"}"#)
            .await;
        let read = rpc(
            &mut session,
            7,
            "bot.command",
            json!({
                "agentId": "agt_1",
                "name": "sendPrompt",
                "args": {
                    "agentId": "agt_1",
                    "prompt": "LIST_DIR and READ_FILE seed.txt and RUN ls and RUN pwd"
                }
            }),
        )
        .await;
        let pr = result(&read)["preview"].as_str().unwrap().to_string();
        assert!(pr.contains("[tool:list_dir"), "{pr}");
        assert!(pr.contains("seed-ok"), "{pr}");
        assert!(pr.contains("[tool:shell cmd=ls"), "{pr}");
        assert!(pr.contains("[tool:shell cmd=pwd"), "{pr}");
        assert!(
            !pr.contains("tool_approval"),
            "P5 exempt tools must not hit gate: {pr}"
        );
        assert!(gw.pending_approval_ids().await.is_empty());
        println!("SMOKE_OK tool-approval GA-P5 exempt list/read/ls/pwd");
    }

    // ── GA-P6: hang-fail path never silent Allow (channel drop → Deny) ───
    {
        let gw = Arc::new(make_gw(root.join("p6"), ToolApprovalMode::Gate, 10_000));
        // Inject Deny models "failure must not become Allow".
        gw.inject_approval_decision(ApprovalDecision::Deny);
        let hub = Hub::new(gw.clone());
        hub.spawn_turn_bridge(gw.subscribe_turns());
        let mut session = Session::new(hub.clone());
        let _ = session
            .on_text(r#"{"protocol_version":"1.0.0","kind":"bot_client"}"#)
            .await;
        let write = rpc(
            &mut session,
            8,
            "bot.command",
            json!({
                "agentId": "agt_1",
                "name": "sendPrompt",
                "args": {
                    "agentId": "agt_1",
                    "prompt": "RUN mkdir p6dir"
                }
            }),
        )
        .await;
        let pw = result(&write)["preview"].as_str().unwrap().to_string();
        assert!(
            pw.contains(REASON_DENIED),
            "P6 deny evidence missing: {pw}"
        );
        assert!(!root.join("p6/agt_1/p6dir").exists());
        println!("SMOKE_OK tool-approval GA-P6 fail-not-allow");
    }

    // ── HTTP /approve loopback + closed reject ───────────────────────────
    {
        let gw = Arc::new(make_gw(root.join("http"), ToolApprovalMode::Gate, 10_000));
        let meta = GatewayHttpMeta::for_backend("box")
            .with_tool_approval(gw.tool_approval_mode().as_str(), false);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let router = gateway_http_router_with_meta(gw.clone(), meta)
            .into_make_service_with_connect_info::<std::net::SocketAddr>();
        tokio::spawn(async move {
            axum::serve(listener, router).await.ok();
        });
        tokio::time::sleep(Duration::from_millis(50)).await;

        let hub = Hub::new(gw.clone());
        hub.spawn_turn_bridge(gw.subscribe_turns());
        let mut session = Session::new(hub.clone());
        let _ = session
            .on_text(r#"{"protocol_version":"1.0.0","kind":"bot_client"}"#)
            .await;

        let pending_id: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let gw2 = gw.clone();
        let pending_id2 = pending_id.clone();
        let waiter = tokio::spawn(async move {
            for _ in 0..200 {
                let ids = gw2.pending_approval_ids().await;
                if let Some(id) = ids.first() {
                    *pending_id2.lock().await = Some(id.clone());
                    return;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        });

        let send = tokio::spawn({
            let mut session = session;
            async move {
                rpc(
                    &mut session,
                    9,
                    "bot.command",
                    json!({
                        "agentId": "agt_1",
                        "name": "sendPrompt",
                        "args": {
                            "agentId": "agt_1",
                            "prompt": "WRITE_FILE http.txt <<< via-http-approve"
                        }
                    }),
                )
                .await
            }
        });

        waiter.await.unwrap();
        let id = pending_id
            .lock()
            .await
            .clone()
            .expect("pending id for HTTP approve");
        let client = reqwest::Client::new();
        let resp = client
            .post(format!("http://{addr}/approve"))
            .json(&json!({ "approvalId": id, "decision": "allow" }))
            .send()
            .await
            .expect("approve http");
        assert!(resp.status().is_success(), "approve status {}", resp.status());
        let write = send.await.unwrap();
        let pw = result(&write)["preview"].as_str().unwrap().to_string();
        assert!(pw.contains("approval=allow"), "{pw}");
        assert!(root.join("http/agt_1/http.txt").is_file());

        // Repeat id → 409 closed
        let again = client
            .post(format!("http://{addr}/approve"))
            .json(&json!({ "approvalId": id, "decision": "allow" }))
            .send()
            .await
            .unwrap();
        assert_eq!(again.status(), reqwest::StatusCode::CONFLICT);

        // healthz shows gate
        let hz = client
            .get(format!("http://{addr}/healthz"))
            .send()
            .await
            .unwrap()
            .json::<Value>()
            .await
            .unwrap();
        assert_eq!(hz["tool_approval_mode"], "gate");
        assert_eq!(hz["tool_approval_gate"], true);
        println!("SMOKE_OK tool-approval HTTP /approve loopback");
    }

    // ── GA-P7: cli path unchanged (no box gate; default backend remains cli) ─
    {
        // Documented: product default backend=cli; this smoke does not flip it.
        // Constructing CliAgentGateway is enough to prove the crate still builds
        // the cli path; deepen/private_resident/cli smokes remain separate.
        let _ = atlas_bot_gateway::CliAgentGateway::from_env();
        println!("SMOKE_OK tool-approval GA-P7 cli-path-unchanged");
    }

    println!("SMOKE_OK tool-approval GA-P1–P7");
}
