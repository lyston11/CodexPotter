//! Startup helpers for locating the upstream `codex` CLI binary.
//!
//! CodexPotter spawns upstream processes (for example `codex app-server`) during interactive
//! workflows. This module resolves the command used for spawning:
//!
//! - When the user passes `--codex-bin` that looks like a path, validate it is an executable file.
//! - Otherwise, resolve it via `$PATH` (e.g. `codex`).
//!
//! It also provides user-facing error messages in both plain text (`Display`) and ANSI-rendered
//! form (used by the CLI/TUI). Snapshot tests cover the most important error formatting.

use std::fmt;
use std::path::Path;
use std::path::PathBuf;

use crate::path_utils;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodexBinError {
    NotFoundInPath { command: String },
    InvalidPath { path: PathBuf, reason: String },
}

impl fmt::Display for CodexBinError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CodexBinError::NotFoundInPath { command } => write!(
                f,
                "Failed to find `{command}` binary. codex-potter requires codex CLI installed locally. See https://developers.openai.com/codex/quickstart?setup=cli"
            ),
            CodexBinError::InvalidPath { path, reason } => write!(
                f,
                "Failed to find codex binary specified by `--codex-bin`: {} ({reason}).",
                path.display()
            ),
        }
    }
}

impl std::error::Error for CodexBinError {}

impl CodexBinError {
    pub fn render_ansi(&self) -> String {
        match self {
            CodexBinError::NotFoundInPath { command } => {
                let url = "https://developers.openai.com/codex/quickstart?setup=cli";
                ansi_red(format!(
                    "Failed to find `{command}` binary.\n\
                     codex-potter requires codex CLI installed locally.\n\
                     \n\
                     See {url}\n",
                    url = ansi_underline(url),
                ))
            }
            CodexBinError::InvalidPath { path, reason } => ansi_red(format!(
                "Failed to find codex binary specified by `--codex-bin`: {} ({reason}).\n",
                path.display()
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedCodexBin {
    pub command_for_spawn: String,
}

pub fn resolve_codex_bin(codex_bin: &str) -> Result<ResolvedCodexBin, CodexBinError> {
    if looks_like_path(codex_bin) {
        let path = path_utils::expand_tilde(Path::new(codex_bin));
        validate_executable_path(&path)?;
        return Ok(ResolvedCodexBin {
            command_for_spawn: path.display().to_string(),
        });
    }

    let resolved = which::which(codex_bin).map_err(|_| CodexBinError::NotFoundInPath {
        command: codex_bin.to_string(),
    })?;

    Ok(ResolvedCodexBin {
        command_for_spawn: resolved.display().to_string(),
    })
}

fn validate_executable_path(path: &Path) -> Result<(), CodexBinError> {
    let meta = std::fs::metadata(path).map_err(|err| CodexBinError::InvalidPath {
        path: path.to_path_buf(),
        reason: describe_metadata_error(&err),
    })?;

    if !meta.is_file() {
        return Err(CodexBinError::InvalidPath {
            path: path.to_path_buf(),
            reason: "not a file".to_string(),
        });
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = meta.permissions().mode();
        if mode & 0o111 == 0 {
            return Err(CodexBinError::InvalidPath {
                path: path.to_path_buf(),
                reason: "not executable".to_string(),
            });
        }
    }

    Ok(())
}

fn describe_metadata_error(err: &std::io::Error) -> String {
    match err.kind() {
        std::io::ErrorKind::NotFound => "does not exist".to_string(),
        std::io::ErrorKind::PermissionDenied => "permission denied".to_string(),
        _ => err.to_string(),
    }
}

fn looks_like_path(value: &str) -> bool {
    let path = Path::new(value);
    path.is_absolute()
        || value.contains('/')
        || value.contains('\\')
        || value.starts_with("./")
        || value.starts_with("../")
        || path_utils::is_home_relative_path_text(value)
}

fn ansi_red(text: String) -> String {
    format!("\u{1b}[31m{text}\u{1b}[0m")
}

fn ansi_underline(text: &str) -> String {
    format!("\u{1b}[4m{text}\u{1b}[24m")
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    #[cfg(windows)]
    use std::ffi::OsString;
    #[cfg(windows)]
    use std::sync::Mutex;
    #[cfg(windows)]
    use std::sync::OnceLock;

    fn normalize_newlines_for_vt100(input: &str) -> String {
        // Most terminals (and tty line disciplines) translate '\n' to '\r\n'. The vt100 parser
        // does not, so do it here to keep snapshots aligned with real output.
        let mut out = String::with_capacity(input.len());
        let mut prev_was_cr = false;
        for ch in input.chars() {
            if ch == '\n' {
                if !prev_was_cr {
                    out.push('\r');
                }
                out.push('\n');
                prev_was_cr = false;
            } else {
                prev_was_cr = ch == '\r';
                out.push(ch);
            }
        }
        out
    }

    fn ansi_to_vt100_contents(rendered: &str) -> String {
        let mut parser = vt100::Parser::new(10, 120, 0);
        let normalized = normalize_newlines_for_vt100(rendered);
        parser.process(normalized.as_bytes());
        parser.screen().contents()
    }

    #[cfg(windows)]
    fn path_env_test_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .expect("lock PATH env test mutex")
    }

    #[cfg(windows)]
    struct PathEnvGuard {
        original: Option<OsString>,
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    #[cfg(windows)]
    impl PathEnvGuard {
        fn set(value: OsString) -> Self {
            let lock = path_env_test_lock();
            let original = std::env::var_os("PATH");
            unsafe {
                std::env::set_var("PATH", value);
            }
            Self {
                original,
                _lock: lock,
            }
        }
    }

    #[cfg(windows)]
    impl Drop for PathEnvGuard {
        fn drop(&mut self) {
            match self.original.take() {
                Some(value) => unsafe { std::env::set_var("PATH", value) },
                None => unsafe { std::env::remove_var("PATH") },
            }
        }
    }

    #[test]
    fn render_ansi_codex_bin_errors_keep_variant_specific_details() {
        let not_found = CodexBinError::NotFoundInPath {
            command: "codex".to_string(),
        };
        let rendered = not_found.render_ansi();

        assert!(rendered.contains("https://developers.openai.com/codex/quickstart?setup=cli"));
        assert!(rendered.contains("\u{1b}[31m"), "should include red ANSI");
        assert!(rendered.contains("\u{1b}[4m"), "should underline link");
        insta::assert_snapshot!(
            "not_found_error_snapshot_includes_link_and_styles",
            ansi_to_vt100_contents(&rendered)
        );

        let invalid_path = CodexBinError::InvalidPath {
            path: PathBuf::from("/nope/codex"),
            reason: "does not exist".to_string(),
        };
        let rendered = invalid_path.render_ansi();

        assert!(!rendered.contains("quickstart?setup=cli"));
        insta::assert_snapshot!(
            "invalid_path_error_snapshot_has_no_quickstart_link",
            ansi_to_vt100_contents(&rendered)
        );
    }

    #[test]
    fn display_codex_bin_errors_are_plain_text_and_include_variant_details() {
        let not_found = CodexBinError::NotFoundInPath {
            command: "codex".to_string(),
        };
        let rendered = not_found.to_string();

        assert!(rendered.contains("Failed to find `codex` binary"));
        assert!(rendered.contains("quickstart?setup=cli"));
        assert!(
            !rendered.contains("\u{1b}"),
            "should not include ANSI sequences"
        );

        let invalid_path = CodexBinError::InvalidPath {
            path: PathBuf::from("/nope/codex"),
            reason: "does not exist".to_string(),
        };
        let rendered = invalid_path.to_string();

        assert!(rendered.contains("--codex-bin"));
        assert!(rendered.contains("/nope/codex"));
        assert!(rendered.contains("does not exist"));
        assert!(
            !rendered.contains("\u{1b}"),
            "should not include ANSI sequences"
        );
    }

    #[test]
    fn validate_executable_path_maps_not_found() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("missing-codex-bin");

        let err = validate_executable_path(&path).expect_err("should fail");
        assert_eq!(
            err,
            CodexBinError::InvalidPath {
                path,
                reason: "does not exist".to_string()
            }
        );
    }

    #[test]
    #[cfg(windows)]
    fn resolve_codex_bin_supports_windows_home_relative_paths_and_cmd_shims() {
        let home = dirs::home_dir().expect("home dir");
        let temp = tempfile::Builder::new()
            .prefix("codex-potter-startup-")
            .tempdir_in(&home)
            .expect("tempdir in home");
        let executable = temp.path().join("codex.exe");
        std::fs::write(&executable, "echo test").expect("write executable placeholder");

        let temp_name = temp
            .path()
            .file_name()
            .and_then(|name| name.to_str())
            .expect("tempdir name");
        let input = format!("~\\{temp_name}\\codex.exe");

        let resolved = resolve_codex_bin(&input).expect("resolve codex bin");

        assert_eq!(resolved.command_for_spawn, executable.display().to_string());

        let shim_dir = tempfile::tempdir().expect("tempdir");
        let shim = shim_dir.path().join("codex.cmd");
        std::fs::write(&shim, "@echo off\r\n").expect("write cmd shim");
        let _path_guard = PathEnvGuard::set(shim_dir.path().as_os_str().to_os_string());

        let resolved = resolve_codex_bin("codex").expect("resolve codex shim");

        assert_eq!(resolved.command_for_spawn, shim.display().to_string());
    }
}
