//! Potter.ProjectStop hook integration.
//!
//! This module lives in the workflow layer because it needs access to `potter-rollout.jsonl`
//! parsing and to upstream rollout JSONL files for extracting round summaries.

use std::path::Path;

use anyhow::Context;
use codex_hooks::Hooks;
use codex_hooks::HooksConfig;
use codex_hooks::ProjectStopRequest;
use codex_protocol::protocol::Event;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::HookStartedEvent;
use codex_protocol::protocol::PotterProjectOutcome;
use codex_protocol::protocol::WarningEvent;

struct PreparedProjectStopHookRequest {
    request: ProjectStopRequest,
    warnings: Vec<String>,
}

pub(crate) fn potter_project_stop_reason_code(outcome: &PotterProjectOutcome) -> &'static str {
    match outcome {
        PotterProjectOutcome::Succeeded => "succeeded",
        PotterProjectOutcome::Interrupted => "interrupted",
        PotterProjectOutcome::BudgetExhausted => "budget_exhausted",
        PotterProjectOutcome::TaskFailed { .. } => "task_failed",
        PotterProjectOutcome::Fatal { .. } => "fatal",
    }
}

fn prepare_project_stop_hook_request(
    workdir: &Path,
    progress_file_rel: &Path,
    potter_rollout_path: &Path,
    baseline_round_count: usize,
    stop_reason_code: &'static str,
) -> anyhow::Result<PreparedProjectStopHookRequest> {
    let progress_file_path = workdir.join(progress_file_rel);
    let project_dir = progress_file_path
        .parent()
        .context("derive project_dir from progress file path")?
        .to_path_buf();

    let potter_lines = crate::workflow::rollout::read_lines(potter_rollout_path)
        .with_context(|| format!("read {}", potter_rollout_path.display()))?;
    let index = crate::workflow::rollout_resume_index::build_resume_index(&potter_lines)
        .with_context(|| format!("parse {}", potter_rollout_path.display()))?;

    let mut all_session_ids = Vec::new();
    let mut all_assistant_messages = Vec::new();

    for round in &index.completed_rounds {
        let (thread_id, rollout_path) = match &round.configured {
            Some(cfg) => (Some(cfg.thread_id), Some(&cfg.rollout_path)),
            None => (None, None),
        };

        all_session_ids.push(thread_id.map(|id| id.to_string()).unwrap_or_default());
        all_assistant_messages.push(
            rollout_path
                .map(|rollout_path| {
                    crate::workflow::replay_session_config::resolve_rollout_path_for_replay(
                        workdir,
                        rollout_path,
                    )
                })
                .and_then(|abs| {
                    crate::workflow::projects_overlay_details::read_final_agent_message_from_rollout(
                        &abs,
                    )
                    .ok()
                })
                .and_then(|(_, message)| message)
                .unwrap_or_default(),
        );
    }

    if let Some(unfinished) = &index.unfinished_round {
        all_session_ids.push(unfinished.thread_id.to_string());
        let abs = crate::workflow::replay_session_config::resolve_rollout_path_for_replay(
            workdir,
            &unfinished.rollout_path,
        );
        all_assistant_messages.push(
            crate::workflow::projects_overlay_details::read_final_agent_message_from_rollout(&abs)
                .ok()
                .and_then(|(_, message)| message)
                .unwrap_or_default(),
        );
    }

    let mut warnings = Vec::new();
    let baseline_round_count = if baseline_round_count > all_session_ids.len() {
        warnings.push(format!(
            "Potter.ProjectStop hooks: baseline round count {baseline_round_count} exceeds recorded rounds {}; treating as empty new_* window",
            all_session_ids.len()
        ));
        all_session_ids.len()
    } else {
        baseline_round_count
    };

    let new_session_ids = all_session_ids
        .get(baseline_round_count..)
        .unwrap_or_default()
        .to_vec();
    let new_assistant_messages = all_assistant_messages
        .get(baseline_round_count..)
        .unwrap_or_default()
        .to_vec();

    Ok(PreparedProjectStopHookRequest {
        request: ProjectStopRequest {
            project_dir,
            project_file_path: progress_file_path,
            cwd: workdir.to_path_buf(),
            user_prompt: index.project_started.user_message.unwrap_or_default(),
            all_session_ids,
            new_session_ids,
            all_assistant_messages,
            new_assistant_messages,
            stop_reason_code: stop_reason_code.to_string(),
        },
        warnings,
    })
}

pub(crate) async fn build_project_stop_hook_events(
    workdir: &Path,
    progress_file_rel: &Path,
    potter_rollout_path: &Path,
    baseline_round_count: usize,
    stop_reason_code: &'static str,
    codex_home_dir: Option<&Path>,
) -> Vec<Event> {
    let hooks = Hooks::new(HooksConfig {
        cwd: Some(workdir.to_path_buf()),
        codex_home_dir: codex_home_dir.map(|dir| dir.to_path_buf()),
        ..HooksConfig::default()
    });

    let mut events = Vec::new();

    for warning in hooks.startup_warnings() {
        events.push(Event {
            id: "".to_string(),
            msg: EventMsg::Warning(WarningEvent {
                message: warning.clone(),
            }),
        });
    }

    let progress_file_path = workdir.join(progress_file_rel);
    let project_dir = match progress_file_path.parent() {
        Some(parent) => parent.to_path_buf(),
        None => {
            events.push(Event {
                id: "".to_string(),
                msg: EventMsg::Warning(WarningEvent {
                    message: format!(
                        "Failed to derive project directory from progress file path: {}",
                        progress_file_path.display()
                    ),
                }),
            });
            return events;
        }
    };

    // ProjectStop does not support matchers, so we can check whether any handlers exist without
    // scanning `potter-rollout.jsonl` first.
    let stub_request = ProjectStopRequest {
        project_dir,
        project_file_path: progress_file_path,
        cwd: workdir.to_path_buf(),
        user_prompt: String::new(),
        all_session_ids: Vec::new(),
        new_session_ids: Vec::new(),
        all_assistant_messages: Vec::new(),
        new_assistant_messages: Vec::new(),
        stop_reason_code: stop_reason_code.to_string(),
    };

    let preview_runs = hooks.preview_project_stop(&stub_request);
    if preview_runs.is_empty() {
        return events;
    }

    let prepared = match prepare_project_stop_hook_request(
        workdir,
        progress_file_rel,
        potter_rollout_path,
        baseline_round_count,
        stop_reason_code,
    ) {
        Ok(prepared) => prepared,
        Err(err) => {
            events.push(Event {
                id: "".to_string(),
                msg: EventMsg::Warning(WarningEvent {
                    message: format!("Failed to prepare Potter.ProjectStop hooks: {err:#}"),
                }),
            });
            return events;
        }
    };

    for warning in prepared.warnings {
        events.push(Event {
            id: "".to_string(),
            msg: EventMsg::Warning(WarningEvent { message: warning }),
        });
    }

    for run in preview_runs {
        events.push(Event {
            id: "".to_string(),
            msg: EventMsg::HookStarted(HookStartedEvent { turn_id: None, run }),
        });
    }

    let hook_outcome = hooks.run_project_stop(prepared.request).await;
    for completed in hook_outcome.hook_events {
        events.push(Event {
            id: "".to_string(),
            msg: EventMsg::HookCompleted(completed),
        });
    }

    events
}
