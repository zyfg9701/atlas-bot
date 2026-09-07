//! Computer Hub Bot-Relay MVP (P3 / P3.5).
//!
//! Cold path (`bot.status`, `bot.roster`, `bot.transcript.offbox`) is
//! answered from in-memory hub state and **never** invokes the gateway.
//! Hot path (`bot.command`) passthroughs `name`/`args` verbatim via
//! [`atlas_bot_gateway::Gateway`].

#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use atlas_bot_gateway::{
    Gateway, GatewayError, InMemoryGateway, TranscriptEntry, DEFAULT_AGENT_ID, DEFAULT_AGENT_NAME,
};
use serde_json::{json, Value};
use tokio::sync::{broadcast, RwLock};
use tracing::{debug, info, warn};
use uuid::Uuid;
use xai_tool_protocol::{
    BotCommandParams, BotEmptyResult, BotEventEnvelope, BotRelayError, BotRelayErrorCode,
    BotRelayErrorDetail, BotRosterEntry, BotRosterResult, BotRunState, BotStatusResult,
    BotSubscribeParams, BotTranscriptOffboxParams, BotTranscriptOffboxResult, ConnectionId,
    ConnectionKind, HelloAckMsg, HelloMsg, HubChannel, HubTurnFinishedEvent, JsonRpcError,
    JsonRpcId, JsonRpcNotification, JsonRpcRequest, JsonRpcResponse, JsonRpcVersion, Method,
    ResponseOutcome, UserId, BOT_RELAY_CAPABILITIES, COMMAND_REJECTED_AGENT_ID_MISMATCH,
    COMMAND_REJECTED_ARGS_INVALID, COMMAND_REJECTED_GATEWAY_UNKNOWN_METHOD,
    COMMAND_REJECTED_NOT_YET_ENABLED, PROTOCOL_VERSION,
};

pub const HUB_VERSION: &str = "0.3.5-p35";

/// Page size for cold `bot.transcript.offbox` stub pagination.
pub const OFFBOX_PAGE_SIZE: usize = 2;

/// Per-connection subscription + seq state.
#[derive(Debug, Default)]
struct ConnSubs {
    /// agent_id -> next seq (starts at 1 after subscribe)
    seqs: HashMap<String, u64>,
    full_fidelity: bool,
}

#[derive(Debug, Default, Clone)]
struct OffboxAgent {
    entries: Vec<Value>,
}

/// Shared hub state.
pub struct Hub {
    gateway: Arc<dyn Gateway>,
    run_state: RwLock<BotRunState>,
    roster: RwLock<Vec<BotRosterEntry>>,
    /// Cold off-box transcript cache (never read via gateway invoke).
    offbox: RwLock<HashMap<String, OffboxAgent>>,
    /// connection_id -> subs
    subs: RwLock<HashMap<String, ConnSubs>>,
    /// Fan-out of serialized `bot.event` notifications keyed by connection.
    event_bus: broadcast::Sender<(String /*conn*/, String /*json*/)>,
    cold_hits: AtomicU64,
    hot_hits: AtomicU64,
    /// When true, `TurnFinishedHint` bridge owns `hub:turn_finished`.
    /// When false (typical HTTP remote), Hub emits after sync-complete sendPrompt.
    turn_bridge_active: AtomicBool,
}

impl Hub {
    pub fn new(gateway: Arc<dyn Gateway>) -> Arc<Self> {
        let (event_bus, _) = broadcast::channel(256);
        let mut offbox = HashMap::new();
        offbox.insert(
            DEFAULT_AGENT_ID.to_string(),
            OffboxAgent {
                entries: vec![json!({
                    "id": "msg_seed",
                    "role": "assistant",
                    "text": "stub ready",
                    "seq": 1,
                })],
            },
        );
        Arc::new(Self {
            gateway,
            run_state: RwLock::new(BotRunState::Hibernated),
            roster: RwLock::new(vec![BotRosterEntry {
                agent_id: DEFAULT_AGENT_ID.to_string(),
                name: DEFAULT_AGENT_NAME.to_string(),
                status: "unknown".to_string(),
                last_turn_at: None,
            }]),
            offbox: RwLock::new(offbox),
            subs: RwLock::new(HashMap::new()),
            event_bus,
            cold_hits: AtomicU64::new(0),
            hot_hits: AtomicU64::new(0),
            turn_bridge_active: AtomicBool::new(false),
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
    pub fn spawn_turn_bridge(
        self: &Arc<Self>,
        mut rx: broadcast::Receiver<atlas_bot_gateway::TurnFinishedHint>,
    ) {
        self.turn_bridge_active.store(true, Ordering::SeqCst);
        let hub = Arc::clone(self);
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(hint) => {
                        hub.ingest_turn_entries(&hint.agent_id, &hint.entries).await;
                        hub.emit_turn_finished(&hint.agent_id, &hint.preview).await;
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }

    async fn ingest_turn_entries(&self, agent_id: &str, entries: &[TranscriptEntry]) {
        let mut offbox = self.offbox.write().await;
        let slot = offbox.entry(agent_id.to_string()).or_default();
        for e in entries {
            let v = json!({
                "id": e.id,
                "role": e.role,
                "text": e.text,
                "seq": e.seq,
            });
            if !slot.entries.iter().any(|x| x.get("id") == v.get("id")) {
                slot.entries.push(v);
            }
        }
    }

    pub async fn upsert_roster_agent(&self, agent_id: &str, name: &str, status: &str) {
        let mut roster = self.roster.write().await;
        if let Some(row) = roster.iter_mut().find(|r| r.agent_id == agent_id) {
            row.name = name.to_string();
            row.status = status.to_string();
        } else {
            roster.push(BotRosterEntry {
                agent_id: agent_id.to_string(),
                name: name.to_string(),
                status: status.to_string(),
                last_turn_at: None,
            });
            roster.sort_by(|a, b| a.agent_id.cmp(&b.agent_id));
        }
    }

    pub async fn seed_offbox(&self, agent_id: &str, entries: Vec<Value>) {
        let mut offbox = self.offbox.write().await;
        offbox.insert(agent_id.to_string(), OffboxAgent { entries });
    }

    pub async fn append_offbox_entry(&self, agent_id: &str, entry: Value) {
        let mut offbox = self.offbox.write().await;
        offbox.entry(agent_id.to_string()).or_default().entries.push(entry);
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

    fn page_offbox(entries: &[Value], cursor: Option<&str>) -> BotTranscriptOffboxResult {
        let start = match cursor {
            None => 0usize,
            Some(c) if c.starts_with('c') => c[1..].parse::<usize>().unwrap_or(0),
            Some(_) => 0,
        };
        if start >= entries.len() {
            return BotTranscriptOffboxResult {
                entries: json!([]),
                next_cursor: None,
            };
        }
        let end = (start + OFFBOX_PAGE_SIZE).min(entries.len());
        let page = entries[start..end].to_vec();
        let next_cursor = if end < entries.len() {
            Some(format!("c{end}"))
        } else {
            None
        };
        BotTranscriptOffboxResult {
            entries: Value::Array(page),
            next_cursor,
        }
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
            Some(Method::BotTranscriptOffbox) => {
                self.cold_hits.fetch_add(1, Ordering::SeqCst);
                debug!(%connection_id, "cold bot.transcript.offbox");
                match serde_json::from_value::<BotTranscriptOffboxParams>(req.params) {
                    Ok(p) => {
                        let offbox = self.offbox.read().await;
                        match offbox.get(&p.agent_id) {
                            Some(store) => {
                                let result =
                                    Self::page_offbox(&store.entries, p.cursor.as_deref());
                                JsonRpcResponse::ok(id, serde_json::to_value(result).unwrap())
                            }
                            None => {
                                // Unknown agent → protocol error (do not crash client).
                                JsonRpcResponse::err(
                                    id,
                                    JsonRpcError::from(BotRelayError {
                                        code: BotRelayErrorCode::CommandRejected,
                                        retryable: false,
                                        detail: BotRelayErrorDetail {
                                            upstream: Some(format!(
                                                "unknown agent {}",
                                                p.agent_id
                                            )),
                                        },
                                        reason: Some(COMMAND_REJECTED_ARGS_INVALID.to_string()),
                                    }),
                                )
                            }
                        }
                    }
                    Err(e) => JsonRpcResponse::err(id, reject_args(e.to_string())),
                }
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
            Some(Method::BotVncDescriptor) | Some(Method::BotBindConversation) => {
                JsonRpcResponse::err(id, not_yet_enabled(&req.method))
            }
            Some(Method::Hello) => match serde_json::from_value::<HelloMsg>(req.params) {
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
            },
            Some(other) => JsonRpcResponse::err(
                id,
                JsonRpcError {
                    code: -32601,
                    message: format!("{}{}`", xai_tool_protocol::UNKNOWN_METHOD_MSG_PREFIX, other),
                    data: None,
                },
            ),
            None => {
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
            .invoke(&params.agent_id, &params.name, params.args.clone())
            .await
        {
            Ok(result) => {
                self.after_command_success(&params.name, &params.agent_id, &params.args, &result)
                    .await;
                JsonRpcResponse::ok(id, result)
            }
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

    pub fn turn_bridge_active(&self) -> bool {
        self.turn_bridge_active.load(Ordering::SeqCst)
    }

    async fn after_command_success(
        &self,
        name: &str,
        envelope_agent_id: &str,
        args: &Value,
        result: &Value,
    ) {
        match name {
            "sendPrompt" => {
                // HTTP remote / sync-complete: emit hub:turn_finished when the
                // gateway result asks for it and no in-process TurnFinishedHint
                // bridge is active (大禹 P3.5 nail).
                if self.turn_bridge_active.load(Ordering::SeqCst) {
                    return;
                }
                let should_emit = result
                    .get("hubEmitTurnFinished")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
                    || result
                        .get("completed")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                if !should_emit {
                    return;
                }
                let preview = result
                    .get("preview")
                    .and_then(|v| v.as_str())
                    .or_else(|| result.get("reply").and_then(|v| v.as_str()))
                    .unwrap_or("")
                    .to_string();
                if let Some(entries) = result.get("entries").and_then(|v| v.as_array()) {
                    let parsed: Vec<TranscriptEntry> = entries
                        .iter()
                        .filter_map(|e| serde_json::from_value(e.clone()).ok())
                        .collect();
                    if !parsed.is_empty() {
                        self.ingest_turn_entries(envelope_agent_id, &parsed).await;
                    } else {
                        // Fallback: ingest raw JSON values into offbox.
                        self.seed_offbox(envelope_agent_id, entries.clone()).await;
                    }
                }
                if !preview.is_empty() {
                    self.emit_turn_finished(envelope_agent_id, &preview).await;
                }
            }
            "createAgent" => {
                let agent_id = result
                    .get("agentId")
                    .and_then(|v| v.as_str())
                    .or_else(|| result.pointer("/agent/id").and_then(|v| v.as_str()));
                let agent_name = result
                    .pointer("/agent/name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("agent");
                if let Some(aid) = agent_id {
                    self.upsert_roster_agent(aid, agent_name, "unknown").await;
                    let entries = result
                        .get("transcript")
                        .cloned()
                        .unwrap_or_else(|| json!([]));
                    if let Some(arr) = entries.as_array() {
                        self.seed_offbox(aid, arr.clone()).await;
                    }
                }
            }
            "interruptAgentRun" => {
                let target = args
                    .get("agentId")
                    .and_then(|v| v.as_str())
                    .or_else(|| args.get("id").and_then(|v| v.as_str()))
                    .unwrap_or(envelope_agent_id);
                if result.get("hadActiveRun").and_then(|v| v.as_bool()) == Some(true) {
                    self.append_offbox_entry(
                        target,
                        json!({
                            "id": format!("offbox_int_{}", Uuid::new_v4()),
                            "role": "system",
                            "text": "run interrupted",
                            "seq": 0,
                        }),
                    )
                    .await;
                    let mut roster = self.roster.write().await;
                    if let Some(row) = roster.iter_mut().find(|r| r.agent_id == target) {
                        row.status = "idle".to_string();
                    }
                }
            }
            "getAgentTranscriptTail" => {
                if let Some(entries) = result.get("entries").and_then(|v| v.as_array()) {
                    let aid = result
                        .get("agentId")
                        .and_then(|v| v.as_str())
                        .unwrap_or(envelope_agent_id);
                    self.seed_offbox(aid, entries.clone()).await;
                }
            }
            _ => {}
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

    pub async fn on_text(&mut self, text: &str) -> Vec<String> {
        let v: Value = match serde_json::from_str(text) {
            Ok(v) => v,
            Err(e) => {
                warn!("parse error: {e}");
                return vec![];
            }
        };

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

        if v.get("jsonrpc").is_some() && v.get("id").is_some() && v.get("method").is_some() {
            match serde_json::from_value::<JsonRpcRequest<Value>>(v) {
                Ok(req) => {
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
    use std::time::Duration;

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
    async fn cold_transcript_offbox_no_gateway_and_paginates() {
        let (hub, gw) = Hub::with_in_memory_gateway();
        hub.register_connection("c1").await;
        // Seed enough entries for two pages (page size 2).
        hub.seed_offbox(
            "agt_1",
            vec![
                json!({"id": "e1", "seq": 1}),
                json!({"id": "e2", "seq": 2}),
                json!({"id": "e3", "seq": 3}),
            ],
        )
        .await;
        let before = gw.invoke_count();
        let r1 = hub
            .handle_rpc(
                "c1",
                rpc(
                    "bot.transcript.offbox",
                    json!({ "agentId": "agt_1" }),
                    1,
                ),
            )
            .await;
        let ResponseOutcome::Result(v1) = r1.outcome else {
            panic!("expected result");
        };
        assert_eq!(v1["entries"].as_array().unwrap().len(), 2);
        assert_eq!(v1["nextCursor"], "c2");
        let r2 = hub
            .handle_rpc(
                "c1",
                rpc(
                    "bot.transcript.offbox",
                    json!({ "agentId": "agt_1", "cursor": "c2" }),
                    2,
                ),
            )
            .await;
        let ResponseOutcome::Result(v2) = r2.outcome else {
            panic!("expected result");
        };
        assert_eq!(v2["entries"].as_array().unwrap().len(), 1);
        assert!(v2.get("nextCursor").is_none() || v2["nextCursor"].is_null());
        assert_eq!(gw.invoke_count(), before);
        assert_eq!(hub.cold_hits(), 2);
    }

    #[tokio::test]
    async fn cold_offbox_unknown_agent_errors() {
        let (hub, gw) = Hub::with_in_memory_gateway();
        hub.register_connection("c1").await;
        let before = gw.invoke_count();
        let r = hub
            .handle_rpc(
                "c1",
                rpc(
                    "bot.transcript.offbox",
                    json!({ "agentId": "agt_missing" }),
                    1,
                ),
            )
            .await;
        assert!(matches!(r.outcome, ResponseOutcome::Error(_)));
        assert_eq!(gw.invoke_count(), before);
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
    async fn create_agent_updates_cold_roster() {
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
                        "args": { "name": "Scout" }
                    }),
                    1,
                ),
            )
            .await;
        let ResponseOutcome::Result(created) = r.outcome else {
            panic!("create failed: {r:?}");
        };
        let new_id = created["agentId"].as_str().unwrap().to_string();
        let roster = hub
            .handle_rpc("c1", rpc("bot.roster", json!({}), 2))
            .await;
        let ResponseOutcome::Result(ro) = roster.outcome else {
            panic!("roster failed");
        };
        assert!(ro["agents"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["agentId"] == new_id && a["name"] == "Scout"));
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
                        "name": "deleteAgents",
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
    async fn interrupt_then_send_again() {
        let gw = Arc::new(InMemoryGateway::with_turn_delay(Duration::from_secs(5)));
        let hub = Hub::new(gw.clone());
        hub.register_connection("c1").await;
        hub.handle_rpc(
            "c1",
            rpc(
                "bot.command",
                json!({
                    "agentId": "agt_1",
                    "name": "sendPrompt",
                    "args": { "agentId": "agt_1", "prompt": "long" }
                }),
                1,
            ),
        )
        .await;
        tokio::time::sleep(Duration::from_millis(30)).await;
        let r = hub
            .handle_rpc(
                "c1",
                rpc(
                    "bot.command",
                    json!({
                        "agentId": "agt_1",
                        "name": "interruptAgentRun",
                        "args": { "agentId": "agt_1" }
                    }),
                    2,
                ),
            )
            .await;
        let ResponseOutcome::Result(v) = r.outcome else {
            panic!("interrupt failed: {r:?}");
        };
        assert_eq!(v["hadActiveRun"], true);
        let r2 = hub
            .handle_rpc(
                "c1",
                rpc(
                    "bot.command",
                    json!({
                        "agentId": "agt_1",
                        "name": "sendPrompt",
                        "args": { "agentId": "agt_1", "prompt": "again", "immediate": true }
                    }),
                    3,
                ),
            )
            .await;
        assert!(matches!(r2.outcome, ResponseOutcome::Result(_)));
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
        let gw = Arc::new(InMemoryGateway::with_turn_delay(Duration::from_millis(20)));
        let hub = Hub::new(gw.clone());
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
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        let mut saw = false;
        while tokio::time::Instant::now() < deadline {
            match tokio::time::timeout(Duration::from_millis(200), bus.recv()).await {
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
    async fn seq_isolated_per_agent() {
        let gw = Arc::new(InMemoryGateway::with_turn_delay(Duration::from_millis(10)));
        let hub = Hub::new(gw.clone());
        hub.spawn_turn_bridge(gw.subscribe_turns());
        hub.register_connection("c1").await;
        let mut bus = hub.subscribe_outbound();
        // create second agent
        let created = hub
            .handle_rpc(
                "c1",
                rpc(
                    "bot.command",
                    json!({
                        "agentId": "agt_1",
                        "name": "createAgent",
                        "args": { "name": "B" }
                    }),
                    1,
                ),
            )
            .await;
        let ResponseOutcome::Result(c) = created.outcome else {
            panic!("create failed");
        };
        let agt2 = c["agentId"].as_str().unwrap().to_string();
        hub.handle_rpc(
            "c1",
            rpc(
                "bot.subscribe",
                json!({ "agentIds": ["agt_1", agt2] }),
                2,
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
                    "args": { "agentId": "agt_1", "prompt": "a", "immediate": true }
                }),
                3,
            ),
        )
        .await;
        hub.handle_rpc(
            "c1",
            rpc(
                "bot.command",
                json!({
                    "agentId": agt2,
                    "name": "sendPrompt",
                    "args": { "agentId": agt2, "prompt": "b", "immediate": true }
                }),
                4,
            ),
        )
        .await;
        let mut seqs: HashMap<String, Vec<u64>> = HashMap::new();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        while tokio::time::Instant::now() < deadline && seqs.values().map(|v| v.len()).sum::<usize>() < 2
        {
            if let Ok(Ok((cid, text))) =
                tokio::time::timeout(Duration::from_millis(200), bus.recv()).await
            {
                if cid != "c1" {
                    continue;
                }
                let v: Value = serde_json::from_str(&text).unwrap();
                if v["method"] == "bot.event" {
                    let aid = v["params"]["agentId"].as_str().unwrap().to_string();
                    let seq = v["params"]["seq"].as_u64().unwrap();
                    seqs.entry(aid).or_default().push(seq);
                }
            }
        }
        assert_eq!(seqs.get("agt_1").map(|v| v.as_slice()), Some([1].as_slice()));
        assert_eq!(seqs.get(&agt2).map(|v| v.as_slice()), Some([1].as_slice()));
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
        assert!(ack.capabilities.iter().any(|c| c == "bot.command"));
        assert!(ack
            .capabilities
            .iter()
            .any(|c| c == "bot.transcript.offbox"));
    }
}
