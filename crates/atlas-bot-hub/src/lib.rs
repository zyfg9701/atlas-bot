//! Computer Hub Bot-Relay MVP (P1).
//!
//! Cold path (`bot.status`, `bot.roster`) is answered from in-memory hub
//! state and **never** invokes the gateway. Hot path (`bot.command`)
//! passthroughs `name`/`args` verbatim via [`atlas_bot_gateway::Gateway`].

#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use atlas_bot_gateway::{
    Gateway, GatewayError, InMemoryGateway, DEFAULT_AGENT_ID, DEFAULT_AGENT_NAME,
};
use serde_json::{json, Value};
use tokio::sync::{broadcast, RwLock};
use tracing::{debug, info, warn};
use uuid::Uuid;
use xai_tool_protocol::{
    BotCommandParams, BotEmptyResult, BotEventEnvelope, BotRelayError, BotRelayErrorCode,
    BotRelayErrorDetail, BotRosterEntry, BotRosterResult, BotRunState, BotStatusResult,
    BotSubscribeParams, ConnectionId, ConnectionKind, HelloAckMsg, HelloMsg, HubChannel,
    HubTurnFinishedEvent, JsonRpcError, JsonRpcId, JsonRpcNotification, JsonRpcRequest,
    JsonRpcResponse, JsonRpcVersion, Method, ResponseOutcome, UserId, BOT_RELAY_CAPABILITIES,
    COMMAND_REJECTED_AGENT_ID_MISMATCH, COMMAND_REJECTED_ARGS_INVALID,
    COMMAND_REJECTED_GATEWAY_UNKNOWN_METHOD, COMMAND_REJECTED_NOT_YET_ENABLED, PROTOCOL_VERSION,
};

pub const HUB_VERSION: &str = "0.1.0-p1";

/// Per-connection subscription + seq state.
#[derive(Debug, Default)]
struct ConnSubs {
    /// agent_id -> next seq (starts at 1 after subscribe)
    seqs: HashMap<String, u64>,
    full_fidelity: bool,
}

/// Shared hub state.
pub struct Hub {
    gateway: Arc<dyn Gateway>,
    run_state: RwLock<BotRunState>,
    roster: RwLock<Vec<BotRosterEntry>>,
    /// connection_id -> subs
    subs: RwLock<HashMap<String, ConnSubs>>,
    /// Fan-out of serialized `bot.event` notifications keyed by connection.
    event_bus: broadcast::Sender<(String /*conn*/, String /*json*/)>,
    /// Counts cold-path method hits (for tests / metrics).
    cold_hits: AtomicU64,
    /// Counts hot-path method hits.
    hot_hits: AtomicU64,
}

impl Hub {
    pub fn new(gateway: Arc<dyn Gateway>) -> Arc<Self> {
        let (event_bus, _) = broadcast::channel(256);
        Arc::new(Self {
            gateway,
            run_state: RwLock::new(BotRunState::Hibernated),
            roster: RwLock::new(vec![BotRosterEntry {
                agent_id: DEFAULT_AGENT_ID.to_string(),
                name: DEFAULT_AGENT_NAME.to_string(),
                status: "unknown".to_string(),
                last_turn_at: None,
            }]),
            subs: RwLock::new(HashMap::new()),
            event_bus,
            cold_hits: AtomicU64::new(0),
            hot_hits: AtomicU64::new(0),
        })
    }

    pub fn with_in_memory_gateway() -> (Arc<Self>, Arc<InMemoryGateway>) {
        let gw = Arc::new(InMemoryGateway::new());
        let hub = Self::new(gw.clone());
        (hub, gw)
    }

    pub fn cold_hits(&self) -> u64 {
        self.cold_hits.load(Ordering::SeqCst)
    }

    pub fn hot_hits(&self) -> u64 {
        self.hot_hits.load(Ordering::SeqCst)
    }

    pub fn gateway_invoke_count(&self) -> u64 {
        self.gateway.invoke_count()
    }

    pub fn subscribe_outbound(&self) -> broadcast::Receiver<(String, String)> {
        self.event_bus.subscribe()
    }

    /// Spawn a task that converts gateway turn hints into `hub:turn_finished` events.
    pub fn spawn_turn_bridge(self: &Arc<Self>, mut rx: broadcast::Receiver<atlas_bot_gateway::TurnFinishedHint>) {
        let hub = Arc::clone(self);
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(hint) => {
                        hub.emit_turn_finished(&hint.agent_id, &hint.preview).await;
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }

    pub async fn emit_turn_finished(&self, agent_id: &str, preview: &str) {
        let body = HubTurnFinishedEvent {
            agent_id: agent_id.to_string(),
            conversation_ids: vec![],
            preview: preview.to_string(),
        };
        let event = serde_json::to_value(&body).unwrap_or(json!({}));
        self.fanout_event(agent_id, HubChannel::TurnFinished.into(), event)
            .await;
        // Update cold roster last_turn_at without waking gateway.
        let mut roster = self.roster.write().await;
        if let Some(row) = roster.iter_mut().find(|r| r.agent_id == agent_id) {
            row.status = "idle".to_string();
            row.last_turn_at = Some(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0),
            );
        }
    }

    /// Inject `hub:resync_required` for tests / ops.
    pub async fn emit_resync_required(&self, agent_id: &str) {
        let event = json!({ "agentId": agent_id });
        self.fanout_event(agent_id, HubChannel::ResyncRequired.into(), event)
            .await;
    }

    async fn fanout_event(
        &self,
        agent_id: &str,
        channel: xai_tool_protocol::BotEventChannel,
        event: Value,
    ) {
        let mut subs = self.subs.write().await;
        for (conn_id, cs) in subs.iter_mut() {
            if !cs.seqs.contains_key(agent_id) {
                continue;
            }
            let seq = {
                let s = cs.seqs.get_mut(agent_id).unwrap();
                let cur = *s;
                *s += 1;
                cur
            };
            let envelope = BotEventEnvelope::new(agent_id, seq, channel.clone(), event.clone());
            let params = serde_json::to_value(&envelope).unwrap_or(json!({}));
            let note = JsonRpcNotification {
                jsonrpc: JsonRpcVersion,
                session_id: None,
                seq: None,
                method: Method::BotEvent.as_wire_str().to_string(),
                params,
            };
            if let Ok(text) = serde_json::to_string(&note) {
                let _ = self.event_bus.send((conn_id.clone(), text));
            }
        }
    }

    pub fn hello_ack(&self, hello: &HelloMsg) -> Result<HelloAckMsg, BotRelayError> {
        if hello.kind != ConnectionKind::BotClient {
            return Err(BotRelayError {
                code: BotRelayErrorCode::UpstreamError,
                retryable: false,
                detail: BotRelayErrorDetail {
                    upstream: Some(format!("unsupported kind {:?}", hello.kind)),
                },
                reason: None,
            });
        }
        let connection_id =
            ConnectionId::new(format!("conn_{}", Uuid::new_v4())).expect("valid id");
        let user_id = UserId::new("user_local_dev").expect("valid id");
        Ok(HelloAckMsg {
            connection_id,
            user_id,
            computer_hub_version: HUB_VERSION.to_string(),
            supported_protocol_versions: vec![PROTOCOL_VERSION.to_string()],
            capabilities: BOT_RELAY_CAPABILITIES
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
        })
    }

    pub async fn register_connection(&self, connection_id: &str) {
        let mut subs = self.subs.write().await;
        subs.entry(connection_id.to_string())
            .or_insert_with(ConnSubs::default);
    }

    pub async fn unregister_connection(&self, connection_id: &str) {
        let mut subs = self.subs.write().await;
        subs.remove(connection_id);
    }

    /// Handle a JSON-RPC request for an established bot_client connection.
    pub async fn handle_rpc(
        &self,
        connection_id: &str,
        req: JsonRpcRequest<Value>,
    ) -> JsonRpcResponse<Value> {
        let id = req.id.clone();
        match Method::from_wire_str(&req.method) {
            Some(Method::BotStatus) => {
                self.cold_hits.fetch_add(1, Ordering::SeqCst);
                debug!(%connection_id, "cold bot.status");
                let run_state = *self.run_state.read().await;
                let result = BotStatusResult { run_state };
                JsonRpcResponse::ok(id, serde_json::to_value(result).unwrap())
            }
            Some(Method::BotRoster) => {
                self.cold_hits.fetch_add(1, Ordering::SeqCst);
                debug!(%connection_id, "cold bot.roster");
                let agents = self.roster.read().await.clone();
                let result = BotRosterResult { agents };
                JsonRpcResponse::ok(id, serde_json::to_value(result).unwrap())
            }
            Some(Method::BotSubscribe) => {
                match serde_json::from_value::<BotSubscribeParams>(req.params) {
                    Ok(p) => {
                        let mut subs = self.subs.write().await;
                        let cs = subs.entry(connection_id.to_string()).or_default();
                        cs.full_fidelity = p.full_fidelity;
                        for a in p.agent_ids {
                            cs.seqs.entry(a).or_insert(1);
                        }
                        JsonRpcResponse::ok(id, serde_json::to_value(BotEmptyResult {}).unwrap())
                    }
                    Err(e) => JsonRpcResponse::err(id, reject_args(e.to_string())),
                }
            }
            Some(Method::BotUnsubscribe) => {
                match serde_json::from_value::<BotSubscribeParams>(req.params) {
                    Ok(p) => {
                        let mut subs = self.subs.write().await;
                        if let Some(cs) = subs.get_mut(connection_id) {
                            for a in p.agent_ids {
                                cs.seqs.remove(&a);
                            }
                        }
                        JsonRpcResponse::ok(id, serde_json::to_value(BotEmptyResult {}).unwrap())
                    }
                    Err(e) => JsonRpcResponse::err(id, reject_args(e.to_string())),
                }
            }
            Some(Method::BotCommand) => {
                self.hot_hits.fetch_add(1, Ordering::SeqCst);
                match serde_json::from_value::<BotCommandParams>(req.params.clone()) {
                    Ok(params) => self.handle_command(id, params).await,
                    Err(e) => JsonRpcResponse::err(id, reject_args(e.to_string())),
                }
            }
            Some(Method::BotVncDescriptor)
            | Some(Method::BotTranscriptOffbox)
            | Some(Method::BotBindConversation) => {
                JsonRpcResponse::err(id, not_yet_enabled(&req.method))
            }
            Some(Method::Hello) => {
                // Allow hello as JSON-RPC too.
                match serde_json::from_value::<HelloMsg>(req.params) {
                    Ok(hello) => match self.hello_ack(&hello) {
                        Ok(ack) => {
                            self.register_connection(ack.connection_id.as_str()).await;
                            JsonRpcResponse::ok(id, serde_json::to_value(ack).unwrap())
                        }
                        Err(e) => JsonRpcResponse::err(id, JsonRpcError::from(e)),
                    },
                    Err(e) => JsonRpcResponse::err(
                        id,
                        JsonRpcError {
                            code: -32602,
                            message: "invalid_params".into(),
                            data: Some(json!({ "detail": e.to_string() })),
                        },
                    ),
                }
            }
            Some(other) => JsonRpcResponse::err(
                id,
                JsonRpcError {
                    code: -32601,
                    message: format!("{}{}`", xai_tool_protocol::UNKNOWN_METHOD_MSG_PREFIX, other),
                    data: None,
                },
            ),
            None => {
                // Unknown wire method → upstream_error style for bot clients
                let err = BotRelayError {
                    code: BotRelayErrorCode::UpstreamError,
                    retryable: false,
                    detail: BotRelayErrorDetail {
                        upstream: Some(format!("unknown method {}", req.method)),
                    },
                    reason: None,
                };
                JsonRpcResponse::err(id, JsonRpcError::from(err))
            }
        }
    }

    async fn handle_command(
        &self,
        id: JsonRpcId,
        params: BotCommandParams,
    ) -> JsonRpcResponse<Value> {
        // agentId mismatch: envelope vs args.agentId when present
        if let Some(args_aid) = params.args.get("agentId").and_then(|v| v.as_str()) {
            if args_aid != params.agent_id {
                let err = BotRelayError {
                    code: BotRelayErrorCode::CommandRejected,
                    retryable: false,
                    detail: BotRelayErrorDetail::default(),
                    reason: Some(COMMAND_REJECTED_AGENT_ID_MISMATCH.to_string()),
                };
                return JsonRpcResponse::err(id, JsonRpcError::from(err));
            }
        }

        info!(
            agent_id = %params.agent_id,
            name = %params.name,
            "hot bot.command -> gateway"
        );

        match self
            .gateway
            .invoke(&params.agent_id, &params.name, params.args)
            .await
        {
            Ok(result) => JsonRpcResponse::ok(id, result),
            Err(GatewayError::UnknownMethod(_)) => {
                let err = BotRelayError {
                    code: BotRelayErrorCode::CommandRejected,
                    retryable: false,
                    detail: BotRelayErrorDetail::default(),
                    reason: Some(COMMAND_REJECTED_GATEWAY_UNKNOWN_METHOD.to_string()),
                };
                JsonRpcResponse::err(id, JsonRpcError::from(err))
            }
            Err(GatewayError::InvalidArgs(msg)) => {
                let err = BotRelayError {
                    code: BotRelayErrorCode::CommandRejected,
                    retryable: false,
                    detail: BotRelayErrorDetail {
                        upstream: Some(msg),
                    },
                    reason: Some(COMMAND_REJECTED_ARGS_INVALID.to_string()),
                };
                JsonRpcResponse::err(id, JsonRpcError::from(err))
            }
            Err(e) => {
                let err = BotRelayError {
                    code: BotRelayErrorCode::UpstreamError,
                    retryable: true,
                    detail: BotRelayErrorDetail {
                        upstream: Some(e.to_string()),
                    },
                    reason: None,
                };
                JsonRpcResponse::err(id, JsonRpcError::from(err))
            }
        }
    }
}

fn reject_args(msg: String) -> JsonRpcError {
    JsonRpcError::from(BotRelayError {
        code: BotRelayErrorCode::CommandRejected,
        retryable: false,
        detail: BotRelayErrorDetail {
            upstream: Some(msg),
        },
        reason: Some(COMMAND_REJECTED_ARGS_INVALID.to_string()),
    })
}

fn not_yet_enabled(method: &str) -> JsonRpcError {
    JsonRpcError::from(BotRelayError {
        code: BotRelayErrorCode::CommandRejected,
        retryable: false,
        detail: BotRelayErrorDetail {
            upstream: Some(method.to_string()),
        },
        reason: Some(COMMAND_REJECTED_NOT_YET_ENABLED.to_string()),
    })
}

/// Decode unknown BotRelayErrorCode wire strings → upstream_error.
pub fn normalize_error_code(wire: &str) -> BotRelayErrorCode {
    BotRelayErrorCode::from_wire(wire)
}

/// Session driver used by the WebSocket binary and integration tests.
pub struct Session {
    pub connection_id: String,
    pub hub: Arc<Hub>,
    hello_done: bool,
}

impl Session {
    pub fn new(hub: Arc<Hub>) -> Self {
        Self {
            connection_id: String::new(),
            hub,
            hello_done: false,
        }
    }

    /// Process one inbound text frame; returns zero or more outbound texts.
    pub async fn on_text(&mut self, text: &str) -> Vec<String> {
        let v: Value = match serde_json::from_str(text) {
            Ok(v) => v,
            Err(e) => {
                warn!("parse error: {e}");
                return vec![];
            }
        };

        // Raw hello (no jsonrpc) — first frame style.
        if !self.hello_done && v.get("jsonrpc").is_none() && v.get("protocol_version").is_some() {
            match serde_json::from_value::<HelloMsg>(v) {
                Ok(hello) => match self.hub.hello_ack(&hello) {
                    Ok(ack) => {
                        self.connection_id = ack.connection_id.as_str().to_string();
                        self.hub.register_connection(&self.connection_id).await;
                        self.hello_done = true;
                        return vec![serde_json::to_string(&ack).unwrap()];
                    }
                    Err(e) => {
                        return vec![serde_json::to_string(&JsonRpcError::from(e)).unwrap()];
                    }
                },
                Err(e) => {
                    warn!("bad hello: {e}");
                    return vec![];
                }
            }
        }

        // JSON-RPC request (has id)
        if v.get("jsonrpc").is_some() && v.get("id").is_some() && v.get("method").is_some() {
            match serde_json::from_value::<JsonRpcRequest<Value>>(v) {
                Ok(req) => {
                    // Allow hello via JSON-RPC before hello_done
                    if !self.hello_done && req.method == Method::Hello.as_wire_str() {
                        let resp = self.hub.handle_rpc("_pending", req).await;
                        if let ResponseOutcome::Result(ref r) = resp.outcome {
                            if let Ok(ack) = serde_json::from_value::<HelloAckMsg>(r.clone()) {
                                self.connection_id = ack.connection_id.as_str().to_string();
                                self.hello_done = true;
                            }
                        }
                        return vec![serde_json::to_string(&resp).unwrap()];
                    }
                    if !self.hello_done {
                        let err = BotRelayError {
                            code: BotRelayErrorCode::IdentityUnavailable,
                            retryable: true,
                            detail: BotRelayErrorDetail {
                                upstream: Some("hello required".into()),
                            },
                            reason: None,
                        };
                        let resp = JsonRpcResponse::<Value>::err(req.id, JsonRpcError::from(err));
                        return vec![serde_json::to_string(&resp).unwrap()];
                    }
                    let resp = self.hub.handle_rpc(&self.connection_id, req).await;
                    return vec![serde_json::to_string(&resp).unwrap()];
                }
                Err(e) => {
                    warn!("bad request: {e}");
                    return vec![];
                }
            }
        }

        debug!("ignored frame");
        vec![]
    }
}

#[cfg(test)]
mod cold_hot_tests {
    use super::*;
    use serde_json::json;

    fn rpc(method: &str, params: Value, id: i64) -> JsonRpcRequest<Value> {
        JsonRpcRequest {
            jsonrpc: JsonRpcVersion,
            id: JsonRpcId::Number(id),
            session_id: None,
            method: method.to_string(),
            params,
        }
    }

    #[tokio::test]
    async fn cold_status_roster_do_not_invoke_gateway() {
        let (hub, gw) = Hub::with_in_memory_gateway();
        hub.register_connection("c1").await;
        let before = gw.invoke_count();
        for i in 0..5 {
            let r = hub
                .handle_rpc("c1", rpc("bot.status", json!({}), i))
                .await;
            assert!(matches!(r.outcome, ResponseOutcome::Result(_)));
            let r = hub
                .handle_rpc("c1", rpc("bot.roster", json!({}), 100 + i))
                .await;
            assert!(matches!(r.outcome, ResponseOutcome::Result(_)));
        }
        assert_eq!(gw.invoke_count(), before, "cold path must not wake gateway");
        assert_eq!(hub.cold_hits(), 10);
        assert_eq!(hub.hot_hits(), 0);
    }

    #[tokio::test]
    async fn hot_command_invokes_gateway() {
        let (hub, gw) = Hub::with_in_memory_gateway();
        hub.register_connection("c1").await;
        let before = gw.invoke_count();
        let r = hub
            .handle_rpc(
                "c1",
                rpc(
                    "bot.command",
                    json!({
                        "agentId": "agt_1",
                        "name": "listAgents",
                        "args": {}
                    }),
                    1,
                ),
            )
            .await;
        assert!(matches!(r.outcome, ResponseOutcome::Result(_)));
        assert_eq!(gw.invoke_count(), before + 1);
        assert_eq!(hub.hot_hits(), 1);
    }

    #[tokio::test]
    async fn agent_id_mismatch_rejected_without_gateway() {
        let (hub, gw) = Hub::with_in_memory_gateway();
        hub.register_connection("c1").await;
        let before = gw.invoke_count();
        let r = hub
            .handle_rpc(
                "c1",
                rpc(
                    "bot.command",
                    json!({
                        "agentId": "agt_1",
                        "name": "sendPrompt",
                        "args": { "agentId": "agt_other", "prompt": "x" }
                    }),
                    1,
                ),
            )
            .await;
        match r.outcome {
            ResponseOutcome::Error(e) => {
                assert_eq!(e.message, "command_rejected");
                let data = e.data.unwrap();
                assert_eq!(data["reason"], COMMAND_REJECTED_AGENT_ID_MISMATCH);
            }
            other => panic!("expected error, got {other:?}"),
        }
        assert_eq!(gw.invoke_count(), before);
    }

    #[tokio::test]
    async fn unknown_gateway_method_maps_reason() {
        let (hub, _gw) = Hub::with_in_memory_gateway();
        hub.register_connection("c1").await;
        let r = hub
            .handle_rpc(
                "c1",
                rpc(
                    "bot.command",
                    json!({
                        "agentId": "agt_1",
                        "name": "createAgent",
                        "args": {}
                    }),
                    1,
                ),
            )
            .await;
        match r.outcome {
            ResponseOutcome::Error(e) => {
                let data = e.data.unwrap();
                assert_eq!(data["reason"], COMMAND_REJECTED_GATEWAY_UNKNOWN_METHOD);
            }
            other => panic!("expected error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn unknown_error_code_degrades_to_upstream_error() {
        assert_eq!(
            normalize_error_code("totally_new_code"),
            BotRelayErrorCode::UpstreamError
        );
        assert_eq!(
            normalize_error_code("command_rejected"),
            BotRelayErrorCode::CommandRejected
        );
    }

    #[tokio::test]
    async fn subscribe_fanout_turn_finished() {
        let (hub, gw) = Hub::with_in_memory_gateway();
        hub.spawn_turn_bridge(gw.subscribe_turns());
        hub.register_connection("c1").await;
        let mut bus = hub.subscribe_outbound();
        hub.handle_rpc(
            "c1",
            rpc(
                "bot.subscribe",
                json!({ "agentIds": ["agt_1"] }),
                1,
            ),
        )
        .await;
        hub.handle_rpc(
            "c1",
            rpc(
                "bot.command",
                json!({
                    "agentId": "agt_1",
                    "name": "sendPrompt",
                    "args": { "agentId": "agt_1", "prompt": "ping" }
                }),
                2,
            ),
        )
        .await;
        // Wait for fan-out
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
        let mut saw = false;
        while tokio::time::Instant::now() < deadline {
            match tokio::time::timeout(std::time::Duration::from_millis(200), bus.recv()).await {
                Ok(Ok((cid, text))) if cid == "c1" => {
                    let v: Value = serde_json::from_str(&text).unwrap();
                    assert_eq!(v["method"], "bot.event");
                    assert_eq!(v["params"]["channel"], "hub:turn_finished");
                    assert_eq!(v["params"]["seq"], 1);
                    assert_eq!(v["params"]["v"], 1);
                    saw = true;
                    break;
                }
                _ => {}
            }
        }
        assert!(saw, "expected bot.event fan-out");
    }

    #[tokio::test]
    async fn session_raw_hello() {
        let (hub, _) = Hub::with_in_memory_gateway();
        let mut s = Session::new(hub);
        let out = s
            .on_text(r#"{"protocol_version":"1.0.0","kind":"bot_client"}"#)
            .await;
        assert_eq!(out.len(), 1);
        let ack: HelloAckMsg = serde_json::from_str(&out[0]).unwrap();
        assert!(!ack.capabilities.is_empty());
        assert!(ack
            .capabilities
            .iter()
            .any(|c| c == "bot.command"));
    }
}
