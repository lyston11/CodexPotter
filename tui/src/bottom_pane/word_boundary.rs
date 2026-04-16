//! Word-boundary helpers used by the composer for word-wise navigation and deletion.
//!
//! # Divergence from upstream Codex TUI
//!
//! `codex-potter` uses ICU4X word segmentation (plus a small set of additional separator
//! characters) to provide more predictable <kbd>Alt</kbd>+<kbd>←</kbd>/<kbd>→</kbd> and
//! <kbd>Alt</kbd>+<kbd>Backspace</kbd> behavior across ASCII and non-ASCII text.
//!
//! Additionally, consecutive identical ASCII separator characters are treated as a single
//! segment (e.g. `====`), while mixed separators are split by character (e.g. `+-`).
//!
//! See `tui/AGENTS.md` ("Better word jump by using ICU4X word segmentations").

use icu_segmenter::WordSegmenter;
use icu_segmenter::options::WordBreakInvariantOptions;

/// ASCII punctuation treated as word separators in addition to ICU4X segmentation boundaries.
pub const WORD_SEPARATORS: &str = "`~!@#$%^&*()-=+[{]}\\|;:'\",.<>/?";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Segment {
    start: usize,
    end: usize,
    is_whitespace: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NonWhitespaceChunkKind {
    NonSeparator,
    Separator(char),
}

/// Return the byte index of the start of the previous word.
pub fn beginning_of_previous_word(text: &str, cursor_pos: usize) -> usize {
    let cursor_pos = clamp_pos_to_char_boundary(text, cursor_pos);
    if cursor_pos == 0 {
        return 0;
    }

    let segments = segments(text);
    let Some((probe_idx, _)) = text[..cursor_pos].char_indices().next_back() else {
        return 0;
    };

    let Some(mut segment_idx) = find_segment_containing(&segments, probe_idx) else {
        return 0;
    };

    while segments[segment_idx].is_whitespace {
        if segment_idx == 0 {
            return 0;
        }
        segment_idx -= 1;
    }

    segments[segment_idx].start
}

/// Return the byte index of the end of the next word.
pub fn end_of_next_word(text: &str, cursor_pos: usize) -> usize {
    let cursor_pos = clamp_pos_to_char_boundary(text, cursor_pos);
    if cursor_pos >= text.len() {
        return text.len();
    }

    let segments = segments(text);
    let Some(mut segment_idx) = segments.iter().position(|s| s.end > cursor_pos) else {
        return text.len();
    };

    while segments[segment_idx].is_whitespace {
        segment_idx += 1;
        if segment_idx >= segments.len() {
            return text.len();
        }
    }

    segments[segment_idx].end
}

fn clamp_pos_to_char_boundary(text: &str, pos: usize) -> usize {
    let mut pos = pos.min(text.len());
    while pos > 0 && !text.is_char_boundary(pos) {
        pos -= 1;
    }
    pos
}

fn segments(text: &str) -> Vec<Segment> {
    if text.is_empty() {
        return Vec::new();
    }

    let segmenter = WordSegmenter::new_auto(WordBreakInvariantOptions::default());

    let mut segments = Vec::new();

    let mut iter = text.char_indices();
    let Some((_, first_ch)) = iter.next() else {
        return Vec::new();
    };

    let mut run_start = 0;
    let mut run_is_whitespace = first_ch.is_whitespace();

    for (idx, ch) in iter {
        let is_whitespace = ch.is_whitespace();
        if is_whitespace == run_is_whitespace {
            continue;
        }

        push_run(
            text,
            &segmenter,
            run_start..idx,
            run_is_whitespace,
            &mut segments,
        );
        run_start = idx;
        run_is_whitespace = is_whitespace;
    }

    push_run(
        text,
        &segmenter,
        run_start..text.len(),
        run_is_whitespace,
        &mut segments,
    );

    segments
}

fn push_run(
    text: &str,
    segmenter: &icu_segmenter::WordSegmenterBorrowed<'static>,
    run: std::ops::Range<usize>,
    is_whitespace: bool,
    out: &mut Vec<Segment>,
) {
    if run.start >= run.end {
        return;
    }

    if is_whitespace {
        out.push(Segment {
            start: run.start,
            end: run.end,
            is_whitespace: true,
        });
        return;
    }

    let slice = &text[run.clone()];
    let mut iter = slice.char_indices();
    let Some((_, first_ch)) = iter.next() else {
        return;
    };

    let mut chunk_start = run.start;
    let mut chunk_kind = non_whitespace_chunk_kind(first_ch);

    for (idx, ch) in iter {
        let kind = non_whitespace_chunk_kind(ch);
        if kind == chunk_kind {
            continue;
        }

        push_non_whitespace_chunk(
            text,
            segmenter,
            chunk_start..run.start + idx,
            chunk_kind,
            out,
        );
        chunk_start = run.start + idx;
        chunk_kind = kind;
    }

    push_non_whitespace_chunk(text, segmenter, chunk_start..run.end, chunk_kind, out);
}

fn non_whitespace_chunk_kind(ch: char) -> NonWhitespaceChunkKind {
    if WORD_SEPARATORS.contains(ch) {
        NonWhitespaceChunkKind::Separator(ch)
    } else {
        NonWhitespaceChunkKind::NonSeparator
    }
}

fn push_non_whitespace_chunk(
    text: &str,
    segmenter: &icu_segmenter::WordSegmenterBorrowed<'static>,
    chunk: std::ops::Range<usize>,
    kind: NonWhitespaceChunkKind,
    out: &mut Vec<Segment>,
) {
    if chunk.start >= chunk.end {
        return;
    }

    match kind {
        NonWhitespaceChunkKind::Separator(_) => out.push(Segment {
            start: chunk.start,
            end: chunk.end,
            is_whitespace: false,
        }),
        NonWhitespaceChunkKind::NonSeparator => push_icu_segments(text, segmenter, chunk, out),
    }
}

fn push_icu_segments(
    text: &str,
    segmenter: &icu_segmenter::WordSegmenterBorrowed<'static>,
    chunk: std::ops::Range<usize>,
    out: &mut Vec<Segment>,
) {
    let slice = &text[chunk.clone()];
    let mut breakpoints: Vec<usize> = segmenter.segment_str(slice).collect();
    if breakpoints.first().copied() != Some(0) {
        breakpoints.insert(0, 0);
    }
    if breakpoints.last().copied() != Some(slice.len()) {
        breakpoints.push(slice.len());
    }

    for w in breakpoints.windows(2) {
        let start = chunk.start + w[0];
        let end = chunk.start + w[1];
        if start >= end {
            continue;
        }
        out.push(Segment {
            start,
            end,
            is_whitespace: false,
        });
    }
}

fn find_segment_containing(segments: &[Segment], pos: usize) -> Option<usize> {
    segments.iter().position(|s| pos >= s.start && pos < s.end)
}
