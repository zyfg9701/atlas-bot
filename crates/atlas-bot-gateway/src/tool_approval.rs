//! GA1 gateway-local tool approval (P0-1).
//!
//! Dangerous Box tools hang pending Allow/Deny; timeout == Deny.
//! Decisions arrive via `POST /approve` (loopback) or in-process hooks.
//! No new `bot.command`. See `docs/tool-approval-runbook.md`.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::Duration;

use tokio::sync::{oneshot, RwLock};

/// `ATLAS_TOOL_APPROVAL_MODE`: `gate` | `auto_deny` | `off`.
pub const ENV_TOOL_APPROVAL_MODE: &str = "ATLAS_TOOL_APPROVAL_MODE";
/// Hang deadline in ms before timeout==Deny. Default [`DEFAULT_TOOL_APPROVAL_TIMEOUT_MS`].
pub const ENV_TOOL_APPROVAL_TIMEOUT_MS: &str = "ATLAS_TOOL_APPROVAL_TIMEOUT_MS";
/// Optional shared token for `POST /approve` (`Authorization: Bearer …` or `X-Atlas-Approval-Token`).
pub const ENV_TOOL_APPROVAL_TOKEN: &str = "ATLAS_TOOL_APPROVAL_TOKEN";

/// Documented default hang timeout (30s). Sync invoke holds until decision/timeout.
pub const DEFAULT_TOOL_APPROVAL_TIMEOUT_MS: u64 = 30_000;

/// Closed-set deny reason strings (greppable).
pub const REASON_DENIED: &str = "tool_approval_denied";
pub const REASON_TIMEOUT: &str = "tool_approval_timeout";
pub const REASON_AUTO_DENY: &str = "tool_approval_auto_deny";

/// Prefix for GA1b thin-reuse pending hint on existing `hub:tool` channel.
pub const APPROVAL_PENDING_PREFIX: &str = "[approval_pending";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolApprovalMode {
    /// Wait for Allow/Deny/timeout (product default for box `from_env`).
    Gate,
    /// Immediately Deny gated tools (CI without UI).
    AutoDeny,
    /// No gate — execute as today (documented; not product default narrative).
    Off,
}

impl ToolApprovalMode {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "gate" | "on" | "1" | "true" => Some(Self::Gate),
            "auto_deny" | "autodeny" | "deny" => Some(Self::AutoDeny),
            "off" | "0" | "false" | "disabled" => Some(Self::Off),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Gate => "gate",
            Self::AutoDeny => "auto_deny",
            Self::Off => "off",
        }
    }

    /// Product box path (`from_env`): unset → `gate`.
    pub fn from_env_product_default() -> Self {
        std::env::var(ENV_TOOL_APPROVAL_MODE)
            .ok()
            .and_then(|s| Self::parse(&s))
            .unwrap_or(Self::Gate)
    }

    /// In-process `new()` constructors: unset → `off` so deepen/resident smokes stay green;
    /// explicit env still honored.
    pub fn from_env_test_default() -> Self {
        std::env::var(ENV_TOOL_APPROVAL_MODE)
            .ok()
            .and_then(|s| Self::parse(&s))
            .unwrap_or(Self::Off)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalDecision {
    Allow,
    Deny,
}

impl ApprovalDecision {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "allow" | "approve" | "yes" => Some(Self::Allow),
            "deny" | "reject" | "no" => Some(Self::Deny),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Deny => "deny",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateOutcome {
    Allow,
    Deny,
    Timeout,
    AutoDeny,
}

impl GateOutcome {
    pub fn reason(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Deny => REASON_DENIED,
            Self::Timeout => REASON_TIMEOUT,
            Self::AutoDeny => REASON_AUTO_DENY,
        }
    }

    pub fn is_allow(self) -> bool {
        matches!(self, Self::Allow)
    }
}

pub struct PendingApprovalSlot {
    pub agent_id: String,
    pub tool: String,
    pub summary: String,
    pub tx: oneshot::Sender<ApprovalDecision>,
}

/// Shared approval state for Box sidecar.
pub struct ApprovalState {
    pub mode: ToolApprovalMode,
    pub timeout: Duration,
    /// Optional token required by HTTP `/approve` (never logged in healthz value).
    pub token: Option<String>,
    pub pending: RwLock<HashMap<String, PendingApprovalSlot>>,
    /// Test hook: pre-queued decisions applied in order when a gate opens.
    pub inject: Mutex<VecDeque<ApprovalDecision>>,
}

impl ApprovalState {
    pub fn new(mode: ToolApprovalMode, timeout: Duration, token: Option<String>) -> Self {
        Self {
            mode,
            timeout,
            token,
            pending: RwLock::new(HashMap::new()),
            inject: Mutex::new(VecDeque::new()),
        }
    }

    pub fn timeout_from_env() -> Duration {
        let ms = std::env::var(ENV_TOOL_APPROVAL_TIMEOUT_MS)
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(DEFAULT_TOOL_APPROVAL_TIMEOUT_MS);
        Duration::from_millis(ms.max(1))
    }

    pub fn token_from_env() -> Option<String> {
        std::env::var(ENV_TOOL_APPROVAL_TOKEN)
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }
}

/// Format GA1b pending summary for `hub:tool` (exitCode null).
pub fn format_pending_summary(approval_id: &str, tool: &str, detail: &str) -> String {
    format!("{APPROVAL_PENDING_PREFIX} approvalId={approval_id} tool={tool}] {detail}")
}
