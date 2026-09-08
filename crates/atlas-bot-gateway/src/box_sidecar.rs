//! R1 Box Sidecar: long-lived in-process runtime behind Gateway HTTP.
//!
//! Deterministic local responder with per-agent conversation history,
//! workspace root (`ATLAS_BOX_WORKSPACE`), and a tiny tool whitelist
//! (`LIST_DIR` / `RUN ls` / `RUN pwd`). Not grok-build; not a real LLM.
//!
//! Interrupt: cancel in-flight `sendPrompt` via oneshot; idle interrupt
//! returns closed-set `command_rejected/no_active_run` (never fake success).

#![allow(dead_code)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::{oneshot, Mutex, RwLock};
use tracing::{info, warn};

use crate::{
    AgentRecord, Gateway, GatewayError, TranscriptEntry, TurnFinishedHint, DEFAULT_AGENT_ID,
    DEFAULT_AGENT_NAME,
};

/// Workspace root for agent subdirs.
pub const ENV_BOX_WORKSPACE: &str = "ATLAS_BOX_WORKSPACE";
/// Interruptible delay before completing a sendPrompt (ms). Default 50.
pub const ENV_BOX_TURN_DELAY_MS: &str = "ATLAS_BOX_TURN_DELAY_MS";
/// Default turn delay when env unset.
pub const DEFAULT_BOX_TURN_DELAY_MS: u64 = 50;

const DEFAULT_WORKSPACE: &str = "./data/box-workspace";

/// Documented tool triggers (see runbook whitelist).
pub const TOOL_TRIGGER_LIST_DIR: &str = "LIST_DIR";
pub const TOOL_TRIGGER_RUN_LS: &str = "RUN ls";
pub const TOOL_TRIGGER_RUN_PWD: &str = "RUN pwd";

#[derive(Default)]
struct Inner {
    agents: HashMap<String, AgentRecord>,
    transcripts: HashMap<String, Vec<TranscriptEntry>>,
    /// Full conversation turns (user/assistant text) for multi-turn context.
    history: HashMap<String, Vec<(String, String)>>, // (role, text)
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
    turn_tx: tokio::sync::broadcast::Sender<TurnFinishedHint>,
    workspace_root: PathBuf,
    turn_delay: Duration,
}

/// Box Sidecar gateway: multi-turn + workspace tools + measurable interrupt.
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
        Self::new(workspace, Duration::from_millis(delay_ms))
    }

    pub fn new(workspace_root: PathBuf, turn_delay: Duration) -> Self {
        let _ = std::fs::create_dir_all(&workspace_root);
        let (turn_tx, _) = tokio::sync::broadcast::channel(64);
        let mut agents = HashMap::new();
        agents.insert(
            DEFAULT_AGENT_ID.to_string(),
            AgentRecord {
                id: DEFAULT_AGENT_ID.to_string(),
                name: DEFAULT_AGENT_NAME.to_string(),
                description: "R1 box sidecar agent".to_string(),
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
        let marker = agt_dir.join(".box-sidecar");
        if !marker.exists() {
            let _ = std::fs::write(&marker, b"r1-box-sidecar\n");
        }

        Self {
            shared: Arc::new(Shared {
                inner: RwLock::new(Inner {
                    agents,
                    transcripts,
                    history,
                    next_entry_seq,
                    next_agent_n: 2,
                }),
                invokes: AtomicU64::new(0),
                pending: RwLock::new(HashMap::new()),
                turn_tx,
                workspace_root,
                turn_delay,
            }),
        }
    }

    pub fn workspace_root(&self) -> &Path {
        &self.shared.workspace_root
    }

    pub fn subscribe_turns(&self) -> tokio::sync::broadcast::Receiver<TurnFinishedHint> {
        self.shared.turn_tx.subscribe()
    }

    pub fn turn_sender(&self) -> tokio::sync::broadcast::Sender<TurnFinishedHint> {
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

    fn agent_dir(&self, agent_id: &str) -> PathBuf {
        self.shared.workspace_root.join(agent_id)
    }

    fn ensure_agent_workspace(&self, agent_id: &str) {
        let dir = self.agent_dir(agent_id);
        let _ = std::fs::create_dir_all(&dir);
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
                    evidences.push(format!(
                        "[tool:list_dir cwd={}]\n{listing}",
                        cwd.display()
                    ));
                }
                Err(e) => evidences.push(format!("[tool:list_dir error] {e}")),
            }
        }

        if prompt.contains(TOOL_TRIGGER_RUN_LS) {
            match run_allowlisted(&cwd, "ls") {
                Ok(out) => evidences.push(format!("[tool:shell cmd=ls cwd={}]\n{out}", cwd.display())),
                Err(e) => evidences.push(format!("[tool:shell cmd=ls error] {e}")),
            }
        }

        if prompt.contains(TOOL_TRIGGER_RUN_PWD) {
            match run_allowlisted(&cwd, "pwd") {
                Ok(out) => evidences.push(format!("[tool:shell cmd=pwd cwd={}]\n{out}", cwd.display())),
                Err(e) => evidences.push(format!("[tool:shell cmd=pwd error] {e}")),
            }
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

        let (preview, entries) = self.commit_turn(agent_id, &prompt, &reply).await?;
        self.clear_pending(agent_id).await;
        let _ = self.shared.turn_tx.send(TurnFinishedHint {
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
impl Gateway for BoxSidecarGateway {
    async fn invoke(&self, agent_id: &str, name: &str, args: Value) -> Result<Value, GatewayError> {
        self.shared.invokes.fetch_add(1, Ordering::SeqCst);
        info!(%agent_id, %name, "box sidecar invoke");
        self.dispatch(agent_id, name, args).await
    }

    fn invoke_count(&self) -> u64 {
        self.shared.invokes.load(Ordering::SeqCst)
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
fn run_allowlisted(cwd: &Path, cmd: &str) -> Result<String, String> {
    match cmd {
        "ls" | "pwd" => {}
        other => return Err(format!("command not on whitelist: {other}")),
    }
    let output = StdCommand::new(cmd)
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
