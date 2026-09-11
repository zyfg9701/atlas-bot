//! RR1 private-resident runtime smoke: box + mock LLM multi-turn, tools,
//! interrupt, deterministic fallback. Default backend remains cli (RR-P4 via
//! existing p35_smoke / cli-primary path — asserted here as a doc guard).
//!
//! cargo test -p atlas-bot-hub --test private_resident_smoke -- --nocapture --test-threads=1

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use atlas_bot_gateway::{BoxLlmConfig, BoxSidecarGateway, Gateway};
use atlas_bot_hub::{Hub, Session};
use axum::routing::post;
use axum::{Json, Router};
use serde_json::{json, Value};
use tokio::sync::Mutex;
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

/// Loopback OpenAI-compat stub: records request bodies; optional artificial delay.
async fn spawn_mock_llm(
    delay: Duration,
) -> (String, Arc<Mutex<Vec<Value>>>, SocketAddr) {
    let recorded: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let rec = recorded.clone();
    let app = Router::new().route(
        "/v1/chat/completions",
        post(move |Json(body): Json<Value>| {
            let rec = rec.clone();
            async move {
                tokio::time::sleep(delay).await;
                let msgs = body
                    .get("messages")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default();
                let n = msgs.len();
                let users: Vec<&str> = msgs
                    .iter()
                    .filter(|m| m.get("role").and_then(|r| r.as_str()) == Some("user"))
                    .filter_map(|m| m.get("content").and_then(|c| c.as_str()))
                    .collect();
                let prior = users.len().saturating_sub(1);
                let saw = users.first().unwrap_or(&"");
                let snippet: String = saw.chars().take(48).collect();
                rec.lock().await.push(body);
                Json(json!({
                    "id": "chatcmpl-mock",
                    "object": "chat.completion",
                    "choices": [{
                        "index": 0,
                        "message": {
                            "role": "assistant",
                            "content": format!(
                                "llm-mock msgs={n} prior_users={prior} saw={snippet} llm=1"
                            )
                        },
                        "finish_reason": "stop"
                    }]
                }))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });
    // Base includes /v1 so client posts {base}/chat/completions.
    (format!("http://{addr}/v1"), recorded, addr)
}

#[tokio::test]
async fn private_resident_smoke() {
    // RR-P4 guard: product default backend is cli (binary default), never box.
    assert_eq!(
        std::env::var("ATLAS_GATEWAY_BACKEND").unwrap_or_else(|_| "cli".into()),
        "cli",
        "RR-P4: tests must not flip default backend to box"
    );

    let root = std::env::temp_dir().join(format!(
        "atlas-private-resident-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();

    // --- RR-P2: unset LLM → deterministic compose_reply (today's Box) ---
    {
        let gw = Arc::new(BoxSidecarGateway::new(
            PathBuf::from(&root).join("p2"),
            Duration::from_millis(5),
        ));
        assert!(!gw.llm_configured());
        let hub = Hub::new(gw.clone());
        hub.spawn_turn_bridge(gw.subscribe_turns());
        let mut session = Session::new(hub);
        let _ = session
            .on_text(r#"{"protocol_version":"1.0.0","kind":"bot_client"}"#)
            .await;
        let send = rpc(
            &mut session,
            1,
            "bot.command",
            json!({
                "agentId": "agt_1",
                "name": "sendPrompt",
                "args": { "agentId": "agt_1", "prompt": "hello-deterministic" }
            }),
        )
        .await;
        let preview = result(&send)["preview"].as_str().unwrap().to_string();
        assert!(preview.contains("box-sidecar"), "{preview}");
        assert!(preview.contains("turn=1"), "{preview}");
        assert!(!preview.contains("llm-mock"), "{preview}");
        assert!(!preview.starts_with("echo:"), "{preview}");
        println!("SMOKE_OK private-resident RR-P2 deterministic-no-llm");
    }

    // --- RR-P1: mock LLM multi-turn; second request messages ≥ 3 ---
    let (base, recorded, _addr) = spawn_mock_llm(Duration::from_millis(20)).await;
    {
        let llm = BoxLlmConfig {
            base: base.clone(),
            api_key: String::new(),
            model: "mock-rr1".into(),
            timeout: Duration::from_secs(5),
        };
        let gw = Arc::new(BoxSidecarGateway::new_with_llm(
            PathBuf::from(&root).join("p1"),
            Duration::from_millis(5),
            Some(llm),
        ));
        assert!(gw.llm_configured());
        let hub = Hub::new(gw.clone());
        hub.spawn_turn_bridge(gw.subscribe_turns());
        let mut session = Session::new(hub);
        let _ = session
            .on_text(r#"{"protocol_version":"1.0.0","kind":"bot_client"}"#)
            .await;

        let send1 = rpc(
            &mut session,
            10,
            "bot.command",
            json!({
                "agentId": "agt_1",
                "name": "sendPrompt",
                "args": { "agentId": "agt_1", "prompt": "token-RR1-ALPHA" }
            }),
        )
        .await;
        let p1 = result(&send1)["preview"].as_str().unwrap().to_string();
        assert!(p1.contains("llm-mock"), "expected mock LLM reply: {p1}");
        assert!(p1.contains("llm=1"), "{p1}");
        assert!(!p1.contains("box-sidecar agent="), "must not be compose_reply: {p1}");
        assert!(!p1.starts_with("echo:"), "{p1}");

        let send2 = rpc(
            &mut session,
            11,
            "bot.command",
            json!({
                "agentId": "agt_1",
                "name": "sendPrompt",
                "args": { "agentId": "agt_1", "prompt": "what did I say before?" }
            }),
        )
        .await;
        let p2 = result(&send2)["preview"].as_str().unwrap().to_string();
        assert!(p2.contains("llm-mock"), "{p2}");
        assert!(
            p2.contains("token-RR1-ALPHA") || p2.contains("saw=token-RR1-ALPHA"),
            "second turn must show prior context: {p2}"
        );
        assert!(
            p2.contains("prior_users=1") || p2.contains("msgs=4"),
            "second turn should carry history in messages: {p2}"
        );

        let rec = recorded.lock().await;
        assert!(rec.len() >= 2, "expected ≥2 LLM calls, got {}", rec.len());
        let msgs2 = rec[1]["messages"].as_array().expect("messages[]");
        assert!(
            msgs2.len() >= 3,
            "RR-P1: second turn messages len ≥3, got {}: {msgs2:?}",
            msgs2.len()
        );
        let roles: Vec<&str> = msgs2
            .iter()
            .filter_map(|m| m.get("role").and_then(|r| r.as_str()))
            .collect();
        assert!(roles.contains(&"system"), "{roles:?}");
        assert!(roles.contains(&"user"), "{roles:?}");
        assert!(roles.contains(&"assistant"), "{roles:?}");
        println!("SMOKE_OK private-resident RR-P1 mock-llm multi-turn");
    }

    // --- RR-P3: tools path still greppable + path escape ---
    {
        let llm = BoxLlmConfig {
            base: base.clone(),
            api_key: String::new(),
            model: "mock-rr1".into(),
            timeout: Duration::from_secs(5),
        };
        let gw = Arc::new(BoxSidecarGateway::new_with_llm(
            PathBuf::from(&root).join("p3"),
            Duration::from_millis(5),
            Some(llm),
        ));
        let hub = Hub::new(gw.clone());
        hub.spawn_turn_bridge(gw.subscribe_turns());
        let mut session = Session::new(hub);
        let _ = session
            .on_text(r#"{"protocol_version":"1.0.0","kind":"bot_client"}"#)
            .await;

        let w = rpc(
            &mut session,
            20,
            "bot.command",
            json!({
                "agentId": "agt_1",
                "name": "sendPrompt",
                "args": {
                    "agentId": "agt_1",
                    "prompt": "WRITE_FILE rr1.txt <<< hello-rr1"
                }
            }),
        )
        .await;
        let pw = result(&w)["preview"].as_str().unwrap().to_string();
        assert!(pw.contains("[tool:write_file"), "tools must win over LLM: {pw}");
        assert!(pw.contains("box-sidecar"), "tool path uses compose_reply: {pw}");
        assert!(!pw.contains("llm-mock"), "{pw}");

        let r = rpc(
            &mut session,
            21,
            "bot.command",
            json!({
                "agentId": "agt_1",
                "name": "sendPrompt",
                "args": { "agentId": "agt_1", "prompt": "READ_FILE rr1.txt" }
            }),
        )
        .await;
        let pr = result(&r)["preview"].as_str().unwrap().to_string();
        assert!(pr.contains("hello-rr1"), "{pr}");

        let bad = rpc(
            &mut session,
            22,
            "bot.command",
            json!({
                "agentId": "agt_1",
                "name": "sendPrompt",
                "args": { "agentId": "agt_1", "prompt": "READ_FILE ../etc/passwd" }
            }),
        )
        .await;
        let pb = result(&bad)["preview"].as_str().unwrap().to_string();
        assert!(
            pb.contains("path escape") || pb.contains(".."),
            "expected escape reject: {pb}"
        );
        println!("SMOKE_OK private-resident RR-P3 tools+escape");
    }

    // --- RR-P5: idle closed-set + in-flight LLM cancel ---
    {
        let (slow_base, _rec, _) = spawn_mock_llm(Duration::from_secs(3)).await;
        let llm = BoxLlmConfig {
            base: slow_base,
            api_key: String::new(),
            model: "mock-slow".into(),
            timeout: Duration::from_secs(10),
        };
        let gw = Arc::new(BoxSidecarGateway::new_with_llm(
            PathBuf::from(&root).join("p5"),
            Duration::from_millis(0), // skip delay so cancel hits LLM HTTP
            Some(llm),
        ));
        let hub = Hub::new(gw.clone());
        hub.spawn_turn_bridge(gw.subscribe_turns());
        let mut session = Session::new(hub.clone());
        let _ = session
            .on_text(r#"{"protocol_version":"1.0.0","kind":"bot_client"}"#)
            .await;

        let idle = rpc(
            &mut session,
            30,
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
            "idle interrupt must error: {idle}"
        );
        assert_eq!(
            idle["error"]["message"].as_str().unwrap_or(""),
            "command_rejected",
            "closed-set: {idle}"
        );

        let hub_c = hub.clone();
        let send_task = tokio::spawn(async move {
            hub_c
                .handle_rpc(
                    "c_rr1_interrupt",
                    rpc_req(
                        90,
                        "bot.command",
                        json!({
                            "agentId": "agt_1",
                            "name": "sendPrompt",
                            "args": { "agentId": "agt_1", "prompt": "slow-llm-turn" }
                        }),
                    ),
                )
                .await
        });
        tokio::time::sleep(Duration::from_millis(80)).await;
        let interrupted = rpc(
            &mut session,
            31,
            "bot.command",
            json!({
                "agentId": "agt_1",
                "name": "interruptAgentRun",
                "args": { "agentId": "agt_1" }
            }),
        )
        .await;
        let ir = result(&interrupted);
        assert_eq!(ir["hadActiveRun"], true, "{ir}");
        assert_eq!(ir["killed"], true, "{ir}");
        let send_outcome = send_task.await.expect("join");
        assert!(
            matches!(send_outcome.outcome, ResponseOutcome::Error(_)),
            "interrupted LLM sendPrompt should error: {send_outcome:?}"
        );
        println!("SMOKE_OK private-resident RR-P5 interrupt");
    }

    // Visible LLM failure (bad base) — not silent echo.
    {
        let llm = BoxLlmConfig {
            base: "http://127.0.0.1:9/v1".into(), // nothing listening
            api_key: String::new(),
            model: "nope".into(),
            timeout: Duration::from_millis(500),
        };
        let gw = Arc::new(BoxSidecarGateway::new_with_llm(
            PathBuf::from(&root).join("fail"),
            Duration::from_millis(0),
            Some(llm),
        ));
        let err = gw
            .invoke(
                "agt_1",
                "sendPrompt",
                json!({ "prompt": "should-fail-visible" }),
            )
            .await;
        assert!(err.is_err(), "LLM fail must be visible, got {err:?}");
        let msg = format!("{:?}", err.err().unwrap());
        assert!(
            msg.contains("box LLM") || msg.contains("Upstream") || msg.contains("failed"),
            "expected upstream-ish error: {msg}"
        );
        println!("SMOKE_OK private-resident LLM-fail-visible");
    }

    let _ = std::fs::remove_dir_all(&root);
    println!("SMOKE_OK private-resident RR-P1–P5 (+P4 default-cli guard)");
}
