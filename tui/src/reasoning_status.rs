//! Shared reasoning-title extraction used by live shimmer updates and append-only exec output.

/// Tracks the latest status header inferred from reasoning markdown.
///
/// The tracker only returns a header when the visible title changes. Repeated deltas that keep the
/// same first bold span update internal state but do not trigger a new header emission.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ReasoningStatusTracker {
    buffer: String,
    current_header: Option<String>,
}

impl ReasoningStatusTracker {
    /// Create an empty tracker.
    pub fn new() -> Self {
        Self::default()
    }

    /// Clear all buffered reasoning text and the current header.
    pub fn reset(&mut self) {
        self.buffer.clear();
        self.current_header = None;
    }

    /// Reset the tracker at a reasoning section boundary.
    pub fn on_section_break(&mut self) {
        self.reset();
    }

    /// Reset the tracker once the reasoning item is finalized.
    pub fn on_final(&mut self) {
        self.reset();
    }

    /// Feed a new reasoning chunk and return the next visible header, if it changed.
    pub fn on_delta(&mut self, delta: &str) -> Option<String> {
        self.buffer.push_str(delta);
        let header = extract_first_bold(&self.buffer)?;
        if self.current_header.as_deref() == Some(header.as_str()) {
            return None;
        }
        self.current_header = Some(header.clone());
        Some(header)
    }

    /// Return the current visible header, if any.
    pub fn current_header(&self) -> Option<String> {
        self.current_header.clone()
    }
}

/// Extract the first Markdown bold span (`**...**`) from `s`.
pub fn extract_first_bold(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut i = 0usize;
    while i + 1 < bytes.len() {
        if bytes[i] == b'*' && bytes[i + 1] == b'*' {
            let start = i + 2;
            let mut j = start;
            while j + 1 < bytes.len() {
                if bytes[j] == b'*' && bytes[j + 1] == b'*' {
                    let trimmed = s[start..j].trim();
                    return (!trimmed.is_empty()).then(|| trimmed.to_string());
                }
                j += 1;
            }
            return None;
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn extract_first_bold_returns_first_markdown_bold_span() {
        assert_eq!(
            extract_first_bold("**Inspecting for code duplication**\n\nmore"),
            Some("Inspecting for code duplication".to_string())
        );
        assert_eq!(extract_first_bold("no bold here"), None);
        assert_eq!(extract_first_bold("**"), None);
        assert_eq!(extract_first_bold("**  ** trailing"), None);
        assert_eq!(
            extract_first_bold("prefix **first** then **second**"),
            Some("first".to_string())
        );
    }

    #[test]
    fn tracker_only_emits_when_header_changes() {
        let mut tracker = ReasoningStatusTracker::new();

        assert_eq!(tracker.on_delta("**Updating"), None);
        assert_eq!(
            tracker.on_delta(" progress file**"),
            Some("Updating progress file".to_string())
        );
        assert_eq!(tracker.on_delta("\nmore details"), None);

        tracker.on_section_break();
        assert_eq!(
            tracker.on_delta("**Updating progress file**"),
            Some("Updating progress file".to_string())
        );
    }
}
