//! Resume picker prompt.
//!
//! # Divergence from upstream Codex TUI
//!
//! `codex-potter` reuses the `/list` projects overlay UI for `codex-potter resume` selection.
//! The overlay provider is owned by the CLI workflow layer, so this module remains UI-only.

use std::path::PathBuf;
use std::time::SystemTime;

use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyEventKind;
use crossterm::event::KeyModifiers;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::prelude::Widget as _;
use ratatui::style::Stylize as _;
use ratatui::text::Line;
use ratatui::widgets::Clear;
use ratatui::widgets::Paragraph;
use tokio_stream::StreamExt;

use crate::key_hint;
use crate::tui::Tui;
use crate::tui::TuiEvent;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResumePickerOutcome {
    StartFresh,
    Resume(PathBuf),
    Exit,
}

pub async fn run_resume_picker_prompt_with_tui(
    tui: &mut Tui,
    projects_overlay_provider: crate::ProjectsOverlayProviderChannels,
) -> anyhow::Result<ResumePickerOutcome> {
    let alt = AltScreenGuard::enter(tui);

    let mut response_rx = projects_overlay_provider.response_rx;
    let request_tx = projects_overlay_provider.request_tx;

    let now = SystemTime::now();
    let (mut screen, request) = ResumePickerOverlay::new(now);
    let _ = request_tx.send(request);

    alt.tui.draw(u16::MAX, |frame| {
        screen.set_now(SystemTime::now());
        screen.render(frame.area(), frame.buffer_mut());
    })?;

    let events = alt.tui.event_stream();
    tokio::pin!(events);

    while !screen.is_done() {
        tokio::select! {
            maybe_event = events.next() => {
                let Some(event) = maybe_event else {
                    break;
                };
                match event {
                    TuiEvent::Key(key_event) => {
                        if let Some(request) = screen.handle_key(key_event) {
                            let _ = request_tx.send(request);
                        }
                        alt.tui.frame_requester().schedule_frame();
                    }
                    TuiEvent::Paste(_) => {}
                    TuiEvent::Draw => {
                        alt.tui.draw(u16::MAX, |frame| {
                            screen.set_now(SystemTime::now());
                            screen.render(frame.area(), frame.buffer_mut());
                        })?;
                    }
                }
            }
            maybe_response = response_rx.recv() => {
                let Some(response) = maybe_response else {
                    break;
                };
                if let Some(request) = screen.handle_overlay_response(response) {
                    let _ = request_tx.send(request);
                }
                alt.tui.frame_requester().schedule_frame();
            }
        }
    }

    Ok(screen
        .take_outcome()
        .unwrap_or(ResumePickerOutcome::StartFresh))
}

/// RAII guard that ensures we leave the alt-screen on scope exit.
struct AltScreenGuard<'a> {
    tui: &'a mut Tui,
}

impl<'a> AltScreenGuard<'a> {
    fn enter(tui: &'a mut Tui) -> Self {
        let _ = tui.enter_alt_screen();
        Self { tui }
    }
}

impl Drop for AltScreenGuard<'_> {
    fn drop(&mut self) {
        let _ = self.tui.leave_alt_screen();
    }
}

struct ResumePickerOverlay {
    overlay: crate::projects_overlay::ProjectsOverlay,
    now: SystemTime,
    outcome: Option<ResumePickerOutcome>,
}

impl ResumePickerOverlay {
    fn new(now: SystemTime) -> (Self, crate::ProjectsOverlayRequest) {
        let mut overlay = crate::projects_overlay::ProjectsOverlay::default();
        let request = overlay.open_or_refresh();
        (
            Self {
                overlay,
                now,
                outcome: None,
            },
            request,
        )
    }

    fn set_now(&mut self, now: SystemTime) {
        self.now = now;
    }

    fn is_done(&self) -> bool {
        self.outcome.is_some()
    }

    fn take_outcome(&mut self) -> Option<ResumePickerOutcome> {
        self.outcome.take()
    }

    fn handle_overlay_response(
        &mut self,
        response: crate::ProjectsOverlayResponse,
    ) -> Option<crate::ProjectsOverlayRequest> {
        match response {
            crate::ProjectsOverlayResponse::List { projects, error } => {
                self.overlay.on_projects_list(projects, error)
            }
            crate::ProjectsOverlayResponse::Details { details } => {
                if self.overlay.is_open() {
                    self.overlay.on_project_details(details);
                }
                None
            }
        }
    }

    fn handle_key(&mut self, key_event: KeyEvent) -> Option<crate::ProjectsOverlayRequest> {
        if key_event.kind == KeyEventKind::Release {
            return None;
        }

        if key_event.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key_event.code, KeyCode::Char('c'))
        {
            self.outcome = Some(ResumePickerOutcome::Exit);
            return None;
        }

        // Keep Ctrl+L reserved for the live overlay toggle; do not close the resume picker.
        if key_event.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key_event.code, KeyCode::Char('l'))
        {
            return None;
        }

        if key_event.modifiers == KeyModifiers::NONE {
            match key_event.code {
                KeyCode::Esc => {
                    self.outcome = Some(ResumePickerOutcome::StartFresh);
                    return None;
                }
                KeyCode::Enter => {
                    if let Some(project_dir) = self.overlay.selected_project_dir() {
                        self.outcome = Some(ResumePickerOutcome::Resume(project_dir));
                    }
                    return None;
                }
                _ => {}
            }
        }

        self.overlay.handle_key_event(key_event)
    }

    fn render(&mut self, area: Rect, buf: &mut Buffer) {
        Clear.render(area, buf);
        self.overlay.render(area, buf, self.now);
        render_resume_picker_footer(area, buf);
    }
}

fn render_resume_picker_footer(area: Rect, buf: &mut Buffer) {
    if area.height == 0 {
        return;
    }

    let footer_area = Rect::new(area.x, area.bottom().saturating_sub(1), area.width, 1);
    if footer_area.is_empty() {
        return;
    }

    Clear.render(footer_area, buf);

    let inset_area = Rect::new(
        footer_area.x.saturating_add(2),
        footer_area.y,
        footer_area.width.saturating_sub(4),
        footer_area.height,
    );
    if inset_area.is_empty() {
        return;
    }

    let hint_line = resume_picker_footer_hint_line(inset_area.width);
    Paragraph::new(hint_line).render(inset_area, buf);
}

fn resume_picker_footer_hint_line(width: u16) -> Line<'static> {
    let variants = resume_picker_footer_hint_variants();
    let fallback = variants.last().cloned().unwrap_or_default();
    variants
        .into_iter()
        .find(|line| line.width() <= usize::from(width))
        .unwrap_or(fallback)
}

fn resume_picker_footer_hint_variants() -> Vec<Line<'static>> {
    vec![
        Line::from(vec![
            key_hint::plain(KeyCode::Enter).into(),
            " resume ".dim(),
            "  ".into(),
            key_hint::plain(KeyCode::Esc).into(),
            " new ".dim(),
            "  ".into(),
            key_hint::ctrl(KeyCode::Char('c')).into(),
            " quit ".dim(),
            "  ".into(),
            key_hint::plain(KeyCode::Tab).into(),
            " maximize ".dim(),
            "  ".into(),
            key_hint::plain(KeyCode::Up).into(),
            "/".dim(),
            key_hint::plain(KeyCode::Down).into(),
            " browse".dim(),
        ]),
        Line::from(vec![
            key_hint::plain(KeyCode::Enter).into(),
            " resume ".dim(),
            "  ".into(),
            key_hint::plain(KeyCode::Esc).into(),
            " new ".dim(),
            "  ".into(),
            key_hint::ctrl(KeyCode::Char('c')).into(),
            " quit ".dim(),
            "  ".into(),
            key_hint::plain(KeyCode::Up).into(),
            "/".dim(),
            key_hint::plain(KeyCode::Down).into(),
            " browse".dim(),
        ]),
        Line::from(vec![
            key_hint::plain(KeyCode::Enter).into(),
            " resume ".dim(),
            "  ".into(),
            key_hint::plain(KeyCode::Esc).into(),
            " new ".dim(),
            "  ".into(),
            key_hint::ctrl(KeyCode::Char('c')).into(),
            " quit ".dim(),
        ]),
        Line::from(vec![
            key_hint::plain(KeyCode::Enter).into(),
            " resume ".dim(),
            "  ".into(),
            key_hint::plain(KeyCode::Esc).into(),
            " new ".dim(),
        ]),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    use codex_protocol::protocol::PotterProjectDetails;
    use codex_protocol::protocol::PotterProjectListEntry;
    use pretty_assertions::assert_eq;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use std::time::Duration;
    use std::time::UNIX_EPOCH;

    #[test]
    fn resume_picker_overlay_renders_projects_overlay_with_resume_footer() {
        let now = UNIX_EPOCH + Duration::from_secs(120);
        let (mut screen, _) = ResumePickerOverlay::new(now);

        screen.handle_overlay_response(crate::ProjectsOverlayResponse::List {
            projects: vec![PotterProjectListEntry {
                project_dir: PathBuf::from(".codexpotter/projects/2026/04/16/1"),
                progress_file: PathBuf::from(".codexpotter/projects/2026/04/16/1/MAIN.md"),
                description: "Resume picker uses /list overlay".to_string(),
                started_at_unix_secs: Some(1),
                rounds: 4,
                status: codex_protocol::protocol::PotterProjectListStatus::Succeeded,
            }],
            error: None,
        });

        screen.handle_overlay_response(crate::ProjectsOverlayResponse::Details {
            details: PotterProjectDetails {
                project_dir: PathBuf::from(".codexpotter/projects/2026/04/16/1"),
                progress_file: PathBuf::from(".codexpotter/projects/2026/04/16/1/MAIN.md"),
                git_branch: Some("main".to_string()),
                rounds: vec![codex_protocol::protocol::PotterProjectRoundSummary {
                    round_current: 1,
                    round_total: 4,
                    final_message_unix_secs: Some(1),
                    final_message: Some(String::from("**Done**")),
                }],
                error: None,
            },
        });

        let mut terminal = Terminal::new(TestBackend::new(80, 18)).expect("terminal");
        terminal
            .draw(|frame| {
                let area = frame.area();
                screen.render(area, frame.buffer_mut());
            })
            .expect("draw");

        insta::assert_snapshot!(terminal.backend());
    }

    #[test]
    fn resume_picker_overlay_esc_starts_fresh() {
        let (mut screen, _) = ResumePickerOverlay::new(SystemTime::UNIX_EPOCH);
        screen.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(screen.take_outcome(), Some(ResumePickerOutcome::StartFresh));
    }

    #[test]
    fn resume_picker_overlay_ctrl_c_exits() {
        let (mut screen, _) = ResumePickerOverlay::new(SystemTime::UNIX_EPOCH);
        screen.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert_eq!(screen.take_outcome(), Some(ResumePickerOutcome::Exit));
    }

    #[test]
    fn resume_picker_overlay_ctrl_l_does_not_close() {
        let (mut screen, _) = ResumePickerOverlay::new(SystemTime::UNIX_EPOCH);
        assert!(screen.overlay.is_open(), "expected overlay to start open");
        screen.handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL));
        assert!(screen.overlay.is_open(), "expected Ctrl+L not to close");
    }

    #[test]
    fn resume_picker_overlay_enter_resumes_selected_project_dir() {
        let (mut screen, _) = ResumePickerOverlay::new(SystemTime::UNIX_EPOCH);
        screen.handle_overlay_response(crate::ProjectsOverlayResponse::List {
            projects: vec![PotterProjectListEntry {
                project_dir: PathBuf::from(".codexpotter/projects/2026/04/16/1"),
                progress_file: PathBuf::from(".codexpotter/projects/2026/04/16/1/MAIN.md"),
                description: "Project".to_string(),
                started_at_unix_secs: Some(1),
                rounds: 1,
                status: codex_protocol::protocol::PotterProjectListStatus::Succeeded,
            }],
            error: None,
        });

        screen.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(
            screen.take_outcome(),
            Some(ResumePickerOutcome::Resume(PathBuf::from(
                ".codexpotter/projects/2026/04/16/1"
            )))
        );
    }
}
