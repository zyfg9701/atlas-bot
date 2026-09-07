//! In-box gateway stub for atlas-bot P3.
//!
//! Hot commands: `listAgents`, `createAgent`, `sendPrompt`,
//! `getAgentTranscriptTail`, `interruptAgentRun`. Other catalog names
//! return [`GatewayError::UnknownMethod`] which the hub maps to
//! `command_rejected` / `gateway/unknown-method`.

#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::{oneshot, Mutex, RwLock};
use tracing::info;

/// Default seed agent used by the in-memory stub.
pub const DEFAULT_AGENT_ID: &str = "agt_1";
pub const DEFAULT_AGENT_NAME: &str = "Watcher";

/// Default delay before a sendPrompt turn finishes (interruptible window).
pub const DEFAULT_TURN_DELAY_MS: u64 = 300;

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
    async fn invoke(&self, agent_id: &str, name: &str, args: Value) -> Result<Value, GatewayError>;
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
    next_agent_n: u64,
}

struct PendingTurn {
    cancel: Mutex<Option<oneshot::Sender<()>>>,
}

struct Shared {
    inner: RwLock<Inner>,
    invokes: AtomicU64,
    pending: RwLock<HashMap<String, Arc<PendingTurn>>>,
    turn_delay: Duration,
    turn_tx: tokio::sync::broadcast::Sender<TurnFinishedHint>,
}

/// In-memory gateway stub.
#[derive(Clone)]
pub struct InMemoryGateway {
    shared: Arc<Shared>,
}

#[derive(Debug, Clone)]
pub struct TurnFinishedHint {
    pub agent_id: String,
    pub preview: String,
    pub user_text: String,
    pub entries: Vec<TranscriptEntry>,
}

impl Default for InMemoryGateway {
    fn default() -> Self {
        Self::new()
    }
}

impl InMemoryGateway {
    pub fn new() -> Self {
        Self::with_turn_delay(Duration::from_millis(DEFAULT_TURN_DELAY_MS))
    }

    pub fn with_turn_delay(turn_delay: Duration) -> Self {
        let (turn_tx, _) = tokio::sync::broadcast::channel(64);
        let mut agents = HashMap::new();
        agents.insert(
            DEFAULT_AGENT_ID.to_string(),
            AgentRecord {
                id: DEFAULT_AGENT_ID.to_string(),
                name: DEFAULT_AGENT_NAME.to_string(),
                description: "P3 stub agent".to_string(),
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
            shared: Arc::new(Shared {
                inner: RwLock::new(Inner {
                    agents,
                    transcripts,
                    next_entry_seq,
                    next_agent_n: 2,
                }),
                invokes: AtomicU64::new(0),
                pending: RwLock::new(HashMap::new()),
                turn_delay,
                turn_tx,
            }),
        }
    }

    pub fn subscribe_turns(&self) -> tokio::sync::broadcast::Receiver<TurnFinishedHint> {
        self.shared.turn_tx.subscribe()
    }

    pub fn turn_sender(&self) -> tokio::sync::broadcast::Sender<TurnFinishedHint> {
        self.shared.turn_tx.clone()
    }

    /// Snapshot transcript (does **not** increment invoke_count).
    pub async fn snapshot_transcript(&self, agent_id: &str) -> Vec<TranscriptEntry> {
        let guard = self.shared.inner.read().await;
        guard
            .transcripts
            .get(agent_id)
            .cloned()
            .unwrap_or_default()
    }

    /// Snapshot agents (does **not** increment invoke_count).
    pub async fn snapshot_agents(&self) -> Vec<AgentRecord> {
        let guard = self.shared.inner.read().await;
        let mut v: Vec<_> = guard.agents.values().cloned().collect();
        v.sort_by(|a, b| a.id.cmp(&b.id));
        v
    }

    fn agent_summary(a: &AgentRecord) -> Value {
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
            "avatarDataUrl": Value::Null,
            "awaitingUserResponse": Value::Null,
            "lastEntry": Value::Null,
            "lastMessageId": Value::Null,
        })
    }

    async fn list_agents(&self) -> Result<Value, GatewayError> {
        let guard = self.shared.inner.read().await;
        let mut list: Vec<Value> = guard.agents.values().map(Self::agent_summary).collect();
        list.sort_by(|a, b| {
            a["id"]
                .as_str()
                .unwrap_or("")
                .cmp(b["id"].as_str().unwrap_or(""))
        });
        Ok(Value::Array(list))
    }

    async fn create_agent(&self, args: &Value) -> Result<Value, GatewayError> {
        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| GatewayError::InvalidArgs("missing name".into()))?
            .to_string();
        let description = args
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let mut guard = self.shared.inner.write().await;
        let n = guard.next_agent_n;
        guard.next_agent_n += 1;
        let id = format!("agt_{n}");
        let created_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as f64)
            .unwrap_or(0.0);
        let record = AgentRecord {
            id: id.clone(),
            name: name.clone(),
            description,
            is_running: false,
            created_at,
        };
        let seed = TranscriptEntry {
            id: format!("msg_seed_{id}"),
            role: "assistant".to_string(),
            text: format!("agent {name} ready"),
            seq: 1,
        };
        guard.agents.insert(id.clone(), record.clone());
        guard.transcripts.insert(id.clone(), vec![seed.clone()]);
        guard.next_entry_seq.insert(id.clone(), 2);
        Ok(json!({
            "agent": Self::agent_summary(&record),
            "transcript": [seed],
            "agentId": id,
        }))
    }

    async fn mark_running(&self, agent_id: &str) -> Result<(), GatewayError> {
        let mut guard = self.shared.inner.write().await;
        let Some(a) = guard.agents.get_mut(agent_id) else {
            return Err(GatewayError::AgentNotFound(agent_id.to_string()));
        };
        if a.is_running {
            return Err(GatewayError::InvalidArgs(
                "agent already running; interrupt first".into(),
            ));
        }
        a.is_running = true;
        Ok(())
    }

    async fn clear_pending(&self, agent_id: &str) {
        let mut pending = self.shared.pending.write().await;
        pending.remove(agent_id);
    }

    async fn finish_turn(&self, agent_id: &str, prompt: &str) -> Result<(), GatewayError> {
        let mut guard = self.shared.inner.write().await;
        if !guard.agents.contains_key(agent_id) {
            return Err(GatewayError::AgentNotFound(agent_id.to_string()));
        }
        let still_running = guard
            .agents
            .get(agent_id)
            .map(|a| a.is_running)
            .unwrap_or(false);
        if !still_running {
            return Ok(());
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
            text: prompt.to_string(),
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
        guard
            .transcripts
            .entry(agent_id.to_string())
            .or_default()
            .push(user_entry.clone());
        guard
            .transcripts
            .get_mut(agent_id)
            .unwrap()
            .push(assistant_entry.clone());
        if let Some(a) = guard.agents.get_mut(agent_id) {
            a.is_running = false;
        }
        drop(guard);
        self.clear_pending(agent_id).await;
        let _ = self.shared.turn_tx.send(TurnFinishedHint {
            agent_id: agent_id.to_string(),
            preview,
            user_text: prompt.to_string(),
            entries: vec![user_entry, assistant_entry],
        });
        Ok(())
    }

    async fn interrupt_agent_run(
        &self,
        envelope_agent_id: &str,
        args: &Value,
    ) -> Result<Value, GatewayError> {
        let target = args
            .get("agentId")
            .and_then(|v| v.as_str())
            .or_else(|| args.get("id").and_then(|v| v.as_str()))
            .unwrap_or(envelope_agent_id)
            .to_string();

        let cancel = {
            let mut pending = self.shared.pending.write().await;
            pending.remove(&target)
        };
        if let Some(p) = cancel {
            let mut slot = p.cancel.lock().await;
            if let Some(tx) = slot.take() {
                let _ = tx.send(());
            }
        }

        let mut guard = self.shared.inner.write().await;
        let Some(agent) = guard.agents.get_mut(&target) else {
            return Err(GatewayError::AgentNotFound(target));
        };
        let had_active = agent.is_running;
        if had_active {
            agent.is_running = false;
            let seq = {
                let n = guard.next_entry_seq.entry(target.clone()).or_insert(1);
                let s = *n;
                *n += 1;
                s
            };
            let notice = TranscriptEntry {
                id: format!("msg_int_{seq}"),
                role: "system".to_string(),
                text: "run interrupted".to_string(),
                seq,
            };
            guard
                .transcripts
                .entry(target)
                .or_default()
                .push(notice);
        }
        Ok(json!({ "hadActiveRun": had_active }))
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
        let guard = self.shared.inner.read().await;
        if !guard.agents.contains_key(agent_id) {
            return Err(GatewayError::AgentNotFound(agent_id.to_string()));
        }
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

    async fn send_prompt(&self, agent_id: &str, args: &Value) -> Result<Value, GatewayError> {
        let prompt = args
            .get("prompt")
            .and_then(|v| v.as_str())
            .ok_or_else(|| GatewayError::InvalidArgs("missing prompt".into()))?
            .to_string();
        let immediate = args
            .get("immediate")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        self.mark_running(agent_id).await?;
        self.clear_pending(agent_id).await;

        let (cancel_tx, cancel_rx) = oneshot::channel::<()>();
        {
            let mut pending = self.shared.pending.write().await;
            pending.insert(
                agent_id.to_string(),
                Arc::new(PendingTurn {
                    cancel: Mutex::new(Some(cancel_tx)),
                }),
            );
        }

        let delay = if immediate {
            Duration::from_millis(0)
        } else {
            self.shared.turn_delay
        };
        let gw = self.clone();
        let aid = agent_id.to_string();
        tokio::spawn(async move {
            let cancelled = if delay.is_zero() {
                false
            } else {
                tokio::select! {
                    _ = tokio::time::sleep(delay) => false,
                    _ = cancel_rx => true,
                }
            };
            if cancelled {
                return;
            }
            let _ = gw.finish_turn(&aid, &prompt).await;
        });
        Ok(json!({ "accepted": true }))
    }

    async fn dispatch(&self, agent_id: &str, name: &str, args: Value) -> Result<Value, GatewayError> {
        match name {
            "listAgents" => self.list_agents().await,
            "createAgent" => self.create_agent(&args).await,
            "sendPrompt" => self.send_prompt(agent_id, &args).await,
            "interruptAgentRun" => self.interrupt_agent_run(agent_id, &args).await,
            "getAgentTranscriptTail" => self.get_transcript_tail(agent_id, &args).await,
            other => Err(GatewayError::UnknownMethod(other.to_string())),
        }
    }
}

#[async_trait]
impl Gateway for InMemoryGateway {
    async fn invoke(&self, agent_id: &str, name: &str, args: Value) -> Result<Value, GatewayError> {
        self.shared.invokes.fetch_add(1, Ordering::SeqCst);
        info!(%agent_id, %name, "gateway invoke");
        self.dispatch(agent_id, name, args).await
    }

    fn invoke_count(&self) -> u64 {
        self.shared.invokes.load(Ordering::SeqCst)
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HttpInvokeRequest {
    pub agent_id: String,
    pub name: String,
    pub args: Value,
}

#[derive(Clone)]
struct HttpState {
    gw: InMemoryGateway,
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

pub fn http_router(gw: Arc<InMemoryGateway>) -> Router {
    Router::new()
        .route("/invoke", post(http_invoke))
        .with_state(HttpState { gw: (*gw).clone() })
}

pub async fn serve_http(
    gw: Arc<InMemoryGateway>,
    addr: std::net::SocketAddr,
) -> Result<(), std::io::Error> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!(%addr, "gateway HTTP listening");
    axum::serve(listener, http_router(gw)).await
}

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
        let gw = InMemoryGateway::with_turn_delay(Duration::from_millis(20));
        let agents = gw.invoke("agt_1", "listAgents", json!({})).await.unwrap();
        assert!(agents.as_array().unwrap().iter().any(|a| a["id"] == "agt_1"));
        gw.invoke(
            "agt_1",
            "sendPrompt",
            json!({ "agentId": "agt_1", "prompt": "hi" }),
        )
        .await
        .unwrap();
        tokio::time::sleep(Duration::from_millis(80)).await;
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
    async fn create_agent_appears_in_list() {
        let gw = InMemoryGateway::new();
        let created = gw
            .invoke("agt_1", "createAgent", json!({ "name": "Scout" }))
            .await
            .unwrap();
        let id = created["agentId"].as_str().unwrap().to_string();
        assert!(id.starts_with("agt_"));
        assert_eq!(created["agent"]["name"], "Scout");
        let agents = gw.invoke("agt_1", "listAgents", json!({})).await.unwrap();
        assert!(agents.as_array().unwrap().iter().any(|a| a["id"] == id));
    }

    #[tokio::test]
    async fn interrupt_running_turn() {
        let gw = InMemoryGateway::with_turn_delay(Duration::from_secs(5));
        gw.invoke(
            "agt_1",
            "sendPrompt",
            json!({ "agentId": "agt_1", "prompt": "long" }),
        )
        .await
        .unwrap();
        tokio::time::sleep(Duration::from_millis(30)).await;
        let r = gw
            .invoke(
                "agt_1",
                "interruptAgentRun",
                json!({ "agentId": "agt_1" }),
            )
            .await
            .unwrap();
        assert_eq!(r["hadActiveRun"], true);
        gw.invoke(
            "agt_1",
            "sendPrompt",
            json!({ "agentId": "agt_1", "prompt": "again", "immediate": true }),
        )
        .await
        .unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
        let agents = gw.invoke("agt_1", "listAgents", json!({})).await.unwrap();
        let a = agents
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["id"] == "agt_1")
            .unwrap();
        assert_eq!(a["isRunning"], false);
    }

    #[tokio::test]
    async fn unknown_method() {
        let gw = InMemoryGateway::new();
        let err = gw
            .invoke("agt_1", "deleteAgents", json!({}))
            .await
            .unwrap_err();
        assert!(matches!(err, GatewayError::UnknownMethod(_)));
    }

    #[tokio::test]
    async fn snapshot_does_not_count_as_invoke() {
        let gw = InMemoryGateway::new();
        let before = gw.invoke_count();
        let _ = gw.snapshot_transcript("agt_1").await;
        let _ = gw.snapshot_agents().await;
        assert_eq!(gw.invoke_count(), before);
    }
}
