//! I1 Hub Auth Gate smoke → SMOKE_OK idp…
//!
//! `cargo test -p atlas-bot-hub --test idp_smoke -- --nocapture --test-threads=1`
//!
//! Covers: dev regression (user_local_dev); static good → user_id=sub;
//! static bad/missing → unauthorized (never reaches bot.command success);
//! oidc-mock with ATLAS_OIDC_JWKS_JSON oct key; optional allowlist → link_required;
//! WS upgrade with Authorization: Bearer via tokio-tungstenite custom request.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use atlas_bot_gateway::{Gateway, InMemoryGateway};
use atlas_bot_hub::auth::{
    mint_hs256, mock_oct_jwks, AuthConfig, AuthMode, DEV_USER_ID, UNAUTHORIZED_NUMERIC,
};
use atlas_bot_hub::{Hub, Session};
use futures_util::{SinkExt, StreamExt};
use http::{Request, Uri};
use serde_json::{json, Value};
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMessage};

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

fn err_code(resp: &Value) -> String {
    // Bare JsonRpcError (raw hello) or JSON-RPC response error.
    if let Some(msg) = resp.get("message").and_then(|m| m.as_str()) {
        if resp.get("jsonrpc").is_none() {
            return msg.to_string();
        }
    }
    if let Some(e) = resp.get("error") {
        if let Some(c) = e.pointer("/data/code").and_then(|c| c.as_str()) {
            return c.to_string();
        }
        if let Some(m) = e.get("message").and_then(|m| m.as_str()) {
            return m.to_string();
        }
    }
    if let Some(c) = resp.pointer("/data/code").and_then(|c| c.as_str()) {
        return c.to_string();
    }
    panic!("no error code in {resp}");
}

#[tokio::test]
async fn idp_smoke_dev_static_oidc_allowlist_ws() {
    // ── 1) dev regression ──────────────────────────────────────────────
    {
        let (hub, _) = Hub::with_in_memory_gateway();
        let mut session = Session::with_auth(hub, AuthConfig::dev(), None);
        let hello = session
            .on_text(r#"{"protocol_version":"1.0.0","kind":"bot_client"}"#)
            .await;
        assert_eq!(hello.len(), 1);
        let ack: Value = serde_json::from_str(&hello[0]).unwrap();
        assert_eq!(ack["user_id"], DEV_USER_ID);
        let st = rpc(&mut session, 1, "bot.status", json!({})).await;
        assert!(st.get("result").is_some());
        println!("SMOKE_OK idp dev user_local_dev");
    }

    // ── 2) static good → user_id = sub ─────────────────────────────────
    let secret = "idp-smoke-secret";
    {
        let (hub, _) = Hub::with_in_memory_gateway();
        let tok = mint_hs256(secret, "user_alice", 3600, None, None).unwrap();
        let mut session =
            Session::with_auth(hub, AuthConfig::static_hs256(secret), Some(tok));
        let hello = session
            .on_text(r#"{"protocol_version":"1.0.0","kind":"bot_client"}"#)
            .await;
        let ack: Value = serde_json::from_str(&hello[0]).unwrap();
        assert_eq!(ack["user_id"], "user_alice");
        println!("SMOKE_OK idp static good user_id=sub");
    }

    // ── 3) static missing → unauthorized (no bot.command success) ──────
    {
        let (hub, gw) = Hub::with_in_memory_gateway();
        let before = gw.invoke_count();
        let mut session = Session::with_auth(hub, AuthConfig::static_hs256(secret), None);
        let hello = session
            .on_text(r#"{"protocol_version":"1.0.0","kind":"bot_client"}"#)
            .await;
        let err: Value = serde_json::from_str(&hello[0]).unwrap();
        assert_eq!(err_code(&err), "unauthorized");
        assert_eq!(err["code"], UNAUTHORIZED_NUMERIC);

        let cmd = rpc(
            &mut session,
            99,
            "bot.command",
            json!({
                "agentId": "agt_1",
                "name": "listAgents",
                "args": {}
            }),
        )
        .await;
        assert!(cmd.get("error").is_some(), "must not succeed: {cmd}");
        assert_eq!(err_code(&cmd), "unauthorized");
        assert_eq!(
            gw.invoke_count(),
            before,
            "bad/missing auth must not wake gateway / bot.command"
        );
        println!("SMOKE_OK idp static missing → unauthorized");
    }

    // ── 4) static bad token → unauthorized ─────────────────────────────
    {
        let (hub, _) = Hub::with_in_memory_gateway();
        let mut session = Session::with_auth(
            hub,
            AuthConfig::static_hs256(secret),
            Some("not.a.valid.jwt".into()),
        );
        let hello = session
            .on_text(r#"{"protocol_version":"1.0.0","kind":"bot_client"}"#)
            .await;
        let err: Value = serde_json::from_str(&hello[0]).unwrap();
        assert_eq!(err_code(&err), "unauthorized");
        println!("SMOKE_OK idp static bad → unauthorized");
    }

    // ── 5) oidc-mock (ATLAS_OIDC_JWKS_JSON oct) ─────────────────────────
    {
        let issuer = "https://idp.example.test";
        let aud = "atlas-hub";
        let jwks = mock_oct_jwks("smoke-k1", secret);
        let cfg = AuthConfig::oidc_mock(issuer, Some(aud.into()), &jwks).expect("oidc mock config");
        let tok = mint_hs256(secret, "user_oidc", 3600, Some(issuer), Some(aud)).unwrap();
        let (hub, _) = Hub::with_in_memory_gateway();
        let mut session = Session::with_auth(hub, cfg, Some(tok));
        let hello = session
            .on_text(r#"{"protocol_version":"1.0.0","kind":"bot_client"}"#)
            .await;
        let ack: Value = serde_json::from_str(&hello[0]).unwrap();
        assert_eq!(ack["user_id"], "user_oidc");
        println!("SMOKE_OK idp oidc-mock user_id=sub");
    }

    // ── 6) allowlist → link_required / not_enrolled ────────────────────
    {
        let (hub, _) = Hub::with_in_memory_gateway();
        let tok = mint_hs256(secret, "user_stranger", 3600, None, None).unwrap();
        let cfg = AuthConfig::static_hs256(secret).with_allowlist(vec!["user_alice".into()]);
        let mut session = Session::with_auth(hub, cfg, Some(tok));
        let hello = session
            .on_text(r#"{"protocol_version":"1.0.0","kind":"bot_client"}"#)
            .await;
        let err: Value = serde_json::from_str(&hello[0]).unwrap();
        assert_eq!(err_code(&err), "link_required");
        assert_eq!(err.pointer("/data/reason").and_then(|r| r.as_str()), Some("not_enrolled"));
        println!("SMOKE_OK idp allowlist → link_required not_enrolled");
    }

    // ── 7) WS upgrade + Authorization header (tokio-tungstenite) ───────
    {
        let gw = Arc::new(InMemoryGateway::new());
        let hub = Hub::new(gw);
        let auth = AuthConfig::static_hs256(secret);
        let app = {
            use atlas_bot_hub::auth::bearer_from_authorization;
            use axum::extract::ws::Message;
            use axum::extract::{State, WebSocketUpgrade};
            use axum::http::HeaderMap;
            use axum::response::IntoResponse;
            use axum::routing::get;
            use axum::Router;
            use tokio::sync::mpsc;

            #[derive(Clone)]
            struct St {
                hub: Arc<Hub>,
                auth: AuthConfig,
            }

            async fn upgrade(
                ws: WebSocketUpgrade,
                State(st): State<St>,
                headers: HeaderMap,
            ) -> impl IntoResponse {
                let bearer = headers
                    .get(axum::http::header::AUTHORIZATION)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| bearer_from_authorization(Some(v)));
                let hub = Arc::clone(&st.hub);
                let auth = st.auth.clone();
                ws.on_upgrade(move |socket| async move {
                    let (mut sink, mut stream) = socket.split();
                    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
                    let mut session = Session::with_auth(hub, auth, bearer);
                    let writer = tokio::spawn(async move {
                        while let Some(text) = rx.recv().await {
                            if sink.send(Message::Text(text.into())).await.is_err() {
                                break;
                            }
                        }
                    });
                    while let Some(Ok(Message::Text(t))) = stream.next().await {
                        for o in session.on_text(&t).await {
                            if tx.send(o).is_err() {
                                break;
                            }
                        }
                    }
                    drop(tx);
                    let _ = writer.await;
                })
            }

            Router::new()
                .route("/ws", get(upgrade))
                .with_state(St {
                    hub: hub.clone(),
                    auth,
                })
        };

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr: SocketAddr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        tokio::time::sleep(Duration::from_millis(50)).await;

        let tok = mint_hs256(secret, "user_ws", 3600, None, None).unwrap();
        let uri: Uri = format!("ws://{addr}/ws").parse().unwrap();
        let req = Request::builder()
            .uri(uri)
            .header("authorization", format!("Bearer {tok}"))
            .header("host", format!("{addr}"))
            .header("connection", "Upgrade")
            .header("upgrade", "websocket")
            .header("sec-websocket-version", "13")
            .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
            .body(())
            .unwrap();
        let (mut ws, _) = connect_async(req).await.expect("ws connect with Authorization");
        ws.send(WsMessage::Text(
            r#"{"protocol_version":"1.0.0","kind":"bot_client"}"#.into(),
        ))
        .await
        .unwrap();
        let msg = ws.next().await.expect("ack").expect("ok");
        let text = match msg {
            WsMessage::Text(t) => t.to_string(),
            other => panic!("unexpected {other:?}"),
        };
        let ack: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(ack["user_id"], "user_ws");
        println!("SMOKE_OK idp ws-upgrade Authorization Bearer user_id=sub");
    }

    // Touch AuthMode so unused-import / dead_code stay quiet in this harness.
    assert_eq!(AuthMode::Dev.as_str(), "dev");

    println!(
        "SMOKE_OK idp gate dev+static+oidc-mock+allowlist+ws-bearer"
    );
}
