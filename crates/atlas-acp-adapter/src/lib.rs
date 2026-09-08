//! α minimal ACP adapter: public ACP-shaped JSON-RPC subset → Hub `bot.*`.
//!
//! **Frozen whitelist** (see `docs/P6-alpha-runbook.md`):
//! `initialize`, `agents/list`, `session/new`, `session/prompt`,
//! `session/cancel`, `transcript/tail`.
//!
//! Unknown methods → stable JSON-RPC `-32601`. ACP is **not** advertised in
//! Hub `bot.*` capabilities. Does not vendor `xai-acp-lib` / `atlas-relay-demo`.

#![forbid(unsafe_code)]

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use atlas_bot_cli::{HubClient, DEFAULT_AGENT_ID};
use axum::extract::ws::{Message, WebSocket};
use axum::extract::{State, WebSocketUpgrade};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use thiserror::Error;
use tokio::sync::Mutex;
use tower_http::trace::TraceLayer;
use tracing::{info, warn};
use uuid::Uuid;

/// Adapter protocol label returned from `initialize` (α subset, not full ACP).
pub const ACP_ADAPTER_PROTOCOL_VERSION: i64 = 1;
pub const DEFAULT_ACP_BIND: &str = "127.0.0.1:8790";
pub const DEFAULT_HUB_WS: &str = "ws://127.0.0.1:7700/ws";

/// Frozen α ACP method whitelist (names locked for this release).
pub const FROZEN_WHITELIST: &[&str] = &[
    "initialize",
    "agents/list",
    "session/new",
    "session/prompt",
    "session/cancel",
    "transcript/tail",
];

const ERR_METHOD_NOT_FOUND: i64 = -32601;
const ERR_INVALID_PARAMS: i64 = -32602;
const ERR_INTERNAL: i64 = -32603;

#[derive(Debug, Error)]
pub enum AdapterError {
    #[error("{0}")]
    Message(String),
}

pub fn is_whitelisted(method: &str) -> bool {
    FROZEN_WHITELIST.contains(&method)
}

/// Per ACP-client session: one Hub `bot_client` connection (1:1).
pub struct AcpSession {
    hub: HubClient,
    selected_agent: String,
    session_id: Option<String>,
    initialized: bool,
}

impl AcpSession {
    pub async fn connect(hub_ws: &str) -> Result<Self, AdapterError> {
        let hub = HubClient::connect(hub_ws)
            .await
            .map_err(|e| AdapterError::Message(e.to_string()))?;
        hub.assert_no_acp_capabilities()
            .map_err(|e| AdapterError::Message(e.to_string()))?;
        Ok(Self {
            hub,
            selected_agent: DEFAULT_AGENT_ID.to_string(),
            session_id: None,
            initialized: false,
        })
    }

    pub fn hub(&self) -> &HubClient {
        &self.hub
    }

    pub async fn dispatch(&mut self, req: &Value) -> Value {
        let id = req.get("id").cloned().unwrap_or(Value::Null);
        let method = req
            .get("method")
            .and_then(|m| m.as_str())
            .unwrap_or("");
        let params = req.get("params").cloned().unwrap_or(json!({}));

        if method.is_empty() {
            return rpc_error(id, ERR_INVALID_PARAMS, "missing method");
        }
        if !is_whitelisted(method) {
            info!(%method, "[acp→bot] unknown method → -32601");
            return rpc_error(
                id,
                ERR_METHOD_NOT_FOUND,
                &format!("method not in α whitelist: {method}"),
            );
        }

        match method {
            "initialize" => self.handle_initialize(id, &params).await,
            "agents/list" => self.handle_agents_list(id).await,
            "session/new" => self.handle_session_new(id, &params).await,
            "session/prompt" => self.handle_session_prompt(id, &params).await,
            "session/cancel" => self.handle_session_cancel(id, &params).await,
            "transcript/tail" => self.handle_transcript_tail(id, &params).await,
            _ => rpc_error(
                id,
                ERR_METHOD_NOT_FOUND,
                &format!("method not in α whitelist: {method}"),
            ),
        }
    }

    async fn handle_initialize(&mut self, id: Value, _params: &Value) -> Value {
        info!("[acp→bot] initialize → hub hello (already connected)");
        self.initialized = true;
        let caps = self.hub.capabilities();
        rpc_ok(
            id,
            json!({
                "protocolVersion": ACP_ADAPTER_PROTOCOL_VERSION,
                "agentCapabilities": {
                    "session": {
                        "prompt": true,
                        "cancel": true,
                    },
                    // α subset disclaimer — not full ACP
                    "atlasAlphaSubset": true,
                },
                "agentInfo": {
                    "name": "atlas-acp-adapter",
                    "version": env!("CARGO_PKG_VERSION"),
                    "title": "atlas-bot α ACP adapter (subset)",
                },
                "hub": {
                    "connection_id": self.hub.hello_ack().get("connection_id"),
                    "computer_hub_version": self.hub.hello_ack().get("computer_hub_version"),
                    "capabilities": caps,
                },
                "whitelist": FROZEN_WHITELIST,
            }),
        )
    }

    async fn handle_agents_list(&mut self, id: Value) -> Value {
        info!("[acp→bot] agents/list → bot.roster");
        match self.hub.roster().await {
            Ok(roster) => {
                let agents = roster
                    .get("agents")
                    .and_then(|a| a.as_array())
                    .cloned()
                    .unwrap_or_default();
                let mapped: Vec<Value> = agents
                    .iter()
                    .map(|a| {
                        json!({
                            "agentId": a.get("agentId").or_else(|| a.get("id")).cloned().unwrap_or(json!(null)),
                            "name": a.get("name").cloned().unwrap_or(json!("")),
                            "status": a.get("status").cloned().unwrap_or(json!("unknown")),
                        })
                    })
                    .collect();
                // Optional hot listAgents (best-effort; ignore failure)
                let hot = self
                    .hub
                    .list_agents(&self.selected_agent)
                    .await
                    .ok();
                if hot.is_some() {
                    info!("[acp→bot] agents/list → listAgents (hot complement)");
                }
                rpc_ok(
                    id,
                    json!({
                        "agents": mapped,
                        "listAgents": hot,
                        "selectedAgentId": self.selected_agent,
                    }),
                )
            }
            Err(e) => rpc_error(id, ERR_INTERNAL, &e.to_string()),
        }
    }

    async fn handle_session_new(&mut self, id: Value, params: &Value) -> Value {
        let agent_id = params
            .get("agentId")
            .or_else(|| params.get("agent_id"))
            .and_then(|a| a.as_str())
            .unwrap_or(self.selected_agent.as_str())
            .to_string();
        self.selected_agent = agent_id.clone();
        let sid = format!("sess_{}", Uuid::new_v4().simple());
        self.session_id = Some(sid.clone());
        info!(
            %agent_id,
            session_id = %sid,
            "[acp→bot] session/new → select agent (session bound)"
        );
        rpc_ok(
            id,
            json!({
                "sessionId": sid,
                "agentId": agent_id,
            }),
        )
    }

    async fn handle_session_prompt(&mut self, id: Value, params: &Value) -> Value {
        if !self.initialized {
            return rpc_error(id, ERR_INVALID_PARAMS, "call initialize first");
        }
        let session_id = params
            .get("sessionId")
            .and_then(|s| s.as_str())
            .map(|s| s.to_string())
            .or_else(|| self.session_id.clone());
        let Some(session_id) = session_id else {
            return rpc_error(id, ERR_INVALID_PARAMS, "missing sessionId; call session/new");
        };
        if let Some(bound) = &self.session_id {
            if bound != &session_id {
                return rpc_error(id, ERR_INVALID_PARAMS, "unknown sessionId");
            }
        }

        let agent_id = params
            .get("agentId")
            .and_then(|a| a.as_str())
            .unwrap_or(self.selected_agent.as_str())
            .to_string();

        let prompt_text = extract_prompt_text(params);
        if prompt_text.is_empty() {
            return rpc_error(id, ERR_INVALID_PARAMS, "empty prompt");
        }

        info!(
            %agent_id,
            %session_id,
            prompt_len = prompt_text.len(),
            "[acp→bot] session/prompt → subscribe → sendPrompt → turn_finished → transcriptTail"
        );

        match self
            .hub
            .prompt_closed_loop(&agent_id, &prompt_text)
            .await
        {
            Ok((send, turn, tail)) => {
                let stop_reason = "end_turn";
                let preview = turn
                    .get("event")
                    .and_then(|e| e.get("preview"))
                    .cloned()
                    .unwrap_or(Value::Null);
                rpc_ok(
                    id,
                    json!({
                        "sessionId": session_id,
                        "agentId": agent_id,
                        "stopReason": stop_reason,
                        "accepted": send.get("accepted").cloned().unwrap_or(json!(true)),
                        "turn": turn,
                        "transcriptTail": tail,
                        "preview": preview,
                    }),
                )
            }
            Err(e) => {
                warn!(error = %e, "[acp→bot] session/prompt failed");
                rpc_error(id, ERR_INTERNAL, &e.to_string())
            }
        }
    }

    async fn handle_session_cancel(&mut self, id: Value, params: &Value) -> Value {
        let agent_id = params
            .get("agentId")
            .and_then(|a| a.as_str())
            .unwrap_or(self.selected_agent.as_str())
            .to_string();
        info!(%agent_id, "[acp→bot] session/cancel → interruptAgentRun");
        match self.hub.interrupt(&agent_id).await {
            Ok(r) => rpc_ok(
                id,
                json!({
                    "agentId": agent_id,
                    "interrupt": r,
                }),
            ),
            Err(e) => rpc_error(id, ERR_INTERNAL, &e.to_string()),
        }
    }

    async fn handle_transcript_tail(&mut self, id: Value, params: &Value) -> Value {
        let agent_id = params
            .get("agentId")
            .or_else(|| params.get("id"))
            .and_then(|a| a.as_str())
            .unwrap_or(self.selected_agent.as_str())
            .to_string();
        let limit = params
            .get("limit")
            .and_then(|l| l.as_u64())
            .unwrap_or(20) as u32;
        info!(%agent_id, %limit, "[acp→bot] transcript/tail → getAgentTranscriptTail");
        match self.hub.transcript_tail(&agent_id, limit).await {
            Ok(tail) => rpc_ok(id, tail),
            Err(e) => rpc_error(id, ERR_INTERNAL, &e.to_string()),
        }
    }
}

fn extract_prompt_text(params: &Value) -> String {
    if let Some(s) = params.get("prompt").and_then(|p| p.as_str()) {
        return s.to_string();
    }
    if let Some(arr) = params.get("prompt").and_then(|p| p.as_array()) {
        let mut parts = Vec::new();
        for item in arr {
            if let Some(t) = item.get("text").and_then(|t| t.as_str()) {
                parts.push(t.to_string());
            } else if let Some(t) = item.as_str() {
                parts.push(t.to_string());
            }
        }
        return parts.join("\n");
    }
    if let Some(s) = params.get("text").and_then(|t| t.as_str()) {
        return s.to_string();
    }
    String::new()
}

fn rpc_ok(id: Value, result: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result,
    })
}

fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": code,
            "message": message,
        },
    })
}

#[derive(Clone)]
pub struct AdapterState {
    pub hub_ws: String,
}

/// Serve ACP-over-WS on `bind`. Path `/` and `/ws` both accept upgrades.
pub async fn serve(bind: SocketAddr, hub_ws: String) -> Result<(), AdapterError> {
    let app = router(hub_ws);
    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .map_err(|e| AdapterError::Message(format!("bind {bind}: {e}")))?;
    info!(%bind, "atlas-acp-adapter listening (ACP JSON-RPC over WS)");
    axum::serve(listener, app)
        .await
        .map_err(|e| AdapterError::Message(e.to_string()))
}

pub fn router(hub_ws: String) -> Router {
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/", get(ws_upgrade))
        .route("/ws", get(ws_upgrade))
        .layer(TraceLayer::new_for_http())
        .with_state(AdapterState { hub_ws })
}

async fn ws_upgrade(ws: WebSocketUpgrade, State(st): State<AdapterState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_acp_socket(socket, st.hub_ws))
}

async fn handle_acp_socket(socket: WebSocket, hub_ws: String) {
    let (mut sink, mut stream) = socket.split();
    let session = match AcpSession::connect(&hub_ws).await {
        Ok(s) => Arc::new(Mutex::new(s)),
        Err(e) => {
            warn!("hub connect failed: {e}");
            let err = rpc_error(Value::Null, ERR_INTERNAL, &format!("hub connect failed: {e}"));
            let _ = sink
                .send(Message::Text(err.to_string().into()))
                .await;
            return;
        }
    };

    while let Some(msg) = stream.next().await {
        let Ok(msg) = msg else { break };
        match msg {
            Message::Text(t) => {
                let Ok(v) = serde_json::from_str::<Value>(&t) else {
                    let err = rpc_error(Value::Null, ERR_INVALID_PARAMS, "invalid JSON");
                    if sink
                        .send(Message::Text(err.to_string().into()))
                        .await
                        .is_err()
                    {
                        break;
                    }
                    continue;
                };
                // Notifications (no id) — ignore quietly for α
                if v.get("id").is_none() && v.get("method").is_some() {
                    info!(
                        method = v.get("method").and_then(|m| m.as_str()).unwrap_or("?"),
                        "[acp→bot] notification ignored (α)"
                    );
                    continue;
                }
                let mut sess = session.lock().await;
                let resp = sess.dispatch(&v).await;
                drop(sess);
                if sink
                    .send(Message::Text(resp.to_string().into()))
                    .await
                    .is_err()
                {
                    break;
                }
            }
            Message::Ping(p) => {
                if sink.send(Message::Pong(p)).await.is_err() {
                    break;
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }
}

/// Helper for tests: bind ephemeral listener and spawn serve.
pub async fn spawn_adapter(hub_ws: String) -> Result<SocketAddr, AdapterError> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| AdapterError::Message(e.to_string()))?;
    let addr = listener
        .local_addr()
        .map_err(|e| AdapterError::Message(e.to_string()))?;
    let app = router(hub_ws);
    tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });
    tokio::task::yield_now().await;
    // Brief settle for accept loop
    tokio::time::sleep(Duration::from_millis(20)).await;
    Ok(addr)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whitelist_covers_required_semantics() {
        for m in [
            "initialize",
            "agents/list",
            "session/new",
            "session/prompt",
            "session/cancel",
            "transcript/tail",
        ] {
            assert!(is_whitelisted(m), "{m}");
        }
        assert!(!is_whitelisted("fs/read_text_file"));
        assert!(!is_whitelisted("bot.command"));
        assert!(!is_whitelisted("session/load"));
    }

    #[test]
    fn extract_prompt_string_and_blocks() {
        assert_eq!(
            extract_prompt_text(&json!({"prompt": "hi"})),
            "hi"
        );
        assert_eq!(
            extract_prompt_text(&json!({"prompt": [{"type":"text","text":"a"},{"type":"text","text":"b"}]})),
            "a\nb"
        );
    }
}
