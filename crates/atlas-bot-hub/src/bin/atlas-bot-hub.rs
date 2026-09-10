//! atlas-bot-hub — Bot-Relay WebSocket Computer Hub (P3.5 + I1 auth gate + B1 ingest + W1 WeCom exchange).
//!
//! Env:
//! - `ATLAS_HUB_BIND` — default `127.0.0.1:7700`
//! - `ATLAS_GATEWAY_URL` — if set, use HTTP gateway at this base URL;
//!   otherwise embed an in-process [`InMemoryGateway`] stub.
//! - `ATLAS_GATEWAY_HTTP_BIND` — embedded stub HTTP bind (default
//!   `127.0.0.1:8787` for P5 VNC stub/proxy). Set `off` to disable. For scheme B,
//!   run `atlas-bot-gateway` separately and set `ATLAS_GATEWAY_URL`.
//! - `ATLAS_HUB_EVENT_BIND` — B1 RuntimeHint ingest (default `127.0.0.1:7701`);
//!   must be loopback. Set `off` to disable.
//! - `ATLAS_HUB_EVENT_TOKEN` / `ATLAS_HUB_EVENT_ALLOW_INSECURE_LOOPBACK` —
//!   see docs/b1-event-ingest-runbook.md.
//! - `ATLAS_VNC_MODE` / `ATLAS_VNC_UPSTREAM` / `ATLAS_ATTACH_MODE` /
//!   `ATLAS_ATTACH_ROOT` — see docs/P5-real-runbook.md (read by InMemoryGateway).
//! - `ATLAS_AUTH_MODE` / `ATLAS_AUTH_JWT_SECRET` / `ATLAS_AUTH_ALLOWLIST` /
//!   `ATLAS_OIDC_*` — see docs/idp-runbook.md (Hub inbound only).
//! - `ATLAS_WECOM_CORP_ID` / `ATLAS_WECOM_AGENT_ID` / `ATLAS_WECOM_SECRET` /
//!   `ATLAS_WECOM_API_BASE` / `ATLAS_WECOM_JWT_TTL` — W1 Hub `POST /auth/wecom/exchange`
//!   (secret Hub-only; see docs/i2-login-runbook.md § WeCom).

use std::net::SocketAddr;
use std::sync::Arc;

use atlas_bot_gateway::{serve_http, Gateway, HttpGatewayClient, InMemoryGateway};
use atlas_bot_hub::auth::{bearer_from_authorization, AuthConfig};
use atlas_bot_hub::wecom_exchange::{self, WeComHubConfig};
use atlas_bot_hub::{serve_event_ingest, EventIngestConfig, Hub, Session};
use axum::extract::ws::{Message, WebSocket};
use axum::extract::{State, WebSocketUpgrade};
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tower_http::trace::TraceLayer;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

#[derive(Clone)]
struct AppState {
    hub: Arc<Hub>,
    auth: AuthConfig,
    wecom: Option<WeComHubConfig>,
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

    let auth = AuthConfig::from_env();
    let wecom = WeComHubConfig::from_env();
    if wecom.is_some() {
        info!("wecom exchange enabled (POST /auth/wecom/exchange)");
    } else {
        info!("wecom exchange not configured (set ATLAS_WECOM_* + ATLAS_AUTH_JWT_SECRET)");
    }

    let (hub, gw_opt): (Arc<Hub>, Option<Arc<InMemoryGateway>>) =
        if let Ok(url) = std::env::var("ATLAS_GATEWAY_URL") {
            info!(%url, "using HTTP gateway");
            let client: Arc<dyn Gateway> = Arc::new(HttpGatewayClient::new(url));
            (Hub::new(client), None)
        } else {
            warn!(
                "ATLAS_GATEWAY_URL unset — using embedded InMemory stub (echo). True CLI path needs ATLAS_GATEWAY_URL + atlas-bot-gateway backend=cli; see docs/cli-primary-runbook.md"
            );
            let (hub, gw) = Hub::with_in_memory_gateway();
            hub.spawn_turn_bridge(gw.subscribe_turns());
            (hub, Some(gw))
        };

    if let Some(gw) = gw_opt {
        // P5: default-enable stub HTTP (VNC placeholder + /invoke) on :8787.
        // Set ATLAS_GATEWAY_HTTP_BIND=off to disable.
        let http_bind = std::env::var("ATLAS_GATEWAY_HTTP_BIND")
            .unwrap_or_else(|_| "127.0.0.1:8787".into());
        if http_bind != "off" && !http_bind.is_empty() {
            let addr: SocketAddr = http_bind.parse().expect("ATLAS_GATEWAY_HTTP_BIND");
            tokio::spawn(async move {
                if let Err(e) = serve_http(gw, addr).await {
                    warn!("gateway HTTP exited: {e}");
                }
            });
            info!(%addr, "embedded gateway HTTP enabled (VNC stub and /vnc/<token>)");
        }
    }

    // B1: loopback RuntimeHint ingest alongside WS. Set ATLAS_HUB_EVENT_BIND=off to disable.
    let event_bind_raw =
        std::env::var("ATLAS_HUB_EVENT_BIND").unwrap_or_else(|_| "127.0.0.1:7701".into());
    if event_bind_raw != "off" && !event_bind_raw.is_empty() {
        match EventIngestConfig::from_env() {
            Ok(cfg) => {
                let hub_ingest = Arc::clone(&hub);
                tokio::spawn(async move {
                    if let Err(e) = serve_event_ingest(hub_ingest, cfg).await {
                        warn!("event ingest exited: {e}");
                    }
                });
            }
            Err(e) => panic!("B1 event ingest config error: {e}"),
        }
    } else {
        info!("B1 event ingest disabled (ATLAS_HUB_EVENT_BIND=off)");
    }

    let app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/ws", get(ws_upgrade))
        .route("/auth/wecom/exchange", post(wecom_exchange_route))
        .layer(TraceLayer::new_for_http())
        .with_state(AppState { hub, auth, wecom });

    let listener = tokio::net::TcpListener::bind(bind).await.expect("bind");
    info!(%bind, "atlas-bot-hub listening (WS /ws + POST /auth/wecom/exchange)");
    axum::serve(listener, app).await.expect("serve");
}

async fn wecom_exchange_route(
    State(st): State<AppState>,
    Json(req): Json<wecom_exchange::ExchangeRequest>,
) -> axum::response::Response {
    wecom_exchange::exchange_handler(st.wecom.clone(), Json(req)).await
}

async fn ws_upgrade(
    ws: WebSocketUpgrade,
    State(st): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    // Credential channel: Authorization: Bearer only (also accepted on WS upgrade).
    let bearer = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| bearer_from_authorization(Some(v)));
    let hub = Arc::clone(&st.hub);
    let auth = st.auth.clone();
    ws.on_upgrade(move |socket| handle_socket(socket, hub, auth, bearer))
}

async fn handle_socket(
    socket: WebSocket,
    hub: Arc<Hub>,
    auth: AuthConfig,
    bearer: Option<String>,
) {
    let (mut sink, mut stream) = socket.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    let mut session = Session::with_auth(Arc::clone(&hub), auth, bearer);
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
