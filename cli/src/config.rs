//! CodexPotter CLI configuration.
//!
//! This module owns reading and writing the user config file at `~/.codexpotter/config.toml`
//! (see [`ConfigStore::new_default`]).
//!
//! Design notes:
//! - Writes are comment-preserving whenever possible (via `toml_edit`).
//! - Reads are intentionally resilient: if the TOML is invalid, we fall back to a tiny
//!   line-based parser for a small set of keys so the CLI can still start and users can fix the
//!   file in place.
//!
//! Current keys:
//! - `[notice] hide_gitignore_prompt` (bool): hides the gitignore startup prompt.
//! - `check_for_update_on_startup` (bool): enables update checks on startup (default: `true`).
//! - `rounds` (integer): default round budget for runs that do not specify `--rounds`
//!   (default: `10`).

use std::io::ErrorKind;
use std::num::NonZeroUsize;
use std::path::Path;
use std::path::PathBuf;

use toml_edit::DocumentMut;
use toml_edit::Item as TomlItem;
use toml_edit::Table as TomlTable;
use toml_edit::value;

use crate::atomic_write::write_atomic_text;

/// Persistent user configuration backed by a TOML file on disk.
#[derive(Debug, Clone)]
pub struct ConfigStore {
    path: PathBuf,
}

impl ConfigStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn new_default() -> anyhow::Result<Self> {
        let Some(home) = dirs::home_dir() else {
            anyhow::bail!("cannot determine home directory for config path");
        };
        Ok(Self::new(default_config_path(&home)))
    }

    pub fn notice_hide_gitignore_prompt(&self) -> anyhow::Result<bool> {
        let Some(content) = read_document_string(&self.path)? else {
            return Ok(false);
        };

        let doc = match content.parse::<DocumentMut>() {
            Ok(doc) => doc,
            Err(_) => {
                return Ok(parse_notice_hide_gitignore_prompt_fallback(&content).unwrap_or(false));
            }
        };

        Ok(read_notice_hide_gitignore_prompt(&doc).unwrap_or(false))
    }

    /// When `true`, checks for CodexPotter updates on startup and surfaces update prompts.
    ///
    /// Defaults to `true`.
    pub fn check_for_update_on_startup(&self) -> anyhow::Result<bool> {
        let Some(content) = read_document_string(&self.path)? else {
            return Ok(true);
        };

        let doc = match content.parse::<DocumentMut>() {
            Ok(doc) => doc,
            Err(_) => {
                return Ok(parse_check_for_update_on_startup_fallback(&content).unwrap_or(true));
            }
        };

        Ok(read_check_for_update_on_startup(&doc).unwrap_or(true))
    }

    pub fn rounds(&self) -> anyhow::Result<Option<NonZeroUsize>> {
        let Some(content) = read_document_string(&self.path)? else {
            return Ok(None);
        };

        let doc = match content.parse::<DocumentMut>() {
            Ok(doc) => doc,
            Err(_) => return parse_rounds_fallback(&content),
        };

        read_rounds(&doc)
    }

    pub fn set_notice_hide_gitignore_prompt(&self, hide: bool) -> anyhow::Result<()> {
        let content = match read_document_string(&self.path) {
            Ok(Some(existing)) => existing,
            Ok(None) => String::new(),
            Err(err) => {
                // If we can't read the existing file, avoid clobbering it; but still allow the
                // application to proceed.
                return Err(err);
            }
        };

        let updated = match content.parse::<DocumentMut>() {
            Ok(mut doc) => {
                set_notice_hide_gitignore_prompt(&mut doc, hide);
                doc.to_string()
            }
            Err(_) => append_notice_fallback(&content, hide),
        };

        write_atomic_text(&self.path, &updated)
    }
}

fn default_config_path(home: &Path) -> PathBuf {
    home.join(".codexpotter").join("config.toml")
}

fn read_notice_hide_gitignore_prompt(doc: &DocumentMut) -> Option<bool> {
    doc.get("notice")
        .and_then(TomlItem::as_table)
        .and_then(|notice| notice.get("hide_gitignore_prompt"))
        .and_then(TomlItem::as_value)
        .and_then(|v| v.as_bool())
}

fn read_check_for_update_on_startup(doc: &DocumentMut) -> Option<bool> {
    doc.get("check_for_update_on_startup")
        .and_then(TomlItem::as_value)
        .and_then(|v| v.as_bool())
}

fn read_rounds(doc: &DocumentMut) -> anyhow::Result<Option<NonZeroUsize>> {
    let Some(item) = doc.get("rounds") else {
        return Ok(None);
    };

    let Some(value) = item.as_value() else {
        anyhow::bail!("`rounds` must be an integer, got {item}");
    };
    let Some(rounds) = value.as_integer() else {
        anyhow::bail!("`rounds` must be an integer, got {value}");
    };
    if rounds <= 0 {
        anyhow::bail!("`rounds` must be >= 1, got {rounds}");
    }

    let rounds_usize = usize::try_from(rounds)
        .map_err(|_| anyhow::anyhow!("`rounds` is too large, got {rounds}"))?;
    Ok(Some(
        NonZeroUsize::new(rounds_usize).expect("positive rounds should be non-zero"),
    ))
}

fn set_notice_hide_gitignore_prompt(doc: &mut DocumentMut, hide: bool) {
    let notice = ensure_table_for_write(doc, "notice");
    notice["hide_gitignore_prompt"] = value(hide);
}

fn parse_check_for_update_on_startup_fallback(contents: &str) -> Option<bool> {
    for line in contents.lines() {
        let trimmed = line.trim_start();
        let Some(line) = strip_toml_comment(trimmed) else {
            continue;
        };
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() != "check_for_update_on_startup" {
            continue;
        }

        let token = value.split_whitespace().next().unwrap_or_default();
        if token == "true" {
            return Some(true);
        }
        if token == "false" {
            return Some(false);
        }
    }

    None
}

fn parse_rounds_fallback(contents: &str) -> anyhow::Result<Option<NonZeroUsize>> {
    for line in contents.lines() {
        let trimmed = line.trim_start();
        let Some(line) = strip_toml_comment(trimmed) else {
            continue;
        };
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() != "rounds" {
            continue;
        }

        let token = value.split_whitespace().next().unwrap_or_default().trim();
        if token.is_empty() {
            continue;
        }

        let normalized = token.replace('_', "");
        let rounds = normalized
            .parse::<i64>()
            .map_err(|_| anyhow::anyhow!("`rounds` must be an integer, got {token}"))?;

        if rounds <= 0 {
            anyhow::bail!("`rounds` must be >= 1, got {rounds}");
        }

        let rounds_usize = usize::try_from(rounds)
            .map_err(|_| anyhow::anyhow!("`rounds` is too large, got {rounds}"))?;
        return Ok(Some(
            NonZeroUsize::new(rounds_usize).expect("positive rounds should be non-zero"),
        ));
    }

    Ok(None)
}

fn parse_notice_hide_gitignore_prompt_fallback(contents: &str) -> Option<bool> {
    let mut in_notice = false;
    let mut result = None;

    for line in contents.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('[') {
            in_notice = matches!(parse_table_header_name(trimmed), Some("notice"));
            continue;
        }

        if !in_notice {
            continue;
        }

        let Some(line) = strip_toml_comment(trimmed) else {
            continue;
        };
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() != "hide_gitignore_prompt" {
            continue;
        }

        let token = value.split_whitespace().next().unwrap_or_default();
        if token == "true" {
            result = Some(true);
        } else if token == "false" {
            result = Some(false);
        }
    }

    result
}

fn parse_table_header_name(line: &str) -> Option<&str> {
    let line = line.trim_start();
    if !line.starts_with('[') {
        return None;
    }
    let end = line.find(']')?;
    if end <= 1 {
        return None;
    }
    let name = line[1..end].trim();
    if name.is_empty() {
        return None;
    }
    Some(name)
}

fn strip_toml_comment(line: &str) -> Option<&str> {
    let line = line.split_once('#').map_or(line, |(head, _)| head).trim();
    if line.is_empty() { None } else { Some(line) }
}

fn ensure_table_for_write<'a>(doc: &'a mut DocumentMut, key: &str) -> &'a mut TomlTable {
    if doc.get(key).and_then(TomlItem::as_table).is_some() {
        match &mut doc[key] {
            TomlItem::Table(table) => return table,
            _ => unreachable!("expected `{key}` to be a table"),
        }
    }

    let mut table = TomlTable::new();
    table.set_implicit(false);
    doc[key] = TomlItem::Table(table);
    match &mut doc[key] {
        TomlItem::Table(table) => table,
        _ => unreachable!("expected inserted `{key}` to be a table"),
    }
}

fn append_notice_fallback(existing: &str, hide: bool) -> String {
    let mut out = existing.to_string();
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push('\n');
    out.push_str("[notice]\n");
    out.push_str(&format!("hide_gitignore_prompt = {hide}\n"));
    out
}

fn read_document_string(path: &Path) -> anyhow::Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(contents) => Ok(Some(contents)),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(None),
        Err(err) => Err(anyhow::Error::new(err).context("read config.toml")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notice_flag_preserves_comments_when_written_and_reads_from_invalid_toml() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"# top comment

[notice] # keep me
# inner comment
hide_gitignore_prompt = false

[other]
key = 1
"#,
        )
        .expect("write config");

        let store = ConfigStore::new(path.clone());
        store
            .set_notice_hide_gitignore_prompt(true)
            .expect("set flag");

        let updated = std::fs::read_to_string(&path).expect("read updated");
        assert!(updated.contains("# top comment"));
        assert!(updated.contains("# inner comment"));
        assert!(updated.contains("[other]"));
        assert!(updated.contains("hide_gitignore_prompt = true"));
        assert!(store.notice_hide_gitignore_prompt().expect("read flag"));

        std::fs::write(
            &path,
            r#"# broken table header makes this TOML invalid
[other
key = 1

[notice]
hide_gitignore_prompt = true # keep me
"#,
        )
        .expect("write config");

        let store = ConfigStore::new(path);
        assert!(store.notice_hide_gitignore_prompt().expect("read flag"));
    }

    #[test]
    fn update_checks_default_to_true_and_read_invalid_toml_override() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        let store = ConfigStore::new(path.clone());
        assert!(store.check_for_update_on_startup().expect("read flag"));
        std::fs::write(
            &path,
            r#"# broken table header makes this TOML invalid
[other
key = 1

check_for_update_on_startup = false # keep me
"#,
        )
        .expect("write config");

        let store = ConfigStore::new(path);
        assert!(!store.check_for_update_on_startup().expect("read flag"));
    }

    #[test]
    fn rounds_read_returns_none_when_unset_and_reads_integer_values() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        let store = ConfigStore::new(path.clone());
        assert_eq!(store.rounds().expect("read rounds"), None);

        std::fs::write(&path, "rounds = 15\n").expect("write config");
        assert_eq!(
            store.rounds().expect("read rounds").map(NonZeroUsize::get),
            Some(15)
        );
    }

    #[test]
    fn rounds_read_rejects_non_positive_values() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "rounds = 0\n").expect("write config");

        let store = ConfigStore::new(path);
        let err = store.rounds().unwrap_err();
        assert!(err.to_string().contains("rounds"));
    }

    #[test]
    fn rounds_read_falls_back_when_toml_invalid_and_respects_underscores() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"# broken table header makes this TOML invalid
[other
key = 1

rounds = 1_000
"#,
        )
        .expect("write config");

        let store = ConfigStore::new(path);
        assert_eq!(
            store.rounds().expect("read rounds").map(NonZeroUsize::get),
            Some(1000)
        );
    }

    #[test]
    fn rounds_read_rejects_invalid_values_even_when_toml_invalid() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"# broken table header makes this TOML invalid
[other
key = 1

rounds = 0
"#,
        )
        .expect("write config");

        let store = ConfigStore::new(path);
        let err = store.rounds().unwrap_err();
        assert!(err.to_string().contains("rounds"));
    }

    #[test]
    fn default_config_path_uses_codexpotter_home_dir() {
        let home = Path::new("home");
        assert_eq!(
            default_config_path(home),
            home.join(".codexpotter").join("config.toml")
        );
    }
}
