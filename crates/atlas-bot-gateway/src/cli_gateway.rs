//! Scheme B: Cursor / Atlas Agent CLI adapter gateway.
//!
//! `sendPrompt` spawns `ATLAS_AGENT_CLI` (default `agent`) with `-p` /
//! `--print` and returns a **non-echo** model reply. Child processes are
//! interruptible via `interruptAgentRun` (measurable `start_kill`).

#![allow(dead_code)]

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::sync::{oneshot, Mutex, RwLock};
use tracing::{info, warn};

use crate::{
    AgentRecord, Gateway, GatewayError, TranscriptEntry, RuntimeHint, DEFAULT_AGENT_ID,
    DEFAULT_AGENT_NAME,
};

/// Env var for the CLI binary path (Cursor `agent` / `cursor-agent` / mock).
pub const ENV_AGENT_CLI: &str = "ATLAS_AGENT_CLI";
/// Optional extra args (JSON array of strings), e.g. `["--output-format","text"]`.
pub const ENV_AGENT_CLI_ARGS: &str = "ATLAS_AGENT_CLI_EXTRA_ARGS";
/// Soft timeout for a single CLI turn (ms). 0 = no timeout.
pub const ENV_AGENT_CLI_TIMEOUT_MS: &str = "ATLAS_AGENT_CLI_TIMEOUT_MS";

const DEFAULT_CLI: &str = "agent";

#[derive(Default)]
struct Inner {
    agents: HashMap<String, AgentRecord>,
    transcripts: HashMap<String, Vec<TranscriptEntry>>,
    next_entry_seq: HashMap<String, u64>,
    next_agent_n: u64,
}

struct PendingTurn {
    /// Signal sendPrompt's select to kill the child.
    cancel: Mutex<Option<oneshot::Sender<()>>>,
    /// Child PID while running (0 if unknown).
    pid: AtomicU32,
}

struct Shared {
    inner: RwLock<Inner>,
    invokes: AtomicU64,
    pending: RwLock<HashMap<String, Arc<PendingTurn>>>,
    turn_tx: tokio::sync::broadcast::Sender<RuntimeHint>,
    cli_path: PathBuf,
    extra_args: Vec<String>,
    timeout: Option<Duration>,
}

/// Real-dialogue gateway backed by a local Agent CLI in print mode.
#[derive(Clone)]
pub struct CliAgentGateway {
    shared: Arc<Shared>,
}

impl CliAgentGateway {
    pub fn from_env() -> Self {
        let cli_path = std::env::var(ENV_AGENT_CLI)
            .unwrap_or_else(|_| DEFAULT_CLI.to_string())
            .into();
        let extra_args = std::env::var(ENV_AGENT_CLI_ARGS)
            .ok()
            .and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok())
            .unwrap_or_else(|| vec!["--output-format".into(), "text".into()]);
        let timeout = std::env::var(ENV_AGENT_CLI_TIMEOUT_MS)
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .filter(|ms| *ms > 0)
            .map(Duration::from_millis);
        Self::new(cli_path, extra_args, timeout)
    }

    pub fn new(cli_path: PathBuf, extra_args: Vec<String>, timeout: Option<Duration>) -> Self {
        let (turn_tx, _) = tokio::sync::broadcast::channel(64);
        let mut agents = HashMap::new();
        agents.insert(
            DEFAULT_AGENT_ID.to_string(),
            AgentRecord {
                id: DEFAULT_AGENT_ID.to_string(),
                name: DEFAULT_AGENT_NAME.to_string(),
                description: "P3.5 CLI agent".to_string(),
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
                text: "cli gateway ready".to_string(),
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
                cli_path,
                extra_args,
                timeout,
            }),
        }
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

    async fn run_cli(
        &self,
        agent_id: &str,
        prompt: &str,
        kill_rx: oneshot::Receiver<()>,
        pending: Arc<PendingTurn>,
    ) -> Result<String, GatewayError> {
        let mut cmd = Command::new(&self.shared.cli_path);
        cmd.arg("-p");
        for a in &self.shared.extra_args {
            cmd.arg(a);
        }
        cmd.arg(prompt)
            .env("ATLAS_AGENT_ID", agent_id)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .stdin(Stdio::null());

        info!(cli = %self.shared.cli_path.display(), %agent_id, "spawning agent CLI");
        let mut child = cmd.spawn().map_err(|e| {
            GatewayError::Upstream(format!(
                "CLI spawn failed ({}): {e}",
                self.shared.cli_path.display()
            ))
        })?;

        if let Some(pid) = child.id() {
            pending.pid.store(pid, Ordering::SeqCst);
        }

        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| GatewayError::Upstream("CLI stdout missing".into()))?;
        let mut stderr = child
            .stderr
            .take()
            .ok_or_else(|| GatewayError::Upstream("CLI stderr missing".into()))?;

        let stdout_task = tokio::spawn(async move {
            let mut buf = Vec::new();
            let _ = stdout.read_to_end(&mut buf).await;
            buf
        });
        let stderr_task = tokio::spawn(async move {
            let mut buf = Vec::new();
            let _ = stderr.read_to_end(&mut buf).await;
            buf
        });

        let wait_fut = child.wait();
        let timeout = self.shared.timeout;

        let status = tokio::select! {
            status = wait_fut => {
                status.map_err(|e| GatewayError::Upstream(format!("CLI wait: {e}")))?
            }
            _ = kill_rx => {
                warn!(%agent_id, pid = pending.pid.load(Ordering::SeqCst), "CLI kill via interrupt");
                let _ = child.start_kill();
                let _ = child.wait().await;
                stdout_task.abort();
                stderr_task.abort();
                pending.pid.store(0, Ordering::SeqCst);
                return Err(GatewayError::Upstream("gateway/run-interrupted".into()));
            }
            _ = async {
                if let Some(t) = timeout {
                    tokio::time::sleep(t).await;
                } else {
                    std::future::pending::<()>().await;
                }
            } => {
                warn!(%agent_id, "CLI soft timeout — killing");
                let _ = child.start_kill();
                let _ = child.wait().await;
                stdout_task.abort();
                stderr_task.abort();
                pending.pid.store(0, Ordering::SeqCst);
                return Err(GatewayError::Upstream("gateway/cli-timeout".into()));
            }
        };

        pending.pid.store(0, Ordering::SeqCst);
        let out_buf = stdout_task
            .await
            .map_err(|e| GatewayError::Upstream(format!("stdout join: {e}")))?;
        let err_buf = stderr_task
            .await
            .unwrap_or_default();

        let text = String::from_utf8_lossy(&out_buf).trim().to_string();
        let err_text = String::from_utf8_lossy(&err_buf).trim().to_string();

        if !status.success() {
            return Err(GatewayError::Upstream(format!(
                "CLI exit {}: {}",
                status.code().unwrap_or(-1),
                if err_text.is_empty() { text } else { err_text }
            )));
        }
        if text.is_empty() {
            return Err(GatewayError::Upstream(
                "CLI produced empty reply".into(),
            ));
        }
        // Hard non-echo guard for acceptance: refuse pure echo of the prompt.
        if text == prompt || text == format!("echo: {prompt}") {
            return Err(GatewayError::Upstream(
                "CLI reply looked like stub echo; refusing".into(),
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
        let pending = Arc::new(PendingTurn {
            cancel: Mutex::new(Some(cancel_tx)),
            pid: AtomicU32::new(0),
        });
        {
            let mut map = self.shared.pending.write().await;
            map.insert(agent_id.to_string(), pending.clone());
        }

        let cli_result = self.run_cli(agent_id, &prompt, cancel_rx, pending).await;
        self.clear_pending(agent_id).await;

        match cli_result {
            Ok(reply) => {
                let (preview, entries) = self.commit_turn(agent_id, &prompt, &reply).await?;
                let _ = self.shared.turn_tx.send(RuntimeHint::Finished {
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
                    // Hint for Hub HTTP path (no TurnFinishedHint bridge).
                    "hubEmitTurnFinished": true,
                }))
            }
            Err(e) => {
                self.set_running(agent_id, false).await;
                if e.to_string().contains("run-interrupted") {
                    self.append_system_notice(agent_id, "run interrupted").await;
                }
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

        let mut killed = false;
        if let Some(p) = pending {
            let pid = p.pid.load(Ordering::SeqCst);
            let mut slot = p.cancel.lock().await;
            if let Some(tx) = slot.take() {
                // Measurable kill path: signal run_cli to start_kill the child.
                killed = tx.send(()).is_ok();
                info!(%target, pid, killed, "interrupt signaled");
            }
        }

        let mut guard = self.shared.inner.write().await;
        let Some(agent) = guard.agents.get_mut(&target) else {
            return Err(GatewayError::AgentNotFound(target));
        };
        let had_active = agent.is_running || killed;
        if agent.is_running {
            agent.is_running = false;
        }
        // Never fake success: hadActiveRun reflects real pending/running state.
        Ok(json!({
            "hadActiveRun": had_active,
            "killed": killed,
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
impl Gateway for CliAgentGateway {
    async fn invoke(&self, agent_id: &str, name: &str, args: Value) -> Result<Value, GatewayError> {
        self.shared.invokes.fetch_add(1, Ordering::SeqCst);
        info!(%agent_id, %name, "cli gateway invoke");
        self.dispatch(agent_id, name, args).await
    }

    fn invoke_count(&self) -> u64 {
        self.shared.invokes.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl Gateway for Arc<CliAgentGateway> {
    async fn invoke(&self, agent_id: &str, name: &str, args: Value) -> Result<Value, GatewayError> {
        (**self).invoke(agent_id, name, args).await
    }

    fn invoke_count(&self) -> u64 {
        (**self).invoke_count()
    }
}
