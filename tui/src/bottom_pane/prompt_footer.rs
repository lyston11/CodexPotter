//! Potter-specific prompt footer rendering kept separate from `bottom_pane::mod`.
//!
//! This footer is not part of upstream Codex's generic composer footer logic. Keeping the
//! rendering and context types here reduces merge pressure on `bottom_pane/mod.rs` while making
//! the local divergence explicit in one place.

use std::path::Path;
use std::path::PathBuf;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::WidgetRef;

use crate::external_editor_integration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptFooterOverride {
    ExternalEditorHint,
}

/// The context shown in the 1-line prompt footer under the composer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptFooterContext {
    /// Working directory shown in the footer.
    pub working_dir: PathBuf,
    /// Current git branch for `working_dir`, when available.
    pub git_branch: Option<String>,
    /// Whether YOLO is active for the current session.
    pub yolo_active: bool,
    /// Whether the CLI `--yolo` flag forces YOLO on for this process.
    pub yolo_cli_override: bool,
}

impl PromptFooterContext {
    /// Create a new prompt footer context.
    ///
    /// Empty or whitespace-only branch names are treated as `None`.
    pub fn new(working_dir: PathBuf, git_branch: Option<String>) -> Self {
        Self {
            working_dir,
            git_branch: git_branch.and_then(|branch| (!branch.trim().is_empty()).then_some(branch)),
            yolo_active: false,
            yolo_cli_override: false,
        }
    }

    /// Set whether the current session should render the YOLO indicator.
    pub fn with_yolo_active(mut self, yolo_active: bool) -> Self {
        self.yolo_active = yolo_active;
        self
    }

    /// Record whether the CLI `--yolo` flag is forcing YOLO on for this process.
    pub fn with_yolo_cli_override(mut self, yolo_cli_override: bool) -> Self {
        self.yolo_cli_override = yolo_cli_override;
        self
    }

    /// Recompute the footer indicator after the persisted default YOLO setting changes.
    pub fn with_persisted_yolo_enabled(mut self, enabled: bool) -> Self {
        self.yolo_active = self.yolo_cli_override || enabled;
        self
    }
}

pub fn render_prompt_footer(
    area: Rect,
    buf: &mut Buffer,
    override_mode: Option<PromptFooterOverride>,
    working_dir: &Path,
    git_branch: Option<&str>,
    yolo_active: bool,
) {
    if area.is_empty() {
        return;
    }

    let line = match override_mode {
        Some(PromptFooterOverride::ExternalEditorHint) => Line::from(vec![
            " ".into(),
            Span::from(external_editor_integration::EXTERNAL_EDITOR_HINT).bold(),
        ]),
        None => {
            let dir_display =
                crate::text_formatting::format_directory_for_display(working_dir, Some(50));
            let mut spans: Vec<Span<'static>> = Vec::new();
            if yolo_active {
                spans.push(Span::from("▲YOLO").red().bold());
                spans.push(Span::from(" · ").dim());
            }

            spans.push(Span::from("ctrl+g"));
            spans.push(Span::from(" editor").dim());
            spans.push(Span::from(" · ").dim());

            if let Some(branch) = git_branch.filter(|branch| !branch.trim().is_empty()) {
                spans.push(Span::from(branch.to_string()).cyan());
                spans.push(Span::from(" ❯ ").dim());
            }

            spans.push(Span::from(dir_display).dim());
            Line::from(spans)
        }
    };

    // Match the legacy footer indent.
    let mut footer_rect = area;
    let indent = crate::ui_consts::LIVE_PREFIX_COLS;
    if footer_rect.width > indent {
        footer_rect.x += indent;
        footer_rect.width = footer_rect.width.saturating_sub(indent);
    }

    WidgetRef::render_ref(&line, footer_rect, buf);
}

#[cfg(test)]
pub fn render_prompt_footer_for_test(
    area: Rect,
    buf: &mut Buffer,
    override_mode: Option<PromptFooterOverride>,
    working_dir: &Path,
    git_branch: Option<&str>,
    yolo_active: bool,
) {
    render_prompt_footer(
        area,
        buf,
        override_mode,
        working_dir,
        git_branch,
        yolo_active,
    );
}
