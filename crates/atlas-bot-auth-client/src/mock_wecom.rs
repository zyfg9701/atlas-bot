//! Offline mock WeCom OAuth for CI (authorize → code → gettoken/getuserinfo).
//!
//! Never talks to qyapi.weixin.qq.com. Hub points ATLAS_WECOM_API_BASE here.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::Mutex;
use uuid::Uuid;

pub const MOCK_CORP_ID: &str = "ww_mock_corp";
pub const MOCK_AGENT_ID: &str = "1000001";
pub const MOCK_SECRET: &str = "atlas-w1-mock-wecom-secret";
pub const MOCK_USERID: &str = "wecom_mock_user";

#[derive(Clone)]
pub struct MockWecom {
    pub addr: SocketAddr,
    pub corp_id: String,
    pub agent_id: String,
    pub secret: String,
    pub default_userid: String,
    inner: Arc<Mutex<MockInner>>,
}

struct MockInner {
    /// code → userid
    codes: HashMap<String, String>,
    /// access_token → valid
    tokens: HashMap<String, bool>,
}

impl MockWecom {
    pub fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    pub fn authorize_url_base(&self) -> String {
        format!("{}/authorize", self.base_url())
    }
}

/// Start mock WeCom on 127.0.0.1:0.
pub async fn start_mock_wecom() -> MockWecom {
    start_mock_wecom_with(MOCK_CORP_ID, MOCK_AGENT_ID, MOCK_SECRET, MOCK_USERID).await
}

pub async fn start_mock_wecom_with(
    corp_id: &str,
    agent_id: &str,
    secret: &str,
    default_userid: &str,
) -> MockWecom {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock wecom");
    let addr = listener.local_addr().unwrap();
    let mock = MockWecom {
        addr,
        corp_id: corp_id.to_string(),
        agent_id: agent_id.to_string(),
        secret: secret.to_string(),
        default_userid: default_userid.to_string(),
        inner: Arc::new(Mutex::new(MockInner {
            codes: HashMap::new(),
            tokens: HashMap::new(),
        })),
    };

    let app = Router::new()
        .route("/authorize", get(authorize_handler))
        .route("/cgi-bin/gettoken", get(gettoken_handler))
        .route("/cgi-bin/user/getuserinfo", get(getuserinfo_handler))
        .route("/healthz", get(|| async { "ok" }))
        .with_state(mock.clone());

    tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });
    tokio::task::yield_now().await;
    mock
}

#[derive(Debug, Deserialize)]
struct AuthorizeQuery {
    appid: Option<String>,
    redirect_uri: Option<String>,
    response_type: Option<String>,
    state: Option<String>,
    /// Optional userid override for tests.
    #[serde(default)]
    userid: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    agentid: Option<String>,
}

async fn authorize_handler(
    State(m): State<MockWecom>,
    Query(q): Query<AuthorizeQuery>,
) -> Response {
    if q.response_type.as_deref() != Some("code") {
        return (StatusCode::BAD_REQUEST, "response_type must be code").into_response();
    }
    let redirect_uri = match q.redirect_uri {
        Some(r) if !r.is_empty() => r,
        _ => return (StatusCode::BAD_REQUEST, "redirect_uri required").into_response(),
    };
    if let Some(appid) = q.appid.as_deref() {
        if appid != m.corp_id {
            return (StatusCode::BAD_REQUEST, "appid mismatch").into_response();
        }
    }

    let code = Uuid::new_v4().to_string();
    let userid = q.userid.unwrap_or_else(|| m.default_userid.clone());
    m.inner.lock().await.codes.insert(code.clone(), userid);

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
struct GetTokenQuery {
    corpid: Option<String>,
    corpsecret: Option<String>,
}

async fn gettoken_handler(
    State(m): State<MockWecom>,
    Query(q): Query<GetTokenQuery>,
) -> Json<Value> {
    if q.corpid.as_deref() != Some(m.corp_id.as_str())
        || q.corpsecret.as_deref() != Some(m.secret.as_str())
    {
        return Json(json!({"errcode": 40001, "errmsg": "invalid corpid or secret"}));
    }
    let tok = format!("mock-wecom-token-{}", Uuid::new_v4());
    m.inner.lock().await.tokens.insert(tok.clone(), true);
    Json(json!({
        "errcode": 0,
        "errmsg": "ok",
        "access_token": tok,
        "expires_in": 7200,
    }))
}

#[derive(Debug, Deserialize)]
struct GetUserInfoQuery {
    access_token: Option<String>,
    code: Option<String>,
}

async fn getuserinfo_handler(
    State(m): State<MockWecom>,
    Query(q): Query<GetUserInfoQuery>,
) -> Json<Value> {
    let tok = match q.access_token.as_deref() {
        Some(t) if !t.is_empty() => t,
        _ => return Json(json!({"errcode": 41001, "errmsg": "access_token missing"})),
    };
    {
        let inner = m.inner.lock().await;
        if !inner.tokens.contains_key(tok) {
            return Json(json!({"errcode": 40014, "errmsg": "invalid access_token"}));
        }
    }
    let code = match q.code.as_deref() {
        Some(c) if !c.is_empty() => c,
        _ => return Json(json!({"errcode": 40029, "errmsg": "invalid code"})),
    };
    let userid = {
        let mut inner = m.inner.lock().await;
        match inner.codes.remove(code) {
            Some(u) => u,
            None => return Json(json!({"errcode": 40029, "errmsg": "invalid code"})),
        }
    };
    Json(json!({
        "errcode": 0,
        "errmsg": "ok",
        "UserId": userid,
        "DeviceId": "mock-device",
    }))
}

/// Drive authorize→redirect without a browser (CI / ATLAS_I2_NO_BROWSER=1).
pub async fn drive_wecom_authorize_for_code(
    authorize_url: &str,
) -> Result<(String, Option<String>), String> {
    crate::mock_oidc::drive_authorize_for_code(authorize_url).await
}
