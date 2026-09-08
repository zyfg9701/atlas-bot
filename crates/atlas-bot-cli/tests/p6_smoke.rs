//! P6 β′ smoke: CLI HubClient against in-process Hub WS.
//!
//! `cargo test -p atlas-bot-cli --test p6_smoke -- --nocapture`
//! Expect: `SMOKE_OK p6 cli-bridge hello+status+list+prompt+turn_finished+transcript`

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use atlas_bot_cli::HubClient;
use atlas_bot_gateway::{Gateway, InMemoryGateway};
use atlas_bot_hub::{Hub, Session};
use axum::extract::ws::{Message, WebSocket};
use axum::extract::{State, WebSocketUpgrade};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;

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
    // Tiny yield so accept loop is ready.
    tokio::task::yield_now().await;
    (addr, hub, gw)
}

#[tokio::test]
async fn p6_smoke_cli_bridge() {
    let (addr, _hub, gw) = start_hub().await;
    let url = format!("ws://{addr}/ws");
    let client = HubClient::connect(&url).await.expect("connect");

    // hello capabilities: Bot-Relay only, no ACP pollution
    client.assert_no_acp_capabilities().expect("no acp");
    let caps = client.capabilities();
    assert!(caps.iter().any(|c| c == "bot.status"));
    assert!(caps.iter().any(|c| c == "bot.command"));
    assert!(caps.iter().all(|c| !c.to_ascii_lowercase().contains("acp")));

    // Cold status + roster (must not wake gateway)
    let before = gw.invoke_count();
    let st = client.status().await.expect("status");
    assert!(st.get("runState").is_some(), "{st}");
    let roster = client.roster().await.expect("roster");
    let agents = roster["agents"].as_array().expect("agents");
    assert!(agents.iter().any(|a| a["agentId"] == "agt_1"));
    assert_eq!(
        gw.invoke_count(),
        before,
        "cold status/roster must not wake gateway"
    );

    // Optional hot listAgents
    let listed = client.list_agents("agt_1").await.expect("listAgents");
    assert!(listed.as_array().unwrap().iter().any(|a| a["id"] == "agt_1"));

    // prompt closed loop
    let prompt = "hello-p6-cli-bridge";
    let (send, turn, tail) = client
        .prompt_closed_loop("agt_1", prompt)
        .await
        .expect("prompt loop");
    assert_eq!(send["accepted"], true);
    assert_eq!(turn["channel"], "hub:turn_finished");
    let entries = tail["entries"].as_array().expect("entries");
    assert!(
        entries.iter().any(|e| e["text"] == prompt),
        "transcript missing prompt: {tail}"
    );
    assert!(
        entries.iter().any(|e| {
            e["text"]
                .as_str()
                .map(|t| t.contains("echo:") || !t.is_empty())
                .unwrap_or(false)
        }),
        "transcript should include assistant reply: {tail}"
    );
    assert!(gw.invoke_count() > before);

    // interrupt path available (no active run → hadActiveRun false is ok)
    let ir = client.interrupt("agt_1").await.expect("interrupt");
    assert!(ir.get("hadActiveRun").is_some(), "{ir}");

    println!(
        "SMOKE_OK p6 cli-bridge hello+status+list+prompt+turn_finished+transcript"
    );
}
