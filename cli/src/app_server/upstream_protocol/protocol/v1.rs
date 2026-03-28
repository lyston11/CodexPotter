//! Upstream app-server protocol v1 payloads.
//!
//! This module contains the request/response payloads for the initial `initialize` request and
//! the approval response payloads used by certain server-initiated requests.

use std::path::PathBuf;

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
    #[serde(default)]
    pub experimental_api: bool,
    /// Exact notification method names that should be suppressed for this connection.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opt_out_notification_methods: Option<Vec<String>>,
}

/// Optional server metadata returned by newer app-server builds.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ServerInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

/// Response payload for the `initialize` request.
///
/// Current upstream releases can omit `codexHome`, and some SDKs also accept
/// `serverInfo`. Keep this shape tolerant so `initialize` decoding does not
/// break when the app-server trims metadata.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct InitializeResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_info: Option<ServerInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codex_home: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform_family: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform_os: Option<String>,
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
