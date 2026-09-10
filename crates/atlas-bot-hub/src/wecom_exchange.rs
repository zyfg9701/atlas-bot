//! W1 Hub thin WeCom exchange: code → userid (WeCom API or mock) → short JWT.
//!
//! `POST /auth/wecom/exchange` — secret stays on Hub. No registration/org/billing.
//! Hot path WS auth still uses I1 Bearer only (no W2 introspection).

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::{info, warn};

use crate::auth::mint_hs256;

/// Default TTL for exchanged Hub JWT (seconds).
pub const DEFAULT_JWT_TTL: i64 = 3600;

#[derive(Debug, Clone)]
pub struct WeComHubConfig {
    pub corp_id: String,
    pub agent_id: String,
    pub secret: String,
    /// API base: live `https://qyapi.weixin.qq.com` or mock-wecom base.
    pub api_base: String,
    /// HS256 secret for minting Hub-verifiable JWT (`ATLAS_AUTH_JWT_SECRET`).
    pub jwt_secret: String,
    pub jwt_ttl_secs: i64,
}

impl WeComHubConfig {
    /// Load from env. Returns `None` if WeCom exchange is not configured.
    pub fn from_env() -> Option<Self> {
        let corp_id = std::env::var("ATLAS_WECOM_CORP_ID").ok().filter(|s| !s.is_empty())?;
        let agent_id = std::env::var("ATLAS_WECOM_AGENT_ID").ok().filter(|s| !s.is_empty())?;
        let secret = std::env::var("ATLAS_WECOM_SECRET").ok().filter(|s| !s.is_empty())?;
        let jwt_secret = std::env::var("ATLAS_AUTH_JWT_SECRET")
            .ok()
            .filter(|s| !s.is_empty())?;
        let api_base = std::env::var("ATLAS_WECOM_API_BASE")
            .unwrap_or_else(|_| "https://qyapi.weixin.qq.com".into())
            .trim_end_matches('/')
            .to_string();
        let jwt_ttl_secs = std::env::var("ATLAS_WECOM_JWT_TTL")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(DEFAULT_JWT_TTL);
        Some(Self {
            corp_id,
            agent_id,
            secret,
            api_base,
            jwt_secret,
            jwt_ttl_secs,
        })
    }
}

#[derive(Debug, Deserialize)]
pub struct ExchangeRequest {
    pub code: String,
    #[serde(default)]
    pub state: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ExchangeResponse {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: i64,
    pub subject: String,
}

/// Nailed `sub` mapping.
pub fn map_subject(corp_id: &str, userid: &str) -> String {
    format!("wecom:{corp_id}:{userid}")
}

pub async fn handle_exchange(
    cfg: &WeComHubConfig,
    req: ExchangeRequest,
) -> Result<ExchangeResponse, ExchangeError> {
    if req.code.is_empty() {
        return Err(ExchangeError::BadRequest("code required".into()));
    }
    let userid = resolve_userid(cfg, &req.code).await?;
    let subject = map_subject(&cfg.corp_id, &userid);
    let token = mint_hs256(
        &cfg.jwt_secret,
        &subject,
        cfg.jwt_ttl_secs,
        None,
        None,
    )
    .map_err(|e| ExchangeError::Internal(format!("mint jwt: {e}")))?;
    info!(%subject, "wecom exchange issued hub jwt");
    let _ = &cfg.agent_id; // reserved / validated at authorize time on client
    let _ = req.state;
    Ok(ExchangeResponse {
        access_token: token,
        token_type: "Bearer".into(),
        expires_in: cfg.jwt_ttl_secs,
        subject,
    })
}

async fn resolve_userid(cfg: &WeComHubConfig, code: &str) -> Result<String, ExchangeError> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| ExchangeError::Upstream(e.to_string()))?;

    // 1) gettoken
    let token_url = format!(
        "{}/cgi-bin/gettoken?corpid={}&corpsecret={}",
        cfg.api_base,
        urlencoding_encode(&cfg.corp_id),
        urlencoding_encode(&cfg.secret)
    );
    let tok_resp = client
        .get(&token_url)
        .send()
        .await
        .map_err(|e| ExchangeError::Upstream(format!("gettoken: {e}")))?;
    let tok_body: serde_json::Value = tok_resp
        .json()
        .await
        .map_err(|e| ExchangeError::Upstream(format!("gettoken json: {e}")))?;
    let errcode = tok_body.get("errcode").and_then(|v| v.as_i64()).unwrap_or(0);
    if errcode != 0 {
        warn!(%tok_body, "wecom gettoken failed");
        return Err(ExchangeError::Upstream(format!(
            "gettoken errcode={errcode}"
        )));
    }
    let access_token = tok_body
        .get("access_token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ExchangeError::Upstream("gettoken missing access_token".into()))?;

    // 2) getuserinfo
    let info_url = format!(
        "{}/cgi-bin/user/getuserinfo?access_token={}&code={}",
        cfg.api_base,
        urlencoding_encode(access_token),
        urlencoding_encode(code)
    );
    let info_resp = client
        .get(&info_url)
        .send()
        .await
        .map_err(|e| ExchangeError::Upstream(format!("getuserinfo: {e}")))?;
    let info_body: serde_json::Value = info_resp
        .json()
        .await
        .map_err(|e| ExchangeError::Upstream(format!("getuserinfo json: {e}")))?;
    let errcode = info_body
        .get("errcode")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    if errcode != 0 {
        warn!(%info_body, "wecom getuserinfo failed");
        return Err(ExchangeError::Unauthorized(format!(
            "getuserinfo errcode={errcode}"
        )));
    }
    let userid = info_body
        .get("UserId")
        .or_else(|| info_body.get("userid"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| ExchangeError::Unauthorized("getuserinfo missing UserId".into()))?;
    Ok(userid.to_string())
}

fn urlencoding_encode(s: &str) -> String {
    // Minimal encode for query values
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[derive(Debug)]
pub enum ExchangeError {
    BadRequest(String),
    Unauthorized(String),
    Upstream(String),
    Internal(String),
    NotConfigured,
}

impl ExchangeError {
    pub fn into_response(self) -> Response {
        let (status, code, msg) = match &self {
            Self::BadRequest(m) => (StatusCode::BAD_REQUEST, "invalid_request", m.clone()),
            Self::Unauthorized(m) => (StatusCode::UNAUTHORIZED, "unauthorized", m.clone()),
            Self::Upstream(m) => (StatusCode::BAD_GATEWAY, "upstream_error", m.clone()),
            Self::Internal(m) => (StatusCode::INTERNAL_SERVER_ERROR, "internal_error", m.clone()),
            Self::NotConfigured => (
                StatusCode::SERVICE_UNAVAILABLE,
                "not_configured",
                "WeCom exchange not configured (set ATLAS_WECOM_* + ATLAS_AUTH_JWT_SECRET)".into(),
            ),
        };
        (
            status,
            Json(json!({
                "error": code,
                "detail": msg,
            })),
        )
            .into_response()
    }
}

/// Axum handler wrapper when config may be absent.
pub async fn exchange_handler(
    cfg: Option<WeComHubConfig>,
    Json(req): Json<ExchangeRequest>,
) -> Response {
    let Some(cfg) = cfg else {
        return ExchangeError::NotConfigured.into_response();
    };
    match handle_exchange(&cfg, req).await {
        Ok(body) => (StatusCode::OK, Json(body)).into_response(),
        Err(e) => e.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subject_formula() {
        assert_eq!(map_subject("ww_x", "alice"), "wecom:ww_x:alice");
    }
}
