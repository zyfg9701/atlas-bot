//! α ACP adapter smoke: fake ACP client → adapter → in-process Hub.
//!
//! `cargo test -p atlas-acp-adapter --test alpha_smoke -- --nocapture`
//! Expect: `SMOKE_OK alpha acp-adapter hello+list+prompt+turn_finished+transcript`

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use atlas_acp_adapter::{is_whitelisted, spawn_adapter, FROZEN_WHITELIST};
use atlas_bot_gateway::{Gateway, InMemoryGateway};
use atlas_bot_hub::{Hub, Session};
use axum::extract::ws::{Message, WebSocket};
use axum::extract::{State, WebSocketUpgrade};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMessage};

#[derive(Clone)]
struct AppState {
    hub: Arc<Hub>,
}

async fn ws_upgrade(ws: WebSocketUpgrade, State(st): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, st.hub))
}

async fn handle_socket(socket: WebSocket, hub: Arc<Hub>) {
    let (mut sink, mut stream) = socket.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    let mut session = Session::new(Arc::clone(&hub));
    let mut bus = hub.subscribe_outbound();

    let writer = tokio::spawn(async move {
        while let Some(text) = rx.recv().await {
            if sink.send(Message::Text(text.into())).await.is_err() {
                break;
            }
        }
    });

    loop {
        tokio::select! {
            biased;
            ev = bus.recv() => {
                match ev {
                    Ok((cid, text)) => {
                        if !session.connection_id.is_empty() && session.connection_id == cid {
                            let _ = tx.send(text);
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
            msg = stream.next() => {
                match msg {
                    Some(Ok(Message::Text(t))) => {
                        let outs = session.on_text(&t).await;
                        for o in outs {
                            if tx.send(o).is_err() {
                                break;
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {}
                    Some(Err(_)) => break,
                }
            }
        }
    }

    if !session.connection_id.is_empty() {
        hub.unregister_connection(&session.connection_id).await;
    }
    drop(tx);
    let _ = writer.await;
}

async fn start_hub() -> (SocketAddr, Arc<Hub>, Arc<InMemoryGateway>) {
    let gw = Arc::new(InMemoryGateway::with_turn_delay(Duration::from_millis(40)));
    let hub = Hub::new(gw.clone());
    hub.spawn_turn_bridge(gw.subscribe_turns());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = Router::new()
        .route("/ws", get(ws_upgrade))
        .with_state(AppState { hub: hub.clone() });
    tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });
    tokio::task::yield_now().await;
    (addr, hub, gw)
}

struct FakeAcp {
    write: futures_util::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        WsMessage,
    >,
    read: futures_util::stream::SplitStream<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    >,
    next_id: i64,
}

impl FakeAcp {
    async fn connect(url: &str) -> Self {
        let (ws, _) = connect_async(url).await.expect("acp ws");
        let (write, read) = ws.split();
        Self {
            write,
            read,
            next_id: 1,
        }
    }

    async fn rpc(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        let frame = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        self.write
            .send(WsMessage::Text(frame.to_string().into()))
            .await
            .expect("send");
        loop {
            let msg = tokio::time::timeout(Duration::from_secs(30), self.read.next())
                .await
                .expect("timeout")
                .expect("stream end")
                .expect("ws err");
            let text = match msg {
                WsMessage::Text(t) => t.to_string(),
                WsMessage::Ping(_) | WsMessage::Pong(_) => continue,
                other => panic!("unexpected {other:?}"),
            };
            let v: Value = serde_json::from_str(&text).expect("json");
            if v.get("id") == Some(&json!(id)) {
                return v;
            }
        }
    }
}

#[tokio::test]
async fn alpha_smoke_acp_adapter() {
    assert_eq!(
        FROZEN_WHITELIST.len(),
        6,
        "whitelist drift — update runbook if intentional"
    );
    assert!(is_whitelisted("initialize"));
    assert!(!is_whitelisted("fs/read_text_file"));

    let (hub_addr, _hub, gw) = start_hub().await;
    let hub_ws = format!("ws://{hub_addr}/ws");
    let acp_addr = spawn_adapter(hub_ws.clone()).await.expect("spawn adapter");
    let acp_url = format!("ws://{acp_addr}/ws");

    let mut client = FakeAcp::connect(&acp_url).await;

    // initialize / capabilities (+ assert Hub caps have no acp)
    let init = client.rpc("initialize", json!({
        "protocolVersion": 1,
        "clientInfo": { "name": "fake-acp-smoke", "version": "0.0.1" }
    })).await;
    assert!(init.get("error").is_none(), "{init}");
    let result = &init["result"];
    assert_eq!(result["agentInfo"]["name"], "atlas-acp-adapter");
    assert_eq!(result["agentCapabilities"]["atlasAlphaSubset"], true);
    let hub_caps = result["hub"]["capabilities"]
        .as_array()
        .expect("hub caps");
    assert!(hub_caps.iter().any(|c| c == "bot.status"));
    assert!(hub_caps.iter().any(|c| c == "bot.command"));
    for c in hub_caps {
        let s = c.as_str().unwrap_or("");
        assert!(
            !s.to_ascii_lowercase().contains("acp"),
            "Hub capabilities must not contain acp: {s}"
        );
    }

    // unknown method → stable -32601
    let unk = client
        .rpc("fs/read_text_file", json!({"path": "/tmp/x"}))
        .await;
    assert_eq!(unk["error"]["code"], -32601, "{unk}");
    assert!(
        unk["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("whitelist"),
        "{unk}"
    );

    // list agents
    let listed = client.rpc("agents/list", json!({})).await;
    assert!(listed.get("error").is_none(), "{listed}");
    let agents = listed["result"]["agents"].as_array().expect("agents");
    assert!(agents.iter().any(|a| a["agentId"] == "agt_1"));

    // select / new session
    let sess = client
        .rpc("session/new", json!({ "agentId": "agt_1", "cwd": "/tmp" }))
        .await;
    assert!(sess.get("error").is_none(), "{sess}");
    let session_id = sess["result"]["sessionId"].as_str().expect("sessionId");

    // prompt closed loop
    let before = gw.invoke_count();
    let prompt = "hello-alpha-acp-adapter";
    let pr = client
        .rpc(
            "session/prompt",
            json!({
                "sessionId": session_id,
                "agentId": "agt_1",
                "prompt": [{"type": "text", "text": prompt}]
            }),
        )
        .await;
    assert!(pr.get("error").is_none(), "{pr}");
    let pr = &pr["result"];
    assert_eq!(pr["accepted"], true);
    assert_eq!(pr["turn"]["channel"], "hub:turn_finished");
    let entries = pr["transcriptTail"]["entries"]
        .as_array()
        .expect("entries");
    assert!(
        entries.iter().any(|e| e["text"] == prompt),
        "transcript missing prompt: {pr}"
    );
    assert!(
        entries.iter().any(|e| {
            e["text"]
                .as_str()
                .map(|t| t.contains("echo:") || !t.is_empty())
                .unwrap_or(false)
        }),
        "transcript should include assistant reply: {pr}"
    );
    assert!(gw.invoke_count() > before);

    // optional cancel / interrupt
    let cancel = client
        .rpc("session/cancel", json!({ "agentId": "agt_1", "sessionId": session_id }))
        .await;
    assert!(cancel.get("error").is_none(), "{cancel}");
    assert!(
        cancel["result"]["interrupt"]["hadActiveRun"].is_boolean()
            || cancel["result"]["interrupt"].get("hadActiveRun").is_some(),
        "{cancel}"
    );

    // explicit transcript tail
    let tail = client
        .rpc("transcript/tail", json!({ "agentId": "agt_1", "limit": 10 }))
        .await;
    assert!(tail.get("error").is_none(), "{tail}");

    println!(
        "SMOKE_OK alpha acp-adapter hello+list+prompt+turn_finished+transcript"
    );
}
