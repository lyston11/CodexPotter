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
    load_config_value(
        path,
        || None,
        read_tui_verbosity,
        parse_tui_verbosity_fallback,
    )
}

fn load_yolo_from_path(path: &Path) -> io::Result<bool> {
    load_config_value(
        path,
        || false,
        |doc| read_yolo(doc).unwrap_or(false),
        |content| parse_yolo_fallback(content).unwrap_or(false),
    )
}

fn persist_tui_verbosity_to_path(path: &Path, verbosity: Verbosity) -> io::Result<()> {
    persist_config_value(
        path,
        |doc| set_tui_verbosity(doc, verbosity),
        |content| append_tui_fallback(content, verbosity),
    )
}

fn persist_yolo_to_path(path: &Path, enabled: bool) -> io::Result<()> {
    persist_config_value(
        path,
        |doc| set_yolo(doc, enabled),
        |content| append_yolo_fallback(content, enabled),
    )
}

fn read_tui_verbosity(doc: &DocumentMut) -> Option<Verbosity> {
    read_table_value(doc, "tui", "verbosity")
        .and_then(|v| v.as_str())
        .and_then(Verbosity::parse_config_value)
}

fn read_yolo(doc: &DocumentMut) -> Option<bool> {
    read_table_value(doc, "potter", "yolo").and_then(toml_edit::Value::as_bool)
}

fn set_tui_verbosity(doc: &mut DocumentMut, verbosity: Verbosity) {
    set_table_value(
        doc,
        "tui",
        "verbosity",
        value(verbosity.config_value().to_string()),
    );
}

fn set_yolo(doc: &mut DocumentMut, enabled: bool) {
    set_table_value(doc, "potter", "yolo", value(enabled));
}

fn parse_tui_verbosity_fallback(contents: &str) -> Option<Verbosity> {
    parse_fallback_table_key(contents, "tui", "verbosity", |value| {
        let token = value
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .trim()
            .trim_matches('"');
        Verbosity::parse_config_value(token)
    })
}

fn parse_yolo_fallback(contents: &str) -> Option<bool> {
    parse_fallback_table_key(contents, "potter", "yolo", |value| {
        match value.split_whitespace().next().unwrap_or_default() {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        }
    })
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

fn load_config_value<T>(
    path: &Path,
    missing: impl FnOnce() -> T,
    read_doc: impl FnOnce(&DocumentMut) -> T,
    parse_fallback: impl FnOnce(&str) -> T,
) -> io::Result<T> {
    let Some(content) = read_document_string(path)? else {
        return Ok(missing());
    };

    let doc = match content.parse::<DocumentMut>() {
        Ok(doc) => doc,
        Err(_) => return Ok(parse_fallback(&content)),
    };

    Ok(read_doc(&doc))
}

fn persist_config_value(
    path: &Path,
    update_doc: impl FnOnce(&mut DocumentMut),
    append_fallback: impl FnOnce(&str) -> String,
) -> io::Result<()> {
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
            update_doc(&mut doc);
            doc.to_string()
        }
        Err(_) => append_fallback(&content),
    };

    crate::path_utils::write_atomically(path, &updated)
}

fn read_table_value<'a>(
    doc: &'a DocumentMut,
    table_key: &str,
    value_key: &str,
) -> Option<&'a toml_edit::Value> {
    doc.get(table_key)
        .and_then(TomlItem::as_table)
        .and_then(|table| table.get(value_key))
        .and_then(TomlItem::as_value)
}

fn set_table_value(doc: &mut DocumentMut, table_key: &str, value_key: &str, value: TomlItem) {
    let table = ensure_table_for_write(doc, table_key);
    table[value_key] = value;
}

fn parse_fallback_table_key<T>(
    contents: &str,
    table_name: &str,
    key_name: &str,
    mut parse_value: impl FnMut(&str) -> Option<T>,
) -> Option<T> {
    let mut in_table = false;
    let mut result = None;

    for line in contents.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('[') {
            in_table = matches!(parse_table_header_name(trimmed), Some(name) if name == table_name);
            continue;
        }

        if !in_table {
            continue;
        }

        let Some(line) = strip_toml_comment(trimmed) else {
            continue;
        };
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() != key_name {
            continue;
        }

        if let Some(parsed) = parse_value(value) {
            result = Some(parsed);
        }
    }

    result
}

fn append_tui_fallback(existing: &str, verbosity: Verbosity) -> String {
    append_table_fallback(
        existing,
        "tui",
        format!("verbosity = \"{}\"\n", verbosity.config_value()),
    )
}

fn append_yolo_fallback(existing: &str, enabled: bool) -> String {
    append_table_fallback(existing, "potter", format!("yolo = {enabled}\n"))
}

fn append_table_fallback(existing: &str, table_name: &str, assignment: String) -> String {
    let mut out = existing.to_string();
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push('\n');
    out.push_str(&format!("[{table_name}]\n"));
    out.push_str(&assignment);
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
    fn fallback_parsers_read_last_value_for_supported_settings() {
        let verbosity_contents = r#"
garbage

[tui]
verbosity = "minimal"

something = else

[tui]
verbosity = "simple"
"#;
        assert_eq!(
            parse_tui_verbosity_fallback(verbosity_contents),
            Some(Verbosity::Simple)
        );

        let yolo_contents = r#"
garbage

[potter]
yolo = true

something = else

[potter]
yolo = false
"#;
        assert_eq!(parse_yolo_fallback(yolo_contents), Some(false));
    }

    #[test]
    fn persist_and_load_roundtrip_for_supported_settings() -> io::Result<()> {
        let dir = tempfile::tempdir()?;
        let verbosity_path = dir.path().join("verbosity.toml");

        persist_tui_verbosity_to_path(&verbosity_path, Verbosity::Minimal)?;
        assert_eq!(
            load_tui_verbosity_from_path(&verbosity_path)?,
            Some(Verbosity::Minimal)
        );
        persist_tui_verbosity_to_path(&verbosity_path, Verbosity::Simple)?;
        assert_eq!(
            load_tui_verbosity_from_path(&verbosity_path)?,
            Some(Verbosity::Simple)
        );

        let yolo_path = dir.path().join("yolo.toml");
        persist_yolo_to_path(&yolo_path, true)?;
        assert_eq!(load_yolo_from_path(&yolo_path)?, true);
        persist_yolo_to_path(&yolo_path, false)?;
        assert_eq!(load_yolo_from_path(&yolo_path)?, false);
        Ok(())
    }

    #[test]
    fn persist_appends_when_toml_is_invalid_for_supported_settings() -> io::Result<()> {
        let dir = tempfile::tempdir()?;
        let verbosity_path = dir.path().join("verbosity.toml");

        std::fs::write(&verbosity_path, "[tui\nverbosity = \"minimal\"\n")?;
        persist_tui_verbosity_to_path(&verbosity_path, Verbosity::Simple)?;
        let verbosity_contents = std::fs::read_to_string(&verbosity_path)?;
        assert!(verbosity_contents.contains("[tui]"));
        assert_eq!(
            parse_tui_verbosity_fallback(&verbosity_contents),
            Some(Verbosity::Simple)
        );

        let yolo_path = dir.path().join("yolo.toml");
        std::fs::write(&yolo_path, "[potter\nx = 1\n")?;
        persist_yolo_to_path(&yolo_path, true)?;
        let yolo_contents = std::fs::read_to_string(&yolo_path)?;
        assert!(yolo_contents.contains("[potter]"));
        assert_eq!(parse_yolo_fallback(&yolo_contents), Some(true));
        Ok(())
    }
}
