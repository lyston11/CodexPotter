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
use crate::status_indicator_widget::fmt_elapsed_compact;
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
    pending_project_summary: Option<PendingProjectSummary>,
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
            pending_project_summary: None,
        }
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
            EventMsg::PotterProjectStarted { .. } => {}
            EventMsg::PotterRoundStarted { current, total } => {
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
                self.saw_agent_delta |= !ev.delta.is_empty();
                let _ = self.stream.push(&ev.delta);
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
                }
            }
            EventMsg::ExecCommandEnd(ev) => {
                if self.verbosity != Verbosity::Minimal {
                    out.extend(self.flush_agent_output(false)?);
                    out.extend(self.flush_plan_stream()?);
                    if let Some(block) = self.render_exec_command_end(ev)? {
                        out.push(block);
                    }
                }
            }
            EventMsg::PatchApplyEnd(ev) => {
                out.extend(self.flush_agent_output(false)?);
                out.extend(self.flush_plan_stream()?);
                if ev.success {
                    out.extend(self.render_patch_blocks(ev.changes.clone())?);
                } else {
                    out.push(
                        self.render_cell_block(Box::new(new_patch_apply_failure(
                            ev.stderr.clone(),
                        )))?,
                    );
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
            }
            EventMsg::CollabAgentSpawnEnd(ev) => {
                out.extend(self.flush_agent_output(false)?);
                out.extend(self.flush_plan_stream()?);
                out.push(self.render_cell_block(Box::new(multi_agents::spawn_end(ev.clone())))?);
            }
            EventMsg::CollabAgentInteractionEnd(ev) => {
                out.extend(self.flush_agent_output(false)?);
                out.extend(self.flush_plan_stream()?);
                out.push(
                    self.render_cell_block(Box::new(multi_agents::interaction_end(ev.clone())))?,
                );
            }
            EventMsg::CollabWaitingBegin(ev) => {
                out.extend(self.flush_agent_output(false)?);
                out.extend(self.flush_plan_stream()?);
                out.push(
                    self.render_cell_block(Box::new(multi_agents::waiting_begin(ev.clone())))?,
                );
            }
            EventMsg::CollabWaitingEnd(ev) => {
                out.extend(self.flush_agent_output(false)?);
                out.extend(self.flush_plan_stream()?);
                out.push(self.render_cell_block(Box::new(multi_agents::waiting_end(ev.clone())))?);
            }
            EventMsg::CollabCloseEnd(ev) => {
                out.extend(self.flush_agent_output(false)?);
                out.extend(self.flush_plan_stream()?);
                out.push(self.render_cell_block(Box::new(multi_agents::close_end(ev.clone())))?);
            }
            EventMsg::CollabResumeBegin(ev) => {
                out.extend(self.flush_agent_output(false)?);
                out.extend(self.flush_plan_stream()?);
                out.push(self.render_cell_block(Box::new(multi_agents::resume_begin(ev.clone())))?);
            }
            EventMsg::CollabResumeEnd(ev) => {
                out.extend(self.flush_agent_output(false)?);
                out.extend(self.flush_plan_stream()?);
                out.push(self.render_cell_block(Box::new(multi_agents::resume_end(ev.clone())))?);
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
                out.push(self.render_agent_message_block(lines, !final_message)?);
            }
            return Ok(out);
        }

        if self.saw_agent_delta {
            let lines = self.stream.take_finalized_lines();
            self.saw_agent_delta = false;
            if !lines.is_empty() {
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
        if let Some(previous) = self.pending_minimal_agent_message_lines.replace(lines) {
            out.push(self.render_agent_message_block(previous, true)?);
        }
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
        Ok(vec![self.render_cell_block(cell)?])
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

    fn render_patch_blocks(
        &self,
        changes: std::collections::HashMap<PathBuf, codex_protocol::protocol::FileChange>,
    ) -> io::Result<Vec<String>> {
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
    use codex_protocol::protocol::EventMsg;
    use codex_protocol::protocol::FileChange;
    use codex_protocol::protocol::PatchApplyEndEvent;
    use codex_protocol::protocol::ViewImageToolCallEvent;
    use codex_protocol::protocol::WebSearchEndEvent;
    use pretty_assertions::assert_eq;
    use std::collections::HashMap;
    use std::path::PathBuf;

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
}
