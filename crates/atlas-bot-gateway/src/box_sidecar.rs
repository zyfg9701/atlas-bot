//! R1/R2 Box Sidecar: long-lived in-process runtime behind Gateway HTTP.
//!
//! Deterministic local responder with per-agent conversation history,
//! workspace root (`ATLAS_BOX_WORKSPACE`), tool whitelist (LIST_DIR / READ /
//! WRITE / RUN ls|pwd|cat|mkdir), streaming [`RuntimeHint`]s, and P5 VNC +
//! attachments rooted under the agent workspace. Not grok-build; not a real LLM.
//!
//! Interrupt: cancel in-flight `sendPrompt` via oneshot; idle interrupt
//! returns closed-set `command_rejected/no_active_run` (never fake success).

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

const DEFAULT_WORKSPACE: &str = "./data/box-workspace";

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
        Self::new_with_event(
            workspace,
            Duration::from_millis(delay_ms),
            event_url,
            event_token,
        )
    }

    pub fn new(workspace_root: PathBuf, turn_delay: Duration) -> Self {
        Self::new_with_event(workspace_root, turn_delay, None, None)
    }

    pub fn new_with_event(
        workspace_root: PathBuf,
        turn_delay: Duration,
        event_url: Option<String>,
        event_token: Option<String>,
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
            }),
        }
    }

    pub fn workspace_root(&self) -> &Path {
        &self.shared.workspace_root
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
    fn maybe_run_tools(&self, agent_id: &str, prompt: &str) -> Option<String> {
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
            match resolve_sandbox_path(&cwd, &rel) {
                Ok(path) => match write_text_capped(&path, &content) {
                    Ok(n) => {
                        let summary =
                            format!("[tool:write_file path={rel} bytes={n}] wrote ok");
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
        }

        if let Some(rel) = parse_trigger_path(prompt, TOOL_TRIGGER_RUN_CAT) {
            match resolve_sandbox_path(&cwd, &rel) {
                Ok(path) => match read_text_capped(&path) {
                    Ok(body) => {
                        let summary =
                            format!("[tool:shell cmd=cat path={rel}]\n{body}");
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
        }

        if let Some(rel) = parse_trigger_path(prompt, TOOL_TRIGGER_RUN_MKDIR) {
            match resolve_sandbox_path(&cwd, &rel) {
                Ok(path) => match std::fs::create_dir_all(&path) {
                    Ok(()) => {
                        let summary = format!("[tool:shell cmd=mkdir path={rel}] ok");
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
        }
        if let Some(a) = guard.agents.get_mut(agent_id) {
            a.is_running = false;
        }
        Ok((preview, vec![user_entry, assistant_entry]))
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
            let mut pending = self.shared.pending.write().await;
            pending.insert(
                agent_id.to_string(),
                Arc::new(PendingTurn {
                    cancel: Mutex::new(Some(cancel_tx)),
                }),
            );
        }

        // Interruptible work window (tools + delay).
        let delay = self.shared.turn_delay;
        let interrupted = tokio::select! {
            _ = tokio::time::sleep(delay) => false,
            _ = cancel_rx => true,
        };

        if interrupted {
            warn!(%agent_id, "box sidecar sendPrompt interrupted");
            self.clear_pending(agent_id).await;
            self.set_running(agent_id, false).await;
            self.append_system_notice(agent_id, "run interrupted").await;
            return Err(GatewayError::Upstream("gateway/run-interrupted".into()));
        }

        // Snapshot prior user texts for context (before committing this turn).
        let prior_users: Vec<String> = {
            let guard = self.shared.inner.read().await;
            guard
                .history
                .get(agent_id)
                .map(|h| {
                    h.iter()
                        .filter(|(role, _)| role == "user")
                        .map(|(_, t)| t.clone())
                        .collect()
                })
                .unwrap_or_default()
        };

        let tool_evidence = self.maybe_run_tools(agent_id, &prompt);
        let reply = self.compose_reply(
            agent_id,
            &prompt,
            &prior_users,
            tool_evidence.as_deref(),
        );

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
