//! R1/R2/RR1 Box Sidecar: long-lived in-process runtime behind Gateway HTTP.
//!
//! Per-agent conversation history, workspace root (`ATLAS_BOX_WORKSPACE`), tool
//! whitelist (LIST_DIR / READ / WRITE / RUN ls|pwd|cat|mkdir), streaming
//! [`RuntimeHint`]s, and P5 VNC + attachments under the agent workspace.
//!
//! RR1: optional OpenAI-compat LLM via `ATLAS_BOX_LLM_*`. When configured,
//! pure-chat turns call `POST {base}/chat/completions` with history→`messages[]`.
//! When unset / `ATLAS_BOX_LLM_MODE=mock|off`, keep deterministic `compose_reply`.
//!
//! **Tool vs LLM order (nailed):** tool triggers run first; if any tool evidence
//! is produced, the turn replies with `compose_reply` (tools stay greppable /
//! deterministic). Pure chat (no tool hit) uses the LLM when configured.
//!
//! Interrupt: cancel in-flight delay **or** LLM HTTP via oneshot; idle interrupt
//! returns closed-set `command_rejected/no_active_run` (never fake success).
//!
//! GA1 tool approval: `WRITE_FILE` / `RUN mkdir` / `RUN cat` hang pending
//! Allow/Deny (timeout=Deny). `LIST_DIR` / `READ_FILE` / `RUN ls|pwd` exempt.
//! Sync invoke **holds until decision/timeout**. See `docs/tool-approval-runbook.md`.

#![allow(dead_code)]

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::process::Command as StdCommand;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::{oneshot, Mutex, RwLock};
use tracing::{info, warn};
use uuid::Uuid;

use crate::tool_approval::{
    format_pending_summary, ApprovalDecision, ApprovalState, GateOutcome, ToolApprovalMode,
};
use crate::{
    is_expired, mint_vnc_descriptor_value, now_ms, AgentRecord, Gateway, GatewayError, RuntimeHint,
    TranscriptEntry, VncMode, VncTokenError, VncTokenRecord, ATTACHMENT_OBJECT_MAX_BYTES,
    BOX_TEXT_FILE_MAX_BYTES, DEFAULT_AGENT_ID, DEFAULT_AGENT_NAME, DEFAULT_ATTACH_TTL_SECS,
    DEFAULT_VNC_STUB_BASE, ENV_ATTACH_TTL_SECS, ENV_VNC_MODE, ENV_VNC_UPSTREAM,
    UPLOAD_ARGS_JSON_MAX_BYTES,
};

/// Workspace root for agent subdirs.
pub const ENV_BOX_WORKSPACE: &str = "ATLAS_BOX_WORKSPACE";
/// Interruptible delay before completing a sendPrompt (ms). Default 50.
pub const ENV_BOX_TURN_DELAY_MS: &str = "ATLAS_BOX_TURN_DELAY_MS";
/// Default turn delay when env unset.
pub const DEFAULT_BOX_TURN_DELAY_MS: u64 = 50;
/// B1: Hub loopback ingest URL (e.g. http://127.0.0.1:7701/internal/runtime-hint).
pub const ENV_HUB_EVENT_URL: &str = "ATLAS_HUB_EVENT_URL";
/// Optional shared token sent as Bearer / X-Atlas-Event-Token.
pub const ENV_HUB_EVENT_TOKEN: &str = "ATLAS_HUB_EVENT_TOKEN";
/// POST timeout for B1 RuntimeHint push (ms).
pub const HUB_EVENT_POST_TIMEOUT_MS: u64 = 500;

/// OpenAI-compat base URL for Box LLM (e.g. `http://127.0.0.1:11434/v1`).
pub const ENV_BOX_LLM_BASE_URL: &str = "ATLAS_BOX_LLM_BASE_URL";
/// API key for Box LLM (may be empty for local). Never logged / never in healthz.
pub const ENV_BOX_LLM_API_KEY: &str = "ATLAS_BOX_LLM_API_KEY";
/// Model name for Box LLM chat/completions.
pub const ENV_BOX_LLM_MODEL: &str = "ATLAS_BOX_LLM_MODEL";
/// `mock` | `off` → force deterministic compose_reply even if URL/model set.
pub const ENV_BOX_LLM_MODE: &str = "ATLAS_BOX_LLM_MODE";
/// Max user+assistant turn pairs retained in memory history (default 32).
pub const ENV_BOX_HISTORY_MAX_TURNS: &str = "ATLAS_BOX_HISTORY_MAX_TURNS";
/// Default history max turns when env unset.
pub const DEFAULT_BOX_HISTORY_MAX_TURNS: usize = 32;
/// Soft session persist: `1`/`true` → append `session.jsonl` under agent dir.
pub const ENV_BOX_SESSION_PERSIST: &str = "ATLAS_BOX_SESSION_PERSIST";
/// HTTP timeout for Box LLM chat/completions (ms). Default 60_000.
pub const ENV_BOX_LLM_TIMEOUT_MS: &str = "ATLAS_BOX_LLM_TIMEOUT_MS";
pub const DEFAULT_BOX_LLM_TIMEOUT_MS: u64 = 60_000;

const DEFAULT_WORKSPACE: &str = "./data/box-workspace";

/// Active OpenAI-compat LLM settings for Box (RR1).
#[derive(Debug, Clone)]
pub struct BoxLlmConfig {
    pub base: String,
    pub api_key: String,
    pub model: String,
    pub timeout: Duration,
}

impl BoxLlmConfig {
    /// Parse from env. Returns `None` when unset or `ATLAS_BOX_LLM_MODE=mock|off`.
    pub fn from_env() -> Option<Self> {
        let mode = std::env::var(ENV_BOX_LLM_MODE)
            .ok()
            .map(|s| s.trim().to_ascii_lowercase())
            .unwrap_or_default();
        if mode == "mock" || mode == "off" {
            return None;
        }
        let base = std::env::var(ENV_BOX_LLM_BASE_URL)
            .ok()
            .map(|s| s.trim().trim_end_matches('/').to_string())
            .filter(|s| !s.is_empty())?;
        let model = std::env::var(ENV_BOX_LLM_MODEL)
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())?;
        let api_key = std::env::var(ENV_BOX_LLM_API_KEY)
            .ok()
            .unwrap_or_default();
        let timeout_ms = std::env::var(ENV_BOX_LLM_TIMEOUT_MS)
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(DEFAULT_BOX_LLM_TIMEOUT_MS);
        Some(Self {
            base,
            api_key,
            model,
            timeout: Duration::from_millis(timeout_ms),
        })
    }
}

/// Documented tool triggers (see runtime-deepen runbook).
pub const TOOL_TRIGGER_LIST_DIR: &str = "LIST_DIR";
pub const TOOL_TRIGGER_RUN_LS: &str = "RUN ls";
pub const TOOL_TRIGGER_RUN_PWD: &str = "RUN pwd";
pub const TOOL_TRIGGER_READ_FILE: &str = "READ_FILE";
pub const TOOL_TRIGGER_WRITE_FILE: &str = "WRITE_FILE";
pub const TOOL_TRIGGER_RUN_CAT: &str = "RUN cat";
pub const TOOL_TRIGGER_RUN_MKDIR: &str = "RUN mkdir";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BoxUploadMeta {
    upload_id: String,
    path: String,
    filename: String,
    bytes: u64,
    created_at: i64,
    state: String,
    agent_id: String,
}

#[derive(Debug, Clone)]
struct BoxUploadRecord {
    path: String,
    filename: String,
    agent_id: String,
    bytes: u64,
    created_at: i64,
    state: String,
}

#[derive(Default)]
struct Inner {
    agents: HashMap<String, AgentRecord>,
    transcripts: HashMap<String, Vec<TranscriptEntry>>,
    /// Full conversation turns (user/assistant text) for multi-turn context.
    history: HashMap<String, Vec<(String, String)>>, // (role, text)
    next_entry_seq: HashMap<String, u64>,
    next_agent_n: u64,
    uploads: HashMap<String, BoxUploadRecord>,
}

struct PendingTurn {
    cancel: Mutex<Option<oneshot::Sender<()>>>,
}

struct Shared {
    inner: RwLock<Inner>,
    invokes: AtomicU64,
    pending: RwLock<HashMap<String, Arc<PendingTurn>>>,
    turn_tx: tokio::sync::broadcast::Sender<RuntimeHint>,
    workspace_root: PathBuf,
    turn_delay: Duration,
    vnc_stub_base: String,
    vnc_mode: VncMode,
    vnc_upstream: Option<String>,
    vnc_tokens: RwLock<HashMap<String, VncTokenRecord>>,
    attach_ttl: Duration,
    /// B1: optional Hub ingest URL for cross-process RuntimeHint POST.
    event_url: Option<String>,
    event_token: Option<String>,
    http: reqwest::Client,
    /// RR1: optional OpenAI-compat LLM (None → deterministic compose_reply).
    llm: Option<BoxLlmConfig>,
    /// Max user+assistant pairs kept in `history` (truncates oldest).
    history_max_turns: usize,
    /// Soft append-only session.jsonl under agent workspace.
    session_persist: bool,
    /// GA1 tool approval gate.
    approval: ApprovalState,
}

/// Box Sidecar gateway: multi-turn + workspace tools + streaming hints + VNC/attach.
#[derive(Clone)]
pub struct BoxSidecarGateway {
    shared: Arc<Shared>,
}

impl BoxSidecarGateway {
    pub fn from_env() -> Self {
        let workspace = std::env::var(ENV_BOX_WORKSPACE)
            .ok()
            .filter(|s| !s.trim().is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(DEFAULT_WORKSPACE));
        let delay_ms = std::env::var(ENV_BOX_TURN_DELAY_MS)
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(DEFAULT_BOX_TURN_DELAY_MS);
        let event_url = std::env::var(ENV_HUB_EVENT_URL)
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let event_token = std::env::var(ENV_HUB_EVENT_TOKEN)
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        if let Some(ref url) = event_url {
            info!(%url, "B1 hub event ingest URL enabled");
        }
        let llm = BoxLlmConfig::from_env();
        if let Some(ref cfg) = llm {
            info!(
                base = %cfg.base,
                model = %cfg.model,
                "RR1 box LLM configured (key redacted)"
            );
        }
        let history_max_turns = std::env::var(ENV_BOX_HISTORY_MAX_TURNS)
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .filter(|&n| n > 0)
            .unwrap_or(DEFAULT_BOX_HISTORY_MAX_TURNS);
        let session_persist = matches!(
            std::env::var(ENV_BOX_SESSION_PERSIST)
                .ok()
                .map(|s| s.trim().to_ascii_lowercase())
                .as_deref(),
            Some("1") | Some("true") | Some("yes") | Some("on")
        );
        Self::new_with_approval(
            workspace,
            Duration::from_millis(delay_ms),
            event_url,
            event_token,
            llm,
            history_max_turns,
            session_persist,
            ToolApprovalMode::from_env_product_default(),
        )
    }

    pub fn new(workspace_root: PathBuf, turn_delay: Duration) -> Self {
        Self::new_with_options(
            workspace_root,
            turn_delay,
            None,
            None,
            None,
            DEFAULT_BOX_HISTORY_MAX_TURNS,
            false,
        )
    }

    /// Test / programmatic constructor with optional LLM.
    pub fn new_with_llm(
        workspace_root: PathBuf,
        turn_delay: Duration,
        llm: Option<BoxLlmConfig>,
    ) -> Self {
        Self::new_with_options(
            workspace_root,
            turn_delay,
            None,
            None,
            llm,
            DEFAULT_BOX_HISTORY_MAX_TURNS,
            false,
        )
    }

    pub fn new_with_event(
        workspace_root: PathBuf,
        turn_delay: Duration,
        event_url: Option<String>,
        event_token: Option<String>,
    ) -> Self {
        Self::new_with_options(
            workspace_root,
            turn_delay,
            event_url,
            event_token,
            None,
            DEFAULT_BOX_HISTORY_MAX_TURNS,
            false,
        )
    }

    pub fn new_with_options(
        workspace_root: PathBuf,
        turn_delay: Duration,
        event_url: Option<String>,
        event_token: Option<String>,
        llm: Option<BoxLlmConfig>,
        history_max_turns: usize,
        session_persist: bool,
    ) -> Self {
        // In-process / test constructors: unset MODE → off (deepen/resident green).
        Self::new_with_approval(
            workspace_root,
            turn_delay,
            event_url,
            event_token,
            llm,
            history_max_turns,
            session_persist,
            ToolApprovalMode::from_env_test_default(),
        )
    }

    /// Product `from_env` path: unset `ATLAS_TOOL_APPROVAL_MODE` → `gate`.
    pub fn new_with_approval(
        workspace_root: PathBuf,
        turn_delay: Duration,
        event_url: Option<String>,
        event_token: Option<String>,
        llm: Option<BoxLlmConfig>,
        history_max_turns: usize,
        session_persist: bool,
        approval_mode: ToolApprovalMode,
    ) -> Self {
        let _ = std::fs::create_dir_all(&workspace_root);
        let (turn_tx, _) = tokio::sync::broadcast::channel(128);

        let mut vnc_mode = VncMode::Stub;
        if let Ok(v) = std::env::var(ENV_VNC_MODE) {
            vnc_mode = VncMode::parse(&v);
        }
        let mut vnc_upstream = None;
        if let Ok(v) = std::env::var(ENV_VNC_UPSTREAM) {
            let t = v.trim().to_string();
            if !t.is_empty() {
                vnc_upstream = Some(t);
            }
        }
        let vnc_stub_base = std::env::var("ATLAS_VNC_STUB_BASE")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_VNC_STUB_BASE.to_string())
            .trim_end_matches('/')
            .to_string();
        let attach_ttl = std::env::var(ENV_ATTACH_TTL_SECS)
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .map(Duration::from_secs)
            .unwrap_or_else(|| Duration::from_secs(DEFAULT_ATTACH_TTL_SECS));

        let mut agents = HashMap::new();
        agents.insert(
            DEFAULT_AGENT_ID.to_string(),
            AgentRecord {
                id: DEFAULT_AGENT_ID.to_string(),
                name: DEFAULT_AGENT_NAME.to_string(),
                description: "R2 box sidecar agent".to_string(),
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
                text: "box sidecar ready".to_string(),
                seq: 1,
            }],
        );
        let mut next_entry_seq = HashMap::new();
        next_entry_seq.insert(DEFAULT_AGENT_ID.to_string(), 2);
        let mut history = HashMap::new();
        history.insert(DEFAULT_AGENT_ID.to_string(), Vec::new());

        // Seed default agent workspace with a marker file for LIST_DIR evidence.
        let agt_dir = workspace_root.join(DEFAULT_AGENT_ID);
        let _ = std::fs::create_dir_all(&agt_dir);
        let _ = std::fs::create_dir_all(agt_dir.join("uploads"));
        let marker = agt_dir.join(".box-sidecar");
        if !marker.exists() {
            let _ = std::fs::write(&marker, b"r2-box-sidecar\n");
        }

        Self {
            shared: Arc::new(Shared {
                inner: RwLock::new(Inner {
                    agents,
                    transcripts,
                    history,
                    next_entry_seq,
                    next_agent_n: 2,
                    uploads: HashMap::new(),
                }),
                invokes: AtomicU64::new(0),
                pending: RwLock::new(HashMap::new()),
                turn_tx,
                workspace_root,
                turn_delay,
                vnc_stub_base,
                vnc_mode,
                vnc_upstream,
                vnc_tokens: RwLock::new(HashMap::new()),
                attach_ttl,
                event_url,
                event_token,
                http: reqwest::Client::new(),
                llm,
                history_max_turns: history_max_turns.max(1),
                session_persist,
                approval: ApprovalState::new(
                    approval_mode,
                    ApprovalState::timeout_from_env(),
                    ApprovalState::token_from_env(),
                ),
            }),
        }
    }

    /// `true` when Box will call OpenAI-compat chat/completions for pure chat.
    pub fn llm_configured(&self) -> bool {
        self.shared.llm.is_some()
    }

    /// Model name when LLM configured (never the API key).
    pub fn llm_model(&self) -> Option<&str> {
        self.shared.llm.as_ref().map(|c| c.model.as_str())
    }

    pub fn workspace_root(&self) -> &Path {
        &self.shared.workspace_root
    }

    pub fn tool_approval_mode(&self) -> ToolApprovalMode {
        self.shared.approval.mode
    }

    pub fn tool_approval_timeout_ms(&self) -> u64 {
        self.shared.approval.timeout.as_millis() as u64
    }

    /// Whether an optional `/approve` token is configured (never returns the secret).
    pub fn tool_approval_token_configured(&self) -> bool {
        self.shared.approval.token.is_some()
    }

    /// Test/CI hook: queue a decision applied to the next gated tool (FIFO).
    pub fn inject_approval_decision(&self, decision: ApprovalDecision) {
        if let Ok(mut q) = self.shared.approval.inject.lock() {
            q.push_back(decision);
        }
    }

    /// Resolve a pending approval (in-process or HTTP `/approve`).
    /// Unknown / already-consumed id → closed reject (never Allow).
    pub async fn submit_tool_approval(
        &self,
        approval_id: &str,
        decision: ApprovalDecision,
    ) -> Result<Value, GatewayError> {
        let slot = {
            let mut map = self.shared.approval.pending.write().await;
            map.remove(approval_id)
        };
        let Some(slot) = slot else {
            return Err(GatewayError::Rejected {
                reason: "approval_closed".into(),
                detail: Some("unknown_or_consumed".into()),
            });
        };
        let _ = slot.tx.send(decision);
        info!(
            approval_id,
            decision = decision.as_str(),
            tool = %slot.tool,
            agent_id = %slot.agent_id,
            "GA1 tool approval resolved"
        );
        Ok(json!({
            "ok": true,
            "approvalId": approval_id,
            "decision": decision.as_str(),
            "tool": slot.tool,
            "agentId": slot.agent_id,
        }))
    }

    /// Snapshot pending approval ids (test helper).
    pub async fn pending_approval_ids(&self) -> Vec<String> {
        let map = self.shared.approval.pending.read().await;
        let mut ids: Vec<_> = map.keys().cloned().collect();
        ids.sort();
        ids
    }

    pub fn subscribe_turns(&self) -> tokio::sync::broadcast::Receiver<RuntimeHint> {
        self.shared.turn_tx.subscribe()
    }

    pub fn turn_sender(&self) -> tokio::sync::broadcast::Sender<RuntimeHint> {
        self.shared.turn_tx.clone()
    }

    pub async fn snapshot_transcript(&self, agent_id: &str) -> Vec<TranscriptEntry> {
        let guard = self.shared.inner.read().await;
        guard
            .transcripts
            .get(agent_id)
            .cloned()
            .unwrap_or_default()
    }

    pub async fn lookup_vnc_token(&self, token: &str) -> Result<VncTokenRecord, VncTokenError> {
        let tokens = self.shared.vnc_tokens.read().await;
        match tokens.get(token) {
            Some(t) if t.expires_at_ms > now_ms() => Ok(t.clone()),
            Some(_) => Err(VncTokenError::Expired),
            None => Err(VncTokenError::Unknown),
        }
    }

    fn agent_dir(&self, agent_id: &str) -> PathBuf {
        self.shared.workspace_root.join(agent_id)
    }

    fn uploads_dir(&self, agent_id: &str) -> PathBuf {
        self.agent_dir(agent_id).join("uploads")
    }

    fn ensure_agent_workspace(&self, agent_id: &str) {
        let dir = self.agent_dir(agent_id);
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::create_dir_all(dir.join("uploads"));
        let marker = dir.join(".box-sidecar");
        if !marker.exists() {
            let _ = std::fs::write(&marker, format!("agent={agent_id}\n").as_bytes());
        }
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

    fn publish_hint(&self, hint: RuntimeHint) {
        let _ = self.shared.turn_tx.send(hint.clone());
        let Some(url) = self.shared.event_url.clone() else {
            return;
        };
        let token = self.shared.event_token.clone();
        let client = self.shared.http.clone();
        tokio::spawn(async move {
            let mut req = client
                .post(&url)
                .timeout(Duration::from_millis(HUB_EVENT_POST_TIMEOUT_MS))
                .json(&hint);
            if let Some(t) = token.as_ref() {
                req = req
                    .header(reqwest::header::AUTHORIZATION, format!("Bearer {t}"))
                    .header("X-Atlas-Event-Token", t.as_str());
            }
            match req.send().await {
                Ok(resp) if resp.status().is_success() => {}
                Ok(resp) => {
                    warn!(
                        status = %resp.status(),
                        %url,
                        "B1 hub event POST non-success (ignored)"
                    );
                }
                Err(e) => {
                    warn!(error = %e, %url, "B1 hub event POST failed (ignored)");
                }
            }
        });
    }

    fn emit_tool_hint(&self, agent_id: &str, tool: &str, summary: &str, exit_code: Option<i32>) {
        self.publish_hint(RuntimeHint::Tool {
            agent_id: agent_id.to_string(),
            tool: tool.to_string(),
            summary: summary.to_string(),
            exit_code,
        });
    }

    fn emit_delta(&self, agent_id: &str, text: &str) {
        if text.is_empty() {
            return;
        }
        self.publish_hint(RuntimeHint::AssistantDelta {
            agent_id: agent_id.to_string(),
            text: text.to_string(),
        });
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
        guard.history.insert(id.clone(), Vec::new());
        drop(guard);
        self.ensure_agent_workspace(&id);
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

    async fn append_system_notice(&self, agent_id: &str, text: &str) {
        let mut guard = self.shared.inner.write().await;
        let seq = {
            let n = guard.next_entry_seq.entry(agent_id.to_string()).or_insert(1);
            let s = *n;
            *n += 1;
            s
        };
        let notice = TranscriptEntry {
            id: format!("msg_sys_{seq}"),
            role: "system".to_string(),
            text: text.to_string(),
            seq,
        };
        guard
            .transcripts
            .entry(agent_id.to_string())
            .or_default()
            .push(notice);
    }

    /// Run whitelist tools if prompt contains documented triggers.
    /// Gated tools (WRITE / RUN mkdir / RUN cat) await approval when mode=gate.
    /// Sync invoke holds here until Allow/Deny/timeout (documented GA1 choice).
    async fn maybe_run_tools(&self, agent_id: &str, prompt: &str) -> Option<String> {
        self.ensure_agent_workspace(agent_id);
        let cwd = self.agent_dir(agent_id);
        let mut evidences: Vec<String> = Vec::new();

        if prompt.contains(TOOL_TRIGGER_LIST_DIR) {
            match list_dir_safe(&cwd) {
                Ok(listing) => {
                    let summary = format!("[tool:list_dir cwd={}]\n{listing}", cwd.display());
                    self.emit_tool_hint(agent_id, "list_dir", &summary, Some(0));
                    evidences.push(summary);
                }
                Err(e) => {
                    let summary = format!("[tool:list_dir error] {e}");
                    self.emit_tool_hint(agent_id, "list_dir", &summary, Some(1));
                    evidences.push(summary);
                }
            }
        }

        if prompt.contains(TOOL_TRIGGER_RUN_LS) {
            match run_allowlisted(&cwd, "ls", &[]) {
                Ok(out) => {
                    let summary =
                        format!("[tool:shell cmd=ls cwd={}]\n{out}", cwd.display());
                    self.emit_tool_hint(agent_id, "shell_ls", &summary, Some(0));
                    evidences.push(summary);
                }
                Err(e) => {
                    let summary = format!("[tool:shell cmd=ls error] {e}");
                    self.emit_tool_hint(agent_id, "shell_ls", &summary, Some(1));
                    evidences.push(summary);
                }
            }
        }

        if prompt.contains(TOOL_TRIGGER_RUN_PWD) {
            match run_allowlisted(&cwd, "pwd", &[]) {
                Ok(out) => {
                    let summary =
                        format!("[tool:shell cmd=pwd cwd={}]\n{out}", cwd.display());
                    self.emit_tool_hint(agent_id, "shell_pwd", &summary, Some(0));
                    evidences.push(summary);
                }
                Err(e) => {
                    let summary = format!("[tool:shell cmd=pwd error] {e}");
                    self.emit_tool_hint(agent_id, "shell_pwd", &summary, Some(1));
                    evidences.push(summary);
                }
            }
        }

        if let Some(rel) = parse_trigger_path(prompt, TOOL_TRIGGER_READ_FILE) {
            match resolve_sandbox_path(&cwd, &rel) {
                Ok(path) => match read_text_capped(&path) {
                    Ok(body) => {
                        let summary = format!(
                            "[tool:read_file path={} bytes={}]\n{body}",
                            rel,
                            body.len()
                        );
                        self.emit_tool_hint(agent_id, "read_file", &summary, Some(0));
                        evidences.push(summary);
                    }
                    Err(e) => {
                        let summary = format!("[tool:read_file error] {e}");
                        self.emit_tool_hint(agent_id, "read_file", &summary, Some(1));
                        evidences.push(summary);
                    }
                },
                Err(e) => {
                    let summary = format!("[tool:read_file error] {e}");
                    self.emit_tool_hint(agent_id, "read_file", &summary, Some(1));
                    evidences.push(summary);
                }
            }
        }

        if let Some((rel, content)) = parse_write_file(prompt) {
            let detail = format!("WRITE_FILE {rel}");
            let outcome = self
                .await_tool_approval(agent_id, "write_file", &detail)
                .await;
            if outcome.is_allow() {
                match resolve_sandbox_path(&cwd, &rel) {
                    Ok(path) => match write_text_capped(&path, &content) {
                        Ok(n) => {
                            let summary = format!(
                                "[tool:write_file path={rel} bytes={n} approval=allow] wrote ok"
                            );
                            self.emit_tool_hint(agent_id, "write_file", &summary, Some(0));
                            evidences.push(summary);
                        }
                        Err(e) => {
                            let summary = format!("[tool:write_file error] {e}");
                            self.emit_tool_hint(agent_id, "write_file", &summary, Some(1));
                            evidences.push(summary);
                        }
                    },
                    Err(e) => {
                        let summary = format!("[tool:write_file error] {e}");
                        self.emit_tool_hint(agent_id, "write_file", &summary, Some(1));
                        evidences.push(summary);
                    }
                }
            } else {
                let reason = outcome.reason();
                let summary = format!(
                    "[tool:write_file path={rel} error] {reason} (no workspace side effect)"
                );
                self.emit_tool_hint(agent_id, "write_file", &summary, Some(1));
                evidences.push(summary);
            }
        }

        if let Some(rel) = parse_trigger_path(prompt, TOOL_TRIGGER_RUN_CAT) {
            let detail = format!("RUN cat {rel}");
            let outcome = self
                .await_tool_approval(agent_id, "shell_cat", &detail)
                .await;
            if outcome.is_allow() {
                match resolve_sandbox_path(&cwd, &rel) {
                    Ok(path) => match read_text_capped(&path) {
                        Ok(body) => {
                            let summary = format!(
                                "[tool:shell cmd=cat path={rel} approval=allow]\n{body}"
                            );
                            self.emit_tool_hint(agent_id, "shell_cat", &summary, Some(0));
                            evidences.push(summary);
                        }
                        Err(e) => {
                            let summary = format!("[tool:shell cmd=cat error] {e}");
                            self.emit_tool_hint(agent_id, "shell_cat", &summary, Some(1));
                            evidences.push(summary);
                        }
                    },
                    Err(e) => {
                        let summary = format!("[tool:shell cmd=cat error] {e}");
                        self.emit_tool_hint(agent_id, "shell_cat", &summary, Some(1));
                        evidences.push(summary);
                    }
                }
            } else {
                let reason = outcome.reason();
                let summary = format!(
                    "[tool:shell cmd=cat path={rel} error] {reason} (no workspace side effect)"
                );
                self.emit_tool_hint(agent_id, "shell_cat", &summary, Some(1));
                evidences.push(summary);
            }
        }

        if let Some(rel) = parse_trigger_path(prompt, TOOL_TRIGGER_RUN_MKDIR) {
            let detail = format!("RUN mkdir {rel}");
            let outcome = self
                .await_tool_approval(agent_id, "shell_mkdir", &detail)
                .await;
            if outcome.is_allow() {
                match resolve_sandbox_path(&cwd, &rel) {
                    Ok(path) => match std::fs::create_dir_all(&path) {
                        Ok(()) => {
                            let summary = format!(
                                "[tool:shell cmd=mkdir path={rel} approval=allow] ok"
                            );
                            self.emit_tool_hint(agent_id, "shell_mkdir", &summary, Some(0));
                            evidences.push(summary);
                        }
                        Err(e) => {
                            let summary = format!("[tool:shell cmd=mkdir error] {e}");
                            self.emit_tool_hint(agent_id, "shell_mkdir", &summary, Some(1));
                            evidences.push(summary);
                        }
                    },
                    Err(e) => {
                        let summary = format!("[tool:shell cmd=mkdir error] {e}");
                        self.emit_tool_hint(agent_id, "shell_mkdir", &summary, Some(1));
                        evidences.push(summary);
                    }
                }
            } else {
                let reason = outcome.reason();
                let summary = format!(
                    "[tool:shell cmd=mkdir path={rel} error] {reason} (no workspace side effect)"
                );
                self.emit_tool_hint(agent_id, "shell_mkdir", &summary, Some(1));
                evidences.push(summary);
            }
        }

        // Reject unknown `RUN <cmd>` that is not on the whitelist (stable error).
        for (cmd, detail) in scan_unknown_run_commands(prompt) {
            let summary = format!(
                "[tool:shell error] command not on whitelist: {cmd} ({detail})"
            );
            self.emit_tool_hint(agent_id, "shell_reject", &summary, Some(1));
            evidences.push(summary);
        }

        if evidences.is_empty() {
            None
        } else {
            Some(evidences.join("\n"))
        }
    }

    /// Gate a dangerous tool. Failures / unknown errors → Deny (never Allow).
    async fn await_tool_approval(
        &self,
        agent_id: &str,
        tool: &str,
        detail: &str,
    ) -> GateOutcome {
        match self.shared.approval.mode {
            ToolApprovalMode::Off => return GateOutcome::Allow,
            ToolApprovalMode::AutoDeny => {
                info!(%agent_id, %tool, "GA1 auto_deny");
                return GateOutcome::AutoDeny;
            }
            ToolApprovalMode::Gate => {}
        }

        // Injected test decisions (FIFO) — applied before hang.
        if let Ok(mut q) = self.shared.approval.inject.lock() {
            if let Some(d) = q.pop_front() {
                return match d {
                    ApprovalDecision::Allow => GateOutcome::Allow,
                    ApprovalDecision::Deny => GateOutcome::Deny,
                };
            }
        }

        let approval_id = format!("appr_{}", Uuid::new_v4().simple());
        let (tx, rx) = oneshot::channel::<ApprovalDecision>();
        {
            let mut map = self.shared.approval.pending.write().await;
            map.insert(
                approval_id.clone(),
                crate::tool_approval::PendingApprovalSlot {
                    agent_id: agent_id.to_string(),
                    tool: tool.to_string(),
                    summary: detail.to_string(),
                    tx,
                },
            );
        }

        // GA1b: thin reuse of hub:tool with documented pending prefix (exitCode null).
        let pending_summary = format_pending_summary(&approval_id, tool, detail);
        self.emit_tool_hint(agent_id, "approval_pending", &pending_summary, None);
        info!(%agent_id, %tool, %approval_id, "GA1 approval pending");

        let timeout = self.shared.approval.timeout;
        let outcome = tokio::select! {
            res = rx => match res {
                Ok(ApprovalDecision::Allow) => GateOutcome::Allow,
                Ok(ApprovalDecision::Deny) => GateOutcome::Deny,
                // Sender dropped without decision → Deny (never Allow).
                Err(_) => {
                    warn!(%approval_id, "GA1 approval channel closed; denying");
                    GateOutcome::Deny
                }
            },
            _ = tokio::time::sleep(timeout) => GateOutcome::Timeout,
        };

        // Ensure pending slot is cleared on timeout / channel close.
        {
            let mut map = self.shared.approval.pending.write().await;
            map.remove(&approval_id);
        }

        info!(
            %agent_id,
            %tool,
            %approval_id,
            outcome = outcome.reason(),
            "GA1 approval settled"
        );
        outcome
    }


    /// Build non-echo reply that references prior turn context when present.
    fn compose_reply(
        &self,
        agent_id: &str,
        prompt: &str,
        prior_users: &[String],
        tool_evidence: Option<&str>,
    ) -> String {
        let turn = prior_users.len() + 1;
        let mut parts = Vec::new();
        parts.push(format!("box-sidecar agent={agent_id} turn={turn}"));

        if let Some(first) = prior_users.first() {
            let marker = context_marker(first);
            let snippet: String = first.chars().take(64).collect();
            parts.push(format!("saw_prior={snippet}"));
            parts.push(format!("ctx={marker}"));
        } else {
            parts.push("saw_prior=none".to_string());
            parts.push("ctx=none".to_string());
        }

        // Never pure echo of the prompt.
        parts.push(format!("ack={prompt}"));

        if let Some(ev) = tool_evidence {
            parts.push(ev.to_string());
        }

        parts.join(" | ")
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
        {
            let hist = guard.history.entry(agent_id.to_string()).or_default();
            hist.push(("user".into(), prompt.to_string()));
            hist.push(("assistant".into(), preview.clone()));
            // Truncate oldest turns (each turn = user+assistant pair).
            let max_msgs = self.shared.history_max_turns.saturating_mul(2);
            if hist.len() > max_msgs {
                let drop_n = hist.len() - max_msgs;
                hist.drain(0..drop_n);
            }
        }
        if let Some(a) = guard.agents.get_mut(agent_id) {
            a.is_running = false;
        }
        let out = (preview.clone(), vec![user_entry, assistant_entry]);
        drop(guard);
        if self.shared.session_persist {
            self.soft_persist_session(agent_id, prompt, &preview);
        }
        Ok(out)
    }

    /// Soft append-only session.jsonl (optional; failures are logged, not fatal).
    fn soft_persist_session(&self, agent_id: &str, prompt: &str, reply: &str) {
        self.ensure_agent_workspace(agent_id);
        let path = self.agent_dir(agent_id).join("session.jsonl");
        let line = json!({
            "ts": now_ms(),
            "user": prompt,
            "assistant": reply,
        });
        match std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            Ok(mut f) => {
                use std::io::Write;
                if let Err(e) = writeln!(f, "{line}") {
                    warn!(%agent_id, error = %e, "box session persist write failed (soft)");
                }
            }
            Err(e) => {
                warn!(%agent_id, error = %e, "box session persist open failed (soft)");
            }
        }
    }

    /// Build OpenAI-compat messages[] from memory history + current user prompt.
    fn build_llm_messages(&self, agent_id: &str, prompt: &str, history: &[(String, String)]) -> Vec<Value> {
        let mut messages = Vec::new();
        messages.push(json!({
            "role": "system",
            "content": format!(
                "You are atlas-bot box sidecar agent {agent_id}. Reply concisely. Do not echo the user prompt verbatim."
            ),
        }));
        for (role, text) in history {
            if role == "user" || role == "assistant" {
                messages.push(json!({ "role": role, "content": text }));
            }
        }
        messages.push(json!({ "role": "user", "content": prompt }));
        messages
    }

    async fn chat_complete(
        &self,
        agent_id: &str,
        prompt: &str,
        history: &[(String, String)],
        mut kill_rx: oneshot::Receiver<()>,
    ) -> Result<String, GatewayError> {
        let cfg = self
            .shared
            .llm
            .as_ref()
            .ok_or_else(|| GatewayError::Upstream("box LLM not configured".into()))?;
        let url = format!("{}/chat/completions", cfg.base);
        let messages = self.build_llm_messages(agent_id, prompt, history);
        let body = json!({
            "model": cfg.model,
            "messages": messages,
        });
        let mut req = self.shared.http.post(&url).json(&body).timeout(cfg.timeout);
        if !cfg.api_key.is_empty() {
            req = req.bearer_auth(&cfg.api_key);
        }
        let send_fut = req.send();

        let resp = tokio::select! {
            r = send_fut => r.map_err(|e| GatewayError::Upstream(format!("box LLM request failed: {e}")))?,
            _ = &mut kill_rx => {
                return Err(GatewayError::Upstream("gateway/run-interrupted".into()));
            }
        };

        let status = resp.status();
        let val: Value = resp
            .json()
            .await
            .map_err(|e| GatewayError::Upstream(format!("box LLM response json: {e}")))?;
        if !status.is_success() {
            return Err(GatewayError::Upstream(format!(
                "box LLM HTTP {status}: {val}"
            )));
        }
        let text = val
            .pointer("/choices/0/message/content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if text.is_empty() {
            return Err(GatewayError::Upstream("box LLM empty content".into()));
        }
        if text == prompt || text == format!("echo: {prompt}") {
            return Err(GatewayError::Upstream(
                "box LLM reply looked like stub echo; refusing".into(),
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

        let (cancel_tx, mut cancel_rx) = oneshot::channel::<()>();
        {
            let mut pending = self.shared.pending.write().await;
            pending.insert(
                agent_id.to_string(),
                Arc::new(PendingTurn {
                    cancel: Mutex::new(Some(cancel_tx)),
                }),
            );
        }

        // Interruptible delay window (kept for R1 interrupt smoke).
        let delay = self.shared.turn_delay;
        if !delay.is_zero() {
            let interrupted = tokio::select! {
                _ = tokio::time::sleep(delay) => false,
                _ = &mut cancel_rx => true,
            };
            if interrupted {
                warn!(%agent_id, "box sidecar sendPrompt interrupted (delay)");
                self.clear_pending(agent_id).await;
                self.set_running(agent_id, false).await;
                self.append_system_notice(agent_id, "run interrupted").await;
                return Err(GatewayError::Upstream("gateway/run-interrupted".into()));
            }
        }

        // Snapshot history before committing this turn.
        let history_snapshot: Vec<(String, String)> = {
            let guard = self.shared.inner.read().await;
            guard
                .history
                .get(agent_id)
                .cloned()
                .unwrap_or_default()
        };
        let prior_users: Vec<String> = history_snapshot
            .iter()
            .filter(|(role, _)| role == "user")
            .map(|(_, t)| t.clone())
            .collect();

        // Tool-vs-LLM order: tools first; tool evidence → compose_reply;
        // pure chat → LLM when configured, else compose_reply.
        let tool_evidence = self.maybe_run_tools(agent_id, &prompt).await;

        let reply = if tool_evidence.is_some() {
            let _ = cancel_rx; // tools path: cancel already survived delay
            self.compose_reply(
                agent_id,
                &prompt,
                &prior_users,
                tool_evidence.as_deref(),
            )
        } else if self.shared.llm.is_some() {
            match self
                .chat_complete(agent_id, &prompt, &history_snapshot, cancel_rx)
                .await
            {
                Ok(text) => text,
                Err(e) => {
                    self.clear_pending(agent_id).await;
                    self.set_running(agent_id, false).await;
                    let interrupted = match &e {
                        GatewayError::Upstream(s) => s == "gateway/run-interrupted",
                        _ => false,
                    };
                    if interrupted {
                        self.append_system_notice(agent_id, "run interrupted").await;
                    }
                    return Err(e);
                }
            }
        } else {
            let _ = cancel_rx;
            self.compose_reply(agent_id, &prompt, &prior_users, None)
        };

        // Hard non-echo guard.
        if reply == prompt || reply == format!("echo: {prompt}") {
            self.clear_pending(agent_id).await;
            self.set_running(agent_id, false).await;
            return Err(GatewayError::Upstream(
                "box reply looked like stub echo; refusing".into(),
            ));
        }

        // Stream a small assistant delta before the finished hint (R2.2).
        let delta_chunk: String = reply.chars().take(48).collect();
        self.emit_delta(agent_id, &delta_chunk);

        let (preview, entries) = self.commit_turn(agent_id, &prompt, &reply).await?;
        self.clear_pending(agent_id).await;
        self.publish_hint(RuntimeHint::Finished {
            agent_id: agent_id.to_string(),
            preview: preview.clone(),
            user_text: prompt.clone(),
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
                info!(%target, cancelled, "box sidecar interrupt signaled");
            }
        }

        let mut guard = self.shared.inner.write().await;
        let Some(agent) = guard.agents.get_mut(&target) else {
            return Err(GatewayError::AgentNotFound(target));
        };
        let had_active = agent.is_running || cancelled;

        // Closed-set: idle interrupt must not look like success.
        if !had_active {
            return Err(GatewayError::rejected("no_active_run"));
        }

        if agent.is_running {
            agent.is_running = false;
        }
        Ok(json!({
            "hadActiveRun": true,
            "killed": cancelled,
        }))
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

    async fn mint_vnc_descriptor(&self, agent_id: &str) -> Result<Value, GatewayError> {
        let mut tokens = self.shared.vnc_tokens.write().await;
        mint_vnc_descriptor_value(
            agent_id,
            &self.shared.vnc_stub_base,
            self.shared.vnc_mode,
            self.shared.vnc_upstream.as_deref(),
            Some(&mut tokens),
        )
    }

    async fn upload_attachment(
        &self,
        agent_id: &str,
        args: &Value,
    ) -> Result<Value, GatewayError> {
        self.sweep_expired_uploads().await;
        self.ensure_agent_workspace(agent_id);
        let args_json = serde_json::to_vec(args).unwrap_or_default();
        if args_json.len() > UPLOAD_ARGS_JSON_MAX_BYTES {
            return Err(GatewayError::rejected(
                xai_tool_protocol::COMMAND_REJECTED_ARGS_TOO_LARGE,
            ));
        }
        let filename = args
            .get("filename")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| GatewayError::InvalidArgs("missing filename".into()))?
            .to_string();
        let safe_name = filename
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect::<String>();
        let b64 = args
            .get("bytesBase64")
            .and_then(|v| v.as_str())
            .ok_or_else(|| GatewayError::InvalidArgs("missing bytesBase64".into()))?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(b64.as_bytes())
            .map_err(|e| GatewayError::InvalidArgs(format!("invalid bytesBase64: {e}")))?;
        if bytes.len() > ATTACHMENT_OBJECT_MAX_BYTES {
            return Err(GatewayError::rejected(
                xai_tool_protocol::COMMAND_REJECTED_ATTACHMENT_TOO_LARGE,
            ));
        }
        let upload_id = format!("upl_{}", Uuid::new_v4().simple());
        let created_at = now_ms();
        let root = self.uploads_dir(agent_id);
        let dir = root.join(&upload_id);
        std::fs::create_dir_all(&dir)
            .map_err(|e| GatewayError::Upstream(format!("mkdir: {e}")))?;
        let file_path = dir.join(&safe_name);
        std::fs::write(&file_path, &bytes)
            .map_err(|e| GatewayError::Upstream(format!("write: {e}")))?;
        let path = file_path.to_string_lossy().to_string();
        let rec = BoxUploadRecord {
            path: path.clone(),
            filename: safe_name,
            agent_id: agent_id.to_string(),
            bytes: bytes.len() as u64,
            created_at,
            state: "ready".to_string(),
        };
        let meta = BoxUploadMeta {
            upload_id: upload_id.clone(),
            path: path.clone(),
            filename: rec.filename.clone(),
            bytes: rec.bytes,
            created_at,
            state: rec.state.clone(),
            agent_id: agent_id.to_string(),
        };
        let meta_path = dir.join("meta.json");
        let meta_s =
            serde_json::to_string_pretty(&meta).map_err(|e| GatewayError::Upstream(e.to_string()))?;
        std::fs::write(meta_path, meta_s)
            .map_err(|e| GatewayError::Upstream(format!("meta: {e}")))?;
        {
            let mut guard = self.shared.inner.write().await;
            guard.uploads.insert(upload_id.clone(), rec);
        }
        Ok(json!({
            "path": path,
            "uploadId": upload_id,
        }))
    }

    async fn attach_upload(&self, agent_id: &str, args: &Value) -> Result<Value, GatewayError> {
        self.sweep_expired_uploads().await;
        let upload_id = args
            .get("uploadId")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| GatewayError::InvalidArgs("missing uploadId".into()))?;
        let guard = self.shared.inner.read().await;
        match guard.uploads.get(upload_id) {
            Some(rec) if !is_expired(rec.created_at, self.shared.attach_ttl) => Ok(json!({
                "path": rec.path,
                "filename": rec.filename,
                "agentId": rec.agent_id,
            })),
            Some(_) | None => {
                let _ = agent_id;
                Err(GatewayError::rejected(
                    xai_tool_protocol::COMMAND_REJECTED_ATTACHMENT_NOT_FOUND,
                ))
            }
        }
    }

    async fn sweep_expired_uploads(&self) {
        let ttl = self.shared.attach_ttl;
        let mut guard = self.shared.inner.write().await;
        let expired: Vec<(String, String)> = guard
            .uploads
            .iter()
            .filter(|(_, r)| is_expired(r.created_at, ttl))
            .map(|(k, r)| (k.clone(), r.path.clone()))
            .collect();
        for (id, path) in expired {
            guard.uploads.remove(&id);
            if let Some(parent) = Path::new(&path).parent() {
                let _ = std::fs::remove_dir_all(parent);
            }
        }
    }

    async fn dispatch(&self, agent_id: &str, name: &str, args: Value) -> Result<Value, GatewayError> {
        match name {
            "listAgents" => self.list_agents().await,
            "createAgent" => self.create_agent(&args).await,
            "sendPrompt" => self.send_prompt(agent_id, &args).await,
            "interruptAgentRun" => self.interrupt_agent_run(agent_id, &args).await,
            "getAgentTranscriptTail" => self.get_transcript_tail(agent_id, &args).await,
            "uploadAttachment" => self.upload_attachment(agent_id, &args).await,
            "attachUpload" => self.attach_upload(agent_id, &args).await,
            other => Err(GatewayError::UnknownMethod(other.to_string())),
        }
    }
}

#[async_trait]
impl Gateway for BoxSidecarGateway {
    async fn invoke(&self, agent_id: &str, name: &str, args: Value) -> Result<Value, GatewayError> {
        self.shared.invokes.fetch_add(1, Ordering::SeqCst);
        info!(%agent_id, %name, "box sidecar invoke");
        self.dispatch(agent_id, name, args).await
    }

    fn invoke_count(&self) -> u64 {
        self.shared.invokes.load(Ordering::SeqCst)
    }

    async fn vnc_descriptor(&self, agent_id: &str) -> Result<Value, GatewayError> {
        self.shared.invokes.fetch_add(1, Ordering::SeqCst);
        info!(%agent_id, "box sidecar vnc_descriptor");
        self.mint_vnc_descriptor(agent_id).await
    }

    async fn tool_approve(
        &self,
        approval_id: &str,
        decision: &str,
    ) -> Result<Value, GatewayError> {
        let Some(d) = ApprovalDecision::parse(decision) else {
            return Err(GatewayError::InvalidArgs(
                "decision must be allow|deny".into(),
            ));
        };
        self.submit_tool_approval(approval_id, d).await
    }

    fn tool_approval_info(&self) -> Option<(String, bool)> {
        Some((
            self.shared.approval.mode.as_str().to_string(),
            self.shared.approval.token.is_some(),
        ))
    }
}

#[async_trait]
impl Gateway for Arc<BoxSidecarGateway> {
    async fn invoke(&self, agent_id: &str, name: &str, args: Value) -> Result<Value, GatewayError> {
        (**self).invoke(agent_id, name, args).await
    }

    fn invoke_count(&self) -> u64 {
        (**self).invoke_count()
    }

    async fn vnc_descriptor(&self, agent_id: &str) -> Result<Value, GatewayError> {
        (**self).vnc_descriptor(agent_id).await
    }

    async fn tool_approve(
        &self,
        approval_id: &str,
        decision: &str,
    ) -> Result<Value, GatewayError> {
        (**self).tool_approve(approval_id, decision).await
    }

    fn tool_approval_info(&self) -> Option<(String, bool)> {
        (**self).tool_approval_info()
    }
}

/// Stable short marker from prior user text (non-echo reproducibility).
fn context_marker(text: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in text.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{:08x}", (h & 0xffff_ffff) as u32)
}

fn list_dir_safe(dir: &Path) -> Result<String, String> {
    let mut names: Vec<String> = Vec::new();
    let rd = std::fs::read_dir(dir).map_err(|e| e.to_string())?;
    for ent in rd {
        let ent = ent.map_err(|e| e.to_string())?;
        names.push(ent.file_name().to_string_lossy().into_owned());
    }
    names.sort();
    Ok(names.join("\n"))
}

/// Allowlisted shell only: `ls` or `pwd` (no args, fixed cwd).
fn run_allowlisted(cwd: &Path, cmd: &str, args: &[&str]) -> Result<String, String> {
    match cmd {
        "ls" | "pwd" => {}
        other => return Err(format!("command not on whitelist: {other}")),
    }
    let output = StdCommand::new(cmd)
        .args(args)
        .current_dir(cwd)
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(format!(
            "exit {}: {}",
            output.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Resolve `<rel>` under agent cwd; reject `..` / absolute escapes.
fn resolve_sandbox_path(agent_cwd: &Path, rel: &str) -> Result<PathBuf, String> {
    let rel = rel.trim();
    if rel.is_empty() {
        return Err("empty relative path".into());
    }
    let p = Path::new(rel);
    if p.is_absolute() {
        return Err(format!("absolute path rejected: {rel}"));
    }
    for c in p.components() {
        match c {
            Component::Normal(_) | Component::CurDir => {}
            Component::ParentDir => {
                return Err(format!("path escape rejected (..): {rel}"));
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(format!("absolute path rejected: {rel}"));
            }
        }
    }
    let joined = agent_cwd.join(p);
    let cwd_canon = std::fs::canonicalize(agent_cwd).unwrap_or_else(|_| agent_cwd.to_path_buf());
    // Parent may not exist yet (WRITE/mkdir); canonicalize existing prefix.
    let candidate = if joined.exists() {
        std::fs::canonicalize(&joined).unwrap_or(joined.clone())
    } else if let Some(parent) = joined.parent() {
        let parent_canon = if parent.exists() {
            std::fs::canonicalize(parent).unwrap_or_else(|_| parent.to_path_buf())
        } else {
            // Ensure parents within sandbox for mkdir/write of nested rel.
            parent.to_path_buf()
        };
        parent_canon.join(joined.file_name().unwrap_or_default())
    } else {
        joined.clone()
    };
    if !candidate.starts_with(&cwd_canon) && !joined.starts_with(agent_cwd) {
        return Err(format!("path escapes workspace: {rel}"));
    }
    // Extra string check against raw `..` already done; return joined (not necessarily canon).
    Ok(agent_cwd.join(p))
}

fn read_text_capped(path: &Path) -> Result<String, String> {
    let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
    if meta.len() as usize > BOX_TEXT_FILE_MAX_BYTES {
        return Err(format!(
            "text exceeds {BOX_TEXT_FILE_MAX_BYTES} byte cap (got {} bytes)",
            meta.len()
        ));
    }
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    if bytes.len() > BOX_TEXT_FILE_MAX_BYTES {
        return Err(format!(
            "text exceeds {BOX_TEXT_FILE_MAX_BYTES} byte cap (got {} bytes)",
            bytes.len()
        ));
    }
    String::from_utf8(bytes).map_err(|e| format!("not utf-8 text: {e}"))
}

fn write_text_capped(path: &Path, content: &str) -> Result<usize, String> {
    if content.len() > BOX_TEXT_FILE_MAX_BYTES {
        return Err(format!(
            "text exceeds {BOX_TEXT_FILE_MAX_BYTES} byte cap (got {} bytes)",
            content.len()
        ));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(path, content.as_bytes()).map_err(|e| e.to_string())?;
    Ok(content.len())
}

fn first_token(s: &str) -> Option<(&str, &str)> {
    let s = s.trim_start();
    if s.is_empty() {
        return None;
    }
    let end = s
        .find(|c: char| c.is_whitespace())
        .unwrap_or(s.len());
    let tok = &s[..end];
    let rest = s[end..].trim_start();
    Some((tok, rest))
}

fn parse_trigger_path(prompt: &str, trigger: &str) -> Option<String> {
    let idx = prompt.find(trigger)?;
    let rest = &prompt[idx + trigger.len()..];
    let (tok, _) = first_token(rest)?;
    if tok.is_empty() {
        return None;
    }
    Some(tok.to_string())
}

fn parse_write_file(prompt: &str) -> Option<(String, String)> {
    let idx = prompt.find(TOOL_TRIGGER_WRITE_FILE)?;
    let rest = &prompt[idx + TOOL_TRIGGER_WRITE_FILE.len()..];
    let (path, after) = first_token(rest)?;
    let content = if let Some(i) = after.find("<<<") {
        after[i + 3..].trim().to_string()
    } else {
        after.trim().to_string()
    };
    Some((path.to_string(), content))
}

/// Detect `RUN <cmd>` forms that are not on the allowlist.
fn scan_unknown_run_commands(prompt: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut search = prompt;
    while let Some(idx) = search.find("RUN ") {
        let after = &search[idx + 4..];
        let cmd_line = after.split('\n').next().unwrap_or(after).trim();
        let cmd = cmd_line.split_whitespace().next().unwrap_or("");
        if cmd.is_empty() {
            search = &search[idx + 4..];
            continue;
        }
        let allowed = matches!(cmd, "ls" | "pwd" | "cat" | "mkdir");
        if !allowed {
            out.push((
                cmd.to_string(),
                "closed-set reject; not silent".to_string(),
            ));
        }
        search = &search[idx + 4..];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn multi_turn_references_prior() {
        let root = std::env::temp_dir().join(format!(
            "atlas-box-mt-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let gw = BoxSidecarGateway::new(root.clone(), Duration::from_millis(5));
        let r1 = gw
            .invoke(
                "agt_1",
                "sendPrompt",
                json!({ "prompt": "secret-token-ALPHA" }),
            )
            .await
            .unwrap();
        assert!(r1["preview"].as_str().unwrap().contains("turn=1"));
        assert!(!r1["preview"].as_str().unwrap().contains("saw_prior=secret"));

        let r2 = gw
            .invoke(
                "agt_1",
                "sendPrompt",
                json!({ "prompt": "what did I say?" }),
            )
            .await
            .unwrap();
        let p2 = r2["preview"].as_str().unwrap();
        assert!(p2.contains("turn=2"), "{p2}");
        assert!(p2.contains("secret-token-ALPHA"), "must cite prior user text: {p2}");
        assert!(p2.contains("ctx="), "{p2}");
        assert_ne!(p2, "what did I say?");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn list_dir_tool_evidence() {
        let root = std::env::temp_dir().join(format!(
            "atlas-box-ld-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let gw = BoxSidecarGateway::new(root.clone(), Duration::from_millis(5));
        let r = gw
            .invoke(
                "agt_1",
                "sendPrompt",
                json!({ "prompt": "please LIST_DIR now" }),
            )
            .await
            .unwrap();
        let p = r["preview"].as_str().unwrap();
        assert!(p.contains("[tool:list_dir"), "{p}");
        assert!(p.contains(".box-sidecar"), "{p}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn read_write_and_path_escape() {
        let root = std::env::temp_dir().join(format!(
            "atlas-box-rw-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let gw = BoxSidecarGateway::new(root.clone(), Duration::from_millis(5));
        let w = gw
            .invoke(
                "agt_1",
                "sendPrompt",
                json!({ "prompt": "WRITE_FILE notes.txt <<< hello-r2" }),
            )
            .await
            .unwrap();
        let pw = w["preview"].as_str().unwrap();
        assert!(pw.contains("[tool:write_file"), "{pw}");
        let r = gw
            .invoke(
                "agt_1",
                "sendPrompt",
                json!({ "prompt": "READ_FILE notes.txt" }),
            )
            .await
            .unwrap();
        let pr = r["preview"].as_str().unwrap();
        assert!(pr.contains("hello-r2"), "{pr}");
        let bad = gw
            .invoke(
                "agt_1",
                "sendPrompt",
                json!({ "prompt": "READ_FILE ../etc/passwd" }),
            )
            .await
            .unwrap();
        let pb = bad["preview"].as_str().unwrap();
        assert!(
            pb.contains("path escape") || pb.contains(".."),
            "expected escape reject: {pb}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn unknown_shell_rejected() {
        let root = std::env::temp_dir().join(format!(
            "atlas-box-sh-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let gw = BoxSidecarGateway::new(root.clone(), Duration::from_millis(5));
        let r = gw
            .invoke(
                "agt_1",
                "sendPrompt",
                json!({ "prompt": "please RUN curl http://evil" }),
            )
            .await
            .unwrap();
        let p = r["preview"].as_str().unwrap();
        assert!(p.contains("command not on whitelist: curl"), "{p}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn idle_interrupt_rejects() {
        let root = std::env::temp_dir().join(format!(
            "atlas-box-idle-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let gw = BoxSidecarGateway::new(root.clone(), Duration::from_millis(5));
        let err = gw
            .invoke(
                "agt_1",
                "interruptAgentRun",
                json!({ "agentId": "agt_1" }),
            )
            .await
            .unwrap_err();
        match err {
            GatewayError::Rejected { reason, .. } => assert_eq!(reason, "no_active_run"),
            other => panic!("expected Rejected, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&root);
    }
}
