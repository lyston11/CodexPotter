use crossterm::event::KeyCode;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;

use crate::key_hint;
use crate::render::renderable::Renderable;
use crate::wrapping::RtOptions;
use crate::wrapping::adaptive_wrap_lines;

/// Widget that displays a list of user messages queued while a task is in progress.
pub struct QueuedUserMessages {
    pub messages: Vec<String>,
}

impl QueuedUserMessages {
    pub fn new() -> Self {
        Self {
            messages: Vec::new(),
        }
    }

    fn as_renderable(&self, width: u16) -> Box<dyn Renderable> {
        if self.messages.is_empty() || width < 4 {
            return Box::new(());
        }

        let mut lines = vec![];

        for message in &self.messages {
            let wrapped = adaptive_wrap_lines(
                message.lines().map(|line| line.dim().italic()),
                RtOptions::new(width as usize)
                    .initial_indent(Line::from("  ↳ ".dim()))
                    .subsequent_indent(Line::from("    ")),
            );

            let len = wrapped.len();
            for line in wrapped.into_iter().take(3) {
                lines.push(line);
            }
            if len > 3 {
                lines.push(Line::from("    …".dim().italic()));
            }
        }

        lines.push(
            Line::from(vec![
                "    ".into(),
                key_hint::alt(KeyCode::Up).into(),
                " edit".into(),
            ])
            .dim(),
        );

        Box::new(Paragraph::new(lines))
    }
}

impl Renderable for QueuedUserMessages {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        if area.is_empty() {
            return;
        }

        self.as_renderable(area.width).render(area, buf);
    }

    fn desired_height(&self, width: u16) -> u16 {
        self.as_renderable(width).desired_height(width)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use insta::assert_snapshot;
    use pretty_assertions::assert_eq;

    #[test]
    fn desired_height_tracks_empty_and_single_message_queues() {
        let queue = QueuedUserMessages::new();
        assert_eq!(queue.desired_height(40), 0);
        let mut queue = QueuedUserMessages::new();
        queue.messages.push("Hello, world!".to_string());
        assert_eq!(queue.desired_height(40), 2);
    }

    fn render_to_debug(queue: &QueuedUserMessages, width: u16) -> String {
        let height = queue.desired_height(width);
        let mut buf = Buffer::empty(Rect::new(0, 0, width, height));
        queue.render(Rect::new(0, 0, width, height), &mut buf);
        format!("{buf:?}")
    }

    #[test]
    fn render_snapshots_cover_message_count_variants() {
        for (snapshot, messages) in [
            ("queued_user_messages_one_message", &["Hello, world!"][..]),
            (
                "queued_user_messages_two_messages",
                &["Hello, world!", "This is another message"][..],
            ),
            (
                "queued_user_messages_more_than_three_messages",
                &[
                    "Hello, world!",
                    "This is another message",
                    "This is a third message",
                    "This is a fourth message",
                ][..],
            ),
        ] {
            let mut queue = QueuedUserMessages::new();
            queue
                .messages
                .extend(messages.iter().map(|message| (*message).to_string()));
            assert_snapshot!(snapshot, render_to_debug(&queue, 40));
        }
    }

    #[test]
    fn render_snapshots_cover_wrapped_and_multiline_messages() {
        for (snapshot, messages) in [
            (
                "queued_user_messages_wrapped_message",
                &[
                    "This is a longer message that should be wrapped",
                    "This is another message",
                ][..],
            ),
            (
                "queued_user_messages_many_line_message",
                &["This is\na message\nwith many\nlines"][..],
            ),
        ] {
            let mut queue = QueuedUserMessages::new();
            queue
                .messages
                .extend(messages.iter().map(|message| (*message).to_string()));
            assert_snapshot!(snapshot, render_to_debug(&queue, 40));
        }
    }
}
