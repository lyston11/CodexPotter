use std::path::PathBuf;

use codex_protocol::protocol::HookCompletedEvent;

/// Input payload for the `Potter.ProjectStop` hook event.
///
/// This event is emitted when a CodexPotter project stops (success, interruption, budget
/// exhaustion, or failure). Lists are ordered by the project round/session sequence.
///
/// `new_*` slices contain only items for rounds completed in the current iteration window (i.e.
/// since the most recent `codex-potter resume` boundary). When resume continues an unfinished
/// round, the reused thread id still appears in `new_session_ids` because that round completed in
/// the current window.
#[derive(Debug, Clone)]
pub struct ProjectStopRequest {
    /// Absolute path to the project directory containing `MAIN.md`.
    pub project_dir: PathBuf,
    /// Absolute path to the project progress file (`MAIN.md`).
    pub project_file_path: PathBuf,
    /// Working directory used for hook discovery and as the hook process CWD.
    pub cwd: PathBuf,
    /// Original user prompt captured when the project started (may be empty).
    pub user_prompt: String,
    /// Session IDs for all rounds in the project (may contain empty entries for malformed logs).
    pub all_session_ids: Vec<String>,
    /// Session IDs for rounds completed since the resume boundary.
    ///
    /// This may include a reused thread id when resume continues an unfinished round.
    pub new_session_ids: Vec<String>,
    /// Final assistant messages for all rounds in the project (best-effort; may contain empty entries).
    pub all_assistant_messages: Vec<String>,
    /// Final assistant messages for rounds completed since the resume boundary.
    pub new_assistant_messages: Vec<String>,
    /// Stable stop reason code (e.g. `succeeded`, `interrupted`, `budget_exhausted`).
    pub stop_reason_code: String,
}

/// Outcome of running `Potter.ProjectStop` hooks.
#[derive(Debug)]
pub struct ProjectStopOutcome {
    /// Completed hook runs emitted for this project-stop execution window.
    pub hook_events: Vec<HookCompletedEvent>,
}
