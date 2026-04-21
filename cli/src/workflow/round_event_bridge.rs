//! Backend event bridge for persistence and synthetic markers.
//!
//! While a round is running, CodexPotter forwards backend `EventMsg` items to the UI. This bridge
//! observes the same events to:
//! - Record `RoundConfigured` / terminal `RoundFinished` (and optional `ProjectSucceeded`)
//!   entries into `potter-rollout.jsonl`.
//! - Inject a `PotterProjectSucceeded` event into the UI stream when `finite_incantatem: true` is
//!   set in the progress file and the current round finishes successfully.
//! - Inject a `PotterProjectBudgetExhausted` event into the UI stream when the last budgeted
//!   round finishes successfully without `finite_incantatem: true`.
//!
//! The bridge is designed to be strict: persistence failures are treated as fatal so resume never
//! reads a partially diverged log.

use std::path::PathBuf;
use std::time::Instant;

use anyhow::Context;
use codex_protocol::protocol::Event;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::PotterRoundOutcome;
use codex_protocol::protocol::SessionConfiguredEvent;

/// Configuration for [`PotterRoundEventBridge`].
#[derive(Debug, Clone)]
pub struct PotterRoundEventBridgeConfig {
    pub record_round_configured: bool,

    pub workdir: PathBuf,
    pub progress_file_rel: PathBuf,
    /// Number of completed rounds that existed before the current iteration window began.
    ///
    /// For fresh projects this is 0. For resumed projects, this is the number of completed rounds
    /// recorded in `potter-rollout.jsonl` at the time of resume.
    pub baseline_round_count: usize,
    /// Override Codex home directory used for `hooks.json` discovery.
    ///
    /// When unset, hook discovery follows the same rules as upstream Codex.
    pub hooks_codex_home_dir: Option<PathBuf>,
    pub potter_xmodel_runtime: bool,
    pub user_prompt_file: PathBuf,
    pub git_commit_start: String,
    pub potter_rollout_path: PathBuf,
    pub project_started_at: Instant,
    pub round_current: u32,
    pub round_total: u32,
    /// Number of rounds executed in the current iteration window, including the active round.
    ///
    /// This value is used for summary markers (`PotterProjectSucceeded` /
    /// `PotterProjectBudgetExhausted`) and for the persisted `ProjectSucceeded` rollout entry. It
    /// intentionally resets when a project is resumed so the round count matches the elapsed
    /// duration for the current iteration window.
    pub project_rounds_run: u32,
}

/// Observes backend `EventMsg` items and produces:
/// - persisted `potter-rollout.jsonl` boundaries, and
/// - optional synthetic project markers (success / budget exhausted), and
/// - optional hook execution events (`Potter.ProjectStop`).
#[derive(Debug, Clone)]
pub struct PotterRoundEventBridge {
    workdir: PathBuf,
    progress_file_rel: PathBuf,
    baseline_round_count: usize,
    hooks_codex_home_dir: Option<PathBuf>,
    potter_xmodel_runtime: bool,
    user_prompt_file: PathBuf,
    git_commit_start: String,
    potter_rollout_path: PathBuf,
    project_started_at: Instant,
    round_current: u32,
    round_total: u32,
    project_rounds_run: u32,
    has_recorded_round_configured: bool,
    session_model: Option<String>,
}

impl PotterRoundEventBridge {
    /// Create a new bridge for an active round.
    pub fn new(config: PotterRoundEventBridgeConfig) -> Self {
        Self {
            has_recorded_round_configured: !config.record_round_configured,
            workdir: config.workdir,
            progress_file_rel: config.progress_file_rel,
            baseline_round_count: config.baseline_round_count,
            hooks_codex_home_dir: config.hooks_codex_home_dir,
            potter_xmodel_runtime: config.potter_xmodel_runtime,
            user_prompt_file: config.user_prompt_file,
            git_commit_start: config.git_commit_start,
            potter_rollout_path: config.potter_rollout_path,
            project_started_at: config.project_started_at,
            round_current: config.round_current,
            round_total: config.round_total,
            project_rounds_run: config.project_rounds_run,
            session_model: None,
        }
    }

    /// Observe one backend event and return any synthetic events to inject into the UI stream.
    pub async fn observe_backend_event(&mut self, event: &Event) -> anyhow::Result<Vec<Event>> {
        if let EventMsg::SessionConfigured(cfg) = &event.msg {
            self.session_model = Some(cfg.model.clone());
        }

        if !self.has_recorded_round_configured
            && let EventMsg::SessionConfigured(cfg) = &event.msg
        {
            self.has_recorded_round_configured = true;
            self.record_round_configured(cfg)
                .context("record potter-rollout round_configured")?;
        }

        let mut injected = Vec::new();
        let mut should_run_project_stop_hook = None;
        if let EventMsg::PotterRoundFinished { outcome } = &event.msg {
            if matches!(outcome, PotterRoundOutcome::Interrupted) {
                // Live ESC interruptions do not immediately finalize the round in
                // `potter-rollout.jsonl`. The round stays open until the user resolves the paused
                // project: `continue iterate` reuses the same thread and later appends a terminal
                // `RoundFinished`, while `stop iterate` records `RoundFinished(Interrupted)` when
                // the stop action is confirmed.
                return Ok(Vec::new());
            }

            if matches!(outcome, PotterRoundOutcome::Completed) {
                let stop_due_to_finite_incantatem =
                    crate::workflow::project::progress_file_has_finite_incantatem_true_after_completed_round(
                        &self.workdir,
                        &self.progress_file_rel,
                    )
                    .context("check progress file finite_incantatem")?;

                if stop_due_to_finite_incantatem {
                    let potter_xmodel_enabled =
                        crate::workflow::project::effective_potter_xmodel_enabled(
                            &self.workdir,
                            &self.progress_file_rel,
                            self.potter_xmodel_runtime,
                        )
                        .context("read potter xmodel mode")?;

                    let should_emit_project_succeeded =
                        crate::workflow::potter_xmodel::should_emit_project_succeeded(
                            potter_xmodel_enabled,
                            self.session_model.as_deref(),
                        );
                    if should_emit_project_succeeded {
                        let git_commit_end =
                            crate::workflow::project::resolve_git_commit(&self.workdir);
                        crate::workflow::rollout::append_line(
                            &self.potter_rollout_path,
                            &crate::workflow::rollout::PotterRolloutLine::ProjectSucceeded {
                                rounds: self.project_rounds_run,
                                duration_secs: self.project_started_at.elapsed().as_secs(),
                                user_prompt_file: self.user_prompt_file.clone(),
                                git_commit_start: self.git_commit_start.clone(),
                                git_commit_end: git_commit_end.clone(),
                            },
                        )
                        .context("append potter-rollout project_succeeded")?;

                        injected.push(Event {
                            id: "".to_string(),
                            msg: EventMsg::PotterProjectSucceeded {
                                rounds: self.project_rounds_run,
                                duration: self.project_started_at.elapsed(),
                                user_prompt_file: self.user_prompt_file.clone(),
                                git_commit_start: self.git_commit_start.clone(),
                                git_commit_end,
                            },
                        });
                        should_run_project_stop_hook = Some("succeeded");
                    }
                } else if self.round_current == self.round_total {
                    let git_commit_end =
                        crate::workflow::project::resolve_git_commit(&self.workdir);
                    injected.push(Event {
                        id: "".to_string(),
                        msg: EventMsg::PotterProjectBudgetExhausted {
                            rounds: self.project_rounds_run,
                            duration: self.project_started_at.elapsed(),
                            user_prompt_file: self.user_prompt_file.clone(),
                            git_commit_start: self.git_commit_start.clone(),
                            git_commit_end,
                        },
                    });
                    should_run_project_stop_hook = Some("budget_exhausted");
                }
            } else if matches!(outcome, PotterRoundOutcome::UserRequested) {
                should_run_project_stop_hook = Some("fatal");
            } else if matches!(outcome, PotterRoundOutcome::TaskFailed { .. }) {
                should_run_project_stop_hook = Some("task_failed");
            } else if matches!(outcome, PotterRoundOutcome::Fatal { .. })
                && self.round_current == self.round_total
            {
                // Fatal rounds only terminate the project when no later budgeted round exists to
                // recover from transient failures.
                should_run_project_stop_hook = Some("fatal");
            }

            crate::workflow::rollout::append_line(
                &self.potter_rollout_path,
                &crate::workflow::rollout::PotterRolloutLine::RoundFinished {
                    outcome: outcome.clone(),
                },
            )
            .context("append potter-rollout round_finished")?;

            if let Some(stop_reason_code) = should_run_project_stop_hook {
                let mut hook_events =
                    crate::workflow::project_stop_hooks::build_project_stop_hook_events(
                        &self.workdir,
                        &self.progress_file_rel,
                        &self.potter_rollout_path,
                        self.baseline_round_count,
                        stop_reason_code,
                        self.hooks_codex_home_dir.as_deref(),
                    )
                    .await;
                injected.append(&mut hook_events);
            }
        }

        Ok(injected)
    }

    fn record_round_configured(&self, cfg: &SessionConfiguredEvent) -> anyhow::Result<()> {
        let (rollout_path, rollout_path_raw, rollout_base_dir) =
            crate::workflow::rollout::resolve_rollout_path_for_recording(
                cfg.rollout_path.clone(),
                &self.workdir,
            );
        crate::workflow::rollout::append_line(
            &self.potter_rollout_path,
            &crate::workflow::rollout::PotterRolloutLine::RoundConfigured {
                thread_id: cfg.session_id,
                rollout_path,
                service_tier: cfg.service_tier,
                rollout_path_raw,
                rollout_base_dir,
            },
        )
        .context("append potter-rollout round_configured")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use codex_protocol::ThreadId;
    use pretty_assertions::assert_eq;
    use std::path::Path;

    fn write_progress_file(workdir: &Path, progress_file_rel: &Path, finite: bool) {
        write_progress_file_with_potter_xmodel(workdir, progress_file_rel, finite, false);
    }

    fn write_progress_file_with_potter_xmodel(
        workdir: &Path,
        progress_file_rel: &Path,
        finite: bool,
        potter_xmodel: bool,
    ) {
        let progress_file = workdir.join(progress_file_rel);
        std::fs::create_dir_all(progress_file.parent().expect("parent")).expect("mkdir");
        std::fs::write(
            &progress_file,
            format!(
                r#"---
finite_incantatem: {finite}
potter.xmodel: {potter_xmodel}
---

# Overall Goal
"#
            ),
        )
        .expect("write progress file");
    }

    fn session_configured_event(
        workdir: &Path,
        rollout_path: PathBuf,
        service_tier: Option<codex_protocol::protocol::ServiceTier>,
        model: &str,
    ) -> Event {
        Event {
            id: "event_1".to_string(),
            msg: EventMsg::SessionConfigured(SessionConfiguredEvent {
                session_id: codex_protocol::ThreadId::from_string(
                    "019ca423-63d9-7641-ae83-db060ad3c000",
                )
                .expect("thread id"),
                forked_from_id: None,
                model: model.to_string(),
                model_provider_id: "test".to_string(),
                service_tier,
                cwd: workdir.to_path_buf(),
                reasoning_effort: None,
                history_log_id: 0,
                history_entry_count: 0,
                initial_messages: None,
                rollout_path,
            }),
        }
    }

    fn write_upstream_rollout_final_answer(path: &Path, message: &str) {
        let value = serde_json::json!({
            "timestamp": "2026-03-01T00:00:00.000Z",
            "type": "event_msg",
            "payload": {
                "type": "agent_message",
                "message": message,
                "phase": "final_answer",
            }
        });
        std::fs::write(path, format!("{value}\n")).expect("write upstream rollout");
    }

    fn append_completed_round(
        potter_rollout_path: &Path,
        round_current: u32,
        round_total: u32,
        thread_id: ThreadId,
        rollout_path: &Path,
    ) {
        crate::workflow::rollout::append_line(
            potter_rollout_path,
            &crate::workflow::rollout::PotterRolloutLine::RoundStarted {
                current: round_current,
                total: round_total,
            },
        )
        .expect("append round_started");
        crate::workflow::rollout::append_line(
            potter_rollout_path,
            &crate::workflow::rollout::PotterRolloutLine::RoundConfigured {
                thread_id,
                rollout_path: rollout_path.to_path_buf(),
                service_tier: None,
                rollout_path_raw: None,
                rollout_base_dir: None,
            },
        )
        .expect("append round_configured");
        crate::workflow::rollout::append_line(
            potter_rollout_path,
            &crate::workflow::rollout::PotterRolloutLine::RoundFinished {
                outcome: PotterRoundOutcome::Completed,
            },
        )
        .expect("append round_finished");
    }

    #[cfg(not(windows))]
    #[tokio::test]
    async fn project_stop_hook_runs_on_project_succeeded_and_slices_new_rounds_on_resume() {
        let dir = tempfile::tempdir().expect("tempdir");
        let workdir = dir.path();
        let hooks_codex_home_dir = workdir.join("codex-home");
        std::fs::create_dir_all(&hooks_codex_home_dir).expect("create hooks codex home dir");

        let progress_file_rel = PathBuf::from(".codexpotter/projects/2026/04/20/1/MAIN.md");
        write_progress_file(workdir, &progress_file_rel, true);

        let potter_rollout_path = workdir.join("potter-rollout.jsonl");
        crate::workflow::rollout::append_line(
            &potter_rollout_path,
            &crate::workflow::rollout::PotterRolloutLine::ProjectStarted {
                user_message: Some("hello from user".to_string()),
                user_prompt_file: progress_file_rel.clone(),
            },
        )
        .expect("append project_started");

        let thread_id_1 =
            ThreadId::from_string("019ca423-63d9-7641-ae83-db060ad3c111").expect("thread id 1");
        let thread_id_2 =
            ThreadId::from_string("019ca423-63d9-7641-ae83-db060ad3c222").expect("thread id 2");
        let thread_id_3 =
            ThreadId::from_string("019ca423-63d9-7641-ae83-db060ad3c333").expect("thread id 3");

        let upstream_1 = workdir.join("upstream-1.jsonl");
        let upstream_2 = workdir.join("upstream-2.jsonl");
        let upstream_3 = workdir.join("upstream-3.jsonl");
        write_upstream_rollout_final_answer(&upstream_1, "round 1 final");
        write_upstream_rollout_final_answer(&upstream_2, "round 2 final");
        write_upstream_rollout_final_answer(&upstream_3, "round 3 final");

        let upstream_1 = upstream_1.canonicalize().expect("canonical upstream 1");
        let upstream_2 = upstream_2.canonicalize().expect("canonical upstream 2");
        let upstream_3 = upstream_3.canonicalize().expect("canonical upstream 3");

        append_completed_round(&potter_rollout_path, 1, 3, thread_id_1, &upstream_1);
        append_completed_round(&potter_rollout_path, 2, 3, thread_id_2, &upstream_2);

        crate::workflow::rollout::append_line(
            &potter_rollout_path,
            &crate::workflow::rollout::PotterRolloutLine::RoundStarted {
                current: 3,
                total: 3,
            },
        )
        .expect("append round 3 started");

        let hook_output_path = workdir.join("hook-input.json");
        let hooks_json = serde_json::json!({
            "hooks": {
                "Potter.ProjectStop": [{
                    "hooks": [{
                        "type": "command",
                        "command": format!("cat > '{}'", hook_output_path.display()),
                    }],
                }],
            },
        });
        std::fs::write(
            hooks_codex_home_dir.join("hooks.json"),
            hooks_json.to_string(),
        )
        .expect("write hooks.json");

        let mut bridge = PotterRoundEventBridge::new(PotterRoundEventBridgeConfig {
            record_round_configured: true,
            workdir: workdir.to_path_buf(),
            progress_file_rel: progress_file_rel.clone(),
            baseline_round_count: 2,
            hooks_codex_home_dir: Some(hooks_codex_home_dir),
            potter_xmodel_runtime: false,
            user_prompt_file: progress_file_rel.clone(),
            git_commit_start: "start".to_string(),
            potter_rollout_path: potter_rollout_path.clone(),
            project_started_at: Instant::now(),
            round_current: 3,
            round_total: 3,
            project_rounds_run: 1,
        });

        let ev = Event {
            id: "event_1".to_string(),
            msg: EventMsg::SessionConfigured(SessionConfiguredEvent {
                session_id: thread_id_3,
                forked_from_id: None,
                model: "test".to_string(),
                model_provider_id: "test".to_string(),
                service_tier: None,
                cwd: workdir.to_path_buf(),
                reasoning_effort: None,
                history_log_id: 0,
                history_entry_count: 0,
                initial_messages: None,
                rollout_path: upstream_3.clone(),
            }),
        };
        bridge
            .observe_backend_event(&ev)
            .await
            .expect("observe configured");

        let finished = Event {
            id: "event_2".to_string(),
            msg: EventMsg::PotterRoundFinished {
                outcome: PotterRoundOutcome::Completed,
            },
        };
        let injected = bridge
            .observe_backend_event(&finished)
            .await
            .expect("observe finished");

        assert!(matches!(
            injected.first().map(|event| &event.msg),
            Some(EventMsg::PotterProjectSucceeded { rounds: 1, .. })
        ));
        assert!(
            injected
                .iter()
                .any(|event| matches!(event.msg, EventMsg::HookStarted(_))),
            "expected HookStarted event, got {injected:?}"
        );
        assert!(
            injected
                .iter()
                .any(|event| matches!(event.msg, EventMsg::HookCompleted(_))),
            "expected HookCompleted event, got {injected:?}"
        );

        let payload: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(&hook_output_path).expect("read hook input"),
        )
        .expect("parse hook input json");

        let expected_project_file_path = workdir.join(&progress_file_rel);
        let expected_project_dir = expected_project_file_path
            .parent()
            .expect("project dir")
            .to_path_buf();
        let expected_project_dir = expected_project_dir.to_string_lossy().to_string();
        let expected_project_file_path = expected_project_file_path.to_string_lossy().to_string();
        let expected_workdir = workdir.to_string_lossy().to_string();
        assert_eq!(
            payload
                .get("project_dir")
                .and_then(serde_json::Value::as_str),
            Some(expected_project_dir.as_str())
        );
        assert_eq!(
            payload
                .get("project_file_path")
                .and_then(serde_json::Value::as_str),
            Some(expected_project_file_path.as_str())
        );
        assert_eq!(
            payload.get("cwd").and_then(serde_json::Value::as_str),
            Some(expected_workdir.as_str())
        );

        assert_eq!(
            payload
                .get("hook_event_name")
                .and_then(serde_json::Value::as_str),
            Some("Potter.ProjectStop")
        );
        assert_eq!(
            payload
                .get("user_prompt")
                .and_then(serde_json::Value::as_str),
            Some("hello from user")
        );
        assert_eq!(
            payload
                .get("stop_reason_code")
                .and_then(serde_json::Value::as_str),
            Some("succeeded")
        );

        assert_eq!(
            payload
                .get("all_session_ids")
                .and_then(serde_json::Value::as_array),
            Some(&vec![
                serde_json::Value::String(thread_id_1.to_string()),
                serde_json::Value::String(thread_id_2.to_string()),
                serde_json::Value::String(thread_id_3.to_string()),
            ])
        );
        assert_eq!(
            payload
                .get("new_session_ids")
                .and_then(serde_json::Value::as_array),
            Some(&vec![serde_json::Value::String(thread_id_3.to_string())])
        );
        assert_eq!(
            payload
                .get("all_assistant_messages")
                .and_then(serde_json::Value::as_array),
            Some(&vec![
                serde_json::Value::String("round 1 final".to_string()),
                serde_json::Value::String("round 2 final".to_string()),
                serde_json::Value::String("round 3 final".to_string()),
            ])
        );
        assert_eq!(
            payload
                .get("new_assistant_messages")
                .and_then(serde_json::Value::as_array),
            Some(&vec![serde_json::Value::String(
                "round 3 final".to_string()
            )])
        );
    }

    #[cfg(not(windows))]
    #[tokio::test]
    async fn project_stop_hook_runs_on_budget_exhausted() {
        let dir = tempfile::tempdir().expect("tempdir");
        let workdir = dir.path();
        let hooks_codex_home_dir = workdir.join("codex-home");
        std::fs::create_dir_all(&hooks_codex_home_dir).expect("create hooks codex home dir");

        let progress_file_rel = PathBuf::from(".codexpotter/projects/2026/04/20/2/MAIN.md");
        write_progress_file(workdir, &progress_file_rel, false);

        let potter_rollout_path = workdir.join("potter-rollout.jsonl");
        crate::workflow::rollout::append_line(
            &potter_rollout_path,
            &crate::workflow::rollout::PotterRolloutLine::ProjectStarted {
                user_message: Some("hello".to_string()),
                user_prompt_file: progress_file_rel.clone(),
            },
        )
        .expect("append project_started");
        crate::workflow::rollout::append_line(
            &potter_rollout_path,
            &crate::workflow::rollout::PotterRolloutLine::RoundStarted {
                current: 1,
                total: 1,
            },
        )
        .expect("append round_started");

        let thread_id =
            ThreadId::from_string("019ca423-63d9-7641-ae83-db060ad3c444").expect("thread id");
        let upstream = workdir.join("upstream.jsonl");
        write_upstream_rollout_final_answer(&upstream, "final");
        let upstream = upstream.canonicalize().expect("canonical upstream");

        let hook_output_path = workdir.join("hook-input.json");
        let hooks_json = serde_json::json!({
            "hooks": {
                "Potter.ProjectStop": [{
                    "hooks": [{
                        "type": "command",
                        "command": format!("cat > '{}'", hook_output_path.display()),
                    }],
                }],
            },
        });
        std::fs::write(
            hooks_codex_home_dir.join("hooks.json"),
            hooks_json.to_string(),
        )
        .expect("write hooks.json");

        let mut bridge = PotterRoundEventBridge::new(PotterRoundEventBridgeConfig {
            record_round_configured: true,
            workdir: workdir.to_path_buf(),
            progress_file_rel: progress_file_rel.clone(),
            baseline_round_count: 0,
            hooks_codex_home_dir: Some(hooks_codex_home_dir),
            potter_xmodel_runtime: false,
            user_prompt_file: progress_file_rel.clone(),
            git_commit_start: "start".to_string(),
            potter_rollout_path: potter_rollout_path.clone(),
            project_started_at: Instant::now(),
            round_current: 1,
            round_total: 1,
            project_rounds_run: 1,
        });

        let ev = Event {
            id: "event_1".to_string(),
            msg: EventMsg::SessionConfigured(SessionConfiguredEvent {
                session_id: thread_id,
                forked_from_id: None,
                model: "test".to_string(),
                model_provider_id: "test".to_string(),
                service_tier: None,
                cwd: workdir.to_path_buf(),
                reasoning_effort: None,
                history_log_id: 0,
                history_entry_count: 0,
                initial_messages: None,
                rollout_path: upstream.clone(),
            }),
        };
        bridge
            .observe_backend_event(&ev)
            .await
            .expect("observe configured");

        let finished = Event {
            id: "event_2".to_string(),
            msg: EventMsg::PotterRoundFinished {
                outcome: PotterRoundOutcome::Completed,
            },
        };
        let injected = bridge
            .observe_backend_event(&finished)
            .await
            .expect("observe finished");

        assert!(matches!(
            injected.first().map(|event| &event.msg),
            Some(EventMsg::PotterProjectBudgetExhausted { rounds: 1, .. })
        ));

        let payload: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(&hook_output_path).expect("read hook input"),
        )
        .expect("parse hook input json");
        assert_eq!(
            payload
                .get("stop_reason_code")
                .and_then(serde_json::Value::as_str),
            Some("budget_exhausted")
        );
    }

    #[tokio::test]
    async fn observe_backend_event_records_round_configured_once() {
        let dir = tempfile::tempdir().expect("tempdir");
        let workdir = dir.path();
        let potter_rollout_path = workdir.join("potter-rollout.jsonl");
        let hooks_codex_home_dir = workdir.join("codex-home");
        std::fs::create_dir_all(&hooks_codex_home_dir).expect("create hooks codex home dir");

        let rollout_path = workdir.join("upstream.jsonl");
        std::fs::write(&rollout_path, "").expect("write upstream rollout");

        let mut bridge = PotterRoundEventBridge::new(PotterRoundEventBridgeConfig {
            record_round_configured: true,
            workdir: workdir.to_path_buf(),
            progress_file_rel: PathBuf::from(".codexpotter/projects/2026/03/04/1/MAIN.md"),
            baseline_round_count: 0,
            hooks_codex_home_dir: Some(hooks_codex_home_dir),
            potter_xmodel_runtime: false,
            user_prompt_file: PathBuf::from(".codexpotter/projects/2026/03/04/1/MAIN.md"),
            git_commit_start: "start".to_string(),
            potter_rollout_path: potter_rollout_path.clone(),
            project_started_at: Instant::now(),
            round_current: 1,
            round_total: 10,
            project_rounds_run: 1,
        });

        let ev = session_configured_event(workdir, PathBuf::from("upstream.jsonl"), None, "test");
        bridge.observe_backend_event(&ev).await.expect("observe #1");
        bridge.observe_backend_event(&ev).await.expect("observe #2");

        let lines = crate::workflow::rollout::read_lines(&potter_rollout_path).expect("read");
        assert_eq!(
            lines,
            vec![
                crate::workflow::rollout::PotterRolloutLine::RoundConfigured {
                    thread_id: codex_protocol::ThreadId::from_string(
                        "019ca423-63d9-7641-ae83-db060ad3c000",
                    )
                    .expect("thread id"),
                    rollout_path: rollout_path.canonicalize().expect("canonical rollout"),
                    service_tier: None,
                    rollout_path_raw: None,
                    rollout_base_dir: None,
                }
            ]
        );
    }

    #[tokio::test]
    async fn observe_backend_event_records_round_configured_service_tier() {
        let dir = tempfile::tempdir().expect("tempdir");
        let workdir = dir.path();
        let potter_rollout_path = workdir.join("potter-rollout.jsonl");
        let hooks_codex_home_dir = workdir.join("codex-home");
        std::fs::create_dir_all(&hooks_codex_home_dir).expect("create hooks codex home dir");

        let rollout_path = workdir.join("upstream.jsonl");
        std::fs::write(&rollout_path, "").expect("write upstream rollout");

        let mut bridge = PotterRoundEventBridge::new(PotterRoundEventBridgeConfig {
            record_round_configured: true,
            workdir: workdir.to_path_buf(),
            progress_file_rel: PathBuf::from(".codexpotter/projects/2026/03/04/1/MAIN.md"),
            baseline_round_count: 0,
            hooks_codex_home_dir: Some(hooks_codex_home_dir),
            potter_xmodel_runtime: false,
            user_prompt_file: PathBuf::from(".codexpotter/projects/2026/03/04/1/MAIN.md"),
            git_commit_start: "start".to_string(),
            potter_rollout_path: potter_rollout_path.clone(),
            project_started_at: Instant::now(),
            round_current: 1,
            round_total: 10,
            project_rounds_run: 1,
        });

        let ev = session_configured_event(
            workdir,
            PathBuf::from("upstream.jsonl"),
            Some(codex_protocol::protocol::ServiceTier::Fast),
            "test",
        );
        bridge.observe_backend_event(&ev).await.expect("observe");

        let lines = crate::workflow::rollout::read_lines(&potter_rollout_path).expect("read");
        assert_eq!(
            lines,
            vec![
                crate::workflow::rollout::PotterRolloutLine::RoundConfigured {
                    thread_id: codex_protocol::ThreadId::from_string(
                        "019ca423-63d9-7641-ae83-db060ad3c000",
                    )
                    .expect("thread id"),
                    rollout_path: rollout_path.canonicalize().expect("canonical rollout"),
                    service_tier: Some(codex_protocol::protocol::ServiceTier::Fast),
                    rollout_path_raw: None,
                    rollout_base_dir: None,
                }
            ]
        );
    }

    #[tokio::test]
    async fn observe_backend_event_project_succeeded_injection_respects_finite_incantatem() {
        {
            let dir = tempfile::tempdir().expect("tempdir");
            let workdir = dir.path();
            let progress_file_rel = PathBuf::from(".codexpotter/projects/2026/03/04/1/MAIN.md");
            let hooks_codex_home_dir = workdir.join("codex-home");
            std::fs::create_dir_all(&hooks_codex_home_dir).expect("create hooks codex home dir");
            write_progress_file(workdir, &progress_file_rel, true);

            let potter_rollout_path = workdir.join("potter-rollout.jsonl");
            let mut bridge = PotterRoundEventBridge::new(PotterRoundEventBridgeConfig {
                record_round_configured: false,
                workdir: workdir.to_path_buf(),
                progress_file_rel: progress_file_rel.clone(),
                baseline_round_count: 0,
                hooks_codex_home_dir: Some(hooks_codex_home_dir),
                potter_xmodel_runtime: false,
                user_prompt_file: progress_file_rel.clone(),
                git_commit_start: "start".to_string(),
                potter_rollout_path: potter_rollout_path.clone(),
                project_started_at: Instant::now(),
                round_current: 3,
                round_total: 10,
                project_rounds_run: 3,
            });

            let finished = Event {
                id: "event_2".to_string(),
                msg: EventMsg::PotterRoundFinished {
                    outcome: PotterRoundOutcome::Completed,
                },
            };

            let injected = bridge
                .observe_backend_event(&finished)
                .await
                .expect("observe finished");
            assert!(matches!(
                injected.first().map(|event| &event.msg),
                Some(EventMsg::PotterProjectSucceeded { rounds: 3, .. })
            ));

            let lines = crate::workflow::rollout::read_lines(&potter_rollout_path).expect("read");
            assert_eq!(lines.len(), 2);
            assert!(matches!(
                &lines[0],
                crate::workflow::rollout::PotterRolloutLine::ProjectSucceeded { rounds: 3, .. }
            ));
            assert!(matches!(
                &lines[1],
                crate::workflow::rollout::PotterRolloutLine::RoundFinished {
                    outcome: PotterRoundOutcome::Completed
                }
            ));
        }

        {
            let dir = tempfile::tempdir().expect("tempdir");
            let workdir = dir.path();
            let progress_file_rel = PathBuf::from(".codexpotter/projects/2026/03/04/1/MAIN.md");
            let hooks_codex_home_dir = workdir.join("codex-home");
            std::fs::create_dir_all(&hooks_codex_home_dir).expect("create hooks codex home dir");
            write_progress_file(workdir, &progress_file_rel, false);

            let potter_rollout_path = workdir.join("potter-rollout.jsonl");
            let mut bridge = PotterRoundEventBridge::new(PotterRoundEventBridgeConfig {
                record_round_configured: false,
                workdir: workdir.to_path_buf(),
                progress_file_rel: progress_file_rel.clone(),
                baseline_round_count: 0,
                hooks_codex_home_dir: Some(hooks_codex_home_dir),
                potter_xmodel_runtime: false,
                user_prompt_file: progress_file_rel.clone(),
                git_commit_start: "start".to_string(),
                potter_rollout_path: potter_rollout_path.clone(),
                project_started_at: Instant::now(),
                round_current: 3,
                round_total: 10,
                project_rounds_run: 3,
            });

            let finished = Event {
                id: "event_2".to_string(),
                msg: EventMsg::PotterRoundFinished {
                    outcome: PotterRoundOutcome::Completed,
                },
            };

            let injected = bridge
                .observe_backend_event(&finished)
                .await
                .expect("observe finished");
            assert!(injected.is_empty());

            let lines = crate::workflow::rollout::read_lines(&potter_rollout_path).expect("read");
            assert_eq!(lines.len(), 1);
            assert!(matches!(
                &lines[0],
                crate::workflow::rollout::PotterRolloutLine::RoundFinished {
                    outcome: PotterRoundOutcome::Completed
                }
            ));
        }
    }

    #[tokio::test]
    async fn observe_backend_event_xmodel_gates_project_succeeded_until_gpt_5_4() {
        let finished = Event {
            id: "event_2".to_string(),
            msg: EventMsg::PotterRoundFinished {
                outcome: PotterRoundOutcome::Completed,
            },
        };

        {
            let dir = tempfile::tempdir().expect("tempdir");
            let workdir = dir.path();
            let progress_file_rel = PathBuf::from(".codexpotter/projects/2026/03/04/1/MAIN.md");
            let hooks_codex_home_dir = workdir.join("codex-home");
            std::fs::create_dir_all(&hooks_codex_home_dir).expect("create hooks codex home dir");
            write_progress_file_with_potter_xmodel(workdir, &progress_file_rel, true, true);

            let potter_rollout_path = workdir.join("potter-rollout.jsonl");
            let mut bridge = PotterRoundEventBridge::new(PotterRoundEventBridgeConfig {
                record_round_configured: false,
                workdir: workdir.to_path_buf(),
                progress_file_rel: progress_file_rel.clone(),
                baseline_round_count: 0,
                hooks_codex_home_dir: Some(hooks_codex_home_dir),
                potter_xmodel_runtime: false,
                user_prompt_file: progress_file_rel.clone(),
                git_commit_start: "start".to_string(),
                potter_rollout_path: potter_rollout_path.clone(),
                project_started_at: Instant::now(),
                round_current: 1,
                round_total: 10,
                project_rounds_run: 1,
            });

            bridge
                .observe_backend_event(&session_configured_event(
                    workdir,
                    PathBuf::from("upstream.jsonl"),
                    None,
                    "gpt-5.2",
                ))
                .await
                .expect("observe session configured");

            let injected = bridge
                .observe_backend_event(&finished)
                .await
                .expect("observe finished");
            assert!(injected.is_empty());

            let lines = crate::workflow::rollout::read_lines(&potter_rollout_path).expect("read");
            assert_eq!(lines.len(), 1);
            assert!(matches!(
                &lines[0],
                crate::workflow::rollout::PotterRolloutLine::RoundFinished {
                    outcome: PotterRoundOutcome::Completed
                }
            ));
        }

        {
            let dir = tempfile::tempdir().expect("tempdir");
            let workdir = dir.path();
            let progress_file_rel = PathBuf::from(".codexpotter/projects/2026/03/04/1/MAIN.md");
            let hooks_codex_home_dir = workdir.join("codex-home");
            std::fs::create_dir_all(&hooks_codex_home_dir).expect("create hooks codex home dir");
            write_progress_file(workdir, &progress_file_rel, true);

            let potter_rollout_path = workdir.join("potter-rollout.jsonl");
            let mut bridge = PotterRoundEventBridge::new(PotterRoundEventBridgeConfig {
                record_round_configured: false,
                workdir: workdir.to_path_buf(),
                progress_file_rel: progress_file_rel.clone(),
                baseline_round_count: 0,
                hooks_codex_home_dir: Some(hooks_codex_home_dir),
                potter_xmodel_runtime: true,
                user_prompt_file: progress_file_rel.clone(),
                git_commit_start: "start".to_string(),
                potter_rollout_path: potter_rollout_path.clone(),
                project_started_at: Instant::now(),
                round_current: 1,
                round_total: 10,
                project_rounds_run: 1,
            });

            bridge
                .observe_backend_event(&session_configured_event(
                    workdir,
                    PathBuf::from("upstream.jsonl"),
                    None,
                    "gpt-5.2",
                ))
                .await
                .expect("observe session configured");

            let injected = bridge
                .observe_backend_event(&finished)
                .await
                .expect("observe finished");
            assert!(injected.is_empty());

            let lines = crate::workflow::rollout::read_lines(&potter_rollout_path).expect("read");
            assert_eq!(lines.len(), 1);
            assert!(matches!(
                &lines[0],
                crate::workflow::rollout::PotterRolloutLine::RoundFinished {
                    outcome: PotterRoundOutcome::Completed
                }
            ));
        }
    }

    #[tokio::test]
    async fn observe_backend_event_injects_budget_exhausted_before_round_finished_when_last_round()
    {
        let dir = tempfile::tempdir().expect("tempdir");
        let workdir = dir.path();
        let progress_file_rel = PathBuf::from(".codexpotter/projects/2026/03/04/1/MAIN.md");
        let hooks_codex_home_dir = workdir.join("codex-home");
        std::fs::create_dir_all(&hooks_codex_home_dir).expect("create hooks codex home dir");
        write_progress_file(workdir, &progress_file_rel, false);

        let potter_rollout_path = workdir.join("potter-rollout.jsonl");
        let mut bridge = PotterRoundEventBridge::new(PotterRoundEventBridgeConfig {
            record_round_configured: false,
            workdir: workdir.to_path_buf(),
            progress_file_rel: progress_file_rel.clone(),
            baseline_round_count: 0,
            hooks_codex_home_dir: Some(hooks_codex_home_dir),
            potter_xmodel_runtime: false,
            user_prompt_file: progress_file_rel.clone(),
            git_commit_start: "start".to_string(),
            potter_rollout_path: potter_rollout_path.clone(),
            project_started_at: Instant::now(),
            round_current: 10,
            round_total: 10,
            project_rounds_run: 10,
        });

        let finished = Event {
            id: "event_2".to_string(),
            msg: EventMsg::PotterRoundFinished {
                outcome: PotterRoundOutcome::Completed,
            },
        };

        let injected = bridge
            .observe_backend_event(&finished)
            .await
            .expect("observe finished");
        assert!(matches!(
            injected.first().map(|event| &event.msg),
            Some(EventMsg::PotterProjectBudgetExhausted { rounds: 10, .. })
        ));

        let lines = crate::workflow::rollout::read_lines(&potter_rollout_path).expect("read");
        assert_eq!(lines.len(), 1);
        assert!(matches!(
            &lines[0],
            crate::workflow::rollout::PotterRolloutLine::RoundFinished {
                outcome: PotterRoundOutcome::Completed
            }
        ));
    }

    #[tokio::test]
    async fn observe_backend_event_does_not_record_interrupted_round_finish() {
        let dir = tempfile::tempdir().expect("tempdir");
        let workdir = dir.path();
        let progress_file_rel = PathBuf::from(".codexpotter/projects/2026/03/04/1/MAIN.md");
        let hooks_codex_home_dir = workdir.join("codex-home");
        std::fs::create_dir_all(&hooks_codex_home_dir).expect("create hooks codex home dir");
        write_progress_file(workdir, &progress_file_rel, false);

        let potter_rollout_path = workdir.join("potter-rollout.jsonl");
        let mut bridge = PotterRoundEventBridge::new(PotterRoundEventBridgeConfig {
            record_round_configured: false,
            workdir: workdir.to_path_buf(),
            progress_file_rel: progress_file_rel.clone(),
            baseline_round_count: 0,
            hooks_codex_home_dir: Some(hooks_codex_home_dir),
            potter_xmodel_runtime: false,
            user_prompt_file: progress_file_rel,
            git_commit_start: "start".to_string(),
            potter_rollout_path: potter_rollout_path.clone(),
            project_started_at: Instant::now(),
            round_current: 3,
            round_total: 10,
            project_rounds_run: 3,
        });

        let finished = Event {
            id: "event_2".to_string(),
            msg: EventMsg::PotterRoundFinished {
                outcome: PotterRoundOutcome::Interrupted,
            },
        };

        let injected = bridge
            .observe_backend_event(&finished)
            .await
            .expect("observe finished");
        assert!(injected.is_empty());
        assert!(
            !potter_rollout_path.exists(),
            "interrupted rounds should remain unfinished in potter-rollout"
        );
    }

    #[tokio::test]
    async fn observe_backend_event_errors_when_progress_file_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let workdir = dir.path();
        let hooks_codex_home_dir = workdir.join("codex-home");
        std::fs::create_dir_all(&hooks_codex_home_dir).expect("create hooks codex home dir");

        let potter_rollout_path = workdir.join("potter-rollout.jsonl");
        let progress_file_rel = PathBuf::from(".codexpotter/projects/2026/03/04/1/MAIN.md");

        let mut bridge = PotterRoundEventBridge::new(PotterRoundEventBridgeConfig {
            record_round_configured: false,
            workdir: workdir.to_path_buf(),
            progress_file_rel: progress_file_rel.clone(),
            baseline_round_count: 0,
            hooks_codex_home_dir: Some(hooks_codex_home_dir),
            potter_xmodel_runtime: false,
            user_prompt_file: progress_file_rel,
            git_commit_start: "start".to_string(),
            potter_rollout_path: potter_rollout_path.clone(),
            project_started_at: Instant::now(),
            round_current: 1,
            round_total: 1,
            project_rounds_run: 1,
        });

        let finished = Event {
            id: "event_2".to_string(),
            msg: EventMsg::PotterRoundFinished {
                outcome: PotterRoundOutcome::Completed,
            },
        };

        let err = bridge
            .observe_backend_event(&finished)
            .await
            .expect_err("expected error");
        assert!(
            err.to_string().contains("finite_incantatem"),
            "error should mention finite_incantatem: {err:#}"
        );
        assert!(
            !potter_rollout_path.exists(),
            "should not write rollout on error"
        );
    }

    #[tokio::test]
    async fn observe_backend_event_degrades_when_progress_file_front_matter_is_malformed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let workdir = dir.path();
        let progress_file_rel = PathBuf::from(".codexpotter/projects/2026/03/04/1/MAIN.md");
        let progress_file = workdir.join(&progress_file_rel);
        std::fs::create_dir_all(progress_file.parent().expect("parent")).expect("mkdir");
        std::fs::write(&progress_file, "status: open\nfinite_incantatem: true\n")
            .expect("write progress file");
        let hooks_codex_home_dir = workdir.join("codex-home");
        std::fs::create_dir_all(&hooks_codex_home_dir).expect("create hooks codex home dir");

        let potter_rollout_path = workdir.join("potter-rollout.jsonl");
        let mut bridge = PotterRoundEventBridge::new(PotterRoundEventBridgeConfig {
            record_round_configured: false,
            workdir: workdir.to_path_buf(),
            progress_file_rel: progress_file_rel.clone(),
            baseline_round_count: 0,
            hooks_codex_home_dir: Some(hooks_codex_home_dir),
            potter_xmodel_runtime: false,
            user_prompt_file: progress_file_rel,
            git_commit_start: "start".to_string(),
            potter_rollout_path: potter_rollout_path.clone(),
            project_started_at: Instant::now(),
            round_current: 10,
            round_total: 10,
            project_rounds_run: 10,
        });

        let finished = Event {
            id: "event_2".to_string(),
            msg: EventMsg::PotterRoundFinished {
                outcome: PotterRoundOutcome::Completed,
            },
        };

        let injected = bridge
            .observe_backend_event(&finished)
            .await
            .expect("observe finished");
        assert!(matches!(
            injected.first().map(|event| &event.msg),
            Some(EventMsg::PotterProjectBudgetExhausted { rounds: 10, .. })
        ));

        let lines = crate::workflow::rollout::read_lines(&potter_rollout_path).expect("read");
        assert_eq!(
            lines,
            vec![crate::workflow::rollout::PotterRolloutLine::RoundFinished {
                outcome: PotterRoundOutcome::Completed,
            }]
        );
    }
}
