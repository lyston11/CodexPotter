//! Helpers for extracting a compact status header from agent commentary markdown.
//!
//! In `Verbosity::Minimal`, CodexPotter treats `phase = commentary` agent messages as progress
//! updates: they update the shimmer/status header instead of rendering as transcript items.

/// Derive a single-line status header from `message`.
///
/// Selection rules:
/// - Prefer the first Markdown bold span (`**...**`), matching reasoning-status extraction.
/// - Otherwise, fall back to the first non-empty trimmed line.
pub fn status_header_from_commentary(message: &str) -> Option<String> {
    let trimmed = message.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Some(header) = crate::reasoning_status::extract_first_bold(trimmed) {
        return Some(header);
    }

    trimmed
        .lines()
        .find_map(|line| (!line.trim().is_empty()).then(|| line.trim().to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn status_header_from_commentary_prefers_first_bold_span() {
        assert_eq!(
            status_header_from_commentary("**Updating progress file**\n\nDetails..."),
            Some("Updating progress file".to_string())
        );
    }

    #[test]
    fn status_header_from_commentary_falls_back_to_first_non_empty_line() {
        assert_eq!(
            status_header_from_commentary("\n\nWorking...\nNext line"),
            Some("Working...".to_string())
        );
    }

    #[test]
    fn status_header_from_commentary_ignores_empty_input() {
        assert_eq!(status_header_from_commentary("   \n"), None);
    }
}
