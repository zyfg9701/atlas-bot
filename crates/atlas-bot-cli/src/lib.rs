//! Thin Bot-Relay `bot_client` for Computer Hub WebSocket (P6 β′).
//!
//! **Bot-Relay ≠ ACP.** This crate speaks Hub `bot.*` JSON-RPC over WS.
//! It does not implement ACP, does not vendor `xai-acp-lib` / `atlas-relay-demo`,
//! and does not advertise ACP method names.
//!
//! Cold paths (`bot.status`, `bot.roster`) never go through `bot.command`.
//! Hot paths (`sendPrompt`, `listAgents`, transcript tail, interrupt) use
//! `bot.command` as passthrough names.

#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use thiserror::Error;
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio::time::timeout;
use tokio_tungstenite::{connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream};
use url::Url;

pub const PROTOCOL_VERSION: &str = "1.0.0";
pub const DEFAULT_HUB_WS: &str = "ws://127.0.0.1:7700/ws";
pub const DEFAULT_AGENT_ID: &str = "agt_1";

/// Known Bot-Relay wire error message tokens (PC client parity).
const KNOWN_CODES: &[&str] = &[
    "command_rejected",
    "identity_unavailable",
    "link_state_unavailable",
    "upstream_error",
    "forbidden",
    "unauthorized",
    "link_required",
];

#[derive(Debug, Error)]
pub enum CliError {
    #[error("{0}")]
    Message(String),
    #[error("rpc: {message}")]
    Rpc {
        message: String,
        code: Option<String>,
        reason: Option<String>,
        raw: Value,
    },
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Url(#[from] url::ParseError),
    #[error("websocket: {0}")]
    Ws(String),
    #[error("timeout waiting for {0}")]
    Timeout(String),
}

impl CliError {
    pub fn display_line(&self) -> String {
        match self {
            CliError::Rpc {
                message,
                code,
                reason,
                ..
            } => {
                let mut s = format!("error: {message}");
                if let Some(c) = code {
                    s.push_str(&format!(" code={c}"));
                }
                if let Some(r) = reason {
                    s.push_str(&format!(" reason={r}"));
                }
                s
            }
            other => format!("error: {other}"),
        }
    }
}

/// Normalize JSON-RPC / Bot-Relay errors. Unknown wire codes degrade to
/// `upstream_error` display without inventing protocol-external codes.
pub fn normalize_error(data: &Value) -> CliError {
    let message = data
        .get("message")
        .and_then(|m| m.as_str())
        .unwrap_or("unknown failure")
        .to_string();
    let payload = data.get("data").unwrap_or(data);
    let reason = payload
        .get("reason")
        .and_then(|r| r.as_str())
        .map(|s| s.to_string());
    if KNOWN_CODES.contains(&message.as_str()) {
        return CliError::Rpc {
            message: message.clone(),
            code: Some(message),
            reason,
            raw: data.clone(),
        };
    }
    // Numeric JSON-RPC errors keep their message; degrade unknown string codes.
    if data.get("code").and_then(|c| c.as_i64()).is_some() && !KNOWN_CODES.contains(&message.as_str())
    {
        // If message looks like a free-form RPC text, keep it but label upstream.
        if message.chars().any(|c| c == ' ' || c == '/') {
            return CliError::Rpc {
                message,
                code: Some("upstream_error".into()),
                reason,
                raw: data.clone(),
            };
        }
        return CliError::Rpc {
            message: format!("failure ({message})"),
            code: Some("upstream_error".into()),
            reason,
            raw: data.clone(),
        };
    }
    CliError::Rpc {
        message: if KNOWN_CODES.contains(&message.as_str()) {
            message
        } else if message == "unknown failure" {
            message
        } else {
            format!("failure ({message})")
        },
        code: Some("upstream_error".into()),
        reason,
        raw: data.clone(),
    }
}

type PendingMap = Arc<Mutex<HashMap<i64, oneshot::Sender<Result<Value, CliError>>>>>;

/// WebSocket Bot-Relay client (`kind=bot_client`).
pub struct HubClient {
    url: String,
    next_id: AtomicI64,
    write_tx: mpsc::UnboundedSender<Message>,
    pending: PendingMap,
    events: Arc<Mutex<mpsc::UnboundedReceiver<Value>>>,
    hello_ack: Value,
    reader_task: tokio::task::JoinHandle<()>,
    writer_task: tokio::task::JoinHandle<()>,
}

impl HubClient {
    /// Connect without Authorization (Hub `dev` / no-auth smoke).
    pub async fn connect(url: &str) -> Result<Self, CliError> {
        Self::connect_with_bearer(url, None).await
    }

    /// Connect with optional `Authorization: Bearer` on the WS upgrade.
    pub async fn connect_with_bearer(
        url: &str,
        bearer: Option<&str>,
    ) -> Result<Self, CliError> {
        let _ = Url::parse(url)?;
        let ws = if let Some(token) = bearer {
            let uri: http::Uri = url
                .parse()
                .map_err(|e: http::uri::InvalidUri| CliError::Message(e.to_string()))?;
            let host = uri
                .authority()
                .map(|a| a.as_str().to_string())
                .unwrap_or_else(|| "127.0.0.1".into());
            let req = http::Request::builder()
                .uri(uri)
                .header("authorization", format!("Bearer {token}"))
                .header("host", host)
                .header("connection", "Upgrade")
                .header("upgrade", "websocket")
                .header("sec-websocket-version", "13")
                .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
                .body(())
                .map_err(|e| CliError::Message(e.to_string()))?;
            let (ws, _) = connect_async(req)
                .await
                .map_err(|e| CliError::Ws(e.to_string()))?;
            ws
        } else {
            let (ws, _) = connect_async(url.to_string())
                .await
                .map_err(|e| CliError::Ws(e.to_string()))?;
            ws
        };
        Self::from_ws(url.to_string(), ws).await
    }

    async fn from_ws(
        url: String,
        ws: WebSocketStream<MaybeTlsStream<TcpStream>>,
    ) -> Result<Self, CliError> {
        let (mut sink, mut stream) = ws.split();
        let (write_tx, mut write_rx) = mpsc::unbounded_channel::<Message>();
        let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));
        let (event_tx, event_rx) = mpsc::unbounded_channel::<Value>();
        let (hello_tx, hello_rx) = oneshot::channel::<Result<Value, CliError>>();
        let hello_tx = Arc::new(Mutex::new(Some(hello_tx)));

        let writer_task = tokio::spawn(async move {
            while let Some(msg) = write_rx.recv().await {
                if sink.send(msg).await.is_err() {
                    break;
                }
            }
        });

        let pending_r = pending.clone();
        let hello_slot = hello_tx.clone();
        let reader_task = tokio::spawn(async move {
            while let Some(msg) = stream.next().await {
                let Ok(msg) = msg else { break };
                let text = match msg {
                    Message::Text(t) => t.to_string(),
                    Message::Ping(_) | Message::Pong(_) => continue,
                    Message::Close(_) => break,
                    _ => continue,
                };
                let Ok(v) = serde_json::from_str::<Value>(&text) else {
                    continue;
                };
                // hello_ack (no jsonrpc) — success has connection_id;
                // auth failure is a bare JsonRpcError (message=unauthorized).
                if v.get("jsonrpc").is_none() {
                    if v.get("connection_id").is_some() {
                        if let Some(tx) = hello_slot.lock().await.take() {
                            let _ = tx.send(Ok(v));
                        }
                        continue;
                    }
                    if v.get("message").and_then(|m| m.as_str()) == Some("unauthorized")
                        || v.get("code").and_then(|c| c.as_i64()) == Some(-32002)
                        || v.get("message").and_then(|m| m.as_str()) == Some("link_required")
                    {
                        if let Some(tx) = hello_slot.lock().await.take() {
                            let _ = tx.send(Err(normalize_error(&v)));
                        }
                        continue;
                    }
                }
                if v.get("jsonrpc") == Some(&json!("2.0")) && v.get("id").is_some() {
                    let id = match &v["id"] {
                        Value::Number(n) => n.as_i64().unwrap_or(-1),
                        Value::String(s) => s.parse().unwrap_or(-1),
                        _ => -1,
                    };
                    let result = if let Some(err) = v.get("error") {
                        Err(normalize_error(err))
                    } else {
                        Ok(v.get("result").cloned().unwrap_or(Value::Null))
                    };
                    if let Some(tx) = pending_r.lock().await.remove(&id) {
                        let _ = tx.send(result);
                    }
                    continue;
                }
                if v.get("jsonrpc") == Some(&json!("2.0"))
                    && v.get("method").and_then(|m| m.as_str()) == Some("bot.event")
                {
                    if let Some(params) = v.get("params") {
                        let _ = event_tx.send(params.clone());
                    }
                }
            }
            // Fail pending on disconnect
            let mut map = pending_r.lock().await;
            for (_, tx) in map.drain() {
                let _ = tx.send(Err(CliError::Message("connection closed".into())));
            }
            if let Some(tx) = hello_slot.lock().await.take() {
                let _ = tx.send(Err(CliError::Message(
                    "connection closed before hello_ack".into(),
                )));
            }
        });

        // Send hello
        let hello = json!({
            "protocol_version": PROTOCOL_VERSION,
            "kind": "bot_client",
        });
        write_tx
            .send(Message::Text(hello.to_string().into()))
            .map_err(|_| CliError::Message("failed to send hello".into()))?;

        let hello_ack = timeout(Duration::from_secs(10), hello_rx)
            .await
            .map_err(|_| CliError::Timeout("hello_ack".into()))?
            .map_err(|_| CliError::Message("hello channel dropped".into()))??;

        Ok(Self {
            url,
            next_id: AtomicI64::new(1),
            write_tx,
            pending,
            events: Arc::new(Mutex::new(event_rx)),
            hello_ack,
            reader_task,
            writer_task,
        })
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn hello_ack(&self) -> &Value {
        &self.hello_ack
    }

    pub fn capabilities(&self) -> Vec<String> {
        self.hello_ack
            .get("capabilities")
            .and_then(|c| c.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Assert hello_ack capabilities contain no ACP pollution.
    pub fn assert_no_acp_capabilities(&self) -> Result<(), CliError> {
        for c in self.capabilities() {
            let lower = c.to_ascii_lowercase();
            if lower.contains("acp") || lower.starts_with("agent/") || c.contains("session/new") {
                return Err(CliError::Message(format!(
                    "ACP pollution in capabilities: {c}"
                )));
            }
        }
        Ok(())
    }

    async fn rpc(&self, method: &str, params: Value) -> Result<Value, CliError> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let frame = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);
        self.write_tx
            .send(Message::Text(frame.to_string().into()))
            .map_err(|_| CliError::Message("ws write failed".into()))?;
        timeout(Duration::from_secs(60), rx)
            .await
            .map_err(|_| CliError::Timeout(method.into()))?
            .map_err(|_| CliError::Message("rpc channel dropped".into()))?
    }

    /// Cold — must NOT use `bot.command`.
    pub async fn status(&self) -> Result<Value, CliError> {
        self.rpc("bot.status", json!({})).await
    }

    /// Cold — must NOT use `bot.command`.
    pub async fn roster(&self) -> Result<Value, CliError> {
        self.rpc("bot.roster", json!({})).await
    }

    pub async fn subscribe(&self, agent_ids: &[&str]) -> Result<Value, CliError> {
        self.rpc(
            "bot.subscribe",
            json!({ "agentIds": agent_ids }),
        )
        .await
    }

    pub async fn command(
        &self,
        agent_id: &str,
        name: &str,
        args: Value,
    ) -> Result<Value, CliError> {
        self.rpc(
            "bot.command",
            json!({
                "agentId": agent_id,
                "name": name,
                "args": args,
            }),
        )
        .await
    }

    /// Hot list via `listAgents` (optional complement to cold roster).
    pub async fn list_agents(&self, routing_agent_id: &str) -> Result<Value, CliError> {
        self.command(routing_agent_id, "listAgents", json!({})).await
    }

    pub async fn send_prompt(
        &self,
        agent_id: &str,
        prompt: &str,
        immediate: bool,
    ) -> Result<Value, CliError> {
        let mut args = json!({ "agentId": agent_id, "prompt": prompt });
        if immediate {
            args["immediate"] = json!(true);
        }
        self.command(agent_id, "sendPrompt", args).await
    }

    pub async fn transcript_tail(
        &self,
        agent_id: &str,
        limit: u32,
    ) -> Result<Value, CliError> {
        self.command(
            agent_id,
            "getAgentTranscriptTail",
            json!({ "id": agent_id, "limit": limit }),
        )
        .await
    }

    pub async fn interrupt(&self, agent_id: &str) -> Result<Value, CliError> {
        self.command(
            agent_id,
            "interruptAgentRun",
            json!({ "agentId": agent_id }),
        )
        .await
    }

    /// Wait for next `hub:turn_finished` event for `agent_id` (or any if None).
    pub async fn wait_turn_finished(
        &self,
        agent_id: Option<&str>,
        wait: Duration,
    ) -> Result<Value, CliError> {
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(CliError::Timeout("hub:turn_finished".into()));
            }
            let mut rx = self.events.lock().await;
            match timeout(remaining, rx.recv()).await {
                Ok(Some(params)) => {
                    let channel = params
                        .get("channel")
                        .and_then(|c| c.as_str())
                        .unwrap_or("");
                    if channel != "hub:turn_finished" {
                        continue;
                    }
                    if let Some(want) = agent_id {
                        let got = params
                            .get("agentId")
                            .and_then(|a| a.as_str())
                            .or_else(|| {
                                params
                                    .get("event")
                                    .and_then(|e| e.get("agentId"))
                                    .and_then(|a| a.as_str())
                            })
                            .unwrap_or("");
                        if got != want {
                            continue;
                        }
                    }
                    return Ok(params);
                }
                Ok(None) => return Err(CliError::Message("event channel closed".into())),
                Err(_) => return Err(CliError::Timeout("hub:turn_finished".into())),
            }
        }
    }

    /// Full chat path: subscribe → sendPrompt → turn_finished → transcriptTail.
    pub async fn prompt_closed_loop(
        &self,
        agent_id: &str,
        prompt: &str,
    ) -> Result<(Value, Value, Value), CliError> {
        let _ = self.subscribe(&[agent_id]).await?;
        let send = self.send_prompt(agent_id, prompt, false).await?;
        let turn = self
            .wait_turn_finished(Some(agent_id), Duration::from_secs(30))
            .await?;
        let tail = self.transcript_tail(agent_id, 20).await?;
        Ok((send, turn, tail))
    }
}

impl Drop for HubClient {
    fn drop(&mut self) {
        self.reader_task.abort();
        self.writer_task.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_unknown_code_degrades() {
        let err = normalize_error(&json!({
            "code": -32000,
            "message": "totally_made_up_wire_code",
            "data": { "reason": "nope" }
        }));
        match err {
            CliError::Rpc { code, reason, .. } => {
                assert_eq!(code.as_deref(), Some("upstream_error"));
                assert_eq!(reason.as_deref(), Some("nope"));
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn normalize_known_code_kept() {
        let err = normalize_error(&json!({
            "message": "command_rejected",
            "data": { "reason": "args_invalid", "retryable": false }
        }));
        match err {
            CliError::Rpc { code, reason, .. } => {
                assert_eq!(code.as_deref(), Some("command_rejected"));
                assert_eq!(reason.as_deref(), Some("args_invalid"));
            }
            other => panic!("unexpected {other:?}"),
        }
    }
}
