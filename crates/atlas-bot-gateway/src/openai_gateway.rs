//! Optional OpenAI-compatible fallback (`ATLAS_GATEWAY_BACKEND=openai`).
//!
//! Not the P3.5 mainline (scheme B is CLI). Sync chat.completions; interrupt
//! aborts the in-flight HTTP request via oneshot cancel (closed-set, no fake
//! success).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::{oneshot, Mutex, RwLock};
use tracing::info;

use crate::{
    AgentRecord, Gateway, GatewayError, TranscriptEntry, RuntimeHint, DEFAULT_AGENT_ID,
    DEFAULT_AGENT_NAME,
};

pub const ENV_OPENAI_BASE: &str = "ATLAS_OPENAI_BASE_URL";
pub const ENV_OPENAI_KEY: &str = "ATLAS_OPENAI_API_KEY";
pub const ENV_OPENAI_MODEL: &str = "ATLAS_OPENAI_MODEL";

#[derive(Default)]
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
    turn_tx: tokio::sync::broadcast::Sender<RuntimeHint>,
    base: String,
    api_key: String,
    model: String,
    client: reqwest::Client,
}

#[derive(Clone)]
pub struct OpenAiCompatGateway {
    shared: Arc<Shared>,
}

impl OpenAiCompatGateway {
    pub fn from_env() -> Result<Self, String> {
        let base = std::env::var(ENV_OPENAI_BASE)
            .unwrap_or_else(|_| "https://api.openai.com/v1".into())
            .trim_end_matches('/')
            .to_string();
        let api_key = std::env::var(ENV_OPENAI_KEY).map_err(|_| {
            format!("missing {ENV_OPENAI_KEY} for ATLAS_GATEWAY_BACKEND=openai")
        })?;
        let model =
            std::env::var(ENV_OPENAI_MODEL).unwrap_or_else(|_| "gpt-4o-mini".into());
        Ok(Self::new(base, api_key, model))
    }

    pub fn new(base: String, api_key: String, model: String) -> Self {
        let (turn_tx, _) = tokio::sync::broadcast::channel(64);
        let mut agents = HashMap::new();
        agents.insert(
            DEFAULT_AGENT_ID.to_string(),
            AgentRecord {
                id: DEFAULT_AGENT_ID.to_string(),
                name: DEFAULT_AGENT_NAME.to_string(),
                description: "P3.5 OpenAI-compat agent".to_string(),
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
                text: "openai gateway ready".to_string(),
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
                turn_tx,
                base,
                api_key,
                model,
                client: reqwest::Client::new(),
            }),
        }
    }

    pub fn subscribe_turns(&self) -> tokio::sync::broadcast::Receiver<RuntimeHint> {
        self.shared.turn_tx.subscribe()
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

    async fn set_running(&self, agent_id: &str, running: bool) {
        let mut guard = self.shared.inner.write().await;
        if let Some(a) = guard.agents.get_mut(agent_id) {
            a.is_running = running;
        }
    }

    async fn clear_pending(&self, agent_id: &str) {
        let mut pending = self.shared.pending.write().await;
        pending.remove(agent_id);
    }

    async fn commit_turn(
        &self,
        agent_id: &str,
        prompt: &str,
        reply: &str,
    ) -> Result<(String, Vec<TranscriptEntry>), GatewayError> {
        let mut guard = self.shared.inner.write().await;
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
            text: prompt.to_string(),
            seq,
        };
        let reply_seq = {
            let n = guard.next_entry_seq.get_mut(agent_id).unwrap();
            let s = *n;
            *n += 1;
            s
        };
        let preview = reply.trim().to_string();
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
        Ok((preview, vec![user_entry, assistant_entry]))
    }

    async fn chat_complete(
        &self,
        agent_id: &str,
        prompt: &str,
        kill_rx: oneshot::Receiver<()>,
    ) -> Result<String, GatewayError> {
        let url = format!("{}/chat/completions", self.shared.base);
        let body = json!({
            "model": self.shared.model,
            "messages": [
                {
                    "role": "system",
                    "content": format!("You are atlas-bot agent {agent_id}. Reply concisely.")
                },
                { "role": "user", "content": prompt }
            ],
        });
        let req = self
            .shared
            .client
            .post(&url)
            .bearer_auth(&self.shared.api_key)
            .json(&body)
            .send();

        let resp = tokio::select! {
            r = req => r.map_err(|e| GatewayError::Upstream(e.to_string()))?,
            _ = kill_rx => {
                return Err(GatewayError::Upstream("gateway/run-interrupted".into()));
            }
        };

        let status = resp.status();
        let val: Value = resp
            .json()
            .await
            .map_err(|e| GatewayError::Upstream(e.to_string()))?;
        if !status.is_success() {
            return Err(GatewayError::Upstream(val.to_string()));
        }
        let text = val
            .pointer("/choices/0/message/content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if text.is_empty() {
            return Err(GatewayError::Upstream("empty OpenAI content".into()));
        }
        if text == prompt || text == format!("echo: {prompt}") {
            return Err(GatewayError::Upstream(
                "OpenAI reply looked like stub echo; refusing".into(),
            ));
        }
        Ok(text)
    }

    async fn send_prompt(&self, agent_id: &str, args: &Value) -> Result<Value, GatewayError> {
        let prompt = args
            .get("prompt")
            .and_then(|v| v.as_str())
            .ok_or_else(|| GatewayError::InvalidArgs("missing prompt".into()))?
            .to_string();
        self.mark_running(agent_id).await?;
        self.clear_pending(agent_id).await;
        let (cancel_tx, cancel_rx) = oneshot::channel::<()>();
        {
            let mut map = self.shared.pending.write().await;
            map.insert(
                agent_id.to_string(),
                Arc::new(PendingTurn {
                    cancel: Mutex::new(Some(cancel_tx)),
                }),
            );
        }
        let result = self.chat_complete(agent_id, &prompt, cancel_rx).await;
        self.clear_pending(agent_id).await;
        match result {
            Ok(reply) => {
                let (preview, entries) = self.commit_turn(agent_id, &prompt, &reply).await?;
                let _ = self.shared.turn_tx.send(RuntimeHint::Finished {
                    agent_id: agent_id.to_string(),
                    preview: preview.clone(),
                    user_text: prompt,
                    entries: entries.clone(),
                });
                Ok(json!({
                    "accepted": true,
                    "completed": true,
                    "preview": preview,
                    "entries": entries,
                    "hubEmitTurnFinished": true,
                }))
            }
            Err(e) => {
                self.set_running(agent_id, false).await;
                Err(e)
            }
        }
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
        let pending = {
            let mut map = self.shared.pending.write().await;
            map.remove(&target)
        };
        let mut cancelled = false;
        if let Some(p) = pending {
            let mut slot = p.cancel.lock().await;
            if let Some(tx) = slot.take() {
                cancelled = tx.send(()).is_ok();
            }
        }
        let mut guard = self.shared.inner.write().await;
        let Some(agent) = guard.agents.get_mut(&target) else {
            return Err(GatewayError::AgentNotFound(target));
        };
        let had_active = agent.is_running || cancelled;
        if agent.is_running {
            agent.is_running = false;
        }
        Ok(json!({ "hadActiveRun": had_active, "killed": cancelled }))
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
impl Gateway for OpenAiCompatGateway {
    async fn invoke(&self, agent_id: &str, name: &str, args: Value) -> Result<Value, GatewayError> {
        self.shared.invokes.fetch_add(1, Ordering::SeqCst);
        info!(%agent_id, %name, "openai gateway invoke");
        self.dispatch(agent_id, name, args).await
    }

    fn invoke_count(&self) -> u64 {
        self.shared.invokes.load(Ordering::SeqCst)
    }
}
