//! Lightweight helpers for user-facing filesystem paths.
//!
//! These helpers are intended for CLI inputs and log messages:
//! - [`expand_tilde`] expands `~` / `~/...` into the user's home directory.
//! - [`display_with_tilde`] converts paths under the home directory back into a `~/...` display
//!   form for readability.
//!
//! This module intentionally does **not** implement full shell expansion (no `$VAR`, no `~user`),
//! and it is best-effort when the platform does not expose a home directory.

use std::path::Path;
use std::path::PathBuf;

/// Expand a `~` / `~/...` path into the user's home directory.
///
/// Returns the original `path` unchanged when:
/// - the input is not valid UTF-8
/// - the input does not start with `~`
/// - the home directory cannot be determined
pub fn expand_tilde(path: &Path) -> PathBuf {
    expand_tilde_from_home(path, dirs::home_dir().as_deref())
}

/// Expand a `~` / `~/...` path against an explicit home directory.
///
/// This keeps all user-facing tilde handling in one place while allowing callers that already
/// resolved a home directory to avoid re-querying global state.
pub fn expand_tilde_from_home(path: &Path, home: Option<&Path>) -> PathBuf {
    let Some(path_str) = path.to_str() else {
        return path.to_path_buf();
    };
    if path_str == "~" {
        return home
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from(path_str));
    }
    let rest = path_str.strip_prefix("~/").or_else(|| {
        if cfg!(windows) {
            path_str.strip_prefix("~\\")
        } else {
            None
        }
    });
    let Some(rest) = rest else {
        return path.to_path_buf();
    };
    let Some(home) = home else {
        return path.to_path_buf();
    };
    home.join(rest)
}

/// Display a path using `~` when it is under the user's home directory.
///
/// Returns the default `path.display()` string when the home directory cannot be determined or
/// when `path` is outside the home directory.
pub fn display_with_tilde(path: &Path) -> String {
    let Some(home) = dirs::home_dir() else {
        return path.display().to_string();
    };

    let Ok(stripped) = path.strip_prefix(&home) else {
        return path.display().to_string();
    };

    if stripped.as_os_str().is_empty() {
        return "~".to_string();
    }

    format!("~/{}", stripped.display())
}

#[cfg(test)]
mod tests {
    use super::display_with_tilde;
    use super::expand_tilde;
    use super::expand_tilde_from_home;
    use pretty_assertions::assert_eq;
    use std::path::Path;
    use std::path::PathBuf;

    #[test]
    fn expand_tilde_returns_original_when_no_tilde_prefix() {
        let path = PathBuf::from("foo").join("bar");
        assert_eq!(expand_tilde(&path), path);
    }

    #[test]
    fn expand_tilde_expands_home_when_available() {
        let Some(home) = dirs::home_dir() else {
            assert_eq!(expand_tilde(Path::new("~")), PathBuf::from("~"));
            return;
        };

        assert_eq!(expand_tilde(Path::new("~")), home);
        assert_eq!(expand_tilde(Path::new("~/nested")), home.join("nested"));
    }

    #[test]
    fn expand_tilde_from_home_uses_explicit_home_directory() {
        let home = Path::new("/tmp/example-home");

        assert_eq!(
            expand_tilde_from_home(Path::new("~/nested"), Some(home)),
            home.join("nested")
        );
    }

    #[test]
    #[cfg(windows)]
    fn expand_tilde_expands_windows_style_home_when_available() {
        let Some(home) = dirs::home_dir() else {
            return;
        };

        assert_eq!(
            expand_tilde(Path::new("~\\nested\\file")),
            home.join("nested").join("file")
        );
    }

    #[test]
    fn display_with_tilde_returns_original_when_not_under_home_or_home_missing() {
        let path = PathBuf::from("foo").join("bar");
        assert_eq!(display_with_tilde(&path), path.display().to_string());
    }

    #[test]
    fn display_with_tilde_uses_tilde_for_home_paths_when_available() {
        let Some(home) = dirs::home_dir() else {
            return;
        };

        assert_eq!(display_with_tilde(&home), "~".to_string());
        assert_eq!(
            display_with_tilde(&home.join("nested").join("file")),
            format!("~/{}", PathBuf::from("nested").join("file").display())
        );
    }
}
