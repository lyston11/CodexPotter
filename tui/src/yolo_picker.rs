//! Builds the `/yolo` picker dialog for the TUI.

use ratatui::style::Stylize as _;
use ratatui::text::Line;

use crate::app_event::AppEvent;
use crate::bottom_pane::SelectionItem;
use crate::bottom_pane::SelectionViewParams;
use crate::bottom_pane::popup_consts::standard_popup_hint_line;

/// Builds [`SelectionViewParams`] for the `/yolo` picker dialog.
pub fn build_yolo_picker_params(current_enabled: bool) -> SelectionViewParams {
    let items = vec![
        SelectionItem {
            name: "Off".to_string(),
            description: Some("Keep approvals and sandboxing enabled (default).".to_string()),
            is_current: !current_enabled,
            is_default: true,
            dismiss_on_select: true,
            actions: vec![Box::new(|tx| {
                tx.send(AppEvent::YoloSelected { enabled: false });
            })],
            ..Default::default()
        },
        SelectionItem {
            name: "On".to_string(),
            description: Some(
                "Disable approvals and sandboxing for all sessions (unsafe).".to_string(),
            ),
            is_current: current_enabled,
            dismiss_on_select: true,
            actions: vec![Box::new(|tx| {
                tx.send(AppEvent::YoloSelected { enabled: true });
            })],
            ..Default::default()
        },
    ];

    let footer_note = Some(
        Line::from(vec![
            "Applies to all sessions. CLI flag ".into(),
            "--yolo".cyan(),
            " always enables YOLO for the current run.".into(),
        ])
        .dim(),
    );

    SelectionViewParams {
        title: Some("YOLO".to_string()),
        subtitle: Some("Choose whether to enable YOLO by default".to_string()),
        footer_note,
        footer_hint: Some(standard_popup_hint_line()),
        items,
        initial_selected_idx: Some(if current_enabled { 1 } else { 0 }),
        ..Default::default()
    }
}
