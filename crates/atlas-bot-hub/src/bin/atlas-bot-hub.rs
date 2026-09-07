//! atlas-bot-hub — Bot-Relay WebSocket Computer Hub (P3.5).
//!
//! Env:
//! - `ATLAS_HUB_BIND` — default `127.0.0.1:7700`
//! - `ATLAS_GATEWAY_URL` — if set, use HTTP gateway at this base URL;
//!   otherwise embed an in-process [`InMemoryGateway`] stub.
//! - `ATLAS_GATEWAY_HTTP_BIND` — optional local HTTP gateway bind for the
//!   embedded stub (e.g. `127.0.0.1:8787`). For scheme B, run
//!   `atlas-bot-gateway` separately and set `ATLAS_GATEWAY_URL`.

use std::net::SocketAddr;
use std::sync::Arc;

use atlas_bot_gateway::{serve_http, Gateway, HttpGatewayClient, InMemoryGateway};
use atlas_bot_hub::{Hub, Session};
use axum::extract::ws::{Message, WebSocket};
use axum::extract::{State, WebSocketUpgrade};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tower_http::trace::TraceLayer;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

#[derive(Clone)]
struct AppState {
    hub: Arc<Hub>,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("info".parse().unwrap()))
        .init();

    let bind: SocketAddr = std::env::var("ATLAS_HUB_BIND")
        .unwrap_or_else(|_| "127.0.0.1:7700".into())
        .parse()
        .expect("ATLAS_HUB_BIND");

    let (hub, gw_opt): (Arc<Hub>, Option<Arc<InMemoryGateway>>) =
        if let Ok(url) = std::env::var("ATLAS_GATEWAY_URL") {
            info!(%url, "using HTTP gateway");
            let client: Arc<dyn Gateway> = Arc::new(HttpGatewayClient::new(url));
            (Hub::new(client), None)
        } else {
            let (hub, gw) = Hub::with_in_memory_gateway();
            hub.spawn_turn_bridge(gw.subscribe_turns());
            (hub, Some(gw))
        };

    if let Some(gw) = gw_opt {
        if let Ok(http_bind) = std::env::var("ATLAS_GATEWAY_HTTP_BIND") {
            let addr: SocketAddr = http_bind.parse().expect("ATLAS_GATEWAY_HTTP_BIND");
            tokio::spawn(async move {
                if let Err(e) = serve_http(gw, addr).await {
                    warn!("gateway HTTP exited: {e}");
                }
            });
            info!(%addr, "embedded gateway HTTP enabled");
        }
    }

    let app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/ws", get(ws_upgrade))
        .layer(TraceLayer::new_for_http())
        .with_state(AppState { hub });

    let listener = tokio::net::TcpListener::bind(bind).await.expect("bind");
    info!(%bind, "atlas-bot-hub listening (WS /ws)");
    axum::serve(listener, app).await.expect("serve");
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
                    Some(Ok(Message::Ping(_))) => {}
                    Some(Ok(_)) => {}
                    Some(Err(e)) => {
                        warn!("ws error: {e}");
                        break;
                    }
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
