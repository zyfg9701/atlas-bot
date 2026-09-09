//! B1 loopback RuntimeHint ingest (`POST /internal/runtime-hint`).
//!
//! Bind is fail-closed to loopback only. Auth: shared token, or explicit
//! `ATLAS_HUB_EVENT_ALLOW_INSECURE_LOOPBACK=1` when token unset.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use atlas_bot_gateway::RuntimeHint;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::post;
use axum::Router;
use tracing::{info, warn};

use crate::Hub;

/// Default bind for Hub event ingest (separate from WS `:7700`).
pub const DEFAULT_EVENT_BIND: &str = "127.0.0.1:7701";
pub const ENV_EVENT_BIND: &str = "ATLAS_HUB_EVENT_BIND";
pub const ENV_EVENT_TOKEN: &str = "ATLAS_HUB_EVENT_TOKEN";
pub const ENV_ALLOW_INSECURE: &str = "ATLAS_HUB_EVENT_ALLOW_INSECURE_LOOPBACK";
/// Body size cap (64 KiB).
pub const EVENT_BODY_MAX_BYTES: usize = 64 * 1024;

#[derive(Clone)]
struct IngestState {
    hub: Arc<Hub>,
    token: Option<String>,
    allow_insecure: bool,
}

#[derive(Debug, Clone)]
pub struct EventIngestConfig {
    pub bind: SocketAddr,
    pub token: Option<String>,
    pub allow_insecure: bool,
}

impl EventIngestConfig {
    pub fn from_env() -> Result<Self, String> {
        let bind_s = std::env::var(ENV_EVENT_BIND).unwrap_or_else(|_| DEFAULT_EVENT_BIND.into());
        let bind: SocketAddr = bind_s
            .parse()
            .map_err(|e| format!("invalid {ENV_EVENT_BIND}={bind_s}: {e}"))?;
        if !is_loopback(bind.ip()) {
            return Err(format!(
                "ATLAS_HUB_EVENT_BIND must be loopback (got {bind}); refuse non-loopback ingest"
            ));
        }
        let token = std::env::var(ENV_EVENT_TOKEN)
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let allow_insecure = std::env::var(ENV_ALLOW_INSECURE)
            .ok()
            .map(|s| s == "1" || s.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        // Listener always starts (loopback-only). Requests without token are
        // rejected unless ATLAS_HUB_EVENT_ALLOW_INSECURE_LOOPBACK=1.
        if token.is_none() && !allow_insecure {
            warn!(
                "{ENV_EVENT_TOKEN} unset and {ENV_ALLOW_INSECURE}!=1: ingest will reject until configured"
            );
        }
        Ok(Self {
            bind,
            token,
            allow_insecure,
        })
    }

    /// Test helper: loopback bind + insecure allowed (or explicit token).
    pub fn for_test(bind: SocketAddr, token: Option<String>) -> Result<Self, String> {
        if !is_loopback(bind.ip()) {
            return Err(format!("event ingest bind must be loopback, got {bind}"));
        }
        let allow_insecure = token.is_none();
        Ok(Self {
            bind,
            token,
            allow_insecure,
        })
    }
}

pub fn is_loopback(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_loopback(),
        IpAddr::V6(v6) => v6.is_loopback(),
    }
}

fn extract_event_token(headers: &HeaderMap) -> Option<String> {
    if let Some(v) = headers.get("x-atlas-event-token").and_then(|v| v.to_str().ok()) {
        let t = v.trim();
        if !t.is_empty() {
            return Some(t.to_string());
        }
    }
    if let Some(v) = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    {
        let v = v.trim();
        if let Some(rest) = v.strip_prefix("Bearer ").or_else(|| v.strip_prefix("bearer ")) {
            let t = rest.trim();
            if !t.is_empty() {
                return Some(t.to_string());
            }
        }
    }
    None
}

async fn post_runtime_hint(
    State(st): State<IngestState>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    if body.len() > EVENT_BODY_MAX_BYTES {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            axum::Json(serde_json::json!({ "error": "body_too_large" })),
        )
            .into_response();
    }

    if let Some(ref expected) = st.token {
        match extract_event_token(&headers) {
            Some(got) if got == *expected => {}
            _ => {
                return (
                    StatusCode::UNAUTHORIZED,
                    axum::Json(serde_json::json!({ "error": "unauthorized" })),
                )
                    .into_response();
            }
        }
    } else if !st.allow_insecure {
        return (
            StatusCode::FORBIDDEN,
            axum::Json(serde_json::json!({
                "error": "insecure_loopback_disabled",
                "hint": format!("set {ENV_EVENT_TOKEN} or {ENV_ALLOW_INSECURE}=1"),
            })),
        )
            .into_response();
    }

    let hint: RuntimeHint = match serde_json::from_slice(&body) {
        Ok(h) => h,
        Err(e) => {
            warn!(error = %e, "B1 ingest: bad RuntimeHint JSON");
            return (
                StatusCode::BAD_REQUEST,
                axum::Json(serde_json::json!({ "error": "invalid_runtime_hint", "detail": e.to_string() })),
            )
                .into_response();
        }
    };

    st.hub.apply_runtime_hint(hint).await;
    StatusCode::NO_CONTENT.into_response()
}

pub fn event_ingest_router(hub: Arc<Hub>, cfg: &EventIngestConfig) -> Router {
    let state = IngestState {
        hub,
        token: cfg.token.clone(),
        allow_insecure: cfg.allow_insecure,
    };
    Router::new()
        .route("/internal/runtime-hint", post(post_runtime_hint))
        .route("/healthz", axum::routing::get(|| async { "ok" }))
        .layer(DefaultBodyLimit::max(EVENT_BODY_MAX_BYTES))
        .with_state(state)
}

/// Bind and serve the B1 ingest listener. Caller should spawn.
pub async fn serve_event_ingest(
    hub: Arc<Hub>,
    cfg: EventIngestConfig,
) -> Result<(), std::io::Error> {
    if !is_loopback(cfg.bind.ip()) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("event ingest bind must be loopback, got {}", cfg.bind),
        ));
    }
    let app = event_ingest_router(Arc::clone(&hub), &cfg);
    let listener = tokio::net::TcpListener::bind(cfg.bind).await?;
    info!(
        bind = %cfg.bind,
        token = cfg.token.is_some(),
        insecure = cfg.allow_insecure,
        "atlas-bot-hub event ingest listening (B1)"
    );
    axum::serve(listener, app).await
}
