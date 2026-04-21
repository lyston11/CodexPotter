use std::path::PathBuf;

use codex_protocol::protocol::HookCompletedEvent;
use codex_protocol::protocol::HookEventName;
use codex_protocol::protocol::HookOutputEntry;
use codex_protocol::protocol::HookOutputEntryKind;
use codex_protocol::protocol::HookRunStatus;
use codex_protocol::protocol::HookRunSummary;

use super::common;
use crate::engine::CommandShell;
use crate::engine::ConfiguredHandler;
use crate::engine::command_runner::CommandRunResult;
use crate::engine::dispatcher;
use crate::schema::PotterProjectStopCommandInput;

/// Input payload for the `Potter.ProjectStop` hook event.
///
/// This event is emitted when a CodexPotter project stops (success, interruption, budget
/// exhaustion, or failure). Lists are ordered by the project round/session sequence.
///
/// `new_*` slices contain only items that were created in the current iteration window (i.e. since
/// the most recent `codex-potter resume` boundary).
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
    /// Session IDs created since the resume boundary.
    pub new_session_ids: Vec<String>,
    /// Final assistant messages for all rounds in the project (best-effort; may contain empty entries).
    pub all_assistant_messages: Vec<String>,
    /// Final assistant messages created since the resume boundary.
    pub new_assistant_messages: Vec<String>,
    /// Stable stop reason code (e.g. `succeeded`, `interrupted`, `budget_exhausted`).
    pub stop_reason_code: String,
}

/// Outcome of running `Potter.ProjectStop` hooks.
#[derive(Debug)]
pub struct ProjectStopOutcome {
    pub hook_events: Vec<HookCompletedEvent>,
}

pub(crate) fn preview(
    handlers: &[ConfiguredHandler],
    _request: &ProjectStopRequest,
) -> Vec<HookRunSummary> {
    dispatcher::select_handlers(
        handlers,
        HookEventName::PotterProjectStop,
        /*matcher_input*/ None,
    )
    .into_iter()
    .map(|handler| dispatcher::running_summary(&handler))
    .collect()
}

pub(crate) async fn run(
    handlers: &[ConfiguredHandler],
    shell: &CommandShell,
    request: ProjectStopRequest,
) -> ProjectStopOutcome {
    let matched = dispatcher::select_handlers(
        handlers,
        HookEventName::PotterProjectStop,
        /*matcher_input*/ None,
    );
    if matched.is_empty() {
        return ProjectStopOutcome {
            hook_events: Vec::new(),
        };
    }

    let input_json = match serde_json::to_string(&PotterProjectStopCommandInput {
        project_dir: request.project_dir.display().to_string(),
        project_file_path: request.project_file_path.display().to_string(),
        cwd: request.cwd.display().to_string(),
        hook_event_name: "Potter.ProjectStop".to_string(),
        user_prompt: request.user_prompt,
        all_session_ids: request.all_session_ids,
        new_session_ids: request.new_session_ids,
        all_assistant_messages: request.all_assistant_messages,
        new_assistant_messages: request.new_assistant_messages,
        stop_reason_code: request.stop_reason_code,
    }) {
        Ok(input_json) => input_json,
        Err(error) => {
            return ProjectStopOutcome {
                hook_events: common::serialization_failure_hook_events(
                    matched,
                    None,
                    format!("failed to serialize project stop hook input: {error}"),
                ),
            };
        }
    };

    let results = dispatcher::execute_handlers(
        shell,
        matched,
        input_json,
        request.cwd.as_path(),
        None,
        parse_completed,
    )
    .await;

    ProjectStopOutcome {
        hook_events: results.into_iter().map(|result| result.completed).collect(),
    }
}

fn parse_completed(
    handler: &ConfiguredHandler,
    run_result: CommandRunResult,
    turn_id: Option<String>,
) -> dispatcher::ParsedHandler<()> {
    let mut status = HookRunStatus::Completed;
    let mut entries = Vec::new();

    match run_result.error.as_deref() {
        Some(error) => {
            status = HookRunStatus::Failed;
            entries.push(HookOutputEntry {
                kind: HookOutputEntryKind::Error,
                text: error.to_string(),
            });
        }
        None => match run_result.exit_code {
            Some(0) => {}
            Some(code) => {
                status = HookRunStatus::Failed;
                entries.push(HookOutputEntry {
                    kind: HookOutputEntryKind::Error,
                    text: format!("hook exited with code {code}"),
                });
                if let Some(stderr) = common::trimmed_non_empty(&run_result.stderr) {
                    entries.push(HookOutputEntry {
                        kind: HookOutputEntryKind::Error,
                        text: stderr,
                    });
                }
            }
            None => {
                status = HookRunStatus::Failed;
                entries.push(HookOutputEntry {
                    kind: HookOutputEntryKind::Error,
                    text: "hook exited without an exit code".to_string(),
                });
            }
        },
    }

    dispatcher::ParsedHandler {
        completed: HookCompletedEvent {
            turn_id,
            run: dispatcher::completed_summary(handler, &run_result, status, entries),
        },
        data: (),
    }
}
