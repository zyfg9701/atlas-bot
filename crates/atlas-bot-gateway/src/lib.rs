//! In-box gateway stub for atlas-bot P1.
//!
//! Implements the minimal command surface used by the Bot-Relay hot path:
//! `listAgents`, `sendPrompt`, `getAgentTranscriptTail`. Other catalog names
//! return [`GatewayError::UnknownMethod`] which the hub maps to
//! `command_rejected` / `gateway/unknown-method`.

#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::RwLock;
use tracing::info;

/// Default seed agent used by the in-memory stub.
pub const DEFAULT_AGENT_ID: &str = "agt_1";
pub const DEFAULT_AGENT_NAME: &str = "Watcher";

#[derive(Debug, thiserror::Error)]
pub enum GatewayError {
    #[error("gateway/unknown-method: {0}")]
    UnknownMethod(String),
    #[error("invalid args: {0}")]
    InvalidArgs(String),
    #[error("agent not found: {0}")]
    AgentNotFound(String),
    #[error("upstream: {0}")]
    Upstream(String),
}

/// Trait the hub uses for in-process passthrough (preferred) or HTTP client.
#[async_trait]
pub trait Gateway: Send + Sync {
    /// Invoke a gateway command. Hub passes `name`/`args` verbatim.
    async fn invoke(&self, agent_id: &str, name: &str, args: Value) -> Result<Value, GatewayError>;

    /// How many times [`Self::invoke`] has been called (cold/hot tests).
    fn invoke_count(&self) -> u64;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRecord {
    pub id: String,
    pub name: String,
    pub description: String,
    pub is_running: bool,
    pub created_at: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptEntry {
    pub id: String,
    pub role: String,
    pub text: String,
    pub seq: u64,
}

#[derive(Debug, Default)]
struct Inner {
    agents: HashMap<String, AgentRecord>,
    transcripts: HashMap<String, Vec<TranscriptEntry>>,
    next_entry_seq: HashMap<String, u64>,
}

/// In-memory gateway stub.
pub struct InMemoryGateway {
    inner: RwLock<Inner>,
    invokes: AtomicU64,
    /// When set, sendPrompt notifies this channel with (agent_id, preview).
    pub turn_tx: tokio::sync::broadcast::Sender<TurnFinishedHint>,
}

#[derive(Debug, Clone)]
pub struct TurnFinishedHint {
    pub agent_id: String,
    pub preview: String,
}

impl Default for InMemoryGateway {
    fn default() -> Self {
        Self::new()
    }
}

impl InMemoryGateway {
    pub fn new() -> Self {
        let (turn_tx, _) = tokio::sync::broadcast::channel(64);
        let mut agents = HashMap::new();
        agents.insert(
            DEFAULT_AGENT_ID.to_string(),
            AgentRecord {
                id: DEFAULT_AGENT_ID.to_string(),
                name: DEFAULT_AGENT_NAME.to_string(),
                description: "P1 stub agent".to_string(),
                is_running: false,
                created_at: 1_700_000_000_000.0,
            },
        );
        let mut transcripts = HashMap::new();
        transcripts.insert(
            DEFAULT_AGENT_ID.to_string(),
            vec![TranscriptEntry {
                id: "msg_seed".to_string(),
                role: "assistant".to_string(),
                text: "stub ready".to_string(),
                seq: 1,
            }],
        );
        let mut next_entry_seq = HashMap::new();
        next_entry_seq.insert(DEFAULT_AGENT_ID.to_string(), 2);
        Self {
            inner: RwLock::new(Inner {
                agents,
                transcripts,
                next_entry_seq,
            }),
            invokes: AtomicU64::new(0),
            turn_tx,
        }
    }

    pub fn subscribe_turns(&self) -> tokio::sync::broadcast::Receiver<TurnFinishedHint> {
        self.turn_tx.subscribe()
    }

    async fn list_agents(&self) -> Result<Value, GatewayError> {
        let guard = self.inner.read().await;
        let list: Vec<Value> = guard
            .agents
            .values()
            .map(|a| {
                json!({
                    "id": a.id,
                    "name": a.name,
                    "description": a.description,
                    "isRunning": a.is_running,
                    "isActive": true,
                    "isGroup": false,
                    "hasUnread": false,
                    "isComposingMessage": false,
                    "createdAt": a.created_at,
                })
            })
            .collect();
        Ok(Value::Array(list))
    }

    async fn send_prompt(&self, agent_id: &str, args: &Value) -> Result<Value, GatewayError> {
        let prompt = args
            .get("prompt")
            .and_then(|v| v.as_str())
            .ok_or_else(|| GatewayError::InvalidArgs("missing prompt".into()))?
            .to_string();
        let mut guard = self.inner.write().await;
        if !guard.agents.contains_key(agent_id) {
            return Err(GatewayError::AgentNotFound(agent_id.to_string()));
        }
        let seq = {
            let n = guard.next_entry_seq.entry(agent_id.to_string()).or_insert(1);
            let s = *n;
            *n += 1;
            s
        };
        let user_entry = TranscriptEntry {
            id: format!("msg_u_{seq}"),
            role: "user".to_string(),
            text: prompt.clone(),
            seq,
        };
        let reply_seq = {
            let n = guard.next_entry_seq.get_mut(agent_id).unwrap();
            let s = *n;
            *n += 1;
            s
        };
        let preview = format!("echo: {prompt}");
        let assistant_entry = TranscriptEntry {
            id: format!("msg_a_{reply_seq}"),
            role: "assistant".to_string(),
            text: preview.clone(),
            seq: reply_seq,
        };
        let t = guard.transcripts.entry(agent_id.to_string()).or_default();
        t.push(user_entry);
        t.push(assistant_entry);
        if let Some(a) = guard.agents.get_mut(agent_id) {
            a.is_running = false;
        }
        drop(guard);
        let _ = self.turn_tx.send(TurnFinishedHint {
            agent_id: agent_id.to_string(),
            preview,
        });
        Ok(json!({ "accepted": true }))
    }

    async fn get_transcript_tail(
        &self,
        agent_id: &str,
        args: &Value,
    ) -> Result<Value, GatewayError> {
        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64().or_else(|| v.as_f64().map(|f| f as u64)))
            .unwrap_or(20) as usize;
        let before_seq = args
            .get("beforeSeq")
            .and_then(|v| v.as_u64().or_else(|| v.as_f64().map(|f| f as u64)));
        let guard = self.inner.read().await;
        let entries = guard
            .transcripts
            .get(agent_id)
            .cloned()
            .unwrap_or_default();
        let filtered: Vec<_> = entries
            .into_iter()
            .filter(|e| before_seq.map(|b| e.seq < b).unwrap_or(true))
            .collect();
        let start = filtered.len().saturating_sub(limit);
        let page = &filtered[start..];
        Ok(json!({
            "entries": page,
            "agentId": agent_id,
        }))
    }
}

#[async_trait]
impl Gateway for InMemoryGateway {
    async fn invoke(&self, agent_id: &str, name: &str, args: Value) -> Result<Value, GatewayError> {
        self.invokes.fetch_add(1, Ordering::SeqCst);
        info!(%agent_id, %name, "gateway invoke");
        match name {
            "listAgents" => self.list_agents().await,
            "sendPrompt" => self.send_prompt(agent_id, &args).await,
            "getAgentTranscriptTail" => self.get_transcript_tail(agent_id, &args).await,
            other => Err(GatewayError::UnknownMethod(other.to_string())),
        }
    }

    fn invoke_count(&self) -> u64 {
        self.invokes.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl Gateway for Arc<InMemoryGateway> {
    async fn invoke(&self, agent_id: &str, name: &str, args: Value) -> Result<Value, GatewayError> {
        (**self).invoke(agent_id, name, args).await
    }

    fn invoke_count(&self) -> u64 {
        (**self).invoke_count()
    }
}

/// HTTP request body for localhost passthrough.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HttpInvokeRequest {
    pub agent_id: String,
    pub name: String,
    pub args: Value,
}

#[derive(Clone)]
struct HttpState {
    gw: Arc<InMemoryGateway>,
}

async fn http_invoke(
    State(st): State<HttpState>,
    Json(body): Json<HttpInvokeRequest>,
) -> Result<Json<Value>, (axum::http::StatusCode, Json<Value>)> {
    match st.gw.invoke(&body.agent_id, &body.name, body.args).await {
        Ok(v) => Ok(Json(v)),
        Err(GatewayError::UnknownMethod(m)) => Err((
            axum::http::StatusCode::NOT_FOUND,
            Json(json!({ "error": "gateway/unknown-method", "method": m })),
        )),
        Err(e) => Err((
            axum::http::StatusCode::BAD_REQUEST,
            Json(json!({ "error": e.to_string() })),
        )),
    }
}

/// Build an axum router exposing `POST /invoke`.
pub fn http_router(gw: Arc<InMemoryGateway>) -> Router {
    Router::new()
        .route("/invoke", post(http_invoke))
        .with_state(HttpState { gw })
}

/// Serve the HTTP gateway on `addr` (e.g. `127.0.0.1:8787`).
pub async fn serve_http(
    gw: Arc<InMemoryGateway>,
    addr: std::net::SocketAddr,
) -> Result<(), std::io::Error> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!(%addr, "gateway HTTP listening");
    axum::serve(listener, http_router(gw)).await
}

/// HTTP client that posts to a localhost gateway.
pub struct HttpGatewayClient {
    base: String,
    client: reqwest::Client,
    invokes: AtomicU64,
}

impl HttpGatewayClient {
    pub fn new(base: impl Into<String>) -> Self {
        Self {
            base: base.into().trim_end_matches('/').to_string(),
            client: reqwest::Client::new(),
            invokes: AtomicU64::new(0),
        }
    }
}

#[async_trait]
impl Gateway for HttpGatewayClient {
    async fn invoke(&self, agent_id: &str, name: &str, args: Value) -> Result<Value, GatewayError> {
        self.invokes.fetch_add(1, Ordering::SeqCst);
        let url = format!("{}/invoke", self.base);
        let body = json!({
            "agentId": agent_id,
            "name": name,
            "args": args,
        });
        let resp = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| GatewayError::Upstream(e.to_string()))?;
        let status = resp.status();
        let val: Value = resp
            .json()
            .await
            .map_err(|e| GatewayError::Upstream(e.to_string()))?;
        if status.as_u16() == 404 {
            let m = val
                .get("method")
                .and_then(|v| v.as_str())
                .unwrap_or(name)
                .to_string();
            return Err(GatewayError::UnknownMethod(m));
        }
        if !status.is_success() {
            return Err(GatewayError::Upstream(val.to_string()));
        }
        Ok(val)
    }

    fn invoke_count(&self) -> u64 {
        self.invokes.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn list_send_tail_roundtrip() {
        let gw = InMemoryGateway::new();
        let agents = gw.invoke("agt_1", "listAgents", json!({})).await.unwrap();
        assert!(agents.as_array().unwrap().iter().any(|a| a["id"] == "agt_1"));
        gw.invoke(
            "agt_1",
            "sendPrompt",
            json!({ "agentId": "agt_1", "prompt": "hi" }),
        )
        .await
        .unwrap();
        let tail = gw
            .invoke(
                "agt_1",
                "getAgentTranscriptTail",
                json!({ "id": "agt_1", "limit": 10 }),
            )
            .await
            .unwrap();
        let entries = tail["entries"].as_array().unwrap();
        assert!(entries.iter().any(|e| e["text"] == "hi"));
        assert_eq!(gw.invoke_count(), 3);
    }

    #[tokio::test]
    async fn unknown_method() {
        let gw = InMemoryGateway::new();
        let err = gw
            .invoke("agt_1", "createAgent", json!({}))
            .await
            .unwrap_err();
        assert!(matches!(err, GatewayError::UnknownMethod(_)));
    }
}
