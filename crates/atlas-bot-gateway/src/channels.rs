//! C1 channel MVP: single-platform Slack stub (local, no egress).
//!
//! Token lives only in process memory keyed by `(agent_id, platform)`.
//! Reply shape is always `SandChannelsView` — never includes `token`.

use std::collections::HashMap;

use serde_json::{json, Value};

use crate::GatewayError;

/// Catalog platform nail for C1 (single stub).
pub const C1_STUB_PLATFORM: &str = "slack";

/// Connection status nail for C1 stub (runbook: always `connected` when stored).
pub const C1_STUB_STATUS: &str = "connected";

/// In-memory channel credential + status. `token` must never appear in reply JSON.
#[derive(Debug, Clone)]
pub struct ChannelTokenEntry {
    pub token: String,
    pub status: String,
    pub detail: String,
}

impl ChannelTokenEntry {
    pub fn stub_connected(token: String) -> Self {
        Self {
            token,
            status: C1_STUB_STATUS.to_string(),
            detail: "local stub, no egress".to_string(),
        }
    }

    pub fn refresh_stub(&mut self) {
        // Re-normalize status only — never probe the network.
        self.status = C1_STUB_STATUS.to_string();
        self.detail = "local stub refreshed (no egress)".to_string();
    }
}

/// agent_id → platform → entry (token in memory only).
pub type ChannelStore = HashMap<String, HashMap<String, ChannelTokenEntry>>;

/// Single stub ConnectorManifest (`platform=slack`, honest local-stub copy).
pub fn slack_stub_manifest() -> Value {
    json!({
        "platform": C1_STUB_PLATFORM,
        "displayName": "Slack (local stub)",
        "availability": "available",
        "blurb": "Local stub connector — no real Slack egress.",
        "connectGuide": "Paste a token to register a local stub connection. This does not call Slack or leave the machine.",
        "credentialLabel": "Bot token (local stub, no egress)",
    })
}

fn connection_value(platform: &str, entry: &ChannelTokenEntry) -> Value {
    json!({
        "label": format!("Slack (local stub)"),
        "platform": platform,
        "status": entry.status,
        "detail": entry.detail,
    })
}

/// Build `SandChannelsView` for one agent from the in-memory store.
/// Never includes token fields.
pub fn sand_channels_view(store: &ChannelStore, agent_id: &str) -> Value {
    let mut connections = Vec::new();
    if let Some(by_platform) = store.get(agent_id) {
        let mut platforms: Vec<_> = by_platform.keys().cloned().collect();
        platforms.sort();
        for platform in platforms {
            if let Some(entry) = by_platform.get(&platform) {
                connections.push(connection_value(&platform, entry));
            }
        }
    }
    json!({
        "connections": connections,
        "manifests": [slack_stub_manifest()],
    })
}

pub fn parse_channel_id(args: &Value) -> Result<String, GatewayError> {
    args.get("id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| GatewayError::InvalidArgs("missing id".into()))
}

pub fn parse_channel_platform(args: &Value) -> Result<String, GatewayError> {
    args.get("platform")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| GatewayError::InvalidArgs("missing platform".into()))
}

/// Require C1-allowed platform (`slack` only).
pub fn require_stub_platform(platform: &str) -> Result<(), GatewayError> {
    if platform == C1_STUB_PLATFORM {
        Ok(())
    } else {
        Err(GatewayError::InvalidArgs(format!(
            "unsupported platform: {platform} (C1 stub allows only {C1_STUB_PLATFORM})"
        )))
    }
}

/// Non-empty token from args. Error messages never echo the token value.
pub fn parse_channel_token(args: &Value) -> Result<String, GatewayError> {
    let Some(raw) = args.get("token").and_then(|v| v.as_str()) else {
        return Err(GatewayError::InvalidArgs("missing token".into()));
    };
    let token = raw.trim();
    if token.is_empty() {
        return Err(GatewayError::InvalidArgs("empty token".into()));
    }
    Ok(token.to_string())
}

/// True if serialized JSON contains `needle` as a substring (token leak check).
pub fn json_contains_substr(v: &Value, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    serde_json::to_string(v)
        .map(|s| s.contains(needle))
        .unwrap_or(false)
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn manifest_is_slack_stub() {
        let m = slack_stub_manifest();
        assert_eq!(m["platform"], C1_STUB_PLATFORM);
        assert_eq!(m["availability"], "available");
        let s = serde_json::to_string(&m).unwrap();
        assert!(s.contains("local stub") || s.contains("no egress") || s.contains("no real Slack"));
    }

    #[test]
    fn view_never_has_token_field() {
        let mut store = ChannelStore::new();
        store
            .entry("agt_1".into())
            .or_default()
            .insert(
                C1_STUB_PLATFORM.into(),
                ChannelTokenEntry::stub_connected("secret-token-xyz".into()),
            );
        let view = sand_channels_view(&store, "agt_1");
        assert!(view.get("connections").is_some());
        assert_eq!(view["connections"][0]["status"], C1_STUB_STATUS);
        assert!(!json_contains_substr(&view, "secret-token-xyz"));
        assert!(view["connections"][0].get("token").is_none());
    }
}
