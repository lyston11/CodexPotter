//! Headless human-readable transcript rendering for `codex-potter exec`.
//!
//! # Divergence from upstream Codex / interactive TUI
//!
//! This renderer is intentionally append-only:
//! - it preserves the same broad visibility policy as codex-potter's interactive verbosity modes
//! - it does **not** use interactive folding/coalescing, because exec cannot rewrite prior output
//! - it renders CodexPotter round / summary markers as plain text blocks instead of interactive
//!   transcript chrome

use std::io;
use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;
use std::time::Instant;

use codex_protocol::protocol::TokenUsage;
use crossterm::queue;
use crossterm::style::Color as CrosstermColor;
use crossterm::style::Colors;
use crossterm::style::Print;
use crossterm::style::SetAttribute;
use crossterm::style::SetBackgroundColor;
use crossterm::style::SetColors;
use crossterm::style::SetForegroundColor;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::text::Span;

use crate::Verbosity;
use crate::diff_render::create_compact_diff_summary;
use crate::diff_render::display_path_for;
use crate::exec_cell::CommandOutput;
use crate::exec_cell::new_active_exec_command;
use crate::history_cell::HistoryCell;
use crate::history_cell::new_deprecation_notice;
use crate::history_cell::new_elicitation_request_event;
use crate::history_cell::new_error_event;
use crate::history_cell::new_guardian_assessment_event;
use crate::history_cell::new_patch_apply_failure;
use crate::history_cell::new_plan_update;
use crate::history_cell::new_request_permissions_event;
use crate::history_cell::new_request_user_input_event;
use crate::history_cell::new_warning_event;
use crate::history_cell_potter::PotterStreamRecoveryRetryCell;
use crate::history_cell_potter::PotterStreamRecoveryUnrecoverableCell;
use crate::markdown;
use crate::multi_agents;
use crate::reasoning_status::ReasoningStatusTracker;
use crate::status_indicator_widget::fmt_elapsed_compact;
use crate::status_line::StatusLine;
use crate::status_line::render_status_line;
use crate::streaming::controller::PlanStreamController;
use crate::streaming::controller::StreamController;
use crate::ui_colors::secondary_color;

const DEFAULT_RENDER_WIDTH: u16 = 120;

#[derive(Debug)]
enum PendingProjectSummaryOutcome {
    Succeeded,
    BudgetExhausted,
}

#[derive(Debug)]
struct PendingProjectSummary {
    outcome: PendingProjectSummaryOutcome,
    rounds: u32,
    duration: Duration,
    user_prompt_file: PathBuf,
    git_commit_start: String,
    git_commit_end: String,
}

/// Append-only human-readable renderer used by `codex-potter exec` without `--json`.
pub struct ExecHumanRenderer {
    cwd: PathBuf,
    width: Option<u16>,
    color_enabled: bool,
    verbosity: Verbosity,
    stream: StreamController,
    saw_agent_delta: bool,
    plan_stream: Option<PlanStreamController>,
    pending_minimal_agent_message_lines: Option<Vec<Line<'static>>>,
    pending_minimal_agent_message_visible: bool,
    pending_project_summary: Option<PendingProjectSummary>,
    pending_simple_final_message_separator: bool,
    separator_baseline: Option<Instant>,
    project_started_at: Option<Instant>,
    status_started_at: Option<Instant>,
    status_header_prefix: Option<String>,
    token_usage: TokenUsage,
    context_usage: TokenUsage,
    model_context_window: Option<i64>,
    reasoning_status: ReasoningStatusTracker,
}

impl ExecHumanRenderer {
    /// Create a renderer.
    pub fn new(verbosity: Verbosity, width: Option<u16>, color_enabled: bool) -> Self {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Self {
            cwd: cwd.clone(),
            width,
            color_enabled,
            verbosity,
            stream: StreamController::new(width.map(usize::from), &cwd),
            saw_agent_delta: false,
            plan_stream: None,
            pending_minimal_agent_message_lines: None,
            pending_minimal_agent_message_visible: false,
            pending_project_summary: None,
            pending_simple_final_message_separator: false,
            separator_baseline: None,
            project_started_at: None,
            status_started_at: None,
            status_header_prefix: None,
            token_usage: TokenUsage::default(),
            context_usage: TokenUsage::default(),
            model_context_window: None,
            reasoning_status: ReasoningStatusTracker::new(),
        }
    }

    /// Provide the project start time used by status hints that mirror the live shimmer line.
    pub fn set_project_started_at(&mut self, started_at: Instant) {
        self.project_started_at = Some(started_at);
    }

    /// Render a fatal error block.
    pub fn render_error_block(&self, message: String) -> io::Result<String> {
        self.render_cell_block(Box::new(new_error_event(message)))
    }

    /// Flush buffered transcript state before an abnormal exit.
    pub fn flush_for_exit(&mut self) -> io::Result<Vec<String>> {
        let mut out = Vec::new();
        out.extend(self.flush_agent_output(false)?);
        out.extend(self.flush_plan_stream()?);
        Ok(out)
    }

    /// Return whether minimal-mode output still has a hidden completed agent message waiting for
    /// an idle flush.
    pub fn needs_idle_agent_message_flush(&self) -> bool {
        self.verbosity == Verbosity::Minimal
            && self.pending_minimal_agent_message_lines.is_some()
            && !self.pending_minimal_agent_message_visible
    }

    /// Render the hidden minimal-mode agent message as a dim block so append-only exec output
    /// keeps pace with the live transcript even without a transient preview area.
    pub fn flush_idle_agent_message(&mut self) -> io::Result<Option<String>> {
        if !self.needs_idle_agent_message_flush() {
            return Ok(None);
        }

        let Some(lines) = self.pending_minimal_agent_message_lines.clone() else {
            return Ok(None);
        };
        self.pending_minimal_agent_message_visible = true;
        Ok(Some(self.render_agent_message_block(lines, true)?))
    }

    /// Render one protocol event into zero or more append-only output blocks.
    pub fn handle_event(
        &mut self,
        msg: &codex_protocol::protocol::EventMsg,
    ) -> io::Result<Vec<String>> {
        use codex_protocol::protocol::EventMsg;

        let mut out = Vec::new();
        match msg {
            EventMsg::SessionConfigured(cfg) => {
                self.cwd = cfg.cwd.clone();
                self.stream = StreamController::new(self.width.map(usize::from), &self.cwd);
            }
            EventMsg::TurnStarted(ev) => {
                self.pending_simple_final_message_separator = false;
                self.separator_baseline = Some(Instant::now());
                self.status_started_at = Some(Instant::now());
                self.model_context_window = ev.model_context_window;
                self.reasoning_status.reset();
            }
            EventMsg::PotterProjectStarted {
                user_prompt_file, ..
            } => {
                self.project_started_at.get_or_insert_with(Instant::now);
                out.push(self.render_project_hint_block(user_prompt_file.as_path())?);
            }
            EventMsg::PotterRoundStarted { current, total } => {
                self.pending_simple_final_message_separator = false;
                self.separator_baseline = Some(Instant::now());
                self.status_header_prefix = Some(format!("Round {current}/{total}"));
                out.push(self.render_lines(vec![Line::from(vec![
                    Span::styled(
                        "CodexPotter: ",
                        Style::default()
                            .fg(secondary_color())
                            .add_modifier(Modifier::BOLD),
                    ),
                    format!("iteration round {current}/{total}").into(),
                ])])?);
            }
            EventMsg::TokenCount(ev) => {
                if let Some(info) = &ev.info {
                    self.token_usage = info.total_token_usage.clone();
                    self.context_usage = info.last_token_usage.clone();
                    self.model_context_window =
                        info.model_context_window.or(self.model_context_window);
                }
            }
            EventMsg::PotterProjectSucceeded {
                rounds,
                duration,
                user_prompt_file,
                git_commit_start,
                git_commit_end,
            } => {
                self.pending_project_summary = Some(PendingProjectSummary {
                    outcome: PendingProjectSummaryOutcome::Succeeded,
                    rounds: *rounds,
                    duration: *duration,
                    user_prompt_file: user_prompt_file.clone(),
                    git_commit_start: git_commit_start.clone(),
                    git_commit_end: git_commit_end.clone(),
                });
            }
            EventMsg::PotterProjectBudgetExhausted {
                rounds,
                duration,
                user_prompt_file,
                git_commit_start,
                git_commit_end,
            } => {
                self.pending_project_summary = Some(PendingProjectSummary {
                    outcome: PendingProjectSummaryOutcome::BudgetExhausted,
                    rounds: *rounds,
                    duration: *duration,
                    user_prompt_file: user_prompt_file.clone(),
                    git_commit_start: git_commit_start.clone(),
                    git_commit_end: git_commit_end.clone(),
                });
            }
            EventMsg::PotterRoundFinished { .. } => {
                out.extend(self.flush_agent_output(false)?);
                out.extend(self.flush_plan_stream()?);
                if let Some(summary) = self.pending_project_summary.take() {
                    out.push(self.render_project_summary(summary)?);
                }
            }
            EventMsg::TurnComplete(_) => {
                out.extend(self.flush_agent_output(true)?);
                out.extend(self.flush_plan_stream()?);
                if let Some(summary) = self.pending_project_summary.take() {
                    out.push(self.render_project_summary(summary)?);
                }
            }
            EventMsg::TurnAborted(ev) => {
                out.extend(self.flush_agent_output(false)?);
                out.extend(self.flush_plan_stream()?);
                if ev.reason == codex_protocol::protocol::TurnAbortReason::Interrupted {
                    out.push(
                        self.render_cell_block(Box::new(new_error_event(
                            "Conversation interrupted - tell the model what to do differently."
                                .to_string(),
                        )))?,
                    );
                }
            }
            EventMsg::AgentMessageDelta(ev) => {
                if self.verbosity == Verbosity::Minimal && !self.saw_agent_delta {
                    out.extend(self.flush_agent_output(false)?);
                }
                self.saw_agent_delta |= !ev.delta.is_empty();
                let _ = self.stream.push(&ev.delta);
            }
            EventMsg::AgentReasoningDelta(ev) => {
                if let Some(block) = self.maybe_render_reasoning_status_hint(&ev.delta)? {
                    out.push(block);
                }
            }
            EventMsg::AgentReasoningRawContentDelta(ev) => {
                if let Some(block) = self.maybe_render_reasoning_status_hint(&ev.delta)? {
                    out.push(block);
                }
            }
            EventMsg::AgentReasoningSectionBreak(_) => {
                self.reasoning_status.on_section_break();
            }
            EventMsg::AgentReasoning(ev) => {
                if let Some(block) = self.maybe_render_reasoning_status_hint(&ev.text)? {
                    out.push(block);
                }
                self.reasoning_status.on_final();
            }
            EventMsg::AgentReasoningRawContent(ev) => {
                if let Some(block) = self.maybe_render_reasoning_status_hint(&ev.text)? {
                    out.push(block);
                }
                self.reasoning_status.on_final();
            }
            EventMsg::AgentMessage(ev) => {
                if self.verbosity == Verbosity::Minimal {
                    let lines = if self.saw_agent_delta {
                        let lines = self.stream.take_finalized_lines();
                        self.saw_agent_delta = false;
                        lines
                    } else {
                        self.build_agent_message_lines(&ev.message)
                    };
                    self.store_pending_minimal_agent_message(lines, &mut out)?;
                } else {
                    let lines = if self.saw_agent_delta {
                        let lines = self.stream.take_finalized_lines();
                        self.saw_agent_delta = false;
                        lines
                    } else {
                        self.build_agent_message_lines(&ev.message)
                    };
                    if !lines.is_empty() {
                        self.push_simple_final_message_separator(&mut out)?;
                        out.push(self.render_lines(lines)?);
                    }
                }
            }
            EventMsg::PlanDelta(ev) => {
                if self.verbosity == Verbosity::Minimal {
                    return Ok(out);
                }
                if self.plan_stream.is_none() {
                    self.plan_stream = Some(PlanStreamController::new(
                        self.width.map(usize::from),
                        &self.cwd,
                    ));
                }
                if let Some(controller) = self.plan_stream.as_mut() {
                    let _ = controller.push(&ev.delta);
                }
            }
            EventMsg::PlanUpdate(ev) => {
                out.extend(self.flush_agent_output(false)?);
                out.extend(self.flush_plan_stream()?);
                if self.verbosity != Verbosity::Minimal {
                    out.push(self.render_cell_block(Box::new(new_plan_update(ev.clone())))?);
                }
            }
            EventMsg::ContextCompacted(_) => {
                out.extend(self.flush_agent_output(false)?);
                out.extend(self.flush_plan_stream()?);
                out.push(self.render_lines(vec![Line::from("Context compacted")])?);
            }
            EventMsg::Warning(ev) => {
                out.extend(self.flush_agent_output(false)?);
                out.extend(self.flush_plan_stream()?);
                out.push(self.render_cell_block(Box::new(new_warning_event(ev.message.clone())))?);
            }
            EventMsg::Error(ev) => {
                out.extend(self.flush_agent_output(false)?);
                out.extend(self.flush_plan_stream()?);
                out.push(self.render_cell_block(Box::new(new_error_event(ev.message.clone())))?);
            }
            EventMsg::DeprecationNotice(ev) => {
                out.extend(self.flush_agent_output(false)?);
                out.extend(self.flush_plan_stream()?);
                out.push(self.render_cell_block(Box::new(new_deprecation_notice(
                    ev.summary.clone(),
                    ev.details.clone(),
                )))?);
            }
            EventMsg::RequestPermissions(ev) => {
                out.extend(self.flush_agent_output(false)?);
                out.extend(self.flush_plan_stream()?);
                out.push(
                    self.render_cell_block(Box::new(new_request_permissions_event(ev.clone())))?,
                );
                self.mark_work_activity();
            }
            EventMsg::RequestUserInput(ev) => {
                out.extend(self.flush_agent_output(false)?);
                out.extend(self.flush_plan_stream()?);
                out.push(
                    self.render_cell_block(Box::new(new_request_user_input_event(ev.clone())))?,
                );
            }
            EventMsg::ElicitationRequest(ev) => {
                out.extend(self.flush_agent_output(false)?);
                out.extend(self.flush_plan_stream()?);
                out.push(
                    self.render_cell_block(Box::new(new_elicitation_request_event(ev.clone())))?,
                );
            }
            EventMsg::GuardianAssessment(ev) => {
                out.extend(self.flush_agent_output(false)?);
                out.extend(self.flush_plan_stream()?);
                out.push(
                    self.render_cell_block(Box::new(new_guardian_assessment_event(ev.clone())))?,
                );
                self.mark_work_activity();
            }
            EventMsg::WebSearchEnd(ev) => {
                if self.verbosity != Verbosity::Minimal {
                    out.extend(self.flush_agent_output(false)?);
                    out.extend(self.flush_plan_stream()?);
                    let block = vec![
                        Line::from(vec!["Searched".bold()]),
                        Line::from(format!("  {}", ev.query)),
                    ];
                    out.push(self.render_lines(block)?);
                    self.mark_work_activity();
                }
            }
            EventMsg::ViewImageToolCall(ev) => {
                if self.verbosity == Verbosity::Simple {
                    out.extend(self.flush_agent_output(false)?);
                    out.extend(self.flush_plan_stream()?);
                    let path = display_path_for(&ev.path, &self.cwd);
                    let block = vec![
                        Line::from(vec!["Viewed Image".bold()]),
                        Line::from(vec![Span::from("  "), Span::from(path).dim()]),
                    ];
                    out.push(self.render_lines(block)?);
                    self.mark_work_activity();
                }
            }
            EventMsg::ExecCommandEnd(ev) => {
                if self.verbosity != Verbosity::Minimal {
                    out.extend(self.flush_agent_output(false)?);
                    out.extend(self.flush_plan_stream()?);
                    if let Some(block) = self.render_exec_command_end(ev)? {
                        out.push(block);
                        self.mark_work_activity();
                    }
                }
            }
            EventMsg::PatchApplyEnd(ev) => {
                out.extend(self.flush_agent_output(false)?);
                out.extend(self.flush_plan_stream()?);
                if ev.success {
                    let patch_blocks = self.render_patch_blocks(ev.changes.clone())?;
                    if !patch_blocks.is_empty() {
                        out.extend(patch_blocks);
                        self.mark_work_activity();
                    }
                } else {
                    out.push(
                        self.render_cell_block(Box::new(new_patch_apply_failure(
                            ev.stderr.clone(),
                        )))?,
                    );
                    self.mark_work_activity();
                }
            }
            EventMsg::HookStarted(ev) => {
                out.extend(self.flush_agent_output(false)?);
                out.extend(self.flush_plan_stream()?);
                let label = hook_event_label(ev.run.event_name);
                let mut message = format!("Running {label} hook");
                if let Some(status_message) = &ev.run.status_message
                    && !status_message.is_empty()
                {
                    message.push_str(": ");
                    message.push_str(status_message);
                }
                out.push(self.render_lines(vec![Line::from(message)])?);
                self.mark_work_activity();
            }
            EventMsg::HookCompleted(ev) => {
                out.extend(self.flush_agent_output(false)?);
                out.extend(self.flush_plan_stream()?);
                let status = format!("{:?}", ev.run.status).to_lowercase();
                let header = format!("{} hook ({status})", hook_event_label(ev.run.event_name));
                let mut lines: Vec<Line<'static>> = vec![Line::from(header)];
                for entry in &ev.run.entries {
                    let prefix = match entry.kind {
                        codex_protocol::protocol::HookOutputEntryKind::Warning => "warning: ",
                        codex_protocol::protocol::HookOutputEntryKind::Stop => "stop: ",
                        codex_protocol::protocol::HookOutputEntryKind::Feedback => "feedback: ",
                        codex_protocol::protocol::HookOutputEntryKind::Context => "hook context: ",
                        codex_protocol::protocol::HookOutputEntryKind::Error => "error: ",
                    };
                    lines.push(Line::from(format!("  {prefix}{}", entry.text)));
                }
                out.push(self.render_lines(lines)?);
                self.mark_work_activity();
            }
            EventMsg::CollabAgentSpawnEnd(ev) => {
                out.extend(self.flush_agent_output(false)?);
                out.extend(self.flush_plan_stream()?);
                out.push(self.render_cell_block(Box::new(multi_agents::spawn_end(ev.clone())))?);
                self.mark_work_activity();
            }
            EventMsg::CollabAgentInteractionEnd(ev) => {
                out.extend(self.flush_agent_output(false)?);
                out.extend(self.flush_plan_stream()?);
                out.push(
                    self.render_cell_block(Box::new(multi_agents::interaction_end(ev.clone())))?,
                );
                self.mark_work_activity();
            }
            EventMsg::CollabWaitingBegin(ev) => {
                out.extend(self.flush_agent_output(false)?);
                out.extend(self.flush_plan_stream()?);
                out.push(
                    self.render_cell_block(Box::new(multi_agents::waiting_begin(ev.clone())))?,
                );
                self.mark_work_activity();
            }
            EventMsg::CollabWaitingEnd(ev) => {
                out.extend(self.flush_agent_output(false)?);
                out.extend(self.flush_plan_stream()?);
                out.push(self.render_cell_block(Box::new(multi_agents::waiting_end(ev.clone())))?);
                self.mark_work_activity();
            }
            EventMsg::CollabCloseEnd(ev) => {
                out.extend(self.flush_agent_output(false)?);
                out.extend(self.flush_plan_stream()?);
                out.push(self.render_cell_block(Box::new(multi_agents::close_end(ev.clone())))?);
                self.mark_work_activity();
            }
            EventMsg::CollabResumeBegin(ev) => {
                out.extend(self.flush_agent_output(false)?);
                out.extend(self.flush_plan_stream()?);
                out.push(self.render_cell_block(Box::new(multi_agents::resume_begin(ev.clone())))?);
                self.mark_work_activity();
            }
            EventMsg::CollabResumeEnd(ev) => {
                out.extend(self.flush_agent_output(false)?);
                out.extend(self.flush_plan_stream()?);
                out.push(self.render_cell_block(Box::new(multi_agents::resume_end(ev.clone())))?);
                self.mark_work_activity();
            }
            EventMsg::PotterStreamRecoveryUpdate {
                attempt,
                max_attempts,
                error_message,
            } => {
                out.extend(self.flush_agent_output(false)?);
                out.extend(self.flush_plan_stream()?);
                out.push(
                    self.render_cell_block(Box::new(PotterStreamRecoveryRetryCell {
                        attempt: *attempt,
                        max_attempts: *max_attempts,
                        error_message: error_message.clone(),
                    }))?,
                );
            }
            EventMsg::PotterStreamRecoveryRecovered => {}
            EventMsg::PotterStreamRecoveryGaveUp {
                error_message,
                max_attempts,
                ..
            } => {
                out.extend(self.flush_agent_output(false)?);
                out.extend(self.flush_plan_stream()?);
                out.push(self.render_cell_block(Box::new(
                    PotterStreamRecoveryUnrecoverableCell {
                        max_attempts: *max_attempts,
                        error_message: error_message.clone(),
                    },
                ))?);
            }
            _ => {}
        }

        Ok(out)
    }

    fn build_agent_message_lines(&self, message: &str) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        markdown::append_markdown(
            message,
            self.width.map(usize::from),
            Some(&self.cwd),
            &mut lines,
        );
        lines
    }

    fn flush_agent_output(&mut self, final_message: bool) -> io::Result<Vec<String>> {
        let mut out = Vec::new();
        if self.verbosity == Verbosity::Minimal {
            if self.saw_agent_delta {
                let lines = self.stream.take_finalized_lines();
                self.saw_agent_delta = false;
                if !lines.is_empty() {
                    out.push(self.render_agent_message_block(lines, !final_message)?);
                }
            } else if let Some(lines) = self.pending_minimal_agent_message_lines.take() {
                let was_visible = self.pending_minimal_agent_message_visible;
                self.pending_minimal_agent_message_visible = false;
                if !was_visible {
                    out.push(self.render_agent_message_block(lines, !final_message)?);
                }
            }
            return Ok(out);
        }

        if self.saw_agent_delta {
            let lines = self.stream.take_finalized_lines();
            self.saw_agent_delta = false;
            if !lines.is_empty() {
                self.push_simple_final_message_separator(&mut out)?;
                out.push(self.render_agent_message_block(lines, false)?);
            }
        }

        Ok(out)
    }

    fn store_pending_minimal_agent_message(
        &mut self,
        lines: Vec<Line<'static>>,
        out: &mut Vec<String>,
    ) -> io::Result<()> {
        if lines.is_empty() {
            return Ok(());
        }
        if let Some(previous) = self.pending_minimal_agent_message_lines.replace(lines)
            && !self.pending_minimal_agent_message_visible
        {
            out.push(self.render_agent_message_block(previous, true)?);
        }
        self.pending_minimal_agent_message_visible = false;
        Ok(())
    }

    fn flush_plan_stream(&mut self) -> io::Result<Vec<String>> {
        if self.verbosity == Verbosity::Minimal {
            self.plan_stream = None;
            return Ok(Vec::new());
        }

        let Some(mut controller) = self.plan_stream.take() else {
            return Ok(Vec::new());
        };
        let Some(cell) = controller.finalize() else {
            return Ok(Vec::new());
        };
        let mut out = Vec::new();
        self.push_simple_final_message_separator(&mut out)?;
        out.push(self.render_cell_block(cell)?);
        Ok(out)
    }

    fn render_exec_command_end(
        &self,
        ev: &codex_protocol::protocol::ExecCommandEndEvent,
    ) -> io::Result<Option<String>> {
        let aggregated_output = if !ev.aggregated_output.is_empty() {
            ev.aggregated_output.clone()
        } else {
            format!("{}{}", ev.stdout, ev.stderr)
        };

        let mut cell = new_active_exec_command(
            ev.call_id.clone(),
            ev.command.clone(),
            ev.parsed_cmd.clone(),
            ev.source,
            ev.interaction_input.clone(),
            false,
        );
        cell.complete_call(
            &ev.call_id,
            CommandOutput {
                exit_code: ev.exit_code,
                aggregated_output,
                formatted_output: ev.formatted_output.clone(),
            },
            ev.duration,
        );

        Ok(Some(self.render_cell_block(Box::new(cell))?))
    }

    fn maybe_render_reasoning_status_hint(&mut self, delta: &str) -> io::Result<Option<String>> {
        let Some(header) = self.reasoning_status.on_delta(delta) else {
            return Ok(None);
        };
        self.render_status_hint_block(header).map(Some)
    }

    fn render_status_hint_block(&self, header: String) -> io::Result<String> {
        self.render_lines(self.build_status_hint_lines(header))
    }

    fn build_status_hint_lines(&self, header: String) -> Vec<Line<'static>> {
        let elapsed = self
            .status_started_at
            .map(|started_at| started_at.elapsed())
            .unwrap_or_default();
        let header_prefix_elapsed = match (self.project_started_at, self.status_started_at) {
            (Some(project_started_at), Some(status_started_at)) => Some(
                status_started_at
                    .saturating_duration_since(project_started_at)
                    .saturating_add(elapsed),
            ),
            (Some(project_started_at), None) => Some(project_started_at.elapsed()),
            (None, _) => None,
        };
        let (context_window_percent, context_window_used_tokens) =
            self.current_context_window_display();
        self.build_status_hint_lines_from_state(
            header,
            elapsed,
            header_prefix_elapsed,
            context_window_percent,
            context_window_used_tokens,
        )
    }

    fn build_status_hint_lines_from_state(
        &self,
        header: String,
        elapsed: Duration,
        header_prefix_elapsed: Option<Duration>,
        context_window_percent: Option<i64>,
        context_window_used_tokens: Option<i64>,
    ) -> Vec<Line<'static>> {
        let mut lines = vec![render_status_line(
            &StatusLine {
                header,
                header_prefix: self.status_header_prefix.clone(),
                header_prefix_elapsed,
                elapsed,
                context_window_percent,
                context_window_used_tokens,
                show_context_window: true,
            },
            None,
            false,
        )];
        crate::render::line_utils::dim_lines(&mut lines);
        lines
    }

    fn current_context_window_display(&self) -> (Option<i64>, Option<i64>) {
        let Some(context_window) = self
            .model_context_window
            .filter(|context_window| *context_window > 0)
        else {
            return (
                None,
                (self.token_usage.total_tokens > 0).then_some(self.token_usage.total_tokens),
            );
        };

        (
            Some(
                self.context_usage
                    .percent_of_context_window_remaining(context_window),
            ),
            None,
        )
    }

    fn render_patch_blocks(
        &self,
        changes: std::collections::HashMap<PathBuf, codex_protocol::protocol::FileChange>,
    ) -> io::Result<Vec<String>> {
        if self.verbosity == Verbosity::Simple {
            return Ok(vec![self.render_cell_block(Box::new(
                crate::history_cell::new_patch_event(changes, &self.cwd, self.verbosity),
            ))?]);
        }

        let lines =
            create_compact_diff_summary(&changes, &self.cwd, usize::from(self.cell_width()));
        if lines.is_empty() {
            return Ok(Vec::new());
        }

        if lines.len() == 1 {
            return Ok(vec![self.render_lines(normalize_general_lines(lines))?]);
        }

        let mut out = Vec::new();
        for line in lines.into_iter().skip(1) {
            out.push(self.render_lines(vec![normalize_standalone_line(line)])?);
        }
        Ok(out)
    }

    fn render_project_summary(&self, summary: PendingProjectSummary) -> io::Result<String> {
        let PendingProjectSummary {
            outcome,
            rounds,
            duration,
            user_prompt_file,
            git_commit_start,
            git_commit_end,
        } = summary;

        let mut header_spans = vec![
            Span::styled(
                "CodexPotter summary:",
                Style::default()
                    .fg(secondary_color())
                    .add_modifier(Modifier::BOLD),
            ),
            " ".into(),
            format!("{rounds} rounds").bold(),
            " in ".into(),
            fmt_elapsed_compact(duration.as_secs()).bold(),
        ];
        match outcome {
            PendingProjectSummaryOutcome::Succeeded => {}
            PendingProjectSummaryOutcome::BudgetExhausted => {
                header_spans.push(" ".into());
                header_spans.push("(Budget exhausted)".red());
            }
        }

        let mut lines = vec![Line::from(header_spans), Line::from("")];
        if !(git_commit_start.is_empty() && git_commit_end.is_empty()) {
            lines.push(Line::from(vec![
                "  Git:               ".into(),
                short_git_commit(&git_commit_start).cyan(),
                " -> ".into(),
                short_git_commit(&git_commit_end).cyan(),
            ]));
        }
        lines.push(Line::from(vec![
            "  Task history:      ".into(),
            user_prompt_file.to_string_lossy().to_string().cyan(),
        ]));

        self.render_lines(lines)
    }

    fn render_project_hint_block(&self, user_prompt_file: &std::path::Path) -> io::Result<String> {
        self.render_lines(vec![Line::from(vec![
            "Project created: ".dim(),
            user_prompt_file.to_string_lossy().to_string().into(),
        ])])
    }

    fn mark_work_activity(&mut self) {
        if self.verbosity != Verbosity::Simple {
            return;
        }
        self.pending_simple_final_message_separator = true;
        if self.separator_baseline.is_none() {
            self.separator_baseline = Some(Instant::now());
        }
    }

    fn push_simple_final_message_separator(&mut self, out: &mut Vec<String>) -> io::Result<()> {
        if self.verbosity != Verbosity::Simple || !self.pending_simple_final_message_separator {
            return Ok(());
        }

        let elapsed_seconds = self
            .separator_baseline
            .map(|baseline| baseline.elapsed().as_secs());
        self.pending_simple_final_message_separator = false;
        self.separator_baseline = Some(Instant::now());
        out.push(self.render_cell_block(Box::new(
            crate::history_cell::FinalMessageSeparator::new(elapsed_seconds),
        ))?);
        Ok(())
    }

    fn render_cell_block(&self, cell: Box<dyn HistoryCell>) -> io::Result<String> {
        self.render_lines(normalize_general_lines(
            cell.display_lines(self.cell_width()),
        ))
    }

    fn render_agent_message_block(
        &self,
        mut lines: Vec<Line<'static>>,
        dim: bool,
    ) -> io::Result<String> {
        if dim {
            crate::render::line_utils::dim_lines(&mut lines);
        }
        self.render_lines(lines)
    }

    fn render_lines(&self, lines: Vec<Line<'static>>) -> io::Result<String> {
        let mut out = Vec::new();
        for (idx, line) in lines.iter().enumerate() {
            if idx > 0 {
                out.write_all(b"\n")?;
            }
            write_line(&mut out, line, self.color_enabled)?;
        }
        String::from_utf8(out)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err.to_string()))
    }

    fn cell_width(&self) -> u16 {
        self.width.unwrap_or(DEFAULT_RENDER_WIDTH).max(1)
    }
}

fn normalize_general_lines(lines: Vec<Line<'static>>) -> Vec<Line<'static>> {
    lines
        .into_iter()
        .enumerate()
        .map(|(idx, line)| {
            let line = replace_prefix(line, "• ", "");
            if idx == 0 {
                line
            } else {
                let line = replace_prefix(line, "  └ ", "  ");
                replace_prefix(line, "    ", "  ")
            }
        })
        .collect()
}

fn normalize_standalone_line(line: Line<'static>) -> Line<'static> {
    let line = replace_prefix(line, "• ", "");
    let line = replace_prefix(line, "  └ ", "");
    replace_prefix(line, "    ", "")
}

fn replace_prefix(mut line: Line<'static>, from: &str, to: &str) -> Line<'static> {
    let Some(first) = line.spans.first().cloned() else {
        return line;
    };
    let Some(rest) = first.content.as_ref().strip_prefix(from) else {
        return line;
    };

    let mut spans = Vec::with_capacity(line.spans.len() + usize::from(!to.is_empty()));
    if !to.is_empty() {
        spans.push(Span::styled(to.to_string(), first.style));
    }
    if !rest.is_empty() {
        spans.push(Span::styled(rest.to_string(), first.style));
    }
    spans.extend(line.spans.into_iter().skip(1));
    line.spans = spans;
    line
}

fn short_git_commit(commit: &str) -> String {
    const SHORT_SHA_LEN: usize = 7;
    if commit.len() <= SHORT_SHA_LEN {
        return commit.to_string();
    }
    commit[..SHORT_SHA_LEN].to_string()
}

fn hook_event_label(event_name: codex_protocol::protocol::HookEventName) -> &'static str {
    match event_name {
        codex_protocol::protocol::HookEventName::SessionStart => "session-start",
        codex_protocol::protocol::HookEventName::Stop => "stop",
    }
}

struct ModifierDiff {
    from: Modifier,
    to: Modifier,
}

impl ModifierDiff {
    fn queue<W: Write>(self, mut writer: W) -> io::Result<()> {
        use crossterm::style::Attribute;

        let removed = self.from - self.to;
        if removed.contains(Modifier::REVERSED) {
            queue!(writer, SetAttribute(Attribute::NoReverse))?;
        }
        if removed.contains(Modifier::BOLD) {
            queue!(writer, SetAttribute(Attribute::NormalIntensity))?;
            if self.to.contains(Modifier::DIM) {
                queue!(writer, SetAttribute(Attribute::Dim))?;
            }
        }
        if removed.contains(Modifier::ITALIC) {
            queue!(writer, SetAttribute(Attribute::NoItalic))?;
        }
        if removed.contains(Modifier::UNDERLINED) {
            queue!(writer, SetAttribute(Attribute::NoUnderline))?;
        }
        if removed.contains(Modifier::DIM) {
            queue!(writer, SetAttribute(Attribute::NormalIntensity))?;
        }
        if removed.contains(Modifier::CROSSED_OUT) {
            queue!(writer, SetAttribute(Attribute::NotCrossedOut))?;
        }
        if removed.contains(Modifier::SLOW_BLINK) || removed.contains(Modifier::RAPID_BLINK) {
            queue!(writer, SetAttribute(Attribute::NoBlink))?;
        }

        let added = self.to - self.from;
        if added.contains(Modifier::REVERSED) {
            queue!(writer, SetAttribute(Attribute::Reverse))?;
        }
        if added.contains(Modifier::BOLD) {
            queue!(writer, SetAttribute(Attribute::Bold))?;
        }
        if added.contains(Modifier::ITALIC) {
            queue!(writer, SetAttribute(Attribute::Italic))?;
        }
        if added.contains(Modifier::UNDERLINED) {
            queue!(writer, SetAttribute(Attribute::Underlined))?;
        }
        if added.contains(Modifier::DIM) {
            queue!(writer, SetAttribute(Attribute::Dim))?;
        }
        if added.contains(Modifier::CROSSED_OUT) {
            queue!(writer, SetAttribute(Attribute::CrossedOut))?;
        }
        if added.contains(Modifier::SLOW_BLINK) {
            queue!(writer, SetAttribute(Attribute::SlowBlink))?;
        }
        if added.contains(Modifier::RAPID_BLINK) {
            queue!(writer, SetAttribute(Attribute::RapidBlink))?;
        }

        Ok(())
    }
}

fn write_line(writer: &mut impl Write, line: &Line<'_>, color_enabled: bool) -> io::Result<()> {
    if !color_enabled {
        for span in &line.spans {
            writer.write_all(span.content.as_ref().as_bytes())?;
        }
        return Ok(());
    }

    let mut fg = ratatui::style::Color::Reset;
    let mut bg = ratatui::style::Color::Reset;
    let mut last_modifier = Modifier::empty();
    for span in &line.spans {
        let style = span.style.patch(line.style);

        let mut modifier = Modifier::empty();
        modifier.insert(style.add_modifier);
        modifier.remove(style.sub_modifier);
        if modifier != last_modifier {
            ModifierDiff {
                from: last_modifier,
                to: modifier,
            }
            .queue(&mut *writer)?;
            last_modifier = modifier;
        }

        let next_fg = style.fg.unwrap_or(ratatui::style::Color::Reset);
        let next_bg = style.bg.unwrap_or(ratatui::style::Color::Reset);
        if next_fg != fg || next_bg != bg {
            queue!(
                writer,
                SetColors(Colors::new(next_fg.into(), next_bg.into()))
            )?;
            fg = next_fg;
            bg = next_bg;
        }

        queue!(writer, Print(span.content.clone()))?;
    }

    queue!(
        writer,
        SetForegroundColor(CrosstermColor::Reset),
        SetBackgroundColor(CrosstermColor::Reset),
        SetAttribute(crossterm::style::Attribute::Reset),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_protocol::AbsolutePathBuf;
    use codex_protocol::approvals::GuardianAssessmentEvent;
    use codex_protocol::approvals::GuardianAssessmentStatus;
    use codex_protocol::approvals::GuardianRiskLevel;
    use codex_protocol::models::FileSystemPermissions;
    use codex_protocol::models::NetworkPermissions;
    use codex_protocol::protocol::AgentReasoningDeltaEvent;
    use codex_protocol::protocol::AgentReasoningEvent;
    use codex_protocol::protocol::EventMsg;
    use codex_protocol::protocol::ExecCommandEndEvent;
    use codex_protocol::protocol::ExecCommandSource;
    use codex_protocol::protocol::FileChange;
    use codex_protocol::protocol::PatchApplyEndEvent;
    use codex_protocol::protocol::TurnStartedEvent;
    use codex_protocol::protocol::ViewImageToolCallEvent;
    use codex_protocol::protocol::WebSearchEndEvent;
    use codex_protocol::request_permissions::RequestPermissionProfile;
    use codex_protocol::request_permissions::RequestPermissionsEvent;
    use pretty_assertions::assert_eq;
    use ratatui::style::Modifier;
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn line_to_plain_string(line: &Line<'_>) -> String {
        let mut out = String::new();
        for span in &line.spans {
            out.push_str(span.content.as_ref());
        }
        out
    }

    fn assert_all_spans_dimmed(lines: &[Line<'_>]) {
        assert!(
            lines
                .iter()
                .flat_map(|line| line.spans.iter())
                .all(|span| span.style.add_modifier.contains(Modifier::DIM)),
            "expected all spans to be dimmed: {:?}",
            lines.iter().map(line_to_plain_string).collect::<Vec<_>>()
        );
    }

    #[test]
    fn minimal_multi_file_patch_renders_each_file_without_changed_header() {
        let mut renderer = ExecHumanRenderer::new(Verbosity::Minimal, Some(120), false);
        let mut changes = HashMap::new();
        changes.insert(
            PathBuf::from("/repo/a.txt"),
            FileChange::Update {
                unified_diff: "@@ -1 +1 @@\n-old\n+new\n".to_string(),
                move_path: None,
            },
        );
        changes.insert(
            PathBuf::from("/repo/b.txt"),
            FileChange::Add {
                content: "hello\n".to_string(),
            },
        );

        renderer.cwd = PathBuf::from("/repo");
        let blocks = renderer
            .handle_event(&EventMsg::PatchApplyEnd(PatchApplyEndEvent {
                call_id: "patch".to_string(),
                turn_id: String::new(),
                stdout: String::new(),
                stderr: String::new(),
                success: true,
                changes,
            }))
            .expect("render patch");

        assert_eq!(blocks.len(), 2);
        assert!(
            blocks
                .iter()
                .all(|block| !block.contains("Changed 2 files"))
        );
        assert!(
            blocks
                .iter()
                .any(|block| block.contains("Edited a.txt (+1 -1)"))
        );
        assert!(
            blocks
                .iter()
                .any(|block| block.contains("Added b.txt (+1 -0)"))
        );
    }

    #[test]
    fn simple_patch_keeps_full_diff_block_visible() {
        let mut renderer = ExecHumanRenderer::new(Verbosity::Simple, Some(120), false);
        let mut changes = HashMap::new();
        changes.insert(
            PathBuf::from("/repo/a.txt"),
            FileChange::Update {
                unified_diff: "@@ -1 +1 @@\n-old\n+new\n".to_string(),
                move_path: None,
            },
        );

        renderer.cwd = PathBuf::from("/repo");
        let blocks = renderer
            .handle_event(&EventMsg::PatchApplyEnd(PatchApplyEndEvent {
                call_id: "patch".to_string(),
                turn_id: String::new(),
                stdout: String::new(),
                stderr: String::new(),
                success: true,
                changes,
            }))
            .expect("render patch");

        assert_eq!(blocks.len(), 1);
        let block = &blocks[0];
        assert!(block.contains("Edited a.txt (+1 -1)"));
        assert!(block.contains("-old"));
        assert!(block.contains("+new"));
    }

    #[test]
    fn summary_strips_interactive_loop_line_and_chrome() {
        let mut renderer = ExecHumanRenderer::new(Verbosity::Minimal, Some(120), false);
        let blocks = renderer
            .handle_event(&EventMsg::PotterProjectBudgetExhausted {
                rounds: 5,
                duration: Duration::from_secs(7328),
                user_prompt_file: PathBuf::from(".codexpotter/projects/2026/03/26/3/MAIN.md"),
                git_commit_start: "96ca8c6abc".to_string(),
                git_commit_end: "0919e7bdef".to_string(),
            })
            .expect("store summary");
        assert!(blocks.is_empty());

        let blocks = renderer
            .handle_event(&EventMsg::PotterRoundFinished {
                outcome: codex_protocol::protocol::PotterRoundOutcome::Completed,
            })
            .expect("emit summary");
        assert_eq!(blocks.len(), 1);
        let block = &blocks[0];
        assert!(block.contains("CodexPotter summary: 5 rounds in 2h 02m 08s (Budget exhausted)"));
        assert!(block.contains("Git:"));
        assert!(block.contains("Task history:"));
        assert!(!block.contains("Loop more rounds:"));
        assert!(!block.contains("──"));
    }

    #[test]
    fn round_marker_has_no_bullet() {
        let mut renderer = ExecHumanRenderer::new(Verbosity::Minimal, Some(120), false);
        let blocks = renderer
            .handle_event(&EventMsg::PotterRoundStarted {
                current: 1,
                total: 10,
            })
            .expect("round marker");
        assert_eq!(
            blocks,
            vec!["CodexPotter: iteration round 1/10".to_string()]
        );
    }

    #[test]
    fn project_started_emits_only_project_hint() {
        let mut renderer = ExecHumanRenderer::new(Verbosity::Minimal, Some(120), false);
        let blocks = renderer
            .handle_event(&EventMsg::PotterProjectStarted {
                user_message: Some("Fix the failing test".to_string()),
                working_dir: PathBuf::from("/repo"),
                project_dir: PathBuf::from(".codexpotter/projects/2026/03/27/1"),
                user_prompt_file: PathBuf::from(".codexpotter/projects/2026/03/27/1/MAIN.md"),
            })
            .expect("project started");

        assert_eq!(
            blocks,
            vec!["Project created: .codexpotter/projects/2026/03/27/1/MAIN.md".to_string(),]
        );
    }

    #[test]
    fn minimal_agent_message_stays_pending_until_turn_complete() {
        let mut renderer = ExecHumanRenderer::new(Verbosity::Minimal, Some(120), false);
        let blocks = renderer
            .handle_event(&EventMsg::AgentMessage(
                codex_protocol::protocol::AgentMessageEvent {
                    message: "done".to_string(),
                    phase: None,
                },
            ))
            .expect("agent message");
        assert!(blocks.is_empty());

        let blocks = renderer
            .handle_event(&EventMsg::TurnComplete(
                codex_protocol::protocol::TurnCompleteEvent {
                    turn_id: "turn-1".to_string(),
                    last_agent_message: None,
                },
            ))
            .expect("turn complete");
        assert_eq!(blocks, vec!["done".to_string()]);
    }

    #[test]
    fn minimal_idle_flush_makes_pending_agent_message_visible_without_duplication() {
        let mut renderer = ExecHumanRenderer::new(Verbosity::Minimal, Some(120), false);
        let blocks = renderer
            .handle_event(&EventMsg::AgentMessage(
                codex_protocol::protocol::AgentMessageEvent {
                    message: "latest".to_string(),
                    phase: None,
                },
            ))
            .expect("agent message");
        assert!(blocks.is_empty());
        assert!(renderer.needs_idle_agent_message_flush());

        let idle_block = renderer
            .flush_idle_agent_message()
            .expect("idle flush")
            .expect("idle block");
        assert_eq!(idle_block, "latest");
        assert!(
            renderer
                .flush_idle_agent_message()
                .expect("second idle flush")
                .is_none()
        );

        let blocks = renderer
            .handle_event(&EventMsg::TurnComplete(
                codex_protocol::protocol::TurnCompleteEvent {
                    turn_id: "turn-1".to_string(),
                    last_agent_message: None,
                },
            ))
            .expect("turn complete");
        assert!(blocks.is_empty());
    }

    #[test]
    fn minimal_new_agent_stream_flushes_previous_pending_message() {
        let mut renderer = ExecHumanRenderer::new(Verbosity::Minimal, Some(120), false);
        let blocks = renderer
            .handle_event(&EventMsg::AgentMessage(
                codex_protocol::protocol::AgentMessageEvent {
                    message: "previous".to_string(),
                    phase: None,
                },
            ))
            .expect("agent message");
        assert!(blocks.is_empty());

        let blocks = renderer
            .handle_event(&EventMsg::AgentMessageDelta(
                codex_protocol::protocol::AgentMessageDeltaEvent {
                    delta: "next".to_string(),
                },
            ))
            .expect("agent delta");
        assert_eq!(blocks, vec!["previous".to_string()]);
    }

    #[test]
    fn simple_mode_keeps_search_and_image_events_visible() {
        let mut renderer = ExecHumanRenderer::new(Verbosity::Simple, Some(120), false);
        renderer.cwd = PathBuf::from("/repo");

        let search_blocks = renderer
            .handle_event(&EventMsg::WebSearchEnd(WebSearchEndEvent {
                call_id: "search-1".to_string(),
                query: "rust fmt".to_string(),
            }))
            .expect("search event");
        assert_eq!(search_blocks, vec!["Searched\n  rust fmt".to_string()]);

        let image_blocks = renderer
            .handle_event(&EventMsg::ViewImageToolCall(ViewImageToolCallEvent {
                call_id: "image-1".to_string(),
                path: PathBuf::from("/repo/screenshot.png"),
            }))
            .expect("image event");
        assert_eq!(
            image_blocks,
            vec!["Viewed Image\n  screenshot.png".to_string()]
        );
    }

    #[test]
    fn simple_mode_inserts_worked_separator_before_follow_up_agent_message() {
        let mut renderer = ExecHumanRenderer::new(Verbosity::Simple, Some(80), false);
        let _ = renderer.handle_event(&EventMsg::TurnStarted(TurnStartedEvent {
            turn_id: "turn-1".to_string(),
            model_context_window: None,
        }));

        let command_blocks = renderer
            .handle_event(&EventMsg::ExecCommandEnd(ExecCommandEndEvent {
                call_id: "cmd-1".to_string(),
                turn_id: "turn-1".to_string(),
                command: vec!["bash".to_string(), "-lc".to_string(), "true".to_string()],
                cwd: PathBuf::from("/repo"),
                aggregated_output: String::new(),
                parsed_cmd: Vec::new(),
                exit_code: 0,
                duration: Duration::from_secs(1),
                formatted_output: String::new(),
                stdout: String::new(),
                stderr: String::new(),
                source: ExecCommandSource::Agent,
                interaction_input: None,
                process_id: None,
            }))
            .expect("exec command");
        assert_eq!(command_blocks.len(), 1);

        let blocks = renderer
            .handle_event(&EventMsg::AgentMessage(
                codex_protocol::protocol::AgentMessageEvent {
                    message: "done".to_string(),
                    phase: None,
                },
            ))
            .expect("agent message");
        assert_eq!(blocks.len(), 2);
        assert!(blocks[0].contains("Worked for "));
        assert_eq!(blocks[1], "done");
    }

    #[test]
    fn simple_mode_inserts_worked_separator_after_request_permissions() {
        let mut renderer = ExecHumanRenderer::new(Verbosity::Simple, Some(80), false);
        let _ = renderer.handle_event(&EventMsg::TurnStarted(TurnStartedEvent {
            turn_id: "turn-1".to_string(),
            model_context_window: None,
        }));

        let write_root =
            AbsolutePathBuf::from_absolute_path("/Users/me/project").expect("absolute path");
        let request_blocks = renderer
            .handle_event(&EventMsg::RequestPermissions(RequestPermissionsEvent {
                call_id: "call-1".to_string(),
                turn_id: "turn-1".to_string(),
                reason: Some("Select a workspace root".to_string()),
                permissions: RequestPermissionProfile {
                    network: Some(NetworkPermissions {
                        enabled: Some(true),
                    }),
                    file_system: Some(FileSystemPermissions {
                        read: None,
                        write: Some(vec![write_root]),
                    }),
                },
            }))
            .expect("request permissions");
        assert_eq!(request_blocks.len(), 1);

        let blocks = renderer
            .handle_event(&EventMsg::AgentMessage(
                codex_protocol::protocol::AgentMessageEvent {
                    message: "done".to_string(),
                    phase: None,
                },
            ))
            .expect("agent message");
        assert_eq!(blocks.len(), 2);
        assert!(blocks[0].contains("Worked for "));
        assert_eq!(blocks[1], "done");
    }

    #[test]
    fn simple_mode_inserts_worked_separator_after_guardian_assessment() {
        let mut renderer = ExecHumanRenderer::new(Verbosity::Simple, Some(80), false);
        let _ = renderer.handle_event(&EventMsg::TurnStarted(TurnStartedEvent {
            turn_id: "turn-1".to_string(),
            model_context_window: None,
        }));

        let guardian_blocks = renderer
            .handle_event(&EventMsg::GuardianAssessment(GuardianAssessmentEvent {
                id: "assessment-1".to_string(),
                turn_id: "turn-1".to_string(),
                status: GuardianAssessmentStatus::Approved,
                risk_score: Some(15),
                risk_level: Some(GuardianRiskLevel::Low),
                rationale: Some("Looks safe.".to_string()),
                action: None,
            }))
            .expect("guardian assessment");
        assert_eq!(guardian_blocks.len(), 1);

        let blocks = renderer
            .handle_event(&EventMsg::AgentMessage(
                codex_protocol::protocol::AgentMessageEvent {
                    message: "done".to_string(),
                    phase: None,
                },
            ))
            .expect("agent message");
        assert_eq!(blocks.len(), 2);
        assert!(blocks[0].contains("Worked for "));
        assert_eq!(blocks[1], "done");
    }

    #[test]
    fn minimal_mode_hides_search_and_image_events() {
        let mut renderer = ExecHumanRenderer::new(Verbosity::Minimal, Some(120), false);
        renderer.cwd = PathBuf::from("/repo");

        let search_blocks = renderer
            .handle_event(&EventMsg::WebSearchEnd(WebSearchEndEvent {
                call_id: "search-1".to_string(),
                query: "rust fmt".to_string(),
            }))
            .expect("search event");
        assert!(search_blocks.is_empty());

        let image_blocks = renderer
            .handle_event(&EventMsg::ViewImageToolCall(ViewImageToolCallEvent {
                call_id: "image-1".to_string(),
                path: PathBuf::from("/repo/screenshot.png"),
            }))
            .expect("image event");
        assert!(image_blocks.is_empty());
    }

    #[test]
    fn reasoning_status_hint_uses_shimmer_line_format_and_dims_output() {
        let mut renderer = ExecHumanRenderer::new(Verbosity::Minimal, Some(120), false);
        renderer.status_header_prefix = Some("Round 1/10".to_string());

        let lines = renderer.build_status_hint_lines_from_state(
            "Updating progress file".to_string(),
            Duration::from_secs(2650),
            Some(Duration::from_secs(2650)),
            Some(12),
            None,
        );

        assert_eq!(lines.len(), 1);
        assert_eq!(
            line_to_plain_string(&lines[0]),
            "• Round 1/10 (44m 10s) · Updating progress file (44m 10s) · 12% context left"
        );
        assert_all_spans_dimmed(&lines);
    }

    #[test]
    fn reasoning_status_hint_emits_once_until_header_changes() {
        let mut renderer = ExecHumanRenderer::new(Verbosity::Minimal, Some(120), false);
        let _ = renderer.handle_event(&EventMsg::TurnStarted(TurnStartedEvent {
            turn_id: "turn-1".to_string(),
            model_context_window: Some(128_000),
        }));

        let first = renderer
            .handle_event(&EventMsg::AgentReasoningDelta(AgentReasoningDeltaEvent {
                delta: "**Updating progress file**".to_string(),
            }))
            .expect("first reasoning delta");
        assert_eq!(first.len(), 1);
        assert!(first[0].contains("Updating progress file"));

        let second = renderer
            .handle_event(&EventMsg::AgentReasoningDelta(AgentReasoningDeltaEvent {
                delta: "\nMore detail without a new title".to_string(),
            }))
            .expect("second reasoning delta");
        assert!(second.is_empty());

        let third = renderer
            .handle_event(&EventMsg::AgentReasoning(AgentReasoningEvent {
                text: "**Updating progress file**\nDone.".to_string(),
            }))
            .expect("reasoning final");
        assert!(third.is_empty());
    }
}
