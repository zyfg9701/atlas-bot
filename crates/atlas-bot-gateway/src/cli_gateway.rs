//! Scheme B: Cursor / Atlas Agent CLI adapter gateway.
//!
//! `sendPrompt` spawns `ATLAS_AGENT_CLI` (default `agent`). The default
//! dialect is Atlas headless (`--output-format json` then `-p <prompt>`,
//! because `-p` / `--single` takes the prompt as its value). JSON stdout
//! uses the top-level `text` field. Set `ATLAS_AGENT_CLI_DIALECT=cursor`
//! to restore the legacy Cursor shape (`-p` flag, then `--output-format text`
//! or `stream-json` + `--stream-partial-output`, then a positional prompt).
//!
//! CS1 streaming (opt-in via `ATLAS_AGENT_CLI_STREAM=1`): line-read stdout.
//! Atlas default is `--output-format streaming-json` (no `--stream-partial-output`).
//! Cursor dialect keeps `stream-json`. Map those lines / `ATLAS_DELTA` →
//! [`RuntimeHint`] mid-turn, then Finished. Never fake-streams a final reply
//! into deltas. Optional B1 POST when `ATLAS_HUB_EVENT_URL` is set (fail-warn only).
//!
//! CG1 tool approval (reuses GA1 [`ApprovalState`] + `POST /approve`):
//! when stream is on, dangerous `tool_call` status=started (and mock
//! `ATLAS_TOOL`) hang for Allow/Deny/timeout; Deny/timeout → interrupt/kill
//! child. Text mode cannot gate mid-turn tools — see
//! `docs/tool-approval-runbook.md` § cli/CG1. Not box-style zero side-effects.

#![allow(dead_code)]

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::{oneshot, Mutex, RwLock};
use tracing::{info, warn};

use uuid::Uuid;

use crate::tool_approval::{
    classify_cli_tool, format_pending_summary, ApprovalDecision, ApprovalState, CliToolGateClass,
    GateOutcome, ToolApprovalMode,
};
use crate::{
    agent_summary_value,
    box_sidecar::{ENV_HUB_EVENT_TOKEN, ENV_HUB_EVENT_URL, HUB_EVENT_POST_TIMEOUT_MS},
    AgentRecord, Gateway, GatewayError, RuntimeHint, TranscriptEntry, DEFAULT_AGENT_ID,
    DEFAULT_AGENT_NAME,
};

/// Env var for the CLI binary path (Cursor `agent` / `cursor-agent` / mock).
pub const ENV_AGENT_CLI: &str = "ATLAS_AGENT_CLI";
/// Optional extra args (JSON array of strings), e.g. `["--output-format","plain"]`.
/// When set, replaces the dialect default. Prompt is still appended by the gateway
/// (`-p <prompt>` on Atlas; positional after `-p` on Cursor). Do not put the prompt here.
pub const ENV_AGENT_CLI_ARGS: &str = "ATLAS_AGENT_CLI_EXTRA_ARGS";
/// Soft timeout for a single CLI turn (ms). 0 = no timeout.
pub const ENV_AGENT_CLI_TIMEOUT_MS: &str = "ATLAS_AGENT_CLI_TIMEOUT_MS";
/// Opt-in CS1 streaming: `1`/`true`/`yes` → line-parse stdout for mid-turn
/// [`RuntimeHint`]s. Unset = text/json document mode (no mid-turn).
/// Default EXTRA_ARGS (when unset) follow [`ENV_AGENT_CLI_DIALECT`]:
/// Atlas `streaming-json`; Cursor `stream-json` + `--stream-partial-output`.
pub const ENV_AGENT_CLI_STREAM: &str = "ATLAS_AGENT_CLI_STREAM";
/// `atlas` (default) or `cursor` / `legacy`. Controls default format flags and
/// whether `-p` takes the prompt as its value (Atlas) or is a boolean print flag
/// (Cursor).
pub const ENV_AGENT_CLI_DIALECT: &str = "ATLAS_AGENT_CLI_DIALECT";

const DEFAULT_CLI: &str = "agent";

/// Which Agent CLI flag dialect to spawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CliDialect {
    /// Atlas / current `agent.exe`: `-p`/`--single <PROMPT>` consumes the next
    /// argv; formats `plain|json|streaming-json|streaming-messages-json`.
    Atlas,
    /// Legacy Cursor agent: `-p`/`--print` is a flag; prompt is positional;
    /// formats `text` and `stream-json` (+ `--stream-partial-output`).
    Cursor,
}

impl CliDialect {
    fn parse(raw: Option<&str>) -> Self {
        match raw
            .map(str::trim)
            .map(|s| s.to_ascii_lowercase())
            .as_deref()
        {
            Some("cursor") | Some("legacy") => CliDialect::Cursor,
            _ => CliDialect::Atlas,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            CliDialect::Atlas => "atlas",
            CliDialect::Cursor => "cursor",
        }
    }
}

struct CliSpawnPlan {
    dialect: CliDialect,
    extra_args: Vec<String>,
    stream: bool,
}

fn resolve_spawn_plan(
    dialect_raw: Option<&str>,
    stream: bool,
    extra_args_raw: Option<&str>,
) -> CliSpawnPlan {
    let dialect = CliDialect::parse(dialect_raw);
    let extra_args = extra_args_raw
        .and_then(|s| serde_json::from_str::<Vec<String>>(s).ok())
        .unwrap_or_else(|| default_extra_args(dialect, stream));
    CliSpawnPlan {
        dialect,
        extra_args,
        stream,
    }
}

/// Argv after the binary name.
///
/// Atlas: format flags, then `-p`, then the prompt as the value of `-p`.
/// Cursor: `-p` flag, then format flags, then the positional prompt.
fn build_cli_argv(dialect: CliDialect, extra_args: &[String], prompt: &str) -> Vec<String> {
    match dialect {
        CliDialect::Atlas => {
            let mut v = extra_args.to_vec();
            v.push("-p".into());
            v.push(prompt.into());
            v
        }
        CliDialect::Cursor => {
            let mut v = Vec::with_capacity(extra_args.len() + 2);
            v.push("-p".into());
            v.extend(extra_args.iter().cloned());
            v.push(prompt.into());
            v
        }
    }
}

fn output_format_value(extra_args: &[String]) -> Option<&str> {
    let mut iter = extra_args.iter().map(String::as_str);
    while let Some(a) = iter.next() {
        if let Some(v) = a.strip_prefix("--output-format=") {
            return Some(v);
        }
        if a == "--output-format" {
            return iter.next();
        }
    }
    None
}

/// Atlas `--output-format json` is one JSON object with top-level `text`.
/// `plain` / Cursor `text` / mock stdout are raw reply text.
fn reply_is_json_document(extra_args: &[String]) -> bool {
    matches!(output_format_value(extra_args), Some("json"))
}

fn env_truthy(name: &str) -> bool {
    std::env::var(name)
        .ok()
        .map(|s| {
            matches!(
                s.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

fn default_extra_args(dialect: CliDialect, stream: bool) -> Vec<String> {
    match (dialect, stream) {
        (CliDialect::Atlas, false) => vec!["--output-format".into(), "json".into()],
        // Atlas rejects `text` / `stream-json` and `--stream-partial-output`.
        (CliDialect::Atlas, true) => vec!["--output-format".into(), "streaming-json".into()],
        (CliDialect::Cursor, false) => vec!["--output-format".into(), "text".into()],
        (CliDialect::Cursor, true) => vec![
            "--output-format".into(),
            "stream-json".into(),
            "--stream-partial-output".into(),
        ],
    }
}

#[derive(Default)]
struct Inner {
    agents: HashMap<String, AgentRecord>,
    transcripts: HashMap<String, Vec<TranscriptEntry>>,
    next_entry_seq: HashMap<String, u64>,
    next_agent_n: u64,
}

#[derive(Debug)]
enum StreamLineKind {
    Final(String),
    /// Atlas `streaming-json` `type=error` (or equivalent). Not a chat reply.
    CliError(String),
    ToolStarted {
        tool: String,
        summary: String,
        status: String,
    },
    ToolObserved {
        tool: String,
        summary: String,
        exit_code: Option<i32>,
    },
    Other,
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
    dialect: CliDialect,
    extra_args: Vec<String>,
    timeout: Option<Duration>,
    /// CS1: line-parse stdout for mid-turn hints.
    stream_enabled: bool,
    /// CS1b: optional Hub ingest URL (same as Box B1).
    event_url: Option<String>,
    event_token: Option<String>,
    http: reqwest::Client,
    /// CG1: reuse GA1 ApprovalState (local gate + /approve).
    approval: ApprovalState,
}

/// Real-dialogue gateway backed by a local Agent CLI in print mode.
#[derive(Clone)]
pub struct CliAgentGateway {
    shared: Arc<Shared>,
}

impl CliAgentGateway {
    /// Product path: unset `ATLAS_TOOL_APPROVAL_MODE` → **`gate`** (CG1; aligned with feasibility §5).
    /// Gate only covers stream mode; text mode cannot mid-turn gate (documented).
    pub fn from_env() -> Self {
        let cli_path = std::env::var(ENV_AGENT_CLI)
            .unwrap_or_else(|_| DEFAULT_CLI.to_string())
            .into();
        let stream = env_truthy(ENV_AGENT_CLI_STREAM);
        let plan = resolve_spawn_plan(
            std::env::var(ENV_AGENT_CLI_DIALECT).ok().as_deref(),
            stream,
            std::env::var(ENV_AGENT_CLI_ARGS).ok().as_deref(),
        );
        let timeout = std::env::var(ENV_AGENT_CLI_TIMEOUT_MS)
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .filter(|ms| *ms > 0)
            .map(Duration::from_millis);
        let event_url = std::env::var(ENV_HUB_EVENT_URL)
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let event_token = std::env::var(ENV_HUB_EVENT_TOKEN)
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let approval_mode = ToolApprovalMode::from_env_product_default();
        Self::new_full(
            cli_path,
            plan.dialect,
            plan.extra_args,
            timeout,
            plan.stream,
            event_url,
            event_token,
            approval_mode,
        )
    }

    /// Text-mode constructor (no mid-turn streaming). Used by p35 smoke.
    /// In-process default approval mode = `off` (env still honored) so deepen/cli smokes stay green.
    pub fn new(cli_path: PathBuf, extra_args: Vec<String>, timeout: Option<Duration>) -> Self {
        Self::new_full(
            cli_path,
            CliDialect::Atlas,
            extra_args,
            timeout,
            false,
            None,
            None,
            ToolApprovalMode::from_env_test_default(),
        )
    }

    /// CS1 streaming constructor (same-process B2 via `turn_tx`).
    pub fn new_streaming(
        cli_path: PathBuf,
        extra_args: Vec<String>,
        timeout: Option<Duration>,
    ) -> Self {
        let args = if extra_args.is_empty() {
            default_extra_args(CliDialect::Atlas, true)
        } else {
            extra_args
        };
        Self::new_full(
            cli_path,
            CliDialect::Atlas,
            args,
            timeout,
            true,
            None,
            None,
            ToolApprovalMode::from_env_test_default(),
        )
    }

    /// CS1b: streaming + optional B1 `ATLAS_HUB_EVENT_URL` POST.
    pub fn new_with_event(
        cli_path: PathBuf,
        extra_args: Vec<String>,
        timeout: Option<Duration>,
        stream: bool,
        event_url: Option<String>,
        event_token: Option<String>,
    ) -> Self {
        let args = if extra_args.is_empty() {
            default_extra_args(CliDialect::Atlas, stream)
        } else {
            extra_args
        };
        Self::new_full(
            cli_path,
            CliDialect::Atlas,
            args,
            timeout,
            stream,
            event_url,
            event_token,
            ToolApprovalMode::from_env_test_default(),
        )
    }

    /// CG1 smoke / explicit approval mode (stream typically true).
    pub fn new_with_approval(
        cli_path: PathBuf,
        extra_args: Vec<String>,
        timeout: Option<Duration>,
        stream: bool,
        approval_mode: ToolApprovalMode,
    ) -> Self {
        let args = if extra_args.is_empty() {
            default_extra_args(CliDialect::Atlas, stream)
        } else {
            extra_args
        };
        Self::new_full(
            cli_path,
            CliDialect::Atlas,
            args,
            timeout,
            stream,
            None,
            None,
            approval_mode,
        )
    }

    fn new_full(
        cli_path: PathBuf,
        dialect: CliDialect,
        extra_args: Vec<String>,
        timeout: Option<Duration>,
        stream_enabled: bool,
        event_url: Option<String>,
        event_token: Option<String>,
        approval_mode: ToolApprovalMode,
    ) -> Self {
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
                is_group: false,
                member_ids: Vec::new(),
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
                dialect,
                extra_args,
                timeout,
                stream_enabled,
                event_url,
                event_token,
                http: reqwest::Client::new(),
                approval: ApprovalState::new(
                    approval_mode,
                    ApprovalState::timeout_from_env(),
                    ApprovalState::token_from_env(),
                ),
            }),
        }
    }

    pub fn tool_approval_mode(&self) -> ToolApprovalMode {
        self.shared.approval.mode
    }

    pub fn tool_approval_timeout_ms(&self) -> u64 {
        self.shared.approval.timeout.as_millis() as u64
    }

    pub fn tool_approval_token_configured(&self) -> bool {
        self.shared.approval.token.is_some()
    }

    pub fn stream_enabled(&self) -> bool {
        self.shared.stream_enabled
    }

    /// Test/CI hook: queue a decision applied to the next gated tool (FIFO).
    pub fn inject_approval_decision(&self, decision: ApprovalDecision) {
        if let Ok(mut q) = self.shared.approval.inject.lock() {
            q.push_back(decision);
        }
    }

    /// Resolve a pending approval (in-process or HTTP `/approve`).
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
            "CG1 cli tool approval resolved"
        );
        Ok(json!({
            "ok": true,
            "approvalId": approval_id,
            "decision": decision.as_str(),
            "tool": slot.tool,
            "agentId": slot.agent_id,
        }))
    }

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

    /// Configured CLI path/name (from `ATLAS_AGENT_CLI` or default `agent`).
    pub fn cli_path(&self) -> &std::path::Path {
        &self.shared.cli_path
    }

    pub async fn snapshot_transcript(&self, agent_id: &str) -> Vec<TranscriptEntry> {
        let guard = self.shared.inner.read().await;
        guard.transcripts.get(agent_id).cloned().unwrap_or_default()
    }

    pub async fn snapshot_agents(&self) -> Vec<AgentRecord> {
        let guard = self.shared.inner.read().await;
        let mut v: Vec<_> = guard.agents.values().cloned().collect();
        v.sort_by(|a, b| a.id.cmp(&b.id));
        v
    }

    fn agent_summary(a: &AgentRecord) -> Value {
        agent_summary_value(a)
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
            is_group: false,
            member_ids: Vec::new(),
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
            let n = guard
                .next_entry_seq
                .entry(agent_id.to_string())
                .or_insert(1);
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
            let n = guard
                .next_entry_seq
                .entry(agent_id.to_string())
                .or_insert(1);
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

    /// Broadcast + optional B1 POST (fail-warn only; never blocks sendPrompt).
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

    fn emit_delta(&self, agent_id: &str, text: &str) {
        let text = truncate_hint(text);
        if text.is_empty() {
            return;
        }
        self.publish_hint(RuntimeHint::AssistantDelta {
            agent_id: agent_id.to_string(),
            text,
        });
    }

    fn emit_tool(&self, agent_id: &str, tool: &str, summary: &str, exit_code: Option<i32>) {
        self.publish_hint(RuntimeHint::Tool {
            agent_id: agent_id.to_string(),
            tool: tool.to_string(),
            summary: truncate_hint(summary),
            exit_code,
        });
    }

    /// Apply one stdout line while streaming (no gate). Returns Some(final) when `result` seen.
    fn handle_stream_line(
        &self,
        agent_id: &str,
        line: &str,
        deltas: &mut Vec<String>,
    ) -> Option<String> {
        match self.classify_stream_line(agent_id, line, deltas) {
            StreamLineKind::Final(s) => Some(s),
            StreamLineKind::CliError(_) => None,
            StreamLineKind::ToolStarted { tool, summary, .. } => {
                self.emit_tool(agent_id, &tool, &summary, None);
                None
            }
            StreamLineKind::ToolObserved {
                tool,
                summary,
                exit_code,
            } => {
                self.emit_tool(agent_id, &tool, &summary, exit_code);
                None
            }
            StreamLineKind::Other => None,
        }
    }

    fn classify_stream_line(
        &self,
        agent_id: &str,
        line: &str,
        deltas: &mut Vec<String>,
    ) -> StreamLineKind {
        let _ = agent_id;
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            return StreamLineKind::Other;
        }

        if let Some(rest) = line.strip_prefix("ATLAS_DELTA\t") {
            self.emit_delta(agent_id, rest);
            deltas.push(rest.to_string());
            return StreamLineKind::Other;
        }
        // Mock ATLAS_TOOL → treat as started (gate-eligible) when CG1 active.
        if let Some(rest) = line.strip_prefix("ATLAS_TOOL\t") {
            let mut parts = rest.splitn(3, '\t');
            let tool = parts.next().unwrap_or("tool").to_string();
            let summary = parts.next().unwrap_or("").to_string();
            let code = parts.next().and_then(|s| s.parse::<i32>().ok());
            // If exit code present, treat as completed observation; else started.
            if code.is_some() {
                return StreamLineKind::ToolObserved {
                    tool,
                    summary,
                    exit_code: code,
                };
            }
            return StreamLineKind::ToolStarted {
                tool,
                summary,
                status: "started".into(),
            };
        }

        let Ok(v) = serde_json::from_str::<Value>(line) else {
            return StreamLineKind::Other;
        };
        let ty = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
        match ty {
            "assistant" => {
                if !is_assistant_partial(&v) {
                    return StreamLineKind::Other;
                }
                if let Some(t) = extract_assistant_text(&v) {
                    if !t.is_empty() {
                        self.emit_delta(agent_id, &t);
                        deltas.push(t);
                    }
                }
                StreamLineKind::Other
            }
            // Atlas streaming-json: one text chunk per line (`data`), not Cursor `assistant`.
            "text" => {
                if let Some(t) = v
                    .get("data")
                    .and_then(|d| d.as_str())
                    .filter(|s| !s.is_empty())
                {
                    self.emit_delta(agent_id, t);
                    deltas.push(t.to_string());
                }
                StreamLineKind::Other
            }
            "tool_call" => classify_tool_call_line(&v),
            // Progress/completion. The matching `tool_call` already gated; do not hang again.
            "tool_call_update" => StreamLineKind::ToolObserved {
                tool: cli_stream_tool_name(&v),
                summary: cli_stream_summary(&v, cli_stream_status(&v).as_str()),
                exit_code: v
                    .get("exit_code")
                    .and_then(|c| c.as_i64())
                    .map(|c| c as i32),
            },
            "result" => {
                if let Some(r) = v.get("result").and_then(|r| r.as_str()) {
                    StreamLineKind::Final(r.to_string())
                } else {
                    StreamLineKind::Other
                }
            }
            "error" => {
                let msg = v
                    .get("message")
                    .and_then(|m| m.as_str())
                    .filter(|s| !s.is_empty())
                    .unwrap_or("CLI stream error");
                StreamLineKind::CliError(msg.to_string())
            }
            // `end` is the Atlas streaming-json trailer. Reply text is the joined `text` chunks.
            "end" => StreamLineKind::Other,
            _ => StreamLineKind::Other,
        }
    }

    /// CG1: hang for Allow/Deny/timeout. Failures → Deny (never Allow).
    async fn await_cli_tool_approval(
        &self,
        agent_id: &str,
        tool: &str,
        detail: &str,
    ) -> GateOutcome {
        match self.shared.approval.mode {
            ToolApprovalMode::Off => return GateOutcome::Allow,
            ToolApprovalMode::AutoDeny => {
                info!(%agent_id, %tool, "CG1 auto_deny");
                return GateOutcome::AutoDeny;
            }
            ToolApprovalMode::Gate => {}
        }

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

        let pending_summary = format_pending_summary(&approval_id, tool, detail);
        self.emit_tool(agent_id, "approval_pending", &pending_summary, None);
        info!(%agent_id, %tool, %approval_id, "CG1 approval pending");

        let timeout = self.shared.approval.timeout;
        let outcome = tokio::select! {
            res = rx => match res {
                Ok(ApprovalDecision::Allow) => GateOutcome::Allow,
                Ok(ApprovalDecision::Deny) => GateOutcome::Deny,
                Err(_) => {
                    warn!(%approval_id, "CG1 approval channel closed; denying");
                    GateOutcome::Deny
                }
            },
            _ = tokio::time::sleep(timeout) => GateOutcome::Timeout,
        };

        {
            let mut map = self.shared.approval.pending.write().await;
            map.remove(&approval_id);
        }

        info!(
            %agent_id,
            %tool,
            %approval_id,
            outcome = outcome.reason(),
            "CG1 approval settled"
        );
        outcome
    }

    async fn run_cli(
        &self,
        agent_id: &str,
        prompt: &str,
        kill_rx: oneshot::Receiver<()>,
        pending: Arc<PendingTurn>,
    ) -> Result<String, GatewayError> {
        let argv = build_cli_argv(self.shared.dialect, &self.shared.extra_args, prompt);
        let mut cmd = Command::new(&self.shared.cli_path);
        for a in &argv {
            cmd.arg(a);
        }
        cmd.env("ATLAS_AGENT_ID", agent_id)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .stdin(Stdio::null());

        info!(
            cli = %self.shared.cli_path.display(),
            %agent_id,
            dialect = self.shared.dialect.as_str(),
            extra = %self.shared.extra_args.join(" "),
            stream = self.shared.stream_enabled,
            approval = self.shared.approval.mode.as_str(),
            "spawning agent CLI"
        );
        let mut child = cmd.spawn().map_err(|e| {
            let path = self.shared.cli_path.display();
            if e.kind() == std::io::ErrorKind::NotFound {
                GatewayError::Upstream(format!(
                    "CLI binary not found ({path}): install and login Cursor/Atlas agent, or set ATLAS_AGENT_CLI to a real binary (see docs/cli-primary-runbook.md)"
                ))
            } else {
                GatewayError::Upstream(format!("CLI spawn failed ({path}): {e}"))
            }
        })?;

        if let Some(pid) = child.id() {
            pending.pid.store(pid, Ordering::SeqCst);
        }

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| GatewayError::Upstream("CLI stdout missing".into()))?;
        let mut stderr = child
            .stderr
            .take()
            .ok_or_else(|| GatewayError::Upstream("CLI stderr missing".into()))?;

        let stderr_task = tokio::spawn(async move {
            let mut buf = Vec::new();
            let _ = stderr.read_to_end(&mut buf).await;
            buf
        });

        let timeout = self.shared.timeout;
        let stream = self.shared.stream_enabled;
        let json_doc = reply_is_json_document(&self.shared.extra_args);

        let text = if stream {
            self.run_cli_streaming(
                agent_id,
                prompt,
                &mut child,
                stdout,
                kill_rx,
                pending.clone(),
                timeout,
            )
            .await?
        } else {
            self.run_cli_text(
                agent_id,
                prompt,
                &mut child,
                stdout,
                kill_rx,
                pending.clone(),
                timeout,
                json_doc,
            )
            .await?
        };

        pending.pid.store(0, Ordering::SeqCst);
        let err_buf = stderr_task.await.unwrap_or_default();
        let err_text = String::from_utf8_lossy(&err_buf).trim().to_string();

        if text.is_empty() {
            return Err(GatewayError::Upstream(if err_text.is_empty() {
                "CLI produced empty reply".into()
            } else {
                format!("CLI produced empty reply; stderr: {err_text}")
            }));
        }
        if text == prompt || text == format!("echo: {prompt}") {
            return Err(GatewayError::Upstream(
                "CLI reply looked like stub echo; refusing".into(),
            ));
        }
        let _ = err_text;
        Ok(text)
    }

    async fn run_cli_text(
        &self,
        agent_id: &str,
        prompt: &str,
        child: &mut tokio::process::Child,
        mut stdout: impl tokio::io::AsyncRead + Unpin + Send + 'static,
        kill_rx: oneshot::Receiver<()>,
        pending: Arc<PendingTurn>,
        timeout: Option<Duration>,
        json_doc: bool,
    ) -> Result<String, GatewayError> {
        // CG1: text mode cannot gate mid-turn tools (no tool_call visibility).
        let stdout_task = tokio::spawn(async move {
            let mut buf = Vec::new();
            let _ = stdout.read_to_end(&mut buf).await;
            buf
        });

        let wait_fut = child.wait();
        let status = tokio::select! {
            status = wait_fut => {
                status.map_err(|e| GatewayError::Upstream(format!("CLI wait: {e}")))?
            }
            _ = kill_rx => {
                warn!(%agent_id, pid = pending.pid.load(Ordering::SeqCst), "CLI kill via interrupt");
                let _ = child.start_kill();
                let _ = child.wait().await;
                stdout_task.abort();
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
                pending.pid.store(0, Ordering::SeqCst);
                return Err(GatewayError::Upstream(
                    "CLI timeout (ATLAS_AGENT_CLI_TIMEOUT_MS exceeded)".into(),
                ));
            }
        };

        let out_buf = stdout_task
            .await
            .map_err(|e| GatewayError::Upstream(format!("stdout join: {e}")))?;
        let raw = String::from_utf8_lossy(&out_buf).to_string();
        if !status.success() {
            let detail = humanize_cli_failure_body(&raw);
            return Err(GatewayError::Upstream(format!(
                "CLI non-zero exit {}: {detail}",
                status.code().unwrap_or(-1)
            )));
        }
        let _ = prompt;
        extract_text_mode_reply(&raw, json_doc)
    }

    async fn run_cli_streaming(
        &self,
        agent_id: &str,
        prompt: &str,
        child: &mut tokio::process::Child,
        stdout: impl tokio::io::AsyncRead + Unpin + Send + 'static,
        mut kill_rx: oneshot::Receiver<()>,
        pending: Arc<PendingTurn>,
        timeout: Option<Duration>,
    ) -> Result<String, GatewayError> {
        let mut reader = BufReader::new(stdout);
        let mut deltas: Vec<String> = Vec::new();
        let mut final_from_result: Option<String> = None;
        let mut cli_error: Option<String> = None;
        let mut plain = String::new();
        let mut line = String::new();
        // Deny/timeout returns early with closed-set preview (no deferred reason).
        let deadline = timeout.map(|t| tokio::time::Instant::now() + t);

        loop {
            if let Some(d) = deadline {
                if tokio::time::Instant::now() >= d {
                    warn!(%agent_id, "CLI soft timeout — killing");
                    let _ = child.start_kill();
                    let _ = child.wait().await;
                    pending.pid.store(0, Ordering::SeqCst);
                    return Err(GatewayError::Upstream(
                        "CLI timeout (ATLAS_AGENT_CLI_TIMEOUT_MS exceeded)".into(),
                    ));
                }
            }

            line.clear();
            let read_deadline = deadline.unwrap_or_else(|| {
                tokio::time::Instant::now() + Duration::from_secs(365 * 24 * 3600)
            });
            let n = tokio::select! {
                biased;
                res = reader.read_line(&mut line) => {
                    res.map_err(|e| GatewayError::Upstream(format!("CLI stdout read: {e}")))?
                }
                _ = &mut kill_rx => {
                    warn!(%agent_id, pid = pending.pid.load(Ordering::SeqCst), "CLI kill via interrupt");
                    let _ = child.start_kill();
                    let _ = child.wait().await;
                    pending.pid.store(0, Ordering::SeqCst);
                    return Err(GatewayError::Upstream("gateway/run-interrupted".into()));
                }
                _ = tokio::time::sleep_until(read_deadline) => {
                    warn!(%agent_id, "CLI soft timeout — killing");
                    let _ = child.start_kill();
                    let _ = child.wait().await;
                    pending.pid.store(0, Ordering::SeqCst);
                    return Err(GatewayError::Upstream(
                        "CLI timeout (ATLAS_AGENT_CLI_TIMEOUT_MS exceeded)".into(),
                    ));
                }
            };

            if n == 0 {
                break;
            }
            let trimmed = line.trim_end_matches(['\r', '\n']);
            if trimmed.is_empty() {
                continue;
            }
            let is_json = trimmed.starts_with('{');
            let is_atlas = trimmed.starts_with("ATLAS_");
            let kind = self.classify_stream_line(agent_id, trimmed, &mut deltas);
            match kind {
                StreamLineKind::Final(fin) => {
                    final_from_result = Some(fin);
                }
                StreamLineKind::CliError(msg) => {
                    cli_error = Some(msg);
                }
                StreamLineKind::ToolObserved {
                    tool,
                    summary,
                    exit_code,
                } => {
                    self.emit_tool(agent_id, &tool, &summary, exit_code);
                }
                StreamLineKind::ToolStarted {
                    tool,
                    summary,
                    status: _,
                } => {
                    let class = classify_cli_tool(&tool, &summary, "started");
                    match class {
                        CliToolGateClass::Exempt | CliToolGateClass::ObserveOnly => {
                            self.emit_tool(agent_id, &tool, &summary, None);
                        }
                        CliToolGateClass::Gated => {
                            // Always emit the started observation, then gate.
                            self.emit_tool(agent_id, &tool, &summary, None);
                            let gate_active = self.shared.approval.mode != ToolApprovalMode::Off;
                            if gate_active {
                                let detail = format!("cli tool_call started name={tool} {summary}");
                                let outcome = tokio::select! {
                                    biased;
                                    o = self.await_cli_tool_approval(agent_id, &tool, &detail) => o,
                                    _ = &mut kill_rx => {
                                        warn!(%agent_id, "interrupt during CG1 approval hang → Deny");
                                        GateOutcome::Deny
                                    }
                                };
                                if !outcome.is_allow() {
                                    let reason = outcome.reason();
                                    // Deny/timeout/auto_deny → interrupt/kill (best-effort).
                                    // Never silent Allow. Kill failure still records Deny.
                                    if let Err(e) = child.start_kill() {
                                        warn!(%agent_id, error = %e, "CG1 start_kill failed; still Deny");
                                    }
                                    let _ = child.wait().await;
                                    pending.pid.store(0, Ordering::SeqCst);
                                    // Closed-set reason as turn reply (greppable preview).
                                    return Ok(format!(
                                        "[cli tool approval] {reason} tool={tool} (child interrupted; not box zero-side-effect)"
                                    ));
                                }
                                // Allow: do not kill; continue consuming stream.
                                self.emit_tool(
                                    agent_id,
                                    &tool,
                                    &format!("{summary} approval=allow"),
                                    None,
                                );
                            }
                        }
                    }
                }
                StreamLineKind::Other => {
                    if !is_json && !is_atlas {
                        if !plain.is_empty() {
                            plain.push('\n');
                        }
                        plain.push_str(trimmed);
                    }
                }
            }
        }

        // Drain process exit (if still running).
        let status = tokio::select! {
            status = child.wait() => {
                status.map_err(|e| GatewayError::Upstream(format!("CLI wait: {e}")))?
            }
            _ = &mut kill_rx => {
                warn!(%agent_id, pid = pending.pid.load(Ordering::SeqCst), "CLI kill via interrupt");
                let _ = child.start_kill();
                let _ = child.wait().await;
                pending.pid.store(0, Ordering::SeqCst);
                return Err(GatewayError::Upstream("gateway/run-interrupted".into()));
            }
        };

        if !status.success() {
            let detail = cli_error
                .clone()
                .filter(|s| !s.trim().is_empty())
                .or_else(|| final_from_result.clone().filter(|s| !s.trim().is_empty()))
                .unwrap_or_else(|| deltas.join(""));
            let detail = detail.trim();
            let code = status.code().unwrap_or(-1);
            return Err(if detail.is_empty() {
                GatewayError::Upstream(format!("CLI non-zero exit {code}"))
            } else {
                GatewayError::Upstream(format!(
                    "CLI non-zero exit {code}: {}",
                    truncate_hint(detail)
                ))
            });
        }
        if let Some(msg) = cli_error.filter(|s| !s.trim().is_empty()) {
            return Err(GatewayError::Upstream(format!(
                "CLI JSON error: {}",
                truncate_hint(msg.trim())
            )));
        }

        let text = if let Some(r) = final_from_result {
            r.trim().to_string()
        } else if !plain.trim().is_empty() {
            plain.trim().to_string()
        } else if !deltas.is_empty() {
            deltas.join("")
        } else {
            String::new()
        };
        let _ = prompt;
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
        let entries = guard.transcripts.get(agent_id).cloned().unwrap_or_default();
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

    async fn dispatch(
        &self,
        agent_id: &str,
        name: &str,
        args: Value,
    ) -> Result<Value, GatewayError> {
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

    async fn tool_approve(&self, approval_id: &str, decision: &str) -> Result<Value, GatewayError> {
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
impl Gateway for Arc<CliAgentGateway> {
    async fn invoke(&self, agent_id: &str, name: &str, args: Value) -> Result<Value, GatewayError> {
        (**self).invoke(agent_id, name, args).await
    }

    fn invoke_count(&self) -> u64 {
        (**self).invoke_count()
    }

    async fn tool_approve(&self, approval_id: &str, decision: &str) -> Result<Value, GatewayError> {
        (**self).tool_approve(approval_id, decision).await
    }

    fn tool_approval_info(&self) -> Option<(String, bool)> {
        (**self).tool_approval_info()
    }
}

/// Text-mode stdout. JSON mode reads one Atlas object `{text, stopReason, sessionId}`.
/// Plain text, mock-cli, and NDJSON (more than one JSON value) pass through unchanged
/// so CI mock stays a reply instead of a parse error.
fn extract_text_mode_reply(raw: &str, json_mode: bool) -> Result<String, GatewayError> {
    let trimmed = raw.trim();
    if !json_mode || trimmed.is_empty() || !trimmed.starts_with('{') {
        return Ok(trimmed.to_string());
    }
    // One value only. Trailing NDJSON lines fail from_str; do not parse the first line.
    let Ok(value) = serde_json::from_str::<Value>(trimmed) else {
        return Ok(trimmed.to_string());
    };
    let Some(obj) = value.as_object() else {
        return Err(GatewayError::Upstream(
            "CLI JSON reply was not an object".into(),
        ));
    };
    if obj.get("type").and_then(|t| t.as_str()) == Some("error") {
        let msg = obj
            .get("message")
            .and_then(|m| m.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("CLI JSON error");
        return Err(GatewayError::Upstream(format!("CLI JSON error: {msg}")));
    }
    if let Some(text) = obj.get("text").and_then(|t| t.as_str()) {
        let text = text.trim();
        if text.is_empty() {
            if let Some(reason) = obj.get("stopReason").and_then(|s| s.as_str()) {
                let reason = reason.trim();
                if !reason.is_empty() && !reason.eq_ignore_ascii_case("end_turn") {
                    return Err(GatewayError::Upstream(format!(
                        "CLI JSON reply empty (stopReason={reason})"
                    )));
                }
            }
        }
        return Ok(text.to_string());
    }
    if obj.get("is_error").and_then(|v| v.as_bool()) == Some(true) {
        let msg = obj
            .get("message")
            .and_then(|m| m.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("is_error");
        return Err(GatewayError::Upstream(format!("CLI JSON error: {msg}")));
    }
    Err(GatewayError::Upstream(
        "CLI JSON reply missing text field".into(),
    ))
}

fn humanize_cli_failure_body(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if let Ok(v) = serde_json::from_str::<Value>(trimmed) {
        if let Some(msg) = v
            .get("message")
            .and_then(|m| m.as_str())
            .filter(|s| !s.is_empty())
        {
            return msg.to_string();
        }
        if let Some(err) = v
            .get("error")
            .and_then(|m| m.as_str())
            .filter(|s| !s.is_empty())
        {
            return err.to_string();
        }
    }
    trimmed.to_string()
}

fn truncate_hint(s: &str) -> String {
    const MAX: usize = 64 * 1024;
    if s.len() <= MAX {
        s.to_string()
    } else {
        let mut t = s.chars().take(MAX).collect::<String>();
        t.push('…');
        t
    }
}

/// Cursor stream-json partial filter: prefer `timestamp_ms` without `model_call_id`.
/// Also accept mock/simple assistant rows that lack both fields (no model_call_id).
fn is_assistant_partial(v: &Value) -> bool {
    let has_model = v.get("model_call_id").is_some();
    let has_ts = v.get("timestamp_ms").is_some();
    if has_model && !has_ts {
        return false;
    }
    if has_ts && !has_model {
        return true;
    }
    // Soft fallback for mock NDJSON without Cursor partial markers.
    !has_model
}

fn classify_tool_call_line(v: &Value) -> StreamLineKind {
    let status = cli_stream_status(v);
    let tool = cli_stream_tool_name(v);
    let summary = cli_stream_summary(v, &status);
    let exit_code = v
        .get("exit_code")
        .and_then(|c| c.as_i64())
        .map(|c| c as i32);
    // `completed` is observe-only. Atlas `in_progress` and Cursor `started` both gate later.
    if status.eq_ignore_ascii_case("completed") {
        StreamLineKind::ToolObserved {
            tool,
            summary,
            exit_code,
        }
    } else {
        StreamLineKind::ToolStarted {
            tool,
            summary,
            status,
        }
    }
}

fn cli_stream_tool_name(v: &Value) -> String {
    v.get("name")
        .or_else(|| v.get("toolName"))
        .or_else(|| v.get("tool"))
        .or_else(|| v.pointer("/tool_call/name"))
        .and_then(|n| n.as_str())
        .unwrap_or("tool_call")
        .to_string()
}

fn cli_stream_status(v: &Value) -> String {
    v.get("status")
        .or_else(|| v.get("subtype"))
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string()
}

fn cli_stream_summary(v: &Value, status: &str) -> String {
    v.get("summary")
        .and_then(|s| s.as_str())
        .or_else(|| v.get("title").and_then(|s| s.as_str()))
        .unwrap_or(status)
        .to_string()
}

fn extract_assistant_text(v: &Value) -> Option<String> {
    if let Some(arr) = v.pointer("/message/content").and_then(|c| c.as_array()) {
        let mut out = String::new();
        for part in arr {
            if part.get("type").and_then(|t| t.as_str()) == Some("text") {
                if let Some(t) = part.get("text").and_then(|t| t.as_str()) {
                    out.push_str(t);
                }
            }
        }
        if !out.is_empty() {
            return Some(out);
        }
    }
    v.get("text")
        .and_then(|t| t.as_str())
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Gateway, DEFAULT_AGENT_ID};
    use serde_json::json;

    fn args(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn atlas_text_argv_is_format_then_single_prompt() {
        let plan = resolve_spawn_plan(None, false, None);
        assert_eq!(plan.dialect, CliDialect::Atlas);
        assert!(!plan.stream);
        assert_eq!(plan.extra_args, args(&["--output-format", "json"]));
        assert_eq!(
            build_cli_argv(plan.dialect, &plan.extra_args, "hello U1"),
            args(&["--output-format", "json", "-p", "hello U1"])
        );
    }

    #[test]
    fn atlas_stream_argv_uses_streaming_json_without_partial_flag() {
        let plan = resolve_spawn_plan(Some("atlas"), true, None);
        assert_eq!(
            plan.extra_args,
            args(&["--output-format", "streaming-json"])
        );
        assert!(!plan
            .extra_args
            .iter()
            .any(|a| a == "--stream-partial-output"));
        assert_eq!(
            build_cli_argv(plan.dialect, &plan.extra_args, "hello U1"),
            args(&["--output-format", "streaming-json", "-p", "hello U1"])
        );
    }

    #[test]
    fn cursor_dialect_keeps_legacy_text_and_stream_json_argv() {
        let text = resolve_spawn_plan(Some("cursor"), false, None);
        assert_eq!(text.dialect, CliDialect::Cursor);
        assert_eq!(text.extra_args, args(&["--output-format", "text"]));
        assert_eq!(
            build_cli_argv(text.dialect, &text.extra_args, "hello U1"),
            args(&["-p", "--output-format", "text", "hello U1"])
        );

        let stream = resolve_spawn_plan(Some("legacy"), true, None);
        assert_eq!(stream.dialect, CliDialect::Cursor);
        assert_eq!(
            stream.extra_args,
            args(&["--output-format", "stream-json", "--stream-partial-output"])
        );
        assert_eq!(
            build_cli_argv(stream.dialect, &stream.extra_args, "hello U1"),
            args(&[
                "-p",
                "--output-format",
                "stream-json",
                "--stream-partial-output",
                "hello U1"
            ])
        );
    }

    #[test]
    fn extra_args_override_keeps_dialect_argv_order() {
        let plain = resolve_spawn_plan(None, false, Some(r#"["--output-format","plain"]"#));
        assert_eq!(plain.extra_args, args(&["--output-format", "plain"]));
        assert_eq!(
            build_cli_argv(plain.dialect, &plain.extra_args, "hi"),
            args(&["--output-format", "plain", "-p", "hi"])
        );

        let cursor = resolve_spawn_plan(
            Some("cursor"),
            true,
            Some(r#"["--output-format","stream-json"]"#),
        );
        assert_eq!(
            build_cli_argv(cursor.dialect, &cursor.extra_args, "hi"),
            args(&["-p", "--output-format", "stream-json", "hi"])
        );
    }

    #[test]
    fn json_reply_uses_text_field_and_surfaces_errors() {
        let body = r#"{"text":"hello U1","stopReason":"end_turn","sessionId":"sid"}"#;
        assert_eq!(extract_text_mode_reply(body, true).unwrap(), "hello U1");

        let err = extract_text_mode_reply(r#"{"type":"error","message":"need login"}"#, true)
            .unwrap_err()
            .to_string();
        assert!(err.contains("need login"), "{err}");

        let empty = extract_text_mode_reply(
            r#"{"text":"","stopReason":"refusal","sessionId":"s"}"#,
            true,
        )
        .unwrap_err()
        .to_string();
        assert!(empty.contains("stopReason=refusal"), "{empty}");

        let missing = extract_text_mode_reply(r#"{"sessionId":"s"}"#, true)
            .unwrap_err()
            .to_string();
        assert!(missing.contains("missing text"), "{missing}");

        // mock-cli ignores --output-format and prints plain text.
        assert_eq!(
            extract_text_mode_reply("atlas-mock-reply agent=agt_1 chars=8", true).unwrap(),
            "atlas-mock-reply agent=agt_1 chars=8"
        );

        // Cursor text / plain mode must not unwrap a JSON-looking reply.
        let raw = r#"{"text":"not a wrapper"}"#;
        assert_eq!(extract_text_mode_reply(raw, false).unwrap(), raw);

        // Text mode + mock NDJSON is not one JSON document; keep the raw reply.
        let ndjson =
            "{\"type\":\"assistant\"}\n{\"type\":\"result\",\"result\":\"atlas-mock-reply\"}\n";
        assert_eq!(
            extract_text_mode_reply(ndjson, true).unwrap(),
            ndjson.trim()
        );
    }

    #[test]
    fn failure_body_prefers_json_message() {
        assert_eq!(
            humanize_cli_failure_body(r#"{"type":"error","message":"Couldn't start session"}"#),
            "Couldn't start session"
        );
        assert_eq!(
            humanize_cli_failure_body("plain stderr-ish"),
            "plain stderr-ish"
        );
    }

    #[test]
    fn streaming_json_text_chunks_and_tool_name() {
        let gw = CliAgentGateway::new(PathBuf::from("agent"), vec![], None);
        let mut deltas = Vec::new();
        let kind =
            gw.classify_stream_line("agt_1", r#"{"type":"text","data":"hello "}"#, &mut deltas);
        assert!(matches!(kind, StreamLineKind::Other));
        let kind = gw.classify_stream_line("agt_1", r#"{"type":"text","data":"U1"}"#, &mut deltas);
        assert!(matches!(kind, StreamLineKind::Other));
        assert_eq!(deltas.join(""), "hello U1");
        let end = gw.classify_stream_line(
            "agt_1",
            r#"{"type":"end","stopReason":"end_turn","sessionId":"sid"}"#,
            &mut deltas,
        );
        assert!(matches!(end, StreamLineKind::Other));
        assert_eq!(deltas.join(""), "hello U1");

        let tool = gw.classify_stream_line(
            "agt_1",
            r#"{"type":"tool_call","toolCallId":"c1","toolName":"run_terminal_cmd","status":"in_progress","title":"Run"}"#,
            &mut deltas,
        );
        match tool {
            StreamLineKind::ToolStarted { tool, status, .. } => {
                assert_eq!(tool, "run_terminal_cmd");
                assert_eq!(status, "in_progress");
            }
            other => panic!("expected started tool, got {other:?}"),
        }

        let cursor = gw.classify_stream_line(
            "agt_1",
            r#"{"type":"result","subtype":"success","result":"atlas-mock-reply"}"#,
            &mut deltas,
        );
        match cursor {
            StreamLineKind::Final(s) => assert_eq!(s, "atlas-mock-reply"),
            other => panic!("expected cursor result, got {other:?}"),
        }
    }

    #[cfg(unix)]
    fn write_executable(path: &std::path::Path, body: &str) {
        std::fs::write(path, body).unwrap();
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(path, perms).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn spawn_atlas_json_argv_parses_text_field() {
        let dir = std::env::temp_dir().join(format!(
            "atlas-cli-dialect-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("fake-agent.sh");
        let args_out = dir.join("args.txt");
        let reply = dir.join("reply.json");
        std::fs::write(
            &reply,
            r#"{"text":"reply-from-text","stopReason":"end_turn","sessionId":"sid"}"#,
        )
        .unwrap();
        write_executable(
            &script,
            &format!(
                "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\ncat '{}'\n",
                args_out.display(),
                reply.display()
            ),
        );
        let gw = CliAgentGateway::new(script, args(&["--output-format", "json"]), None);
        let v = gw
            .invoke(
                DEFAULT_AGENT_ID,
                "sendPrompt",
                json!({"prompt": "hello U1"}),
            )
            .await
            .unwrap();
        assert_eq!(v["preview"], "reply-from-text");
        let recorded = std::fs::read_to_string(&args_out).unwrap();
        assert_eq!(recorded, "--output-format\njson\n-p\nhello U1\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn spawn_atlas_json_error_exit_uses_message() {
        let dir = std::env::temp_dir().join(format!(
            "atlas-cli-dialect-err-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("fake-agent.sh");
        write_executable(
            &script,
            "#!/bin/sh\nprintf '%s\\n' '{\"type\":\"error\",\"message\":\"need login\"}'\nexit 2\n",
        );
        let gw = CliAgentGateway::new(script, args(&["--output-format", "json"]), None);
        let err = gw
            .invoke(
                DEFAULT_AGENT_ID,
                "sendPrompt",
                json!({"prompt": "hello U1"}),
            )
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("non-zero exit 2"), "{err}");
        assert!(err.contains("need login"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn mock_cli_plain_reply_survives_atlas_json_argv() {
        let cli = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tools/mock-cli/mock-atlas-agent-cli.sh");
        assert!(cli.exists(), "mock cli missing at {}", cli.display());
        let gw = CliAgentGateway::new(cli, args(&["--output-format", "json"]), None);
        let v = gw
            .invoke(
                DEFAULT_AGENT_ID,
                "sendPrompt",
                json!({"prompt": "hello U1"}),
            )
            .await
            .unwrap();
        let preview = v["preview"].as_str().unwrap();
        assert!(preview.contains("atlas-mock-reply"), "{preview}");
        assert!(!preview.starts_with("echo:"), "{preview}");
    }
}
