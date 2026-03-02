use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::widgets::WidgetRef;

use super::popup_consts::MAX_POPUP_ROWS;
use super::scroll_state::ScrollState;
use super::selection_popup_common::GenericDisplayRow;
use super::selection_popup_common::measure_rows_height;
use super::selection_popup_common::render_rows;
use super::slash_commands;
use crate::render::Insets;
use crate::render::RectExt;
use crate::slash_command::SlashCommand;

/// Stateful popup UI for selecting a built-in slash command.
pub struct CommandPopup {
    command_filter: String,
    builtins: Vec<(&'static str, SlashCommand)>,
    state: ScrollState,
}

impl CommandPopup {
    pub fn new() -> Self {
        Self {
            command_filter: String::new(),
            builtins: slash_commands::builtins_for_input(),
            state: ScrollState::new(),
        }
    }

    /// Update the filter string based on the current composer text.
    ///
    /// The text passed in is expected to start with a leading '/'. Everything after the
    /// *first* '/' on the *first* line becomes the active filter that is used
    /// to narrow down the list of available commands.
    pub fn on_composer_text_change(&mut self, text: String) {
        let first_line = text.lines().next().unwrap_or("");

        if let Some(stripped) = first_line.strip_prefix('/') {
            // Extract the *first* token (sequence of non-whitespace
            // characters) after the slash so that `/mention something` still
            // shows the help for `/mention`.
            let token = stripped.trim_start();
            let cmd_token = token.split_whitespace().next().unwrap_or("");
            self.command_filter = cmd_token.to_string();
        } else {
            self.command_filter.clear();
        }

        let matches_len = self.filtered_items().len();
        self.state.clamp_selection(matches_len);
        self.state
            .ensure_visible(matches_len, MAX_POPUP_ROWS.min(matches_len));
    }

    /// Determine the preferred height of the popup for a given width.
    /// Accounts for wrapped descriptions so that long tooltips don't overflow.
    pub fn calculate_required_height(&self, width: u16) -> u16 {
        let rows = self.rows_from_matches(self.filtered());
        measure_rows_height(&rows, &self.state, MAX_POPUP_ROWS, width)
    }

    fn filtered(&self) -> Vec<(SlashCommand, Option<Vec<usize>>)> {
        let filter = self.command_filter.trim();
        let mut out: Vec<(SlashCommand, Option<Vec<usize>>)> = Vec::new();

        if filter.is_empty() {
            for (_, cmd) in self.builtins.iter() {
                out.push((*cmd, None));
            }
            return out;
        }

        let filter_lower = filter.to_lowercase();
        let filter_chars = filter.chars().count();
        let indices_for = || Some((0..filter_chars).collect());

        let mut exact: Vec<(SlashCommand, Option<Vec<usize>>)> = Vec::new();
        let mut prefix: Vec<(SlashCommand, Option<Vec<usize>>)> = Vec::new();

        for (_, cmd) in self.builtins.iter() {
            let display = cmd.command();
            let display_lower = display.to_lowercase();
            if display_lower == filter_lower {
                exact.push((*cmd, indices_for()));
                continue;
            }
            if display_lower.starts_with(&filter_lower) {
                prefix.push((*cmd, indices_for()));
            }
        }

        out.extend(exact);
        out.extend(prefix);
        out
    }

    fn filtered_items(&self) -> Vec<SlashCommand> {
        self.filtered().into_iter().map(|(c, _)| c).collect()
    }

    fn rows_from_matches(
        &self,
        matches: Vec<(SlashCommand, Option<Vec<usize>>)>,
    ) -> Vec<GenericDisplayRow> {
        matches
            .into_iter()
            .map(|(cmd, indices)| {
                let name = format!("/{}", cmd.command());
                GenericDisplayRow {
                    name,
                    // Indices are relative to `cmd.command()`, so add 1 to
                    // account for the leading '/' we render.
                    match_indices: indices.map(|v| v.into_iter().map(|i| i + 1).collect()),
                    display_shortcut: None,
                    description: Some(cmd.description().to_string()),
                    is_disabled: false,
                    disabled_reason: None,
                    wrap_indent: None,
                }
            })
            .collect()
    }

    /// Move the selection cursor one step up.
    pub fn move_up(&mut self) {
        let len = self.filtered_items().len();
        self.state.move_up_wrap(len);
        self.state.ensure_visible(len, MAX_POPUP_ROWS.min(len));
    }

    /// Move the selection cursor one step down.
    pub fn move_down(&mut self) {
        let matches_len = self.filtered_items().len();
        self.state.move_down_wrap(matches_len);
        self.state
            .ensure_visible(matches_len, MAX_POPUP_ROWS.min(matches_len));
    }

    /// Return currently selected command, if any.
    pub fn selected_item(&self) -> Option<SlashCommand> {
        let matches = self.filtered_items();
        self.state
            .selected_idx
            .and_then(|idx| matches.get(idx).copied())
    }
}

impl WidgetRef for &CommandPopup {
    fn render_ref(&self, area: Rect, buf: &mut Buffer) {
        let rows = self.rows_from_matches(self.filtered());
        render_rows(
            area.inset(Insets::tlbr(0, 2, 0, 0)),
            buf,
            &rows,
            &self.state,
            MAX_POPUP_ROWS,
            "no matches",
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn selecting_mention_by_exact_match() {
        let mut popup = CommandPopup::new();
        popup.on_composer_text_change("/mention".to_string());

        let selected = popup.selected_item();
        assert_eq!(selected, Some(SlashCommand::Mention));
    }
}
