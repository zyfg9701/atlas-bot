//! CG1 / cli tool-approval smoke (stream-json gate + Deny kill).
//!
//! cargo test -p atlas-bot-hub --test tool_approval_cli_smoke -- --nocapture --test-threads=1
//!
//! CG-P1–P8 via mock-cli stream-json. Does not claim box zero side-effects.
//! Keeps GA1 box smoke separate (`tool_approval_smoke`).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use atlas_bot_gateway::{
    gateway_http_router_with_meta, ApprovalDecision, CliAgentGateway, GatewayHttpMeta,
    ToolApprovalMode, APPROVAL_PENDING_PREFIX, CHANNEL_HUB_TOOL, REASON_AUTO_DENY, REASON_DENIED,
    REASON_TIMEOUT,
};
use atlas_bot_hub::{Hub, Session};
use serde_json::{json, Value};
use tokio::sync::Mutex;

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

fn make_cli_gw(mode: ToolApprovalMode, timeout_ms: u64, stream: bool) -> CliAgentGateway {
    std::env::set_var("ATLAS_TOOL_APPROVAL_MODE", mode.as_str());
    std::env::set_var("ATLAS_TOOL_APPROVAL_TIMEOUT_MS", timeout_ms.to_string());
    std::env::remove_var("ATLAS_TOOL_APPROVAL_TOKEN");
    std::env::set_var("MOCK_CLI_STREAM", "1");
    // Keep child alive long enough for gate hang / kill evidence.
    std::env::set_var("MOCK_CLI_SLEEP_MS", "400");
    CliAgentGateway::new_with_approval(mock_cli_path(), vec![], None, stream, mode)
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
async fn tool_approval_cli_smoke() {
    let cli = mock_cli_path();
    assert!(cli.exists(), "mock cli missing: {}", cli.display());

    // ── CG-P1: stream + dangerous Shell + auto_deny → kill + closed reason ─
    {
        std::env::set_var("MOCK_CLI_TOOL", "Shell");
        std::env::set_var("MOCK_CLI_TOOL_STATUS", "started");
        let gw = Arc::new(make_cli_gw(ToolApprovalMode::AutoDeny, 5_000, true));
        let hub = Hub::new(gw.clone());
        hub.spawn_turn_bridge(gw.subscribe_turns());
        let mut session = Session::new(hub.clone());
        let _ = session
            .on_text(r#"{"protocol_version":"1.0.0","kind":"bot_client"}"#)
            .await;
        let resp = rpc(
            &mut session,
            1,
            "bot.command",
            json!({
                "agentId": "agt_1",
                "name": "sendPrompt",
                "args": { "agentId": "agt_1", "prompt": "please run shell" }
            }),
        )
        .await;
        let pw = result(&resp)["preview"].as_str().unwrap().to_string();
        assert!(
            pw.contains(REASON_AUTO_DENY) || pw.contains("tool_approval"),
            "CG-P1 deny evidence missing: {pw}"
        );
        assert!(
            pw.contains("interrupted") || pw.contains("cli tool approval"),
            "CG-P1 kill/interrupt narrative missing: {pw}"
        );
        println!("SMOKE_OK tool-approval-cli CG-P1 auto_deny Shell kill");
    }

    // ── CG-P2: inject Allow → turn continues ─────────────────────────────
    {
        std::env::set_var("MOCK_CLI_TOOL", "Write");
        std::env::set_var("MOCK_CLI_SLEEP_MS", "50");
        let gw = Arc::new(make_cli_gw(ToolApprovalMode::Gate, 10_000, true));
        gw.inject_approval_decision(ApprovalDecision::Allow);
        let hub = Hub::new(gw.clone());
        hub.spawn_turn_bridge(gw.subscribe_turns());
        let mut session = Session::new(hub.clone());
        let _ = session
            .on_text(r#"{"protocol_version":"1.0.0","kind":"bot_client"}"#)
            .await;
        let resp = rpc(
            &mut session,
            2,
            "bot.command",
            json!({
                "agentId": "agt_1",
                "name": "sendPrompt",
                "args": { "agentId": "agt_1", "prompt": "write something" }
            }),
        )
        .await;
        let pw = result(&resp)["preview"].as_str().unwrap().to_string();
        assert!(
            pw.contains("atlas-mock-reply"),
            "CG-P2 Allow should continue to final reply: {pw}"
        );
        assert!(
            !pw.contains(REASON_DENIED) && !pw.contains(REASON_AUTO_DENY),
            "CG-P2 must not deny: {pw}"
        );
        println!("SMOKE_OK tool-approval-cli CG-P2 inject-allow continues");
    }

    // ── CG-P3: explicit Deny via submit_tool_approval + GA1b pending ─────
    {
        std::env::set_var("MOCK_CLI_TOOL", "Shell");
        std::env::set_var("MOCK_CLI_SLEEP_MS", "800");
        let gw = Arc::new(make_cli_gw(ToolApprovalMode::Gate, 10_000, true));
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
            3,
            "bot.subscribe",
            json!({ "agentIds": ["agt_1"] }),
        )
        .await;
        let mut bus = hub.subscribe_outbound();

        let gw2 = gw.clone();
        let deny_task = tokio::spawn(async move {
            for _ in 0..400 {
                let ids = gw2.pending_approval_ids().await;
                if let Some(id) = ids.first() {
                    let r = gw2
                        .submit_tool_approval(id, ApprovalDecision::Deny)
                        .await
                        .expect("deny");
                    assert_eq!(r["decision"], "deny");
                    let again = gw2
                        .submit_tool_approval(id, ApprovalDecision::Allow)
                        .await;
                    assert!(again.is_err(), "repeat approve must closed-reject");
                    return;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            panic!("CG-P3: never saw pending approval");
        });

        let resp = rpc(
            &mut session,
            4,
            "bot.command",
            json!({
                "agentId": "agt_1",
                "name": "sendPrompt",
                "args": { "agentId": "agt_1", "prompt": "danger shell" }
            }),
        )
        .await;
        deny_task.await.unwrap();
        let pw = result(&resp)["preview"].as_str().unwrap().to_string();
        assert!(pw.contains(REASON_DENIED), "CG-P3 deny reason missing: {pw}");
        assert!(
            !pw.contains("atlas-mock-reply"),
            "CG-P3 must not complete dangerous turn successfully: {pw}"
        );

        let events = collect_tool_events(&mut bus, &conn, Duration::from_millis(400)).await;
        let pending_seen = events.iter().any(|e| {
            e["params"]["event"]["summary"]
                .as_str()
                .unwrap_or("")
                .contains(APPROVAL_PENDING_PREFIX)
        });
        assert!(pending_seen, "CG-P3 GA1b pending hint missing: {events:?}");
        println!("SMOKE_OK tool-approval-cli CG-P3 explicit-deny + pending");
    }

    // ── CG-P4: timeout == Deny ───────────────────────────────────────────
    {
        std::env::set_var("MOCK_CLI_TOOL", "Delete");
        std::env::set_var("MOCK_CLI_SLEEP_MS", "800");
        let gw = Arc::new(make_cli_gw(ToolApprovalMode::Gate, 80, true));
        let hub = Hub::new(gw.clone());
        hub.spawn_turn_bridge(gw.subscribe_turns());
        let mut session = Session::new(hub.clone());
        let _ = session
            .on_text(r#"{"protocol_version":"1.0.0","kind":"bot_client"}"#)
            .await;
        let resp = rpc(
            &mut session,
            5,
            "bot.command",
            json!({
                "agentId": "agt_1",
                "name": "sendPrompt",
                "args": { "agentId": "agt_1", "prompt": "delete file" }
            }),
        )
        .await;
        let pw = result(&resp)["preview"].as_str().unwrap().to_string();
        assert!(pw.contains(REASON_TIMEOUT), "CG-P4 timeout reason missing: {pw}");
        println!("SMOKE_OK tool-approval-cli CG-P4 timeout-deny");
    }

    // ── CG-P5: exempt Read — no approval, turn continues ─────────────────
    {
        std::env::set_var("MOCK_CLI_TOOL", "Read");
        std::env::set_var("MOCK_CLI_SLEEP_MS", "30");
        let gw = Arc::new(make_cli_gw(ToolApprovalMode::Gate, 5_000, true));
        let hub = Hub::new(gw.clone());
        hub.spawn_turn_bridge(gw.subscribe_turns());
        let mut session = Session::new(hub.clone());
        let _ = session
            .on_text(r#"{"protocol_version":"1.0.0","kind":"bot_client"}"#)
            .await;
        let resp = rpc(
            &mut session,
            6,
            "bot.command",
            json!({
                "agentId": "agt_1",
                "name": "sendPrompt",
                "args": { "agentId": "agt_1", "prompt": "read a file" }
            }),
        )
        .await;
        let pw = result(&resp)["preview"].as_str().unwrap().to_string();
        assert!(
            pw.contains("atlas-mock-reply"),
            "CG-P5 Read exempt should finish: {pw}"
        );
        assert!(
            !pw.contains("tool_approval"),
            "CG-P5 must not gate Read: {pw}"
        );
        assert!(gw.pending_approval_ids().await.is_empty());
        println!("SMOKE_OK tool-approval-cli CG-P5 exempt Read");
    }

    // ── CG-P6: unknown name → default gated (auto_deny) ──────────────────
    {
        std::env::set_var("MOCK_CLI_TOOL", "MysteryExec");
        std::env::set_var("MOCK_CLI_SLEEP_MS", "200");
        let gw = Arc::new(make_cli_gw(ToolApprovalMode::AutoDeny, 5_000, true));
        let hub = Hub::new(gw.clone());
        hub.spawn_turn_bridge(gw.subscribe_turns());
        let mut session = Session::new(hub.clone());
        let _ = session
            .on_text(r#"{"protocol_version":"1.0.0","kind":"bot_client"}"#)
            .await;
        let resp = rpc(
            &mut session,
            7,
            "bot.command",
            json!({
                "agentId": "agt_1",
                "name": "sendPrompt",
                "args": { "agentId": "agt_1", "prompt": "mystery" }
            }),
        )
        .await;
        let pw = result(&resp)["preview"].as_str().unwrap().to_string();
        assert!(
            pw.contains(REASON_AUTO_DENY),
            "CG-P6 unknown must default gated: {pw}"
        );
        println!("SMOKE_OK tool-approval-cli CG-P6 unknown default-gated");
    }

    // ── CG-P7: text mode / stream off — document cannot gate ─────────────
    {
        std::env::remove_var("MOCK_CLI_TOOL");
        std::env::set_var("MOCK_CLI_STREAM", "0");
        std::env::set_var("MOCK_CLI_SLEEP_MS", "10");
        // Even with Gate mode, text path has no mid-turn tool_call → no deny.
        let gw = Arc::new(make_cli_gw(ToolApprovalMode::AutoDeny, 5_000, false));
        assert!(!gw.stream_enabled());
        let hub = Hub::new(gw.clone());
        hub.spawn_turn_bridge(gw.subscribe_turns());
        let mut session = Session::new(hub.clone());
        let _ = session
            .on_text(r#"{"protocol_version":"1.0.0","kind":"bot_client"}"#)
            .await;
        let resp = rpc(
            &mut session,
            8,
            "bot.command",
            json!({
                "agentId": "agt_1",
                "name": "sendPrompt",
                "args": { "agentId": "agt_1", "prompt": "text mode cannot gate tools" }
            }),
        )
        .await;
        let pw = result(&resp)["preview"].as_str().unwrap().to_string();
        assert!(
            pw.contains("atlas-mock-reply"),
            "CG-P7 text mode still returns reply: {pw}"
        );
        assert!(
            !pw.contains("tool_approval"),
            "CG-P7 must NOT claim mid-turn gate coverage in text mode: {pw}"
        );
        println!("SMOKE_OK tool-approval-cli CG-P7 text-mode-no-coverage");
    }

    // ── CG-P8: healthz + HTTP /approve on cli backend ────────────────────
    {
        std::env::set_var("MOCK_CLI_STREAM", "1");
        std::env::set_var("MOCK_CLI_TOOL", "Bash");
        std::env::set_var("MOCK_CLI_SLEEP_MS", "800");
        let gw = Arc::new(make_cli_gw(ToolApprovalMode::Gate, 10_000, true));
        let meta = GatewayHttpMeta::for_backend("cli")
            .with_tool_approval(gw.tool_approval_mode().as_str(), false);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let router = gateway_http_router_with_meta(gw.clone(), meta)
            .into_make_service_with_connect_info::<std::net::SocketAddr>();
        tokio::spawn(async move {
            axum::serve(listener, router).await.ok();
        });
        tokio::time::sleep(Duration::from_millis(50)).await;

        let client = reqwest::Client::new();
        let hz = client
            .get(format!("http://{addr}/healthz"))
            .send()
            .await
            .unwrap()
            .json::<Value>()
            .await
            .unwrap();
        assert_eq!(hz["backend"], "cli");
        assert_eq!(hz["tool_approval_mode"], "gate");
        assert_eq!(hz["tool_approval_gate"], true);

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
            for _ in 0..400 {
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
                        "args": { "agentId": "agt_1", "prompt": "bash via http approve" }
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
        let resp = client
            .post(format!("http://{addr}/approve"))
            .json(&json!({ "approvalId": id, "decision": "allow" }))
            .send()
            .await
            .expect("approve http");
        assert!(resp.status().is_success(), "approve status {}", resp.status());
        let write = send.await.unwrap();
        let pw = result(&write)["preview"].as_str().unwrap().to_string();
        assert!(
            pw.contains("atlas-mock-reply") || pw.contains("approval=allow"),
            "CG-P8 allow via /approve: {pw}"
        );
        println!("SMOKE_OK tool-approval-cli CG-P8 healthz+/approve");
    }

    // Note: GA1 box regression (CG-P8 acceptance) is `tool_approval_smoke` — keep separate CI step.
    println!("SMOKE_OK tool-approval-cli CG-P1–P8");
}
