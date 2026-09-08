//! Offline mock OIDC IdP for CI (authorize → code → token with JWT + JWKS).

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Form, Json, Router};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;
use uuid::Uuid;

/// Default HS256 secret for mock tokens (must match Hub `ATLAS_OIDC_JWKS_JSON`).
pub const MOCK_SECRET: &str = "atlas-i2-mock-oidc-secret";
pub const MOCK_KID: &str = "mock-k1";
pub const MOCK_AUD: &str = "atlas-hub";
pub const MOCK_SUB: &str = "user_i2_mock";

#[derive(Clone)]
pub struct MockOidc {
    pub addr: SocketAddr,
    pub issuer: String,
    pub secret: String,
    pub audience: String,
    pub default_sub: String,
    inner: Arc<Mutex<MockInner>>,
}

struct MockInner {
    /// code → pending auth
    codes: HashMap<String, PendingCode>,
}

struct PendingCode {
    client_id: String,
    redirect_uri: String,
    challenge: String,
    #[allow(dead_code)]
    challenge_method: String, // kept for clarity
    sub: String,
}

impl MockOidc {
    pub fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    pub fn authorize_url(&self) -> String {
        format!("{}/authorize", self.base_url())
    }

    pub fn token_url(&self) -> String {
        format!("{}/token", self.base_url())
    }

    pub fn jwks_json(&self) -> String {
        mock_oct_jwks(MOCK_KID, &self.secret)
    }

    pub fn discovery(&self) -> Value {
        json!({
            "issuer": self.issuer,
            "authorization_endpoint": self.authorize_url(),
            "token_endpoint": self.token_url(),
            "jwks_uri": format!("{}/jwks.json", self.base_url()),
            "response_types_supported": ["code"],
            "code_challenge_methods_supported": ["S256"],
            "subject_types_supported": ["public"],
            "id_token_signing_alg_values_supported": ["HS256"],
        })
    }
}

/// Build oct JWKS (same shape as Hub `mock_oct_jwks`).
pub fn mock_oct_jwks(kid: &str, secret: &str) -> String {
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

/// Start mock IdP on `127.0.0.1:0`.
pub async fn start_mock_oidc() -> MockOidc {
    start_mock_oidc_with(MOCK_SECRET, MOCK_AUD, MOCK_SUB).await
}

pub async fn start_mock_oidc_with(secret: &str, audience: &str, default_sub: &str) -> MockOidc {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock oidc");
    let addr = listener.local_addr().unwrap();
    let issuer = format!("http://{addr}");
    let mock = MockOidc {
        addr,
        issuer: issuer.clone(),
        secret: secret.to_string(),
        audience: audience.to_string(),
        default_sub: default_sub.to_string(),
        inner: Arc::new(Mutex::new(MockInner {
            codes: HashMap::new(),
        })),
    };

    let app = Router::new()
        .route(
            "/.well-known/openid-configuration",
            get(discovery_handler),
        )
        .route("/jwks.json", get(jwks_handler))
        .route("/authorize", get(authorize_handler))
        .route("/token", post(token_handler))
        .with_state(mock.clone());

    tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });
    tokio::task::yield_now().await;
    mock
}

async fn discovery_handler(State(m): State<MockOidc>) -> Json<Value> {
    Json(m.discovery())
}

async fn jwks_handler(State(m): State<MockOidc>) -> Response {
    (
        StatusCode::OK,
        [("content-type", "application/json")],
        m.jwks_json(),
    )
        .into_response()
}

#[derive(Debug, Deserialize)]
struct AuthorizeQuery {
    response_type: Option<String>,
    client_id: Option<String>,
    redirect_uri: Option<String>,
    state: Option<String>,
    code_challenge: Option<String>,
    code_challenge_method: Option<String>,
    /// Optional override subject for tests (`login_hint` / `sub`).
    #[serde(default)]
    login_hint: Option<String>,
    #[serde(default)]
    sub: Option<String>,
}

async fn authorize_handler(
    State(m): State<MockOidc>,
    Query(q): Query<AuthorizeQuery>,
) -> Response {
    if q.response_type.as_deref() != Some("code") {
        return (StatusCode::BAD_REQUEST, "response_type must be code").into_response();
    }
    let client_id = match q.client_id {
        Some(c) if !c.is_empty() => c,
        _ => return (StatusCode::BAD_REQUEST, "client_id required").into_response(),
    };
    let redirect_uri = match q.redirect_uri {
        Some(r) if !r.is_empty() => r,
        _ => return (StatusCode::BAD_REQUEST, "redirect_uri required").into_response(),
    };
    let challenge = match q.code_challenge {
        Some(c) if !c.is_empty() => c,
        _ => return (StatusCode::BAD_REQUEST, "code_challenge required").into_response(),
    };
    let method = q
        .code_challenge_method
        .unwrap_or_else(|| "S256".into());
    if method != "S256" {
        return (StatusCode::BAD_REQUEST, "only S256 supported").into_response();
    }

    let code = Uuid::new_v4().to_string();
    let sub = q
        .sub
        .or(q.login_hint)
        .unwrap_or_else(|| m.default_sub.clone());

    m.inner.lock().await.codes.insert(
        code.clone(),
        PendingCode {
            client_id,
            redirect_uri: redirect_uri.clone(),
            challenge,
            challenge_method: method,
            sub,
        },
    );

    let mut loc = url::Url::parse(&redirect_uri).unwrap_or_else(|_| {
        url::Url::parse("http://127.0.0.1/callback").expect("fallback")
    });
    {
        let mut qp = loc.query_pairs_mut();
        qp.append_pair("code", &code);
        if let Some(st) = q.state.as_deref() {
            qp.append_pair("state", st);
        }
    }
    Redirect::temporary(loc.as_str()).into_response()
}

#[derive(Debug, Deserialize)]
struct TokenForm {
    grant_type: Option<String>,
    code: Option<String>,
    redirect_uri: Option<String>,
    client_id: Option<String>,
    code_verifier: Option<String>,
}

async fn token_handler(State(m): State<MockOidc>, Form(form): Form<TokenForm>) -> Response {
    if form.grant_type.as_deref() != Some("authorization_code") {
        return json_err(StatusCode::BAD_REQUEST, "unsupported_grant_type");
    }
    let code = match form.code {
        Some(c) => c,
        None => return json_err(StatusCode::BAD_REQUEST, "invalid_request"),
    };
    let verifier = match form.code_verifier {
        Some(v) => v,
        None => return json_err(StatusCode::BAD_REQUEST, "invalid_request"),
    };

    let pending = {
        let mut inner = m.inner.lock().await;
        match inner.codes.remove(&code) {
            Some(p) => p,
            None => return json_err(StatusCode::BAD_REQUEST, "invalid_grant"),
        }
    };

    if let Some(cid) = form.client_id.as_deref() {
        if cid != pending.client_id {
            return json_err(StatusCode::BAD_REQUEST, "invalid_client");
        }
    }
    if let Some(ru) = form.redirect_uri.as_deref() {
        if ru != pending.redirect_uri {
            return json_err(StatusCode::BAD_REQUEST, "invalid_grant");
        }
    }

    let expected = s256(&verifier);
    if expected != pending.challenge {
        return json_err(StatusCode::BAD_REQUEST, "invalid_grant");
    }

    let id_token = mint_token(
        &m.secret,
        &pending.sub,
        &m.issuer,
        &m.audience,
        3600,
    )
    .expect("mint id_token");
    // Also mint a JWT access_token so either can be handed to Hub.
    let access_token = mint_token(
        &m.secret,
        &pending.sub,
        &m.issuer,
        &m.audience,
        3600,
    )
    .expect("mint access");

    Json(json!({
        "access_token": access_token,
        "id_token": id_token,
        "token_type": "Bearer",
        "expires_in": 3600,
        "scope": "openid profile",
    }))
    .into_response()
}

fn json_err(status: StatusCode, err: &str) -> Response {
    (status, Json(json!({ "error": err }))).into_response()
}

fn s256(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}

fn mint_token(
    secret: &str,
    sub: &str,
    iss: &str,
    aud: &str,
    ttl_secs: i64,
) -> Result<String, String> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_secs() as i64;

    #[derive(Serialize)]
    struct Claims<'a> {
        sub: &'a str,
        iss: &'a str,
        aud: &'a str,
        exp: i64,
        iat: i64,
    }

    let mut header = Header::new(Algorithm::HS256);
    header.kid = Some(MOCK_KID.into());
    encode(
        &header,
        &Claims {
            sub,
            iss,
            aud,
            exp: now + ttl_secs,
            iat: now,
        },
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(|e| e.to_string())
}

/// Drive authorize→redirect without a browser (CI / `ATLAS_I2_NO_BROWSER=1`).
///
/// Performs an HTTP GET to the authorize URL with redirect following disabled,
/// then extracts `code` from the `Location` header.
pub async fn drive_authorize_for_code(authorize_url: &str) -> Result<(String, Option<String>), String> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .get(authorize_url)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_redirection() {
        return Err(format!(
            "expected redirect from authorize, got {}",
            resp.status()
        ));
    }
    let loc = resp
        .headers()
        .get(reqwest::header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| "missing Location".to_string())?;
    let url = url::Url::parse(loc).map_err(|e| e.to_string())?;
    let mut code = None;
    let mut state = None;
    for (k, v) in url.query_pairs() {
        if k == "code" {
            code = Some(v.to_string());
        }
        if k == "state" {
            state = Some(v.to_string());
        }
    }
    let code = code.ok_or_else(|| format!("no code in Location {loc}"))?;
    Ok((code, state))
}
