//! OIDC Authorization Code + PKCE: authorize URL + token exchange.

use serde::{Deserialize, Serialize};
use thiserror::Error;
use url::Url;

use crate::pkce::PkcePair;

#[derive(Debug, Error)]
pub enum OidcError {
    #[error("url: {0}")]
    Url(#[from] url::ParseError),
    #[error("http: {0}")]
    Http(String),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Message(String),
}

#[derive(Debug, Clone)]
pub struct OidcClientConfig {
    pub issuer: String,
    pub client_id: String,
    pub authorize_url: String,
    pub token_url: String,
    pub scopes: Vec<String>,
    /// Optional audience hint (may be ignored by IdP).
    pub audience: Option<String>,
}

impl OidcClientConfig {
    /// Build from issuer base: `{issuer}/authorize` and `{issuer}/token`.
    pub fn from_issuer(issuer: impl Into<String>, client_id: impl Into<String>) -> Self {
        let issuer = issuer.into().trim_end_matches('/').to_string();
        Self {
            authorize_url: format!("{issuer}/authorize"),
            token_url: format!("{issuer}/token"),
            issuer,
            client_id: client_id.into(),
            scopes: vec![
                "openid".into(),
                "profile".into(),
            ],
            audience: None,
        }
    }

    pub fn with_audience(mut self, aud: impl Into<String>) -> Self {
        self.audience = Some(aud.into());
        self
    }

    pub fn with_scopes(mut self, scopes: Vec<String>) -> Self {
        self.scopes = scopes;
        self
    }
}

#[derive(Debug, Clone)]
pub struct AuthorizeRequest {
    pub url: String,
    pub state: String,
    pub pkce: PkcePair,
    pub redirect_uri: String,
}

/// Build the authorize URL with PKCE S256 + state.
pub fn build_authorize_request(
    cfg: &OidcClientConfig,
    redirect_uri: &str,
    pkce: PkcePair,
    state: impl Into<String>,
) -> Result<AuthorizeRequest, OidcError> {
    let state = state.into();
    let mut url = Url::parse(&cfg.authorize_url)?;
    {
        let mut q = url.query_pairs_mut();
        q.append_pair("response_type", "code");
        q.append_pair("client_id", &cfg.client_id);
        q.append_pair("redirect_uri", redirect_uri);
        q.append_pair("scope", &cfg.scopes.join(" "));
        q.append_pair("state", &state);
        q.append_pair("code_challenge", &pkce.challenge);
        q.append_pair("code_challenge_method", pkce.method);
        if let Some(aud) = cfg.audience.as_deref() {
            q.append_pair("audience", aud);
        }
    }
    Ok(AuthorizeRequest {
        url: url.to_string(),
        state,
        pkce,
        redirect_uri: redirect_uri.to_string(),
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenResponse {
    #[serde(default)]
    pub access_token: Option<String>,
    #[serde(default)]
    pub id_token: Option<String>,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub token_type: Option<String>,
    #[serde(default)]
    pub expires_in: Option<i64>,
    #[serde(default)]
    pub scope: Option<String>,
}

/// True if the string looks like a compact JWT (three base64url segments).
pub fn looks_like_jwt(token: &str) -> bool {
    let parts: Vec<&str> = token.split('.').collect();
    parts.len() == 3 && parts.iter().all(|p| !p.is_empty())
}

/// Prefer **id_token** when present; else **access_token** if JWT; else whichever exists.
///
/// Nailed for Hub I1 `oidc` JWKS validation: opaque access tokens cannot be verified.
pub fn pick_hub_bearer(tr: &TokenResponse) -> Option<String> {
    if let Some(id) = tr.id_token.as_deref() {
        if !id.is_empty() {
            return Some(id.to_string());
        }
    }
    if let Some(access) = tr.access_token.as_deref() {
        if looks_like_jwt(access) {
            return Some(access.to_string());
        }
    }
    tr.access_token
        .clone()
        .filter(|s| !s.is_empty())
        .or_else(|| tr.id_token.clone().filter(|s| !s.is_empty()))
}

/// Exchange authorization code at the token endpoint (PKCE).
pub async fn exchange_code(
    cfg: &OidcClientConfig,
    redirect_uri: &str,
    code: &str,
    code_verifier: &str,
) -> Result<TokenResponse, OidcError> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| OidcError::Http(e.to_string()))?;

    let form = [
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", redirect_uri),
        ("client_id", cfg.client_id.as_str()),
        ("code_verifier", code_verifier),
    ];

    let resp = client
        .post(&cfg.token_url)
        .header("accept", "application/json")
        .form(&form)
        .send()
        .await
        .map_err(|e| OidcError::Http(e.to_string()))?;

    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| OidcError::Http(e.to_string()))?;
    if !status.is_success() {
        return Err(OidcError::Http(format!(
            "token endpoint {status}: {body}"
        )));
    }
    let tr: TokenResponse = serde_json::from_str(&body)?;
    if pick_hub_bearer(&tr).is_none() {
        return Err(OidcError::Message(
            "token response missing id_token/access_token".into(),
        ));
    }
    Ok(tr)
}

/// Best-effort decode of JWT payload `sub` without verifying (display only).
pub fn peek_sub(jwt: &str) -> Option<String> {
    let payload = jwt.split('.').nth(1)?;
    use base64::engine::general_purpose::{URL_SAFE_NO_PAD, URL_SAFE};
    use base64::Engine;
    let bytes = URL_SAFE_NO_PAD
        .decode(payload)
        .or_else(|_| URL_SAFE.decode(payload))
        .ok()?;
    let v: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    v.get("sub")?.as_str().map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pick_prefers_id_token() {
        let tr = TokenResponse {
            access_token: Some("a.b.c".into()),
            id_token: Some("x.y.z".into()),
            refresh_token: None,
            token_type: Some("Bearer".into()),
            expires_in: Some(3600),
            scope: None,
        };
        assert_eq!(pick_hub_bearer(&tr).as_deref(), Some("x.y.z"));
    }

    #[test]
    fn pick_falls_back_to_jwt_access() {
        let tr = TokenResponse {
            access_token: Some("a.b.c".into()),
            id_token: None,
            refresh_token: None,
            token_type: None,
            expires_in: None,
            scope: None,
        };
        assert_eq!(pick_hub_bearer(&tr).as_deref(), Some("a.b.c"));
    }

    #[test]
    fn authorize_url_contains_pkce() {
        let cfg = OidcClientConfig::from_issuer("http://127.0.0.1:9", "cli");
        let pkce = crate::pkce::PkcePair::from_verifier("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk");
        let req = build_authorize_request(&cfg, "http://127.0.0.1:1/callback", pkce, "st").unwrap();
        assert!(req.url.contains("code_challenge="));
        assert!(req.url.contains("code_challenge_method=S256"));
        assert!(req.url.contains("response_type=code"));
        assert!(req.url.contains("state=st"));
    }
}
