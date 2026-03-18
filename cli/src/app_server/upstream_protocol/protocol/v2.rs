//! Upstream app-server protocol v2 payloads.
//!
//! This module contains request/response structs for v2 JSON-RPC methods (for example
//! `thread/start`, `thread/resume`, `thread/rollback`, `turn/start`) and the configuration types
//! they depend on.
//!
//! The shapes here intentionally mirror upstream Codex so the CLI can drive the `codex app-server`
//! subprocess without depending on its internal Rust types.

use std::collections::HashMap;
use std::path::PathBuf;

use codex_protocol::AbsolutePathBuf;
use codex_protocol::models::MessagePhase;
use codex_protocol::openai_models::ReasoningEffort;
use codex_protocol::protocol::CodexErrorInfo;
use codex_protocol::user_input::ByteRange as CoreByteRange;
use codex_protocol::user_input::TextElement as CoreTextElement;
use codex_protocol::user_input::UserInput as CoreUserInput;
use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;
use serde_json::Value as JsonValue;

fn deserialize_upstream_codex_error_info_opt<'de, D>(
    deserializer: D,
) -> Result<Option<CodexErrorInfo>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<JsonValue>::deserialize(deserializer)?;
    Ok(value.and_then(upstream_codex_error_info_from_value))
}

fn upstream_codex_error_info_from_value(value: JsonValue) -> Option<CodexErrorInfo> {
    serde_json::from_value::<CodexErrorInfo>(value.clone())
        .ok()
        .or_else(|| upstream_codex_error_info_from_camel_case_value(value))
}

fn upstream_codex_error_info_from_camel_case_value(value: JsonValue) -> Option<CodexErrorInfo> {
    match value {
        JsonValue::String(name) => match name.as_str() {
            "contextWindowExceeded" => Some(CodexErrorInfo::ContextWindowExceeded),
            "usageLimitExceeded" => Some(CodexErrorInfo::UsageLimitExceeded),
            "serverOverloaded" => Some(CodexErrorInfo::ServerOverloaded),
            "internalServerError" => Some(CodexErrorInfo::InternalServerError),
            "unauthorized" => Some(CodexErrorInfo::Unauthorized),
            "badRequest" => Some(CodexErrorInfo::BadRequest),
            "threadRollbackFailed" => Some(CodexErrorInfo::ThreadRollbackFailed),
            "sandboxError" => Some(CodexErrorInfo::SandboxError),
            "other" => Some(CodexErrorInfo::Other),
            _ => None,
        },
        JsonValue::Object(fields) => {
            let mut entries = fields.into_iter();
            let (name, payload) = entries.next()?;
            if entries.next().is_some() {
                return None;
            }

            match name.as_str() {
                "httpConnectionFailed" => Some(CodexErrorInfo::HttpConnectionFailed {
                    http_status_code: upstream_http_status_code(payload),
                }),
                "responseStreamConnectionFailed" => {
                    Some(CodexErrorInfo::ResponseStreamConnectionFailed {
                        http_status_code: upstream_http_status_code(payload),
                    })
                }
                "responseStreamDisconnected" => Some(CodexErrorInfo::ResponseStreamDisconnected {
                    http_status_code: upstream_http_status_code(payload),
                }),
                "responseTooManyFailedAttempts" => {
                    Some(CodexErrorInfo::ResponseTooManyFailedAttempts {
                        http_status_code: upstream_http_status_code(payload),
                    })
                }
                _ => None,
            }
        }
        _ => None,
    }
}

fn upstream_http_status_code(value: JsonValue) -> Option<u16> {
    let JsonValue::Object(mut fields) = value else {
        return None;
    };

    fields
        .remove("httpStatusCode")
        .or_else(|| fields.remove("http_status_code"))
        .and_then(|status| status.as_u64())
        .and_then(|status| u16::try_from(status).ok())
}

/// Upstream approval policy for agent tool executions.
///
/// CodexPotter typically sets this to [`AskForApproval::Never`] and handles any "approval"-like
/// UX at a higher level.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum AskForApproval {
    #[serde(rename = "untrusted")]
    UnlessTrusted,
    OnFailure,
    OnRequest,
    Never,
}

/// CLI-selected sandbox mode hint sent to the upstream app-server.
///
/// The app-server resolves this into a concrete [`SandboxPolicy`] and echoes the result back in
/// `thread/start` / `thread/resume` responses.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SandboxMode {
    ReadOnly,
    WorkspaceWrite,
    DangerFullAccess,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CommandExecutionApprovalDecision {
    Accept,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum FileChangeApprovalDecision {
    Accept,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CommandExecutionRequestApprovalResponse {
    pub decision: CommandExecutionApprovalDecision,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FileChangeRequestApprovalResponse {
    pub decision: FileChangeApprovalDecision,
}

#[derive(Serialize, Deserialize, Debug, Default, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AdditionalNetworkPermissions {
    pub enabled: Option<bool>,
}

#[derive(Serialize, Deserialize, Debug, Default, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AdditionalFileSystemPermissions {
    pub read: Option<Vec<AbsolutePathBuf>>,
    pub write: Option<Vec<AbsolutePathBuf>>,
}

#[derive(Serialize, Deserialize, Debug, Default, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RequestPermissionProfile {
    pub network: Option<AdditionalNetworkPermissions>,
    pub file_system: Option<AdditionalFileSystemPermissions>,
}

#[derive(Serialize, Deserialize, Debug, Default, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GrantedPermissionProfile {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network: Option<AdditionalNetworkPermissions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_system: Option<AdditionalFileSystemPermissions>,
}

impl From<RequestPermissionProfile> for GrantedPermissionProfile {
    fn from(value: RequestPermissionProfile) -> Self {
        Self {
            network: value.network,
            file_system: value.file_system,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum PermissionGrantScope {
    #[default]
    Turn,
    Session,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PermissionsRequestApprovalParams {
    pub thread_id: String,
    pub turn_id: String,
    pub item_id: String,
    pub reason: Option<String>,
    pub permissions: RequestPermissionProfile,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PermissionsRequestApprovalResponse {
    pub permissions: GrantedPermissionProfile,
    #[serde(default)]
    pub scope: PermissionGrantScope,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ToolRequestUserInputOption {
    pub label: String,
    pub description: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ToolRequestUserInputQuestion {
    pub id: String,
    pub header: String,
    pub question: String,
    #[serde(default)]
    pub is_other: bool,
    #[serde(default)]
    pub is_secret: bool,
    pub options: Option<Vec<ToolRequestUserInputOption>>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ToolRequestUserInputParams {
    pub thread_id: String,
    pub turn_id: String,
    pub item_id: String,
    pub questions: Vec<ToolRequestUserInputQuestion>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ToolRequestUserInputAnswer {
    pub answers: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug, Default, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ToolRequestUserInputResponse {
    pub answers: HashMap<String, ToolRequestUserInputAnswer>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum McpServerElicitationAction {
    Accept,
    Decline,
    Cancel,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "mode", rename_all = "camelCase")]
pub enum McpServerElicitationRequest {
    #[serde(rename_all = "camelCase")]
    Form {
        #[serde(rename = "_meta")]
        meta: Option<JsonValue>,
        message: String,
        requested_schema: JsonValue,
    },
    #[serde(rename_all = "camelCase")]
    Url {
        #[serde(rename = "_meta")]
        meta: Option<JsonValue>,
        message: String,
        url: String,
        elicitation_id: String,
    },
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct McpServerElicitationRequestParams {
    pub thread_id: String,
    pub turn_id: Option<String>,
    pub server_name: String,
    #[serde(flatten)]
    pub request: McpServerElicitationRequest,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct McpServerElicitationRequestResponse {
    pub action: McpServerElicitationAction,
    pub content: Option<JsonValue>,
    #[serde(rename = "_meta")]
    pub meta: Option<JsonValue>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DynamicToolCallParams {
    pub thread_id: String,
    pub turn_id: String,
    pub call_id: String,
    pub tool: String,
    pub arguments: JsonValue,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DynamicToolCallResponse {
    pub content_items: Vec<DynamicToolCallOutputContentItem>,
    pub success: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum DynamicToolCallOutputContentItem {
    #[serde(rename_all = "camelCase")]
    InputText { text: String },
    #[serde(rename_all = "camelCase")]
    InputImage { image_url: String },
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ChatgptAuthTokensRefreshReason {
    Unauthorized,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ChatgptAuthTokensRefreshParams {
    pub reason: ChatgptAuthTokensRefreshReason,
    pub previous_account_id: Option<String>,
}

/// Network access configuration for external sandbox policies.
#[derive(Serialize, Deserialize, Debug, Default, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum NetworkAccess {
    #[default]
    Restricted,
    Enabled,
}

/// Concrete sandbox policy resolved by the upstream app-server.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum SandboxPolicy {
    DangerFullAccess,
    ReadOnly,
    #[serde(rename_all = "camelCase")]
    ExternalSandbox {
        #[serde(default)]
        network_access: NetworkAccess,
    },
    #[serde(rename_all = "camelCase")]
    WorkspaceWrite {
        #[serde(default)]
        writable_roots: Vec<AbsolutePathBuf>,
        #[serde(default)]
        network_access: bool,
        #[serde(default)]
        exclude_tmpdir_env_var: bool,
        #[serde(default)]
        exclude_slash_tmp: bool,
    },
}

/// Parameters for the `thread/start` JSON-RPC method.
///
/// Note: optional fields are intentionally serialized as `null` when unset to match upstream.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ThreadStartParams {
    pub model: Option<String>,
    pub model_provider: Option<String>,
    pub cwd: Option<String>,
    pub approval_policy: Option<AskForApproval>,
    pub sandbox: Option<SandboxMode>,
    pub config: Option<HashMap<String, JsonValue>>,
    pub base_instructions: Option<String>,
    pub developer_instructions: Option<String>,
    #[serde(default)]
    pub experimental_raw_events: bool,
}

/// Response payload for `thread/start`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ThreadStartResponse {
    pub thread: Thread,
    pub model: String,
    pub model_provider: String,
    pub cwd: PathBuf,
    pub approval_policy: AskForApproval,
    pub sandbox: SandboxPolicy,
    pub reasoning_effort: Option<ReasoningEffort>,
}

/// Parameters for the `thread/resume` JSON-RPC method.
///
/// Note: optional fields are intentionally serialized as `null` when unset to match upstream.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ThreadResumeParams {
    pub thread_id: String,
    pub model: Option<String>,
    pub model_provider: Option<String>,
    pub cwd: Option<String>,
    pub approval_policy: Option<AskForApproval>,
    pub sandbox: Option<SandboxMode>,
    pub config: Option<HashMap<String, JsonValue>>,
    pub base_instructions: Option<String>,
    pub developer_instructions: Option<String>,
}

/// Response payload for `thread/resume`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ThreadResumeResponse {
    pub thread: Thread,
    pub model: String,
    pub model_provider: String,
    pub cwd: PathBuf,
    pub approval_policy: AskForApproval,
    pub sandbox: SandboxPolicy,
    pub reasoning_effort: Option<ReasoningEffort>,
}

/// Upstream thread metadata returned by `thread/*` methods.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Thread {
    pub id: String,
    pub path: PathBuf,
}

/// Parameters for the `thread/rollback` JSON-RPC method.
///
/// Rollback only affects the thread history and does **not** revert any local file changes made by
/// the agent.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ThreadRollbackParams {
    pub thread_id: String,
    /// The number of turns to drop from the end of the thread. Must be >= 1.
    pub num_turns: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ThreadRollbackResponse {
    pub thread: Thread,
}

/// Parameters for the `turn/start` JSON-RPC method.
#[derive(Serialize, Deserialize, Debug, Default, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TurnStartParams {
    pub thread_id: String,
    pub input: Vec<UserInput>,
    pub cwd: Option<PathBuf>,
    pub approval_policy: Option<AskForApproval>,
    pub sandbox_policy: Option<SandboxPolicy>,
    pub model: Option<String>,
    pub effort: Option<JsonValue>,
    pub summary: Option<JsonValue>,
    pub output_schema: Option<JsonValue>,
    pub collaboration_mode: Option<JsonValue>,
}

/// Upstream turn metadata returned by `turn/*` methods.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Turn {
    pub id: String,
}

/// Response payload for `turn/start`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TurnStartResponse {
    pub turn: Turn,
}

/// Parameters for the `turn/interrupt` JSON-RPC method.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TurnInterruptParams {
    pub thread_id: String,
    pub turn_id: String,
}

/// Response payload for `turn/interrupt`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TurnInterruptResponse {}

/// Byte range into the prompt string, used to map UI placeholders.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ByteRange {
    pub start: usize,
    pub end: usize,
}

impl From<CoreByteRange> for ByteRange {
    fn from(value: CoreByteRange) -> Self {
        Self {
            start: value.start,
            end: value.end,
        }
    }
}

/// Prompt metadata for UI placeholders (for example mentions).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TextElement {
    pub byte_range: ByteRange,
    pub placeholder: Option<String>,
}

impl From<CoreTextElement> for TextElement {
    fn from(value: CoreTextElement) -> Self {
        Self {
            byte_range: value.byte_range.into(),
            placeholder: value.placeholder,
        }
    }
}

/// User input items passed to `turn/start`.
///
/// This is a JSON-RPC-friendly subset of [`codex_protocol::user_input::UserInput`]. Unknown
/// variants are treated as a programmer error to surface protocol drift early.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum UserInput {
    Text {
        text: String,
        #[serde(default)]
        text_elements: Vec<TextElement>,
    },
    Image {
        url: String,
    },
    LocalImage {
        path: PathBuf,
    },
    Skill {
        name: String,
        path: PathBuf,
    },
    Mention {
        name: String,
        path: String,
    },
}

impl From<CoreUserInput> for UserInput {
    fn from(value: CoreUserInput) -> Self {
        match value {
            CoreUserInput::Text {
                text,
                text_elements,
            } => UserInput::Text {
                text,
                text_elements: text_elements.into_iter().map(Into::into).collect(),
            },
            CoreUserInput::Image { image_url } => UserInput::Image { url: image_url },
            CoreUserInput::LocalImage { path } => UserInput::LocalImage { path },
            CoreUserInput::Skill { name, path } => UserInput::Skill { name, path },
            CoreUserInput::Mention { name, path } => UserInput::Mention { name, path },
            _ => unreachable!("unsupported user input variant"),
        }
    }
}

// === Server notifications (subset) ===
//
// Newer upstream Codex app-server versions translate internal `EventMsg` values into typed JSON-RPC
// notifications (and no longer forward legacy `codex/event/*` notifications over stdio/websocket
// transports). CodexPotter parses a minimal subset of these payloads and maps them back into the
// legacy `EventMsg` stream that the workflow/TUI expects.

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum HookEventName {
    SessionStart,
    Stop,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum HookHandlerType {
    Command,
    Prompt,
    Agent,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum HookExecutionMode {
    Sync,
    Async,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum HookScope {
    Thread,
    Turn,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum HookRunStatus {
    Running,
    Completed,
    Failed,
    Blocked,
    Stopped,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum HookOutputEntryKind {
    Warning,
    Stop,
    Feedback,
    Context,
    Error,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HookOutputEntry {
    pub kind: HookOutputEntryKind,
    pub text: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HookRunSummary {
    pub id: String,
    pub event_name: HookEventName,
    pub handler_type: HookHandlerType,
    pub execution_mode: HookExecutionMode,
    pub scope: HookScope,
    pub source_path: PathBuf,
    pub display_order: i64,
    pub status: HookRunStatus,
    pub status_message: Option<String>,
    pub started_at: i64,
    pub completed_at: Option<i64>,
    pub duration_ms: Option<i64>,
    pub entries: Vec<HookOutputEntry>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TurnStartedNotification {
    pub thread_id: String,
    pub turn: ThreadTurn,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HookStartedNotification {
    pub thread_id: String,
    pub turn_id: Option<String>,
    pub run: HookRunSummary,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TurnCompletedNotification {
    pub thread_id: String,
    pub turn: ThreadTurn,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HookCompletedNotification {
    pub thread_id: String,
    pub turn_id: Option<String>,
    pub run: HookRunSummary,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ThreadTurn {
    pub id: String,
    #[serde(default)]
    pub items: Vec<JsonValue>,
    pub status: TurnStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<TurnError>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TurnStatus {
    Completed,
    Interrupted,
    Failed,
    InProgress,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TurnError {
    pub message: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_upstream_codex_error_info_opt"
    )]
    pub codex_error_info: Option<CodexErrorInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub additional_details: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ErrorNotification {
    pub error: TurnError,
    pub will_retry: bool,
    pub thread_id: String,
    pub turn_id: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ThreadTokenUsageUpdatedNotification {
    pub thread_id: String,
    pub turn_id: String,
    pub token_usage: ThreadTokenUsage,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ThreadTokenUsage {
    pub total: TokenUsageBreakdown,
    pub last: TokenUsageBreakdown,
    pub model_context_window: Option<i64>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsageBreakdown {
    pub total_tokens: i64,
    pub input_tokens: i64,
    pub cached_input_tokens: i64,
    pub output_tokens: i64,
    pub reasoning_output_tokens: i64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentMessageDeltaNotification {
    pub thread_id: String,
    pub turn_id: String,
    pub item_id: String,
    pub delta: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PlanDeltaNotification {
    pub thread_id: String,
    pub turn_id: String,
    pub item_id: String,
    pub delta: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReasoningSummaryTextDeltaNotification {
    pub thread_id: String,
    pub turn_id: String,
    pub item_id: String,
    pub delta: String,
    pub summary_index: i64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReasoningTextDeltaNotification {
    pub thread_id: String,
    pub turn_id: String,
    pub item_id: String,
    pub delta: String,
    pub content_index: i64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TerminalInteractionNotification {
    pub thread_id: String,
    pub turn_id: String,
    pub item_id: String,
    pub process_id: String,
    pub stdin: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ItemStartedNotification {
    pub thread_id: String,
    pub turn_id: String,
    pub item: JsonValue,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum GuardianApprovalReviewStatus {
    InProgress,
    Approved,
    Denied,
    Aborted,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum GuardianRiskLevel {
    Low,
    Medium,
    High,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GuardianApprovalReview {
    pub status: GuardianApprovalReviewStatus,
    #[serde(alias = "risk_score")]
    pub risk_score: Option<u8>,
    #[serde(alias = "risk_level")]
    pub risk_level: Option<GuardianRiskLevel>,
    pub rationale: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ItemGuardianApprovalReviewStartedNotification {
    pub thread_id: String,
    pub turn_id: String,
    pub target_item_id: String,
    pub review: GuardianApprovalReview,
    pub action: Option<JsonValue>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ItemGuardianApprovalReviewCompletedNotification {
    pub thread_id: String,
    pub turn_id: String,
    pub target_item_id: String,
    pub review: GuardianApprovalReview,
    pub action: Option<JsonValue>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ItemCompletedNotification {
    pub thread_id: String,
    pub turn_id: String,
    pub item: JsonValue,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentMessageThreadItem {
    pub id: String,
    pub text: String,
    #[serde(default)]
    pub phase: Option<MessagePhase>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FileChangeThreadItem {
    pub id: String,
    #[serde(default)]
    pub changes: Vec<FileUpdateChange>,
    pub status: PatchApplyStatus,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FileUpdateChange {
    pub path: String,
    pub kind: PatchChangeKind,
    pub diff: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum PatchChangeKind {
    Add,
    Delete,
    Update { move_path: Option<PathBuf> },
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PatchApplyStatus {
    InProgress,
    Completed,
    Failed,
    Declined,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CommandExecutionThreadItem {
    pub id: String,
    pub command: String,
    pub cwd: PathBuf,
    pub process_id: Option<String>,
    pub status: CommandExecutionStatus,
    #[serde(default)]
    pub command_actions: Vec<CommandAction>,
    pub aggregated_output: Option<String>,
    pub exit_code: Option<i32>,
    pub duration_ms: Option<i64>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CommandExecutionStatus {
    InProgress,
    Completed,
    Failed,
    Declined,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum CommandAction {
    Read {
        command: String,
        name: String,
        path: PathBuf,
    },
    ListFiles {
        command: String,
        path: Option<String>,
    },
    Search {
        command: String,
        query: Option<String>,
        path: Option<String>,
    },
    Unknown {
        command: String,
    },
}

#[cfg(test)]
mod tests {
    use codex_protocol::protocol::CodexErrorInfo;
    use pretty_assertions::assert_eq;
    use serde_json::json;

    use super::TurnError;

    #[test]
    fn turn_error_deserializes_known_camel_case_codex_error_info() {
        let error: TurnError = serde_json::from_value(json!({
            "message": "exceeded retry limit, last status: 429 Too Many Requests",
            "codexErrorInfo": {
                "responseTooManyFailedAttempts": {
                    "httpStatusCode": 429
                }
            }
        }))
        .expect("deserialize turn error");

        assert_eq!(
            error.codex_error_info,
            Some(CodexErrorInfo::ResponseTooManyFailedAttempts {
                http_status_code: Some(429),
            })
        );
    }

    #[test]
    fn turn_error_deserializes_server_overloaded_codex_error_info() {
        let error: TurnError = serde_json::from_value(json!({
            "message": "server overloaded",
            "codexErrorInfo": "serverOverloaded"
        }))
        .expect("deserialize turn error");

        assert_eq!(
            error.codex_error_info,
            Some(CodexErrorInfo::ServerOverloaded)
        );
    }

    #[test]
    fn turn_error_ignores_unknown_codex_error_info_variant() {
        let error: TurnError = serde_json::from_value(json!({
            "message": "fatal error",
            "codexErrorInfo": {
                "brandNewProblem": {
                    "httpStatusCode": 503
                }
            }
        }))
        .expect("deserialize turn error");

        assert_eq!(error.message, "fatal error");
        assert_eq!(error.codex_error_info, None);
    }
}
