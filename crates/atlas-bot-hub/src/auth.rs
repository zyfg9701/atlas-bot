//! Hub inbound auth gate (I1).
//!
//! Modes: `ATLAS_AUTH_MODE=dev|static|oidc` (default **dev**).
//! Credential channel: `Authorization: Bearer <token>` only (WS upgrade HTTP header).
//! Bad/missing/expired → JSON-RPC closed-set **`unauthorized`** (-32002).
//! Optional `ATLAS_AUTH_ALLOWLIST` → `link_required` + reason `not_enrolled`.

#![allow(dead_code)]

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use jsonwebtoken::{
    decode, decode_header,
    jwk::{AlgorithmParameters, JwkSet},
    Algorithm, DecodingKey, Validation,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use thiserror::Error;
use tracing::{debug, info, warn};
use xai_tool_protocol::{
    BotRelayError, BotRelayErrorCode, BotRelayErrorDetail, JsonRpcError, UserId,
};

/// Default development subject (unchanged from pre-I1 Hub behavior).
pub const DEV_USER_ID: &str = "user_local_dev";

/// JSON-RPC numeric for protocol ERROR_CODES `unauthorized`.
pub const UNAUTHORIZED_NUMERIC: i32 = -32002;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthMode {
    Dev,
    Static,
    Oidc,
}

impl AuthMode {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "dev" => Some(Self::Dev),
            "static" => Some(Self::Static),
            "oidc" => Some(Self::Oidc),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dev => "dev",
            Self::Static => "static",
            Self::Oidc => "oidc",
        }
    }
}

/// Authenticated subject bound to a Hub session after a successful gate.
#[derive(Debug, Clone)]
pub struct AuthContext {
    pub user_id: UserId,
}

impl AuthContext {
    pub fn dev() -> Self {
        Self {
            user_id: UserId::new(DEV_USER_ID).expect("dev user id"),
        }
    }

    pub fn from_sub(sub: &str) -> Result<Self, AuthError> {
        let user_id = UserId::new(sub).map_err(|_| AuthError::Unauthorized {
            detail: format!("invalid subject {sub:?}"),
        })?;
        Ok(Self { user_id })
    }
}

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("unauthorized: {detail}")]
    Unauthorized { detail: String },
    #[error("link_required: {reason}")]
    LinkRequired { reason: String },
}

impl AuthError {
    pub fn to_jsonrpc(&self) -> JsonRpcError {
        match self {
            Self::Unauthorized { detail } => unauthorized_error(detail),
            Self::LinkRequired { reason } => JsonRpcError::from(BotRelayError {
                code: BotRelayErrorCode::LinkRequired,
                retryable: false,
                detail: BotRelayErrorDetail {
                    upstream: Some(format!("allowlist: {reason}")),
                },
                reason: Some(reason.clone()),
            }),
        }
    }
}

/// Build a wire error with `data.code = "unauthorized"` (protocol ERROR_CODES).
///
/// Not a [`BotRelayErrorCode`] variant — Hub emits the shared JSON-RPC table
/// code `-32002` / `unauthorized` so clients can nail on the string without
/// inventing a new bot-relay enum member.
pub fn unauthorized_error(detail: &str) -> JsonRpcError {
    JsonRpcError {
        code: UNAUTHORIZED_NUMERIC,
        message: "unauthorized".into(),
        data: Some(json!({
            "code": "unauthorized",
            "retryable": false,
            "detail": { "upstream": detail },
        })),
    }
}

#[derive(Debug, Clone)]
pub struct AuthConfig {
    pub mode: AuthMode,
    /// HS256 secret for `static` (and optionally mock OIDC oct keys).
    pub jwt_secret: Option<String>,
    /// Comma-separated allowlist of JWT `sub` values. Empty / unset = allow all.
    pub allowlist: Option<Vec<String>>,
    pub oidc_issuer: Option<String>,
    pub oidc_audience: Option<String>,
    /// Pre-parsed JWKS keys: (kid, alg, key).
    jwks_keys: Arc<Vec<JwkEntry>>,
    /// When JWKS was loaded (for optional refresh bookkeeping).
    jwks_loaded_at: Option<Instant>,
}

#[derive(Clone)]
struct JwkEntry {
    kid: Option<String>,
    alg: Algorithm,
    key: DecodingKey,
}

impl std::fmt::Debug for JwkEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JwkEntry")
            .field("kid", &self.kid)
            .field("alg", &self.alg)
            .finish()
    }
}

impl AuthConfig {
    /// Always-dev config (ignores process env). Safe default for unit tests.
    pub fn dev() -> Self {
        Self {
            mode: AuthMode::Dev,
            jwt_secret: None,
            allowlist: None,
            oidc_issuer: None,
            oidc_audience: None,
            jwks_keys: Arc::new(Vec::new()),
            jwks_loaded_at: None,
        }
    }

    /// Load from environment. For `oidc`, prefers `ATLAS_OIDC_JWKS_JSON`
    /// (inline JSON or file path) so CI works offline; otherwise attempts
    /// a blocking-ish JWKS fetch from the issuer (best-effort).
    pub fn from_env() -> Self {
        let mode = std::env::var("ATLAS_AUTH_MODE")
            .ok()
            .and_then(|s| AuthMode::parse(&s))
            .unwrap_or(AuthMode::Dev);

        let jwt_secret = std::env::var("ATLAS_AUTH_JWT_SECRET").ok().filter(|s| !s.is_empty());
        let allowlist = parse_allowlist(std::env::var("ATLAS_AUTH_ALLOWLIST").ok());
        let oidc_issuer = std::env::var("ATLAS_OIDC_ISSUER").ok().filter(|s| !s.is_empty());
        let oidc_audience = std::env::var("ATLAS_OIDC_AUDIENCE")
            .ok()
            .filter(|s| !s.is_empty());

        let mut cfg = Self {
            mode,
            jwt_secret,
            allowlist,
            oidc_issuer: oidc_issuer.clone(),
            oidc_audience,
            jwks_keys: Arc::new(Vec::new()),
            jwks_loaded_at: None,
        };

        if mode == AuthMode::Oidc {
            match load_jwks_from_env() {
                Ok(keys) if !keys.is_empty() => {
                    info!(count = keys.len(), "oidc JWKS loaded (mock/env)");
                    cfg.jwks_keys = Arc::new(keys);
                    cfg.jwks_loaded_at = Some(Instant::now());
                }
                Ok(_) => {
                    warn!("oidc mode but JWKS empty; set ATLAS_OIDC_JWKS_JSON for offline smoke");
                }
                Err(e) => {
                    warn!(error = %e, "failed to load ATLAS_OIDC_JWKS_JSON");
                    // Best-effort live fetch when issuer is set.
                    if let Some(issuer) = oidc_issuer.as_deref() {
                        match fetch_jwks_blocking(issuer) {
                            Ok(keys) => {
                                info!(count = keys.len(), %issuer, "oidc JWKS fetched from issuer");
                                cfg.jwks_keys = Arc::new(keys);
                                cfg.jwks_loaded_at = Some(Instant::now());
                            }
                            Err(fe) => warn!(error = %fe, "oidc JWKS fetch failed"),
                        }
                    }
                }
            }
        }

        info!(mode = cfg.mode.as_str(), "hub auth gate configured");
        cfg
    }

    /// Explicit static mode (tests).
    pub fn static_hs256(secret: impl Into<String>) -> Self {
        Self {
            mode: AuthMode::Static,
            jwt_secret: Some(secret.into()),
            allowlist: None,
            oidc_issuer: None,
            oidc_audience: None,
            jwks_keys: Arc::new(Vec::new()),
            jwks_loaded_at: None,
        }
    }

    /// Explicit oidc mode with preloaded JWKS JSON (tests / mock).
    pub fn oidc_mock(
        issuer: impl Into<String>,
        audience: Option<String>,
        jwks_json: &str,
    ) -> Result<Self, String> {
        let keys = parse_jwks_json(jwks_json)?;
        Ok(Self {
            mode: AuthMode::Oidc,
            jwt_secret: None,
            allowlist: None,
            oidc_issuer: Some(issuer.into()),
            oidc_audience: audience,
            jwks_keys: Arc::new(keys),
            jwks_loaded_at: Some(Instant::now()),
        })
    }

    pub fn with_allowlist(mut self, subs: Vec<String>) -> Self {
        self.allowlist = if subs.is_empty() { None } else { Some(subs) };
        self
    }
}

fn parse_allowlist(raw: Option<String>) -> Option<Vec<String>> {
    let raw = raw?;
    let list: Vec<String> = raw
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if list.is_empty() {
        None
    } else {
        Some(list)
    }
}

/// Extract Bearer token from an `Authorization` header value.
pub fn bearer_from_authorization(value: Option<&str>) -> Option<String> {
    let v = value?.trim();
    let rest = v.strip_prefix("Bearer ").or_else(|| v.strip_prefix("bearer "))?;
    let tok = rest.trim();
    if tok.is_empty() {
        None
    } else {
        Some(tok.to_string())
    }
}

/// Extract Bearer from a header map (case-insensitive `authorization`).
pub fn bearer_from_headers<'a, I, K, V>(headers: I) -> Option<String>
where
    I: IntoIterator<Item = (K, V)>,
    K: AsRef<str>,
    V: AsRef<str>,
{
    for (k, v) in headers {
        if k.as_ref().eq_ignore_ascii_case("authorization") {
            return bearer_from_authorization(Some(v.as_ref()));
        }
    }
    None
}

#[derive(Debug, Serialize, Deserialize)]
struct Claims {
    sub: String,
    #[serde(default)]
    exp: Option<i64>,
    #[serde(default)]
    iss: Option<String>,
    #[serde(default)]
    aud: Option<Aud>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
enum Aud {
    One(String),
    Many(Vec<String>),
}

impl Aud {
    fn contains(&self, want: &str) -> bool {
        match self {
            Aud::One(s) => s == want,
            Aud::Many(v) => v.iter().any(|s| s == want),
        }
    }
}

/// Validate the optional Bearer token according to `config`.
pub fn authenticate(config: &AuthConfig, bearer: Option<&str>) -> Result<AuthContext, AuthError> {
    let ctx = match config.mode {
        AuthMode::Dev => {
            // Ignore token; keep historical subject.
            debug!("auth mode=dev → user_local_dev");
            AuthContext::dev()
        }
        AuthMode::Static => {
            let token = bearer.ok_or_else(|| AuthError::Unauthorized {
                detail: "missing bearer token".into(),
            })?;
            let secret = config.jwt_secret.as_deref().ok_or_else(|| AuthError::Unauthorized {
                detail: "ATLAS_AUTH_JWT_SECRET not configured".into(),
            })?;
            let claims = decode_hs256(token, secret)?;
            AuthContext::from_sub(&claims.sub)?
        }
        AuthMode::Oidc => {
            let token = bearer.ok_or_else(|| AuthError::Unauthorized {
                detail: "missing bearer token".into(),
            })?;
            let claims = decode_oidc(config, token)?;
            AuthContext::from_sub(&claims.sub)?
        }
    };

    if let Some(list) = config.allowlist.as_ref() {
        if !list.iter().any(|s| s == ctx.user_id.as_str()) {
            return Err(AuthError::LinkRequired {
                reason: "not_enrolled".into(),
            });
        }
    }

    Ok(ctx)
}

fn decode_hs256(token: &str, secret: &str) -> Result<Claims, AuthError> {
    let mut validation = Validation::new(Algorithm::HS256);
    validation.validate_exp = true;
    // We only require `sub`; iss/aud optional in static mode.
    validation.set_required_spec_claims(&["sub", "exp"]);
    let key = DecodingKey::from_secret(secret.as_bytes());
    decode::<Claims>(token, &key, &validation)
        .map(|d| d.claims)
        .map_err(|e| AuthError::Unauthorized {
            detail: format!("jwt: {e}"),
        })
}

fn decode_oidc(config: &AuthConfig, token: &str) -> Result<Claims, AuthError> {
    if config.jwks_keys.is_empty() {
        return Err(AuthError::Unauthorized {
            detail: "oidc JWKS not loaded (set ATLAS_OIDC_JWKS_JSON or reachable issuer)".into(),
        });
    }

    let header = decode_header(token).map_err(|e| AuthError::Unauthorized {
        detail: format!("jwt header: {e}"),
    })?;
    let kid = header.kid.clone();

    let entry = config
        .jwks_keys
        .iter()
        .find(|e| match (&kid, &e.kid) {
            (Some(want), Some(have)) => want == have,
            (None, _) => true, // no kid in token → first matching alg
            (Some(_), None) => true,
        })
        .or_else(|| config.jwks_keys.first())
        .ok_or_else(|| AuthError::Unauthorized {
            detail: "no JWKS key matched".into(),
        })?;

    let mut validation = Validation::new(entry.alg);
    validation.validate_exp = true;
    validation.set_required_spec_claims(&["sub", "exp"]);

    if let Some(iss) = config.oidc_issuer.as_deref() {
        validation.set_issuer(&[iss]);
    }
    if let Some(aud) = config.oidc_audience.as_deref() {
        validation.set_audience(&[aud]);
    } else {
        validation.validate_aud = false;
    }

    let data = decode::<Claims>(token, &entry.key, &validation).map_err(|e| {
        AuthError::Unauthorized {
            detail: format!("oidc jwt: {e}"),
        }
    })?;

    // Extra audience check if claim present as multi-aud and Validation passed.
    if let Some(want) = config.oidc_audience.as_deref() {
        if let Some(aud) = &data.claims.aud {
            if !aud.contains(want) {
                return Err(AuthError::Unauthorized {
                    detail: "audience mismatch".into(),
                });
            }
        }
    }

    Ok(data.claims)
}

fn load_jwks_from_env() -> Result<Vec<JwkEntry>, String> {
    let raw = std::env::var("ATLAS_OIDC_JWKS_JSON").map_err(|_| "ATLAS_OIDC_JWKS_JSON unset".to_string())?;
    let json = if Path::new(&raw).is_file() {
        fs::read_to_string(&raw).map_err(|e| format!("read JWKS file: {e}"))?
    } else {
        raw
    };
    parse_jwks_json(&json)
}

fn parse_jwks_json(json: &str) -> Result<Vec<JwkEntry>, String> {
    let set: JwkSet = serde_json::from_str(json).map_err(|e| format!("jwks json: {e}"))?;
    let mut out = Vec::new();
    for jwk in set.keys {
        let kid = jwk.common.key_id.clone();
        let alg = jwk
            .common
            .key_algorithm
            .map(|a| a.to_string())
            .and_then(|s| Algorithm::from_str_safe(&s))
            .unwrap_or(Algorithm::HS256);

        let key = DecodingKey::from_jwk(&jwk).map_err(|e| format!("jwk→key: {e}"))?;
        // Prefer algorithm from JWK alg field; fall back by kty.
        let alg = match &jwk.algorithm {
            AlgorithmParameters::RSA(_) => {
                if matches!(
                    alg,
                    Algorithm::RS256 | Algorithm::RS384 | Algorithm::RS512 | Algorithm::PS256
                ) {
                    alg
                } else {
                    Algorithm::RS256
                }
            }
            AlgorithmParameters::EllipticCurve(_) => Algorithm::ES256,
            AlgorithmParameters::OctetKeyPair(_) => Algorithm::EdDSA,
            AlgorithmParameters::OctetKey(_) => Algorithm::HS256,
        };
        out.push(JwkEntry { kid, alg, key });
    }
    if out.is_empty() {
        return Err("JWKS contained no keys".into());
    }
    Ok(out)
}

trait AlgParse {
    fn from_str_safe(s: &str) -> Option<Algorithm>;
}

impl AlgParse for Algorithm {
    fn from_str_safe(s: &str) -> Option<Algorithm> {
        match s {
            "HS256" => Some(Algorithm::HS256),
            "HS384" => Some(Algorithm::HS384),
            "HS512" => Some(Algorithm::HS512),
            "RS256" => Some(Algorithm::RS256),
            "RS384" => Some(Algorithm::RS384),
            "RS512" => Some(Algorithm::RS512),
            "ES256" => Some(Algorithm::ES256),
            "ES384" => Some(Algorithm::ES384),
            "PS256" => Some(Algorithm::PS256),
            "PS384" => Some(Algorithm::PS384),
            "PS512" => Some(Algorithm::PS512),
            "EdDSA" => Some(Algorithm::EdDSA),
            _ => None,
        }
    }
}

fn fetch_jwks_blocking(issuer: &str) -> Result<Vec<JwkEntry>, String> {
    // Best-effort: try OpenID discovery, then `{issuer}/jwks.json`.
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| e.to_string())?;

    let issuer = issuer.trim_end_matches('/');
    let discovery = format!("{issuer}/.well-known/openid-configuration");
    let jwks_uri = match client.get(&discovery).send() {
        Ok(resp) if resp.status().is_success() => {
            let v: Value = resp.json().map_err(|e| e.to_string())?;
            v.get("jwks_uri")
                .and_then(|x| x.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("{issuer}/jwks.json"))
        }
        _ => format!("{issuer}/.well-known/jwks.json"),
    };

    let text = client
        .get(&jwks_uri)
        .send()
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .text()
        .map_err(|e| e.to_string())?;
    parse_jwks_json(&text)
}

/// Mint an HS256 JWT for static-mode smoke / runbook (also used by mint-jwt bin).
pub fn mint_hs256(
    secret: &str,
    sub: &str,
    ttl_secs: i64,
    issuer: Option<&str>,
    audience: Option<&str>,
) -> Result<String, String> {
    use jsonwebtoken::{encode, EncodingKey, Header};
    use std::time::{SystemTime, UNIX_EPOCH};

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_secs() as i64;

    #[derive(Serialize)]
    struct MintClaims<'a> {
        sub: &'a str,
        exp: i64,
        iat: i64,
        #[serde(skip_serializing_if = "Option::is_none")]
        iss: Option<&'a str>,
        #[serde(skip_serializing_if = "Option::is_none")]
        aud: Option<&'a str>,
    }

    let claims = MintClaims {
        sub,
        exp: now + ttl_secs,
        iat: now,
        iss: issuer,
        aud: audience,
    };
    encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(|e| e.to_string())
}

/// Build a minimal oct JWKS for HS256 mock OIDC (secret as raw bytes → base64url).
pub fn mock_oct_jwks(kid: &str, secret: &str) -> String {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine;
    let k = URL_SAFE_NO_PAD.encode(secret.as_bytes());
    json!({
        "keys": [{
            "kty": "oct",
            "kid": kid,
            "alg": "HS256",
            "k": k
        }]
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dev_ignores_token() {
        let cfg = AuthConfig::dev();
        let ctx = authenticate(&cfg, None).unwrap();
        assert_eq!(ctx.user_id.as_str(), DEV_USER_ID);
        let ctx2 = authenticate(&cfg, Some("garbage")).unwrap();
        assert_eq!(ctx2.user_id.as_str(), DEV_USER_ID);
    }

    #[test]
    fn static_good_and_bad() {
        let secret = "test-secret-please-change";
        let cfg = AuthConfig::static_hs256(secret);
        let tok = mint_hs256(secret, "user_alice", 3600, None, None).unwrap();
        let ctx = authenticate(&cfg, Some(&tok)).unwrap();
        assert_eq!(ctx.user_id.as_str(), "user_alice");

        let err = authenticate(&cfg, None).unwrap_err();
        assert!(matches!(err, AuthError::Unauthorized { .. }));
        let j = err.to_jsonrpc();
        assert_eq!(j.message, "unauthorized");
        assert_eq!(j.code, UNAUTHORIZED_NUMERIC);

        let err = authenticate(&cfg, Some("not.a.jwt")).unwrap_err();
        assert!(matches!(err, AuthError::Unauthorized { .. }));
    }

    #[test]
    fn allowlist_link_required() {
        let secret = "sec";
        let cfg = AuthConfig::static_hs256(secret).with_allowlist(vec!["user_ok".into()]);
        let tok = mint_hs256(secret, "user_other", 3600, None, None).unwrap();
        let err = authenticate(&cfg, Some(&tok)).unwrap_err();
        match err {
            AuthError::LinkRequired { reason } => assert_eq!(reason, "not_enrolled"),
            other => panic!("expected link_required, got {other:?}"),
        }
    }

    #[test]
    fn oidc_mock_oct() {
        let secret = "oidc-mock-secret";
        let issuer = "https://idp.example.test";
        let jwks = mock_oct_jwks("k1", secret);
        let cfg = AuthConfig::oidc_mock(issuer, Some("atlas-hub".into()), &jwks).unwrap();
        let tok = mint_hs256(secret, "user_oidc", 3600, Some(issuer), Some("atlas-hub")).unwrap();
        let ctx = authenticate(&cfg, Some(&tok)).unwrap();
        assert_eq!(ctx.user_id.as_str(), "user_oidc");
    }

    #[test]
    fn bearer_parse() {
        assert_eq!(
            bearer_from_authorization(Some("Bearer abc.def")),
            Some("abc.def".into())
        );
        assert_eq!(bearer_from_authorization(Some("Basic x")), None);
        assert_eq!(bearer_from_authorization(None), None);
    }
}
