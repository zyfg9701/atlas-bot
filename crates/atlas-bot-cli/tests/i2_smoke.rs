//! I2.1 login smoke: mock OIDC → PKCE exchange → Hub oidc → hello user_id=sub.
//!
//! `cargo test -p atlas-bot-cli --test i2_smoke -- --nocapture --test-threads=1`

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use atlas_bot_auth_client::mock_oidc::{
    drive_authorize_for_code, start_mock_oidc, MOCK_AUD, MOCK_SUB,
};
use atlas_bot_auth_client::{
    build_authorize_request, exchange_code, peek_sub, pick_hub_bearer, start_loopback,
    wait_for_callback, OidcClientConfig, PkcePair, StoredCredentials, save_credentials,
    delete_credentials, load_bearer,
};
use atlas_bot_cli::HubClient;
use atlas_bot_gateway::InMemoryGateway;
use atlas_bot_hub::auth::{bearer_from_authorization, AuthConfig};
use atlas_bot_hub::{Hub, Session};
use axum::extract::ws::{Message, WebSocket};
use axum::extract::{State, WebSocketUpgrade};
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use uuid::Uuid;

#[derive(Clone)]
struct AppState {
    hub: Arc<Hub>,
    auth: AuthConfig,
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

async fn start_hub_oidc(jwks: &str, issuer: &str, audience: &str) -> SocketAddr {
    let gw = Arc::new(InMemoryGateway::new());
    let hub = Hub::new(gw);
    let auth = AuthConfig::oidc_mock(issuer, Some(audience.into()), jwks).expect("oidc cfg");

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = Router::new()
        .route("/ws", get(ws_upgrade))
        .with_state(AppState { hub, auth });
    tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });
    tokio::task::yield_now().await;
    addr
}

#[tokio::test]
async fn i2_smoke_pkce_login_hub_hello_sub() {
    // Isolated credentials dir for this test.
    let cred_dir = std::env::temp_dir().join(format!("atlas-i2-smoke-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&cred_dir);
    std::fs::create_dir_all(&cred_dir).unwrap();
    std::env::set_var("ATLAS_BOT_CONFIG_DIR", &cred_dir);

    let mock = start_mock_oidc().await;
    let issuer = mock.issuer.clone();
    let jwks = mock.jwks_json();

    // ── PKCE loopback path (also exercises callback server) ───────────
    let (_addr, redirect_uri, wait) = start_loopback(0).await.expect("loopback");
    let cfg = OidcClientConfig::from_issuer(&issuer, "atlas-bot-cli").with_audience(MOCK_AUD);
    let pkce = PkcePair::generate();
    let state = Uuid::new_v4().to_string();
    let auth_req =
        build_authorize_request(&cfg, &redirect_uri, pkce.clone(), &state).expect("auth url");

    // Drive mock authorize → it redirects to our loopback with ?code=
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .unwrap();
    // Fire-and-forget GET that follows redirect into loopback.
    let url = auth_req.url.clone();
    tokio::spawn(async move {
        let _ = client.get(url).send().await;
    });

    let cb = wait_for_callback(wait, Duration::from_secs(10))
        .await
        .expect("callback code");
    assert_eq!(cb.state.as_deref(), Some(state.as_str()));

    let tr = exchange_code(&cfg, &redirect_uri, &cb.code, &pkce.verifier)
        .await
        .expect("token exchange");
    let bearer = pick_hub_bearer(&tr).expect("hub bearer");
    assert!(
        tr.id_token.as_deref() == Some(bearer.as_str()),
        "nail: prefer id_token when present"
    );
    let sub = peek_sub(&bearer).expect("sub");
    assert_eq!(sub, MOCK_SUB);

    let stored = StoredCredentials {
        access_token: bearer.clone(),
        id_token: tr.id_token.clone(),
        refresh_token: None,
        expires_at: None,
        token_type: Some("Bearer".into()),
        subject: Some(sub.clone()),
        provider: Some("oidc".into()),
    };
    save_credentials(&stored).unwrap();
    assert_eq!(load_bearer().unwrap().as_deref(), Some(bearer.as_str()));
    println!("SMOKE_OK i2 pkce-loopback+exchange id_token sub={sub}");

    // ── Hub oidc + Bearer → hello user_id=sub ─────────────────────────
    let hub_addr = start_hub_oidc(&jwks, &issuer, MOCK_AUD).await;
    let hub_url = format!("ws://{hub_addr}/ws");
    let client = HubClient::connect_with_bearer(&hub_url, Some(&bearer))
        .await
        .expect("hub connect with bearer");
    let ack = client.hello_ack();
    assert_eq!(ack["user_id"], MOCK_SUB);
    println!("SMOKE_OK i2 hub oidc hello user_id=sub");

    // ── missing token → unauthorized ──────────────────────────────────
    let err = match HubClient::connect_with_bearer(&hub_url, None).await {
        Ok(_) => panic!("missing bearer must fail under oidc"),
        Err(e) => e,
    };
    let line = err.display_line();
    assert!(
        line.contains("unauthorized") || line.contains("unauthorized"),
        "expected unauthorized, got {line}"
    );
    println!("SMOKE_OK i2 missing token → unauthorized");

    // ── NO_BROWSER drive path (CI primary) ────────────────────────────
    let (_a2, redirect2, wait2) = start_loopback(0).await.unwrap();
    drop(wait2); // drive path exchanges without loopback hit
    let pkce2 = PkcePair::generate();
    let state2 = Uuid::new_v4().to_string();
    let auth2 = build_authorize_request(&cfg, &redirect2, pkce2.clone(), &state2).unwrap();
    let (code2, st2) = drive_authorize_for_code(&auth2.url).await.expect("drive");
    assert_eq!(st2.as_deref(), Some(state2.as_str()));
    let tr2 = exchange_code(&cfg, &redirect2, &code2, &pkce2.verifier)
        .await
        .unwrap();
    let bearer2 = pick_hub_bearer(&tr2).unwrap();
    let client2 = HubClient::connect_with_bearer(&hub_url, Some(&bearer2))
        .await
        .unwrap();
    assert_eq!(client2.hello_ack()["user_id"], MOCK_SUB);
    println!("SMOKE_OK i2 no-browser drive authorize → hub hello sub");

    delete_credentials().unwrap();
    std::env::remove_var("ATLAS_BOT_CONFIG_DIR");
    let _ = std::fs::remove_dir_all(&cred_dir);

    println!("SMOKE_OK i2 login pc+cli mock-oidc pkce→bearer→hello");
}
