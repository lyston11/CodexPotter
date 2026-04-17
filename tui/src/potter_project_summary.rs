//! Shared helpers for CodexPotter project-summary detail lines.
//!
//! # Divergence from upstream Codex
//!
//! Upstream Codex does not render CodexPotter project summaries. This module keeps the
//! CodexPotter-specific summary body (`View changes:`, `Task history:`, `Loop more rounds:`) in a
//! single place so interactive and headless renderers stay aligned.

use std::path::Path;

use ratatui::style::Stylize;
use ratatui::text::Line;

const VIEW_CHANGES_LABEL: &str = "View changes:";
const TASK_HISTORY_LABEL: &str = "Task history:";
const LOOP_MORE_ROUNDS_LABEL: &str = "Loop more rounds:";

/// Build the shared detail rows for a CodexPotter project summary.
///
/// The returned lines keep the `View changes:`, `Task history:`, and optional
/// `Loop more rounds:` labels aligned from the interactive transcript through
/// `codex-potter exec`, so the command-oriented summary copy cannot drift across
/// renderers.
pub fn build_potter_project_summary_detail_lines(
    user_prompt_file: &Path,
    git_commit_start: &str,
    git_commit_end: &str,
    loop_more_rounds_command: Option<&str>,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    if !git_commit_start.is_empty() && !git_commit_end.is_empty() {
        lines.push(render_summary_detail_line(
            VIEW_CHANGES_LABEL,
            format!(
                "git diff {}...{}",
                short_git_commit(git_commit_start),
                short_git_commit(git_commit_end)
            ),
        ));
    }

    lines.push(render_summary_detail_line(
        TASK_HISTORY_LABEL,
        user_prompt_file.to_string_lossy().to_string(),
    ));

    if let Some(loop_more_rounds_command) = loop_more_rounds_command {
        lines.push(render_summary_detail_line(
            LOOP_MORE_ROUNDS_LABEL,
            loop_more_rounds_command.to_string(),
        ));
    }

    lines
}

fn render_summary_detail_line(label: &str, value: String) -> Line<'static> {
    let label_width = LOOP_MORE_ROUNDS_LABEL.len();
    Line::from(vec![
        "  ".into(),
        format!("{label:<label_width$}").into(),
        "  ".into(),
        value.cyan(),
    ])
}

fn short_git_commit(commit: &str) -> String {
    const SHORT_SHA_LEN: usize = 7;
    if commit.len() <= SHORT_SHA_LEN {
        return commit.to_string();
    }
    commit[..SHORT_SHA_LEN].to_string()
}
