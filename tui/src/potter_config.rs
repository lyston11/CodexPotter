use std::io;
use std::path::Path;
use std::path::PathBuf;

use toml_edit::DocumentMut;
use toml_edit::Item as TomlItem;
use toml_edit::Table as TomlTable;
use toml_edit::value;

use crate::verbosity::Verbosity;

pub fn load_potter_tui_verbosity() -> io::Result<Option<Verbosity>> {
    let path = potter_config_path()?;
    load_tui_verbosity_from_path(&path)
}

pub fn persist_potter_tui_verbosity(verbosity: Verbosity) -> io::Result<()> {
    let path = potter_config_path()?;
    persist_tui_verbosity_to_path(&path, verbosity)
}

/// Load whether YOLO is enabled by default for CodexPotter sessions.
///
/// This is backed by `~/.codexpotter/config.toml` under `[potter].yolo`.
///
/// Returns `false` when the key is missing.
pub fn load_potter_yolo_enabled() -> io::Result<bool> {
    let path = potter_config_path()?;
    load_yolo_from_path(&path)
}

/// Load whether YOLO is enabled by default from the provided config path.
///
/// Returns `false` when the file or key is missing.
pub fn load_potter_yolo_enabled_from_path(path: &Path) -> io::Result<bool> {
    load_yolo_from_path(path)
}

/// Persist whether YOLO is enabled by default for CodexPotter sessions.
///
/// Writes `~/.codexpotter/config.toml` under `[potter].yolo`.
pub fn persist_potter_yolo_enabled(enabled: bool) -> io::Result<()> {
    let path = potter_config_path()?;
    persist_yolo_to_path(&path, enabled)
}

fn potter_config_path() -> io::Result<PathBuf> {
    let Some(home) = dirs::home_dir() else {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "cannot determine home directory for config path",
        ));
    };
    Ok(home.join(".codexpotter").join("config.toml"))
}

fn load_tui_verbosity_from_path(path: &Path) -> io::Result<Option<Verbosity>> {
    let Some(content) = read_document_string(path)? else {
        return Ok(None);
    };

    let doc = match content.parse::<DocumentMut>() {
        Ok(doc) => doc,
        Err(_) => return Ok(parse_tui_verbosity_fallback(&content)),
    };

    Ok(read_tui_verbosity(&doc))
}

fn load_yolo_from_path(path: &Path) -> io::Result<bool> {
    let Some(content) = read_document_string(path)? else {
        return Ok(false);
    };

    let doc = match content.parse::<DocumentMut>() {
        Ok(doc) => doc,
        Err(_) => return Ok(parse_yolo_fallback(&content).unwrap_or(false)),
    };

    Ok(read_yolo(&doc).unwrap_or(false))
}

fn persist_tui_verbosity_to_path(path: &Path, verbosity: Verbosity) -> io::Result<()> {
    let content = match read_document_string(path) {
        Ok(Some(existing)) => existing,
        Ok(None) => String::new(),
        Err(err) => {
            // Avoid clobbering a file we can't read.
            return Err(err);
        }
    };

    let updated = match content.parse::<DocumentMut>() {
        Ok(mut doc) => {
            set_tui_verbosity(&mut doc, verbosity);
            doc.to_string()
        }
        Err(_) => append_tui_fallback(&content, verbosity),
    };

    crate::path_utils::write_atomically(path, &updated)
}

fn persist_yolo_to_path(path: &Path, enabled: bool) -> io::Result<()> {
    let content = match read_document_string(path) {
        Ok(Some(existing)) => existing,
        Ok(None) => String::new(),
        Err(err) => {
            // Avoid clobbering a file we can't read.
            return Err(err);
        }
    };

    let updated = match content.parse::<DocumentMut>() {
        Ok(mut doc) => {
            set_yolo(&mut doc, enabled);
            doc.to_string()
        }
        Err(_) => append_yolo_fallback(&content, enabled),
    };

    crate::path_utils::write_atomically(path, &updated)
}

fn read_tui_verbosity(doc: &DocumentMut) -> Option<Verbosity> {
    doc.get("tui")
        .and_then(TomlItem::as_table)
        .and_then(|tui| tui.get("verbosity"))
        .and_then(TomlItem::as_value)
        .and_then(|v| v.as_str())
        .and_then(Verbosity::parse_config_value)
}

fn read_yolo(doc: &DocumentMut) -> Option<bool> {
    doc.get("potter")
        .and_then(TomlItem::as_table)
        .and_then(|potter| potter.get("yolo"))
        .and_then(TomlItem::as_value)
        .and_then(toml_edit::Value::as_bool)
}

fn set_tui_verbosity(doc: &mut DocumentMut, verbosity: Verbosity) {
    let tui = ensure_table_for_write(doc, "tui");
    tui["verbosity"] = value(verbosity.config_value().to_string());
}

fn set_yolo(doc: &mut DocumentMut, enabled: bool) {
    let potter = ensure_table_for_write(doc, "potter");
    potter["yolo"] = value(enabled);
}

fn parse_tui_verbosity_fallback(contents: &str) -> Option<Verbosity> {
    let mut in_tui = false;
    let mut result = None;

    for line in contents.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('[') {
            in_tui = matches!(parse_table_header_name(trimmed), Some("tui"));
            continue;
        }

        if !in_tui {
            continue;
        }

        let Some(line) = strip_toml_comment(trimmed) else {
            continue;
        };
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() != "verbosity" {
            continue;
        }

        let token = value
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .trim()
            .trim_matches('"');
        if let Some(verbosity) = Verbosity::parse_config_value(token) {
            result = Some(verbosity);
        }
    }

    result
}

fn parse_yolo_fallback(contents: &str) -> Option<bool> {
    let mut in_potter = false;
    let mut result = None;

    for line in contents.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('[') {
            in_potter = matches!(parse_table_header_name(trimmed), Some("potter"));
            continue;
        }

        if !in_potter {
            continue;
        }

        let Some(line) = strip_toml_comment(trimmed) else {
            continue;
        };
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() != "yolo" {
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

fn append_tui_fallback(existing: &str, verbosity: Verbosity) -> String {
    let mut out = existing.to_string();
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push('\n');
    out.push_str("[tui]\n");
    out.push_str(&format!("verbosity = \"{}\"\n", verbosity.config_value()));
    out
}

fn append_yolo_fallback(existing: &str, enabled: bool) -> String {
    let mut out = existing.to_string();
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push('\n');
    out.push_str("[potter]\n");
    out.push_str(&format!("yolo = {enabled}\n"));
    out
}

fn read_document_string(path: &Path) -> io::Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(contents) => Ok(Some(contents)),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn parse_fallback_reads_last_value() {
        let contents = r#"
garbage

[tui]
verbosity = "minimal"

something = else

[tui]
verbosity = "simple"
"#;

        assert_eq!(
            parse_tui_verbosity_fallback(contents),
            Some(Verbosity::Simple)
        );
    }

    #[test]
    fn persist_and_load_roundtrip() -> io::Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("config.toml");

        persist_tui_verbosity_to_path(&path, Verbosity::Minimal)?;
        assert_eq!(
            load_tui_verbosity_from_path(&path)?,
            Some(Verbosity::Minimal)
        );

        persist_tui_verbosity_to_path(&path, Verbosity::Simple)?;
        assert_eq!(
            load_tui_verbosity_from_path(&path)?,
            Some(Verbosity::Simple)
        );
        Ok(())
    }

    #[test]
    fn persist_appends_when_toml_invalid() -> io::Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("config.toml");

        std::fs::write(&path, "[tui\nverbosity = \"minimal\"\n")?;
        persist_tui_verbosity_to_path(&path, Verbosity::Simple)?;

        let contents = std::fs::read_to_string(&path)?;
        assert!(contents.contains("[tui]"));
        assert_eq!(
            parse_tui_verbosity_fallback(&contents),
            Some(Verbosity::Simple)
        );
        Ok(())
    }

    #[test]
    fn parse_yolo_fallback_reads_last_value() {
        let contents = r#"
garbage

[potter]
yolo = true

something = else

[potter]
yolo = false
"#;

        assert_eq!(parse_yolo_fallback(contents), Some(false));
    }

    #[test]
    fn persist_and_load_yolo_roundtrip() -> io::Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("config.toml");

        persist_yolo_to_path(&path, true)?;
        assert_eq!(load_yolo_from_path(&path)?, true);

        persist_yolo_to_path(&path, false)?;
        assert_eq!(load_yolo_from_path(&path)?, false);
        Ok(())
    }

    #[test]
    fn persist_yolo_appends_when_toml_invalid() -> io::Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("config.toml");

        std::fs::write(&path, "[potter\nx = 1\n")?;
        persist_yolo_to_path(&path, true)?;

        let contents = std::fs::read_to_string(&path)?;
        assert!(contents.contains("[potter]"));
        assert_eq!(parse_yolo_fallback(&contents), Some(true));
        Ok(())
    }
}
