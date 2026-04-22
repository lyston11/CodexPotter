//! Helpers for setting terminal window titles.
//!
//! CodexPotter runs primarily in interactive TUI mode. Setting the terminal title makes it
//! easier to identify the active workdir among multiple running sessions.

use std::io;
use std::io::Write as _;
use std::path::Path;

use anyhow::Context;

const CODEX_POTTER_TITLE_PREFIX: &str = "CodexPotter | ";

/// Returns the CodexPotter terminal title for an interactive session in `workdir`.
pub fn codexpotter_terminal_title(workdir: &Path) -> String {
    let workdir_display = crate::path_utils::display_with_tilde(workdir);
    sanitize_terminal_title(format!("{CODEX_POTTER_TITLE_PREFIX}{workdir_display}"))
}

/// Best-effort helper for setting the terminal title in interactive sessions.
///
/// This function is intentionally side-effectful (writes control sequences to stdout).
pub fn set_codexpotter_terminal_title(workdir: &Path) -> anyhow::Result<()> {
    let title = codexpotter_terminal_title(workdir);
    set_terminal_title(&title).with_context(|| format!("set terminal title to `{title}`"))
}

fn set_terminal_title(title: &str) -> anyhow::Result<()> {
    let mut out = io::stdout();
    crossterm::execute!(&mut out, crossterm::terminal::SetTitle(title))
        .context("execute SetTitle")?;
    out.flush().context("flush terminal title write")?;
    Ok(())
}

fn sanitize_terminal_title(title: String) -> String {
    title
        .chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use std::path::PathBuf;

    #[test]
    fn codexpotter_terminal_title_includes_prefix_and_workdir() {
        assert_eq!(
            codexpotter_terminal_title(&PathBuf::from("workdir")),
            "CodexPotter | workdir".to_string()
        );
    }

    #[test]
    fn codexpotter_terminal_title_sanitizes_control_chars() {
        assert_eq!(
            codexpotter_terminal_title(&PathBuf::from("work\ndir")),
            "CodexPotter | work dir".to_string()
        );
    }
}
