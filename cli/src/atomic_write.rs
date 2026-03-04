//! Small utilities for atomic-ish file writes.
//!
//! Currently this module provides [`write_atomic_text`], which writes a text file by creating a
//! temporary file in the same directory and then persisting it to the target path. This keeps the
//! final replacement atomic on platforms/filesystems where rename is atomic.

use std::path::Path;

use anyhow::Context;
use tempfile::NamedTempFile;

/// Write text to `path` by writing into a temp file in the same directory and persisting it.
///
/// Behavior:
/// - Creates the parent directory when missing.
/// - Ensures the resulting file ends with a trailing newline.
pub fn write_atomic_text(path: &Path, contents: &str) -> anyhow::Result<()> {
    let Some(parent) = path.parent() else {
        anyhow::bail!("invalid path for atomic write: {}", path.display());
    };
    std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;

    let mut tmp = NamedTempFile::new_in(parent).context("create temp file")?;
    use std::io::Write as _;
    tmp.write_all(contents.as_bytes())
        .context("write temp file")?;
    if !contents.ends_with('\n') {
        tmp.write_all(b"\n").context("write temp newline")?;
    }
    tmp.flush().context("flush temp file")?;

    tmp.persist(path).map_err(|err| {
        anyhow::Error::new(err.error).context(format!("persist file to {}", path.display()))
    })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_trailing_newline_and_creates_parent_dir() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("nested").join("file.txt");

        write_atomic_text(&path, "hello").expect("write atomic");

        let contents = std::fs::read_to_string(&path).expect("read");
        assert_eq!(contents, "hello\n");
    }
}
