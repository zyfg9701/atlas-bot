//! WeCom ticket provider (W1): authorize URL + Hub thin exchange.
//!
//! Shape mirrors OIDC Login: authorize/扫码 → code → Hub exchange → local store → WS Bearer.
//! WeCom web OAuth typically lacks PKCE; we use **state + one-time code** and Hub-side secret.

use serde::{Deserialize, Serialize};
use thiserror::Error;
use url::Url;

#[derive(Debug, Error)]
pub enum WeComError {
    #[error("url: {0}")]
    Url(#[from] url::ParseError),
    #[error("http: {0}")]
    Http(String),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Message(String),
}

/// Client-side WeCom config (no secret).
#[derive(Debug, Clone)]
pub struct WeComClientConfig {
    pub corp_id: String,
    pub agent_id: String,
    /// Base for authorize page (mock: `http://127.0.0.1:PORT`; live: open.weixin.qq.com host path).
    pub authorize_base: String,
    /// Hub HTTP base for exchange (e.g. `http://127.0.0.1:7700`).
    pub hub_http_base: String,
}

impl WeComClientConfig {
    pub fn from_env() -> Result<Self, WeComError> {
        let corp_id = std::env::var("ATLAS_WECOM_CORP_ID")
            .map_err(|_| WeComError::Message("ATLAS_WECOM_CORP_ID required".into()))?;
        let agent_id = std::env::var("ATLAS_WECOM_AGENT_ID")
            .map_err(|_| WeComError::Message("ATLAS_WECOM_AGENT_ID required".into()))?;
        let authorize_base = std::env::var("ATLAS_WECOM_AUTHORIZE_BASE")
            .or_else(|_| std::env::var("ATLAS_WECOM_API_BASE"))
            .unwrap_or_else(|_| "https://open.weixin.qq.com/connect/oauth2".into());
        let hub_http_base = hub_http_base_from_env()
            .ok_or_else(|| WeComError::Message(
                "set ATLAS_HUB_HTTP or ATLAS_HUB_WS to derive Hub exchange URL".into(),
            ))?;
        Ok(Self {
            corp_id,
            agent_id,
            authorize_base: authorize_base.trim_end_matches('/').to_string(),
            hub_http_base: hub_http_base.trim_end_matches('/').to_string(),
        })
    }
}

/// Derive Hub HTTP base from `ATLAS_HUB_HTTP` or `ATLAS_HUB_WS`.
pub fn hub_http_base_from_env() -> Option<String> {
    if let Ok(h) = std::env::var("ATLAS_HUB_HTTP") {
        if !h.is_empty() {
            return Some(h.trim_end_matches('/').to_string());
        }
    }
    let ws = std::env::var("ATLAS_HUB_WS").ok()?;
    Some(ws_to_http_base(&ws))
}

pub fn ws_to_http_base(ws: &str) -> String {
    let mut s = ws.trim().to_string();
    if let Some(rest) = s.strip_prefix("ws://") {
        s = format!("http://{rest}");
    } else if let Some(rest) = s.strip_prefix("wss://") {
        s = format!("https://{rest}");
    }
    if let Some(stripped) = s.strip_suffix("/ws") {
        s = stripped.to_string();
    }
    s.trim_end_matches('/').to_string()
}

#[derive(Debug, Clone)]
pub struct WeComAuthorizeRequest {
    pub url: String,
    pub state: String,
    pub redirect_uri: String,
}

/// Build WeCom (or mock-wecom) authorize URL.
///
/// Live shape (approx): `{base}/authorize?appid=CORPID&redirect_uri=...&response_type=code&scope=snsapi_base&state=...&agentid=...`
/// Mock uses the same query params on `{mock}/authorize`.
pub fn build_wecom_authorize_request(
    cfg: &WeComClientConfig,
    redirect_uri: &str,
    state: impl Into<String>,
) -> Result<WeComAuthorizeRequest, WeComError> {
    let state = state.into();
    let base = cfg.authorize_base.trim_end_matches('/');
    let authorize = if base.ends_with("/authorize") {
        base.to_string()
    } else {
        format!("{base}/authorize")
    };
    let mut url = Url::parse(&authorize)?;
    {
        let mut q = url.query_pairs_mut();
        q.append_pair("appid", &cfg.corp_id);
        q.append_pair("redirect_uri", redirect_uri);
        q.append_pair("response_type", "code");
        q.append_pair("scope", "snsapi_base");
        q.append_pair("state", &state);
        q.append_pair("agentid", &cfg.agent_id);
    }
    // Live WeCom often requires `#wechat_redirect` fragment; harmless for mock.
    url.set_fragment(Some("wechat_redirect"));
    Ok(WeComAuthorizeRequest {
        url: url.to_string(),
        state,
        redirect_uri: redirect_uri.to_string(),
    })
}

/// Map WeCom userid → Hub JWT `sub` (nailed).
pub fn wecom_subject(corp_id: &str, userid: &str) -> String {
    format!("wecom:{corp_id}:{userid}")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WeComExchangeResponse {
    pub access_token: String,
    #[serde(default)]
    pub token_type: Option<String>,
    #[serde(default)]
    pub expires_in: Option<i64>,
    #[serde(default)]
    pub subject: Option<String>,
}

/// POST Hub `/auth/wecom/exchange` with authorization code (secret stays on Hub).
pub async fn exchange_wecom_code(
    hub_http_base: &str,
    code: &str,
    state: Option<&str>,
) -> Result<WeComExchangeResponse, WeComError> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| WeComError::Http(e.to_string()))?;

    let url = format!(
        "{}/auth/wecom/exchange",
        hub_http_base.trim_end_matches('/')
    );
    let mut body = serde_json::json!({ "code": code });
    if let Some(st) = state {
        body["state"] = serde_json::Value::String(st.to_string());
    }

    let resp = client
        .post(&url)
        .header("accept", "application/json")
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| WeComError::Http(e.to_string()))?;

    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| WeComError::Http(e.to_string()))?;
    if !status.is_success() {
        return Err(WeComError::Http(format!(
            "wecom exchange {status}: {text}"
        )));
    }
    let parsed: WeComExchangeResponse = serde_json::from_str(&text)?;
    if parsed.access_token.is_empty() {
        return Err(WeComError::Message(
            "exchange response missing access_token".into(),
        ));
    }
    Ok(parsed)
}

/// Ticket provider selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TicketProvider {
    Oidc,
    WeCom,
}

impl TicketProvider {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "oidc" => Some(Self::Oidc),
            "wecom" => Some(Self::WeCom),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Oidc => "oidc",
            Self::WeCom => "wecom",
        }
    }

    /// From `ATLAS_TICKET_PROVIDER` (default oidc).
    pub fn from_env() -> Self {
        std::env::var("ATLAS_TICKET_PROVIDER")
            .ok()
            .and_then(|s| Self::parse(&s))
            .unwrap_or(Self::Oidc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subject_mapping_nailed() {
        assert_eq!(
            wecom_subject("ww_abc", "zhangsan"),
            "wecom:ww_abc:zhangsan"
        );
    }

    #[test]
    fn authorize_url_has_state_no_pkce() {
        let cfg = WeComClientConfig {
            corp_id: "ww_c".into(),
            agent_id: "1001".into(),
            authorize_base: "http://127.0.0.1:9".into(),
            hub_http_base: "http://127.0.0.1:7700".into(),
        };
        let req = build_wecom_authorize_request(&cfg, "http://127.0.0.1:1/callback", "st").unwrap();
        assert!(req.url.contains("response_type=code"));
        assert!(req.url.contains("appid=ww_c"));
        assert!(req.url.contains("agentid=1001"));
        assert!(req.url.contains("state=st"));
        assert!(!req.url.contains("code_challenge"));
    }

    #[test]
    fn ws_to_http() {
        assert_eq!(
            ws_to_http_base("ws://127.0.0.1:7700/ws"),
            "http://127.0.0.1:7700"
        );
        assert_eq!(
            ws_to_http_base("wss://hub.example/ws"),
            "https://hub.example"
        );
    }

    #[test]
    fn provider_parse() {
        assert_eq!(TicketProvider::parse("wecom"), Some(TicketProvider::WeCom));
        assert_eq!(TicketProvider::parse("OIDC"), Some(TicketProvider::Oidc));
        assert_eq!(TicketProvider::parse("nope"), None);
    }
}
