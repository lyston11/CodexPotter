//! Upstream app-server protocol v1 payloads.
//!
//! This module contains the request/response payloads for the initial `initialize` request and
//! the approval response payloads used by certain server-initiated requests.

use codex_protocol::protocol::ReviewDecision;
use serde::Deserialize;
use serde::Serialize;

/// Parameters for the `initialize` request.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct InitializeParams {
    pub client_info: ClientInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<InitializeCapabilities>,
}

/// Identifies the client for display/telemetry purposes.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ClientInfo {
    pub name: String,
    pub title: Option<String>,
    pub version: String,
}

/// Client-declared capabilities negotiated during initialize.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct InitializeCapabilities {
    /// Opt into experimental API methods and fields such as `turn/start.collaborationMode`.
    pub experimental_api: bool,
}

/// Response payload for an `applyPatch` approval request.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ApplyPatchApprovalResponse {
    pub decision: ReviewDecision,
}

/// Response payload for an `execCommand` approval request.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ExecCommandApprovalResponse {
    pub decision: ReviewDecision,
}
