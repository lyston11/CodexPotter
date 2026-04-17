use std::io;
use std::path::Path;
use std::path::PathBuf;

/// Resolve the effective Codex home directory for config/theme/runtime consumers.
///
/// Divergence (codex-potter): expand `CODEX_HOME=~/...` and Windows-native
/// `CODEX_HOME=~\...` before validating the directory so shell-independent
/// environment settings behave the same as CLI path inputs.
pub fn find_codex_home() -> io::Result<PathBuf> {
    let home_dir = dirs::home_dir();
    let codex_home_env = std::env::var("CODEX_HOME").ok();
    find_codex_home_from_env(home_dir.as_deref(), codex_home_env.as_deref())
}

/// Resolve the configured Codex home path for discovery flows that do not require the directory to
/// exist yet (for example skill root enumeration).
pub fn codex_home_from_env_or_home(home_dir: Option<&Path>) -> Option<PathBuf> {
    let codex_home_env = std::env::var("CODEX_HOME").ok();
    codex_home_from_env_or_home_with_env(home_dir, codex_home_env.as_deref())
}

fn codex_home_from_env_or_home_with_env(
    home_dir: Option<&Path>,
    codex_home_env: Option<&str>,
) -> Option<PathBuf> {
    expand_codex_home_env_path(home_dir, codex_home_env)
        .or_else(|| home_dir.map(|home_dir| home_dir.join(".codex")))
}

fn find_codex_home_from_env(
    home_dir: Option<&Path>,
    codex_home_env: Option<&str>,
) -> io::Result<PathBuf> {
    match codex_home_env.filter(|value| !value.is_empty()) {
        Some(val) => {
            let path = expand_home_relative_path(val, home_dir);
            let metadata = std::fs::metadata(&path).map_err(|err| match err.kind() {
                std::io::ErrorKind::NotFound => std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("CODEX_HOME points to {val:?}, but that path does not exist"),
                ),
                _ => std::io::Error::new(
                    err.kind(),
                    format!("failed to read CODEX_HOME {val:?}: {err}"),
                ),
            })?;

            if !metadata.is_dir() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("CODEX_HOME points to {val:?}, but that path is not a directory"),
                ));
            }

            path.canonicalize().map_err(|err| {
                std::io::Error::new(
                    err.kind(),
                    format!("failed to canonicalize CODEX_HOME {val:?}: {err}"),
                )
            })
        }
        None => {
            let Some(home_dir) = home_dir else {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "Could not find home directory",
                ));
            };
            Ok(home_dir.join(".codex"))
        }
    }
}

fn expand_codex_home_env_path(
    home_dir: Option<&Path>,
    codex_home_env: Option<&str>,
) -> Option<PathBuf> {
    let value = codex_home_env.filter(|value| !value.is_empty())?;
    Some(expand_home_relative_path(value, home_dir))
}

fn expand_home_relative_path(path_text: &str, home_dir: Option<&Path>) -> PathBuf {
    let Some(home_dir) = home_dir else {
        return PathBuf::from(path_text);
    };

    if path_text == "~" {
        return home_dir.to_path_buf();
    }

    let rest = path_text.strip_prefix("~/").or_else(|| {
        if cfg!(windows) {
            path_text.strip_prefix("~\\")
        } else {
            None
        }
    });
    let Some(rest) = rest else {
        return PathBuf::from(path_text);
    };

    home_dir.join(rest)
}

#[cfg(test)]
mod tests {
    use super::codex_home_from_env_or_home_with_env;
    use super::find_codex_home_from_env;
    use pretty_assertions::assert_eq;

    #[test]
    fn codex_home_from_env_or_home_supports_default_and_home_relative_env_paths() {
        let home_dir = tempfile::tempdir().expect("home dir");
        let mut cases = vec![
            (None, home_dir.path().join(".codex")),
            (Some("~/custom-codex"), home_dir.path().join("custom-codex")),
        ];
        if cfg!(windows) {
            cases.push((
                Some("~\\custom-codex"),
                home_dir.path().join("custom-codex"),
            ));
        }

        for (input, expected) in cases {
            assert_eq!(
                codex_home_from_env_or_home_with_env(Some(home_dir.path()), input),
                Some(expected),
                "input: {input:?}"
            );
        }
    }

    #[test]
    fn find_codex_home_from_env_expands_supported_home_relative_env_paths_before_canonicalizing() {
        let home_dir = tempfile::tempdir().expect("home dir");
        let codex_home = home_dir.path().join("custom-codex");
        std::fs::create_dir_all(&codex_home).expect("create codex home");
        let expected = codex_home.canonicalize().expect("canonicalize codex home");
        let mut inputs = vec!["~/custom-codex"];
        if cfg!(windows) {
            inputs.push("~\\custom-codex");
        }

        for input in inputs {
            let resolved = find_codex_home_from_env(Some(home_dir.path()), Some(input))
                .expect("resolve codex home");
            assert_eq!(resolved, expected, "input: {input}");
        }
    }
}
