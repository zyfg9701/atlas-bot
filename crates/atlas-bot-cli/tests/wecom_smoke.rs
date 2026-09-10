//! W1 WeCom ticket smoke: mock-wecom → Hub exchange → static Bearer → hello user_id=sub.
//!
//! `cargo test -p atlas-bot-cli --test wecom_smoke -- --nocapture --test-threads=1`

use std::net::SocketAddr;
use std::sync::Arc;

use atlas_bot_auth_client::mock_wecom::{
    drive_wecom_authorize_for_code, start_mock_wecom, MOCK_AGENT_ID, MOCK_CORP_ID, MOCK_SECRET,
    MOCK_USERID,
};
use atlas_bot_auth_client::{
    build_wecom_authorize_request, exchange_wecom_code, peek_sub, save_credentials, wecom_subject,
    delete_credentials, WeComClientConfig, StoredCredentials,
};
use atlas_bot_cli::HubClient;
use atlas_bot_gateway::InMemoryGateway;
use atlas_bot_hub::auth::{bearer_from_authorization, AuthConfig};
use atlas_bot_hub::wecom_exchange::{self, WeComHubConfig};
use atlas_bot_hub::{Hub, Session};
use axum::extract::ws::{Message, WebSocket};
use axum::extract::{State, WebSocketUpgrade};
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use uuid::Uuid;

const JWT_SECRET: &str = "atlas-w1-smoke-jwt-secret";

#[derive(Clone)]
struct AppState {
    hub: Arc<Hub>,
    auth: AuthConfig,
    wecom: WeComHubConfig,
}

async fn ws_upgrade(
    ws: WebSocketUpgrade,
    State(st): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
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
    let mut session = Session::with_auth(hub.clone(), auth, bearer);
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
                        for o in session.on_text(&t).await {
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

async fn wecom_route(
    State(st): State<AppState>,
    Json(req): Json<wecom_exchange::ExchangeRequest>,
) -> axum::response::Response {
    wecom_exchange::exchange_handler(Some(st.wecom.clone()), Json(req)).await
}

async fn start_hub_with_wecom(mock_api_base: &str) -> SocketAddr {
    let gw = Arc::new(InMemoryGateway::new());
    let hub = Hub::new(gw);
    let auth = AuthConfig::static_hs256(JWT_SECRET);
    let wecom = WeComHubConfig {
        corp_id: MOCK_CORP_ID.into(),
        agent_id: MOCK_AGENT_ID.into(),
        secret: MOCK_SECRET.into(),
        api_base: mock_api_base.trim_end_matches('/').to_string(),
        jwt_secret: JWT_SECRET.into(),
        jwt_ttl_secs: 3600,
    };

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = Router::new()
        .route("/ws", get(ws_upgrade))
        .route("/auth/wecom/exchange", post(wecom_route))
        .with_state(AppState { hub, auth, wecom });
    tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });
    tokio::task::yield_now().await;
    addr
}

#[tokio::test]
async fn wecom_smoke_mock_exchange_bearer_hello() {
    let cred_dir = std::env::temp_dir().join(format!("atlas-wecom-smoke-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&cred_dir);
    std::fs::create_dir_all(&cred_dir).unwrap();
    std::env::set_var("ATLAS_BOT_CONFIG_DIR", &cred_dir);

    let mock = start_mock_wecom().await;
    let hub_addr = start_hub_with_wecom(&mock.base_url()).await;
    let hub_http = format!("http://{hub_addr}");
    let hub_ws = format!("ws://{hub_addr}/ws");

    let cfg = WeComClientConfig {
        corp_id: MOCK_CORP_ID.into(),
        agent_id: MOCK_AGENT_ID.into(),
        authorize_base: mock.base_url(),
        hub_http_base: hub_http.clone(),
    };
    let redirect = "http://127.0.0.1:9/callback";
    let state = Uuid::new_v4().to_string();
    let auth_req = build_wecom_authorize_request(&cfg, redirect, &state).expect("auth url");

    let (code, got_state) = drive_wecom_authorize_for_code(&auth_req.url)
        .await
        .expect("drive authorize");
    assert_eq!(got_state.as_deref(), Some(state.as_str()));

    let tr = exchange_wecom_code(&hub_http, &code, Some(&state))
        .await
        .expect("hub exchange");
    let expect_sub = wecom_subject(MOCK_CORP_ID, MOCK_USERID);
    assert_eq!(tr.subject.as_deref(), Some(expect_sub.as_str()));
    let sub = peek_sub(&tr.access_token).expect("jwt sub");
    assert_eq!(sub, expect_sub);
    println!("SMOKE_OK wecom mock exchange sub={sub}");

    let stored = StoredCredentials {
        access_token: tr.access_token.clone(),
        id_token: None,
        refresh_token: None,
        expires_at: None,
        token_type: Some("Bearer".into()),
        subject: Some(sub.clone()),
        provider: Some("wecom".into()),
    };
    save_credentials(&stored).unwrap();
    assert_eq!(
        atlas_bot_auth_client::load_credentials()
            .unwrap()
            .unwrap()
            .provider
            .as_deref(),
        Some("wecom")
    );

    let client = HubClient::connect_with_bearer(&hub_ws, Some(&tr.access_token))
        .await
        .expect("hub connect");
    let ack = client.hello_ack();
    assert_eq!(ack["user_id"], expect_sub);
    println!("SMOKE_OK wecom mock exchange→bearer→hello user_id=sub");

    // Bad / missing token under static → unauthorized
    let err = match HubClient::connect_with_bearer(&hub_ws, None).await {
        Ok(_) => panic!("missing bearer must fail under static"),
        Err(e) => e,
    };
    let line = err.display_line();
    assert!(
        line.contains("unauthorized"),
        "expected unauthorized, got {line}"
    );
    println!("SMOKE_OK wecom missing token → unauthorized");

    // Opaque WeCom-style token must NOT be accepted as Hub Bearer (we only accept minted JWT)
    let bad = HubClient::connect_with_bearer(&hub_ws, Some("not-a-jwt-opaque-wecom-token")).await;
    assert!(bad.is_err());
    println!("SMOKE_OK wecom opaque token rejected");

    delete_credentials().unwrap();
    std::env::remove_var("ATLAS_BOT_CONFIG_DIR");
    let _ = std::fs::remove_dir_all(&cred_dir);

    println!("SMOKE_OK wecom login cli mock-wecom code→exchange→bearer→hello");
}
