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

/// CG1: classify a CLI stream-json / ATLAS_TOOL name for human gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliToolGateClass {
    /// Read-only tools — emit hub:tool only; do not hang.
    Exempt,
    /// Dangerous / unknown — hang for Allow/Deny (or auto_deny).
    Gated,
    /// `tool_call` status=completed — observe only; never re-gate.
    ObserveOnly,
}

/// Case-insensitive exempt names (CG1 / runbook).
const CLI_EXEMPT: &[&str] = &[
    "read",
    "readfile",
    "grep",
    "glob",
    "ls",
    "listdir",
    "semanticsearch",
    "websearch",
    "fetch",
    "webfetch",
];

/// Case-insensitive explicitly gated names (CG1 / runbook).
const CLI_GATED: &[&str] = &[
    "shell",
    "bash",
    "run",
    "write",
    "writefile",
    "edit",
    "delete",
    "remove",
    "applypatch",
    "notebookedit",
];

fn normalize_tool_token(name: &str) -> String {
    name.trim()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// Write-ish MCP heuristic: name/summary contains `mcp` and a write/exec keyword.
pub fn cli_tool_is_writeish_mcp(name: &str, summary: &str) -> bool {
    let blob = format!("{name} {summary}").to_ascii_lowercase();
    if !blob.contains("mcp") {
        return false;
    }
    const KEYS: &[&str] = &[
        "write", "edit", "delete", "remove", "exec", "shell", "run", "patch", "create", "mkdir",
        "bash", "apply",
    ];
    KEYS.iter().any(|k| blob.contains(k))
}

/// Classify CLI tool for CG1 gate.
///
/// - `status == "completed"` → [`CliToolGateClass::ObserveOnly`]
/// - known exempt → Exempt
/// - known gated / write-ish MCP / **unknown** → Gated (default deny-pending)
pub fn classify_cli_tool(name: &str, summary: &str, status: &str) -> CliToolGateClass {
    let status = status.trim().to_ascii_lowercase();
    if status == "completed" {
        return CliToolGateClass::ObserveOnly;
    }
    let key = normalize_tool_token(name);
    if key.is_empty() {
        return CliToolGateClass::Gated;
    }
    if CLI_EXEMPT.iter().any(|e| *e == key.as_str()) {
        return CliToolGateClass::Exempt;
    }
    if CLI_GATED.iter().any(|e| *e == key.as_str()) {
        return CliToolGateClass::Gated;
    }
    if cli_tool_is_writeish_mcp(name, summary) {
        return CliToolGateClass::Gated;
    }
    // Unknown names: default gated (CG1 hard rule).
    CliToolGateClass::Gated
}

#[cfg(test)]
mod cli_classify_tests {
    use super::*;

    #[test]
    fn exempt_read_case_insensitive() {
        assert_eq!(
            classify_cli_tool("Read", "", "started"),
            CliToolGateClass::Exempt
        );
        assert_eq!(
            classify_cli_tool("READFILE", "x", "started"),
            CliToolGateClass::Exempt
        );
        assert_eq!(
            classify_cli_tool("WebSearch", "q", ""),
            CliToolGateClass::Exempt
        );
    }

    #[test]
    fn gated_shell_and_unknown() {
        assert_eq!(
            classify_cli_tool("Shell", "rm -rf", "started"),
            CliToolGateClass::Gated
        );
        assert_eq!(
            classify_cli_tool("MysteryTool", "", "started"),
            CliToolGateClass::Gated
        );
    }

    #[test]
    fn completed_observe_only() {
        assert_eq!(
            classify_cli_tool("Shell", "done", "completed"),
            CliToolGateClass::ObserveOnly
        );
    }

    #[test]
    fn mcp_writeish() {
        assert_eq!(
            classify_cli_tool("mcp_fs", "write file", "started"),
            CliToolGateClass::Gated
        );
    }
}
