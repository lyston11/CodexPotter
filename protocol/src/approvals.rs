use std::collections::HashMap;
use std::path::PathBuf;

use serde::Deserialize;
use serde::Serialize;
use serde_json::Value as JsonValue;

use crate::mcp::RequestId;
use crate::parse_command::ParsedCommand;
use crate::protocol::FileChange;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ExecApprovalRequestEvent {
    /// Identifier for the associated command execution item.
    pub call_id: String,
    /// Identifier for this specific approval callback.
    ///
    /// When absent, the approval is for the command item itself (`call_id`).
    /// This is present for subcommand approvals (via execve intercept).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval_id: Option<String>,
    /// Turn ID that this command belongs to.
    /// Uses `#[serde(default)]` for backwards compatibility.
    #[serde(default)]
    pub turn_id: String,
    /// The command to be executed.
    #[serde(default)]
    pub command: Vec<String>,
    /// The command's working directory.
    #[serde(default)]
    pub cwd: PathBuf,
    /// Optional human-readable reason for the approval.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Parsed command metadata for UI display.
    #[serde(default)]
    pub parsed_cmd: Vec<ParsedCommand>,
}

impl ExecApprovalRequestEvent {
    pub fn effective_approval_id(&self) -> String {
        self.approval_id
            .clone()
            .unwrap_or_else(|| self.call_id.clone())
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum GuardianRiskLevel {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GuardianAssessmentStatus {
    InProgress,
    Approved,
    Denied,
    Aborted,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct GuardianAssessmentEvent {
    /// Stable identifier for this guardian review lifecycle.
    pub id: String,
    /// Turn ID that this assessment belongs to.
    /// Uses `#[serde(default)]` for backwards compatibility.
    #[serde(default)]
    pub turn_id: String,
    pub status: GuardianAssessmentStatus,
    /// Numeric risk score from 0-100. Omitted while the assessment is in progress.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub risk_score: Option<u8>,
    /// Coarse risk label paired with `risk_score`. Omitted while in progress.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub risk_level: Option<GuardianRiskLevel>,
    /// Human-readable explanation of the final assessment. Omitted while in progress.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
    /// Canonical action payload that was reviewed, when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<JsonValue>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum ElicitationRequest {
    Form {
        #[serde(rename = "_meta", default, skip_serializing_if = "Option::is_none")]
        meta: Option<JsonValue>,
        message: String,
        requested_schema: JsonValue,
    },
    Url {
        #[serde(rename = "_meta", default, skip_serializing_if = "Option::is_none")]
        meta: Option<JsonValue>,
        message: String,
        url: String,
        elicitation_id: String,
    },
}

impl ElicitationRequest {
    pub fn message(&self) -> &str {
        match self {
            Self::Form { message, .. } | Self::Url { message, .. } => message,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct ElicitationRequestEvent {
    /// Turn ID that this elicitation belongs to, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    pub server_name: String,
    pub id: RequestId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request: Option<ElicitationRequest>,
    /// Backward-compatible message field (pre-request schema).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl ElicitationRequestEvent {
    pub fn message(&self) -> Option<&str> {
        match (&self.request, &self.message) {
            (Some(request), _) => Some(request.message()),
            (None, Some(message)) => Some(message),
            (None, None) => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ElicitationAction {
    Accept,
    Decline,
    Cancel,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ApplyPatchApprovalRequestEvent {
    /// Identifier for the associated patch apply call.
    pub call_id: String,
    /// Turn ID that this patch belongs to.
    /// Uses `#[serde(default)]` for backwards compatibility.
    #[serde(default)]
    pub turn_id: String,
    /// Proposed changes.
    #[serde(default)]
    pub changes: HashMap<PathBuf, FileChange>,
    /// Optional explanatory reason (e.g. request for extra write access).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// When set, the agent is asking the user to allow writes under this root for the remainder
    /// of the session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grant_root: Option<PathBuf>,
}
