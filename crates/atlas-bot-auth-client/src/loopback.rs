//! Loopback HTTP callback server for OIDC redirect (`http://127.0.0.1:<port>/callback`).

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Query, State};
use axum::response::Html;
use axum::routing::get;
use axum::Router;
use serde::Deserialize;
use thiserror::Error;
use tokio::sync::{oneshot, Mutex};

#[derive(Debug, Error)]
pub enum LoopbackError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("timeout waiting for OIDC callback")]
    Timeout,
    #[error("callback channel closed")]
    Closed,
    #[error("oidc error: {0}")]
    Oidc(String),
    #[error("state mismatch")]
    StateMismatch,
}

#[derive(Debug, Clone)]
pub struct CallbackResult {
    pub code: String,
    pub state: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

struct CbState {
    tx: Mutex<Option<oneshot::Sender<Result<CallbackResult, LoopbackError>>>>,
}

/// Bind `127.0.0.1:port` (`port=0` → ephemeral) and wait for `/callback`.
///
/// Returns `(bound_addr, redirect_uri, future that resolves when code arrives)`.
pub async fn start_loopback(
    port: u16,
) -> Result<
    (
        SocketAddr,
        String,
        impl std::future::Future<Output = Result<CallbackResult, LoopbackError>>,
    ),
    LoopbackError,
> {
    let listener = tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], port))).await?;
    let addr = listener.local_addr()?;
    let redirect_uri = format!("http://{addr}/callback");

    let (tx, rx) = oneshot::channel();
    let st = Arc::new(CbState {
        tx: Mutex::new(Some(tx)),
    });

    let app = Router::new()
        .route("/callback", get(callback_handler))
        .with_state(st);

    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    // Brief yield so the accept loop is live before the browser hits it.
    tokio::task::yield_now().await;

    let wait = async move {
        rx.await.map_err(|_| LoopbackError::Closed)?
    };

    Ok((addr, redirect_uri, wait))
}

/// Wait with timeout wrapper.
pub async fn wait_for_callback<F>(
    wait: F,
    timeout: Duration,
) -> Result<CallbackResult, LoopbackError>
where
    F: std::future::Future<Output = Result<CallbackResult, LoopbackError>>,
{
    match tokio::time::timeout(timeout, wait).await {
        Ok(r) => r,
        Err(_) => Err(LoopbackError::Timeout),
    }
}

async fn callback_handler(
    State(st): State<Arc<CbState>>,
    Query(q): Query<CallbackQuery>,
) -> Html<&'static str> {
    let result = if let Some(err) = q.error {
        let detail = q
            .error_description
            .unwrap_or_else(|| err.clone());
        Err(LoopbackError::Oidc(detail))
    } else if let Some(code) = q.code {
        Ok(CallbackResult {
            code,
            state: q.state,
        })
    } else {
        Err(LoopbackError::Oidc("missing code".into()))
    };

    if let Some(tx) = st.tx.lock().await.take() {
        let _ = tx.send(result);
    }

    Html(
        "<!doctype html><html><body><h1>atlas-bot login</h1><p>You can close this window.</p></body></html>",
    )
}

/// Port from `ATLAS_OIDC_REDIRECT_PORT` or `0` (ephemeral).
pub fn redirect_port_from_env() -> u16 {
    std::env::var("ATLAS_OIDC_REDIRECT_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}
