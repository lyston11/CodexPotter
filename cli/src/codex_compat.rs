//! Creates a Codex-compatible home directory for upstream processes.
//!
//! CodexPotter needs to spawn the upstream `codex` backend while keeping its own state under
//! `~/.codexpotter/`. The upstream backend expects a `CODEX_HOME` directory containing config,
//! auth, agent configs, skills, and rules. To avoid mutating the user's real Codex home, we create a
//! `~/.codexpotter/codex-compat/` directory that symlinks to the corresponding files/dirs in the
//! real `CODEX_HOME` (or `~/.codex` when unset).
//!
//! The resulting path is passed to upstream via `CODEX_HOME` so existing Codex configuration is
//! honored while CodexPotter continues to own its own on-disk artifacts.

use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;

const CODEX_COMPAT_ENTRY_NAMES: &[&str] = &[
    "AGENTS.md",
    "config.toml",
    "auth.json",
    "agents",
    "skills",
    "rules",
];

pub fn ensure_default_codex_compat_home() -> anyhow::Result<Option<PathBuf>> {
    let Some(home) = dirs::home_dir() else {
        return Ok(None);
    };
    let real_codex_home = resolve_real_codex_home(&home)?;
    ensure_codex_compat_home(&home, &real_codex_home).map(Some)
}

fn resolve_real_codex_home(home: &Path) -> anyhow::Result<PathBuf> {
    let codex_home_env = std::env::var("CODEX_HOME").ok();
    resolve_real_codex_home_from_env(home, codex_home_env.as_deref())
}

fn resolve_real_codex_home_from_env(
    home: &Path,
    codex_home_env: Option<&str>,
) -> anyhow::Result<PathBuf> {
    let Some(val) = codex_home_env.filter(|val| !val.is_empty()) else {
        return Ok(home.join(".codex"));
    };

    let path = PathBuf::from(val);
    let metadata = std::fs::metadata(&path).map_err(|err| match err.kind() {
        std::io::ErrorKind::NotFound => {
            anyhow::anyhow!("CODEX_HOME points to {val:?}, but that path does not exist")
        }
        _ => anyhow::anyhow!("failed to read CODEX_HOME {val:?}: {err}"),
    })?;
    if !metadata.is_dir() {
        anyhow::bail!("CODEX_HOME points to {val:?}, but that path is not a directory");
    }
    path.canonicalize()
        .map_err(|err| anyhow::anyhow!("failed to canonicalize CODEX_HOME {val:?}: {err}"))
}

fn ensure_codex_compat_home(home: &Path, real_codex_home: &Path) -> anyhow::Result<PathBuf> {
    let codex_home = home.join(".codexpotter").join("codex-compat");
    std::fs::create_dir_all(&codex_home)
        .with_context(|| format!("create directory {}", codex_home.display()))?;

    for entry_name in CODEX_COMPAT_ENTRY_NAMES {
        ensure_symlink(
            &codex_home.join(entry_name),
            &real_codex_home.join(entry_name),
        )?;
    }

    Ok(codex_home)
}

fn ensure_symlink(link_path: &Path, target_path: &Path) -> anyhow::Result<()> {
    if link_path == target_path {
        return Ok(());
    }
    if std::fs::symlink_metadata(link_path).is_ok() {
        return Ok(());
    }

    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target_path, link_path)
            .with_context(|| format!("create symlink {}", link_path.display()))?;
        Ok(())
    }

    #[cfg(windows)]
    {
        std::os::windows::fs::symlink_file(target_path, link_path)
            .with_context(|| format!("create symlink {}", link_path.display()))?;
        Ok(())
    }

    #[cfg(not(any(unix, windows)))]
    anyhow::bail!("symlinks are not supported on this platform");
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    #[cfg(unix)]
    fn ensures_codex_compat_home_and_links() {
        let home_dir = tempfile::tempdir().expect("home dir");
        let real_codex_home = tempfile::tempdir().expect("real codex home");
        let codex_home =
            ensure_codex_compat_home(home_dir.path(), real_codex_home.path()).expect("ensure home");

        assert!(codex_home.is_dir());

        for entry_name in CODEX_COMPAT_ENTRY_NAMES {
            let link_path = codex_home.join(entry_name);
            let link_meta = std::fs::symlink_metadata(&link_path)
                .unwrap_or_else(|err| panic!("missing symlink {entry_name}: {err}"));
            assert!(
                link_meta.file_type().is_symlink(),
                "{entry_name} should be a symlink"
            );
            assert_eq!(
                std::fs::read_link(&link_path)
                    .unwrap_or_else(|err| panic!("failed to read {entry_name} symlink: {err}")),
                real_codex_home.path().join(entry_name),
            );
        }

        // Running it again should be a no-op (even if the targets are missing).
        let codex_home_again = ensure_codex_compat_home(home_dir.path(), real_codex_home.path())
            .expect("ensure home again");
        assert_eq!(codex_home_again, codex_home);
    }

    #[test]
    fn resolve_real_codex_home_falls_back_to_dot_codex() {
        let home_dir = tempfile::tempdir().expect("home dir");
        let resolved = resolve_real_codex_home_from_env(home_dir.path(), None).expect("resolve");
        assert_eq!(resolved, home_dir.path().join(".codex"));
    }

    #[test]
    fn resolve_real_codex_home_uses_canonicalized_env_path() {
        let home_dir = tempfile::tempdir().expect("home dir");
        let codex_home = tempfile::tempdir().expect("codex home");
        let codex_home_env = codex_home.path().to_string_lossy().to_string();
        let resolved = resolve_real_codex_home_from_env(home_dir.path(), Some(&codex_home_env))
            .expect("resolve");
        let expected = codex_home.path().canonicalize().expect("canonicalize");
        assert_eq!(resolved, expected);
    }
}
