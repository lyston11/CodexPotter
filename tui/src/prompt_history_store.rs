//! Persistent prompt history store.
//!
//! # Divergence from upstream Codex TUI
//!
//! Upstream Codex manages prompt history in the core/session layer and serves it via protocol
//! messages. `codex-potter` keeps a simple text-only history log under
//! `~/.codexpotter/history.jsonl` and serves `Op::GetHistoryEntryRequest` directly from the
//! round renderer. See `tui/AGENTS.md`.
//!
//! # Concurrency
//!
//! The history file can be written by multiple `codex-potter` processes concurrently. To avoid
//! last-writer-wins data loss, writes are coordinated via advisory file locks and are always based
//! on the current on-disk state (not an in-memory snapshot from process start).

use std::fs::File;
use std::fs::OpenOptions;
use std::io;
use std::io::BufRead;
use std::io::BufReader;
use std::io::Read;
use std::io::Seek;
use std::io::SeekFrom;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use serde::Deserialize;
use serde::Serialize;

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

const MAX_ENTRIES: usize = 500;
const LOCK_MAX_RETRIES: usize = 10;
const LOCK_RETRY_SLEEP: Duration = Duration::from_millis(100);

/// Persistent prompt history backed by a JSONL file.
///
/// This is a deliberately small, text-only log used by CodexPotter's round renderer. See the
/// module docs for how this differs from upstream Codex.
pub struct PromptHistoryStore {
    path: Option<PathBuf>,
    entries: Vec<HistoryEntry>,
}

impl PromptHistoryStore {
    /// Create a store that loads from the default history path (if any).
    pub fn new() -> Self {
        Self::new_with_path(resolve_history_path())
    }

    /// Create a store backed by the provided history file path.
    ///
    /// This is primarily used by tests. When `path` is `None`, persistence is disabled and the
    /// store retains history entries in memory for the lifetime of the store.
    pub fn new_with_path(path: Option<PathBuf>) -> Self {
        if let Some(path) = path.as_deref()
            && path.exists()
            && let Err(err) = repair_history_file(path)
        {
            tracing::warn!(
                "failed to repair prompt history at {}: {err}",
                path.display()
            );
        }

        Self {
            path,
            entries: Vec::new(),
        }
    }

    /// Return the current history metadata as `(log_id, entry_count)`.
    ///
    /// `log_id` is derived from the backing file metadata (inode on Unix). Callers can use this to
    /// detect when the history file was replaced between requests.
    pub fn metadata(&self) -> (u64, usize) {
        let Some(path) = self.path.as_deref() else {
            return (0, self.entries.len());
        };

        match history_metadata(path) {
            Ok(metadata) => metadata,
            Err(err) => {
                tracing::warn!(
                    "failed to read prompt history metadata from {}: {err}",
                    path.display()
                );
                (0, 0)
            }
        }
    }

    /// Look up a history entry by `(log_id, offset)` and return its text.
    ///
    /// Returns `None` when `log_id` does not match the current store (history file changed) or
    /// when the `offset` is out of bounds.
    pub fn lookup_text(&self, log_id: u64, offset: usize) -> Option<String> {
        let Some(path) = self.path.as_deref() else {
            if log_id != 0 {
                return None;
            }
            return self.entries.get(offset).map(|entry| entry.text.clone());
        };

        lookup_history_entry_text(path, log_id, offset)
    }

    /// Record a prompt submission (trimmed), dedupe consecutive duplicates, and persist
    /// best-effort.
    pub fn record_submission(&mut self, text: &str) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }

        let Some(path) = self.path.as_deref() else {
            if self.entries.last().is_some_and(|prev| prev.text == text) {
                return;
            }

            self.entries.push(HistoryEntry {
                ts: unix_timestamp_secs(),
                text: text.to_string(),
            });

            if self.entries.len() > MAX_ENTRIES {
                let start = self.entries.len() - MAX_ENTRIES;
                self.entries.drain(0..start);
            }
            return;
        };

        if let Err(err) = append_history_entry(path, text) {
            tracing::warn!(
                "failed to persist prompt history to {}: {err}",
                path.display()
            );
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HistoryEntry {
    ts: u64,
    text: String,
}

fn unix_timestamp_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn resolve_history_path() -> Option<PathBuf> {
    #[cfg(test)]
    {
        None
    }

    #[cfg(not(test))]
    {
        let home = dirs::home_dir()?;
        Some(home.join(".codexpotter").join("history.jsonl"))
    }
}

fn append_history_entry(path: &Path, text: &str) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("history path has no parent directory"))?;
    std::fs::create_dir_all(parent)?;

    let entry = HistoryEntry {
        ts: unix_timestamp_secs(),
        text: text.to_string(),
    };

    let mut line =
        serde_json::to_string(&entry).map_err(|err| io::Error::other(err.to_string()))?;
    line.push('\n');

    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    {
        options.append(true);
        options.mode(0o600);
    }

    let mut file = options.open(path)?;
    ensure_owner_only_permissions(&file)?;

    lock_exclusive_with_retries(&file)?;

    let loaded = load_history_entries(&file)?;
    let mut entries = loaded.entries;

    if entries.last().is_some_and(|prev| prev.text == text) {
        return Ok(());
    }

    let should_append_only = !loaded.should_rewrite && entries.len() < MAX_ENTRIES;

    if should_append_only {
        file.seek(SeekFrom::End(0))?;
        if !file_ends_with_newline(&file)? {
            file.write_all(b"\n")?;
        }
        file.write_all(line.as_bytes())?;
        file.flush()?;
        return Ok(());
    }

    entries.push(entry);
    if entries.len() > MAX_ENTRIES {
        let start = entries.len() - MAX_ENTRIES;
        entries = entries.split_off(start);
    }

    rewrite_history_file(&mut file, &entries)?;
    Ok(())
}

fn history_metadata(path: &Path) -> io::Result<(u64, usize)> {
    let file = match OpenOptions::new().read(true).open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok((0, 0)),
        Err(err) => return Err(err),
    };

    let metadata = file.metadata()?;
    let log_id = history_log_id(&metadata).unwrap_or(0);

    lock_shared_with_retries(&file)?;
    let loaded = load_history_entries(&file)?;
    Ok((log_id, loaded.entries.len()))
}

fn lookup_history_entry_text(path: &Path, log_id: u64, offset: usize) -> Option<String> {
    let file = match OpenOptions::new().read(true).open(path) {
        Ok(file) => file,
        Err(err) => {
            tracing::warn!("failed to open prompt history at {}: {err}", path.display());
            return None;
        }
    };

    let metadata = match file.metadata() {
        Ok(metadata) => metadata,
        Err(err) => {
            tracing::warn!("failed to stat prompt history at {}: {err}", path.display());
            return None;
        }
    };

    let current_log_id = history_log_id(&metadata).unwrap_or(0);
    if log_id != 0 && current_log_id != log_id {
        return None;
    }

    if let Err(err) = lock_shared_with_retries(&file) {
        tracing::warn!(
            "failed to acquire shared lock on prompt history {}: {err}",
            path.display()
        );
        return None;
    }

    let loaded = match load_history_entries(&file) {
        Ok(loaded) => loaded,
        Err(err) => {
            tracing::warn!("failed to read prompt history at {}: {err}", path.display());
            return None;
        }
    };

    loaded.entries.get(offset).map(|entry| entry.text.clone())
}

fn repair_history_file(path: &Path) -> io::Result<()> {
    let mut options = OpenOptions::new();
    options.read(true).write(true);
    #[cfg(unix)]
    {
        options.mode(0o600);
    }

    let mut file = options.open(path)?;
    ensure_owner_only_permissions(&file)?;
    lock_exclusive_with_retries(&file)?;

    let loaded = load_history_entries(&file)?;
    if !loaded.should_rewrite {
        return Ok(());
    }

    rewrite_history_file(&mut file, &loaded.entries)
}

fn rewrite_history_file(file: &mut File, entries: &[HistoryEntry]) -> io::Result<()> {
    file.set_len(0)?;
    file.seek(SeekFrom::Start(0))?;
    for entry in entries {
        let mut line =
            serde_json::to_string(entry).map_err(|err| io::Error::other(err.to_string()))?;
        line.push('\n');
        file.write_all(line.as_bytes())?;
    }
    file.flush()?;
    Ok(())
}

struct LoadedHistory {
    entries: Vec<HistoryEntry>,
    should_rewrite: bool,
}

fn load_history_entries(file: &File) -> io::Result<LoadedHistory> {
    let mut reader_file = file.try_clone()?;
    reader_file.seek(SeekFrom::Start(0))?;
    let reader = BufReader::new(reader_file);

    let mut out = Vec::new();
    let mut should_rewrite = false;

    for line_res in reader.lines() {
        let line = line_res?;
        let line = line.trim();
        if line.is_empty() {
            should_rewrite = true;
            continue;
        }
        let entry: HistoryEntry = match serde_json::from_str(line) {
            Ok(entry) => entry,
            Err(_) => {
                should_rewrite = true;
                continue;
            }
        };
        if entry.text.is_empty() {
            should_rewrite = true;
            continue;
        }
        out.push(entry);
    }

    if out.len() <= MAX_ENTRIES {
        return Ok(LoadedHistory {
            entries: out,
            should_rewrite,
        });
    }

    let start = out.len() - MAX_ENTRIES;
    Ok(LoadedHistory {
        entries: out.split_off(start),
        should_rewrite: true,
    })
}

fn file_ends_with_newline(file: &File) -> io::Result<bool> {
    let mut reader = file.try_clone()?;
    let len = reader.metadata()?.len();
    if len == 0 {
        return Ok(true);
    }

    let Some(offset) = len.checked_sub(1) else {
        return Ok(true);
    };
    reader.seek(SeekFrom::Start(offset))?;
    let mut buf = [0u8; 1];
    reader.read_exact(&mut buf)?;
    Ok(buf[0] == b'\n')
}

fn lock_exclusive_with_retries(file: &File) -> io::Result<()> {
    for _ in 0..LOCK_MAX_RETRIES {
        match file.try_lock() {
            Ok(()) => return Ok(()),
            Err(std::fs::TryLockError::WouldBlock) => thread::sleep(LOCK_RETRY_SLEEP),
            Err(err) => return Err(err.into()),
        }
    }

    Err(io::Error::new(
        io::ErrorKind::WouldBlock,
        "could not acquire exclusive lock on prompt history file after multiple attempts",
    ))
}

fn lock_shared_with_retries(file: &File) -> io::Result<()> {
    for _ in 0..LOCK_MAX_RETRIES {
        match file.try_lock_shared() {
            Ok(()) => return Ok(()),
            Err(std::fs::TryLockError::WouldBlock) => thread::sleep(LOCK_RETRY_SLEEP),
            Err(err) => return Err(err.into()),
        }
    }

    Err(io::Error::new(
        io::ErrorKind::WouldBlock,
        "could not acquire shared lock on prompt history file after multiple attempts",
    ))
}

#[cfg(unix)]
fn ensure_owner_only_permissions(file: &File) -> io::Result<()> {
    let metadata = file.metadata()?;
    let current_mode = metadata.permissions().mode() & 0o777;
    if current_mode == 0o600 {
        return Ok(());
    }

    let mut perms = metadata.permissions();
    perms.set_mode(0o600);
    file.set_permissions(perms)?;
    Ok(())
}

#[cfg(not(unix))]
fn ensure_owner_only_permissions(_file: &File) -> io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn history_log_id(metadata: &std::fs::Metadata) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    Some(metadata.ino())
}

#[cfg(windows)]
fn history_log_id(metadata: &std::fs::Metadata) -> Option<u64> {
    use std::os::windows::fs::MetadataExt;
    Some(metadata.creation_time())
}

#[cfg(not(any(unix, windows)))]
fn history_log_id(_metadata: &std::fs::Metadata) -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn record_submission_dedupes_consecutive() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("history.jsonl");
        let mut store = PromptHistoryStore::new_with_path(Some(path));

        store.record_submission("");
        store.record_submission("hello");
        store.record_submission("hello");
        store.record_submission("world");

        let contents =
            std::fs::read_to_string(dir.path().join("history.jsonl")).expect("read history");
        let lines: Vec<&str> = contents.lines().collect();
        assert_eq!(lines.len(), 2);

        let first: HistoryEntry = serde_json::from_str(lines.first().unwrap()).expect("json");
        assert_eq!(first.text, "hello");
        let last: HistoryEntry = serde_json::from_str(lines.last().unwrap()).expect("json");
        assert_eq!(last.text, "world");
    }

    #[test]
    fn persists_and_truncates_to_max_entries() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("history.jsonl");

        let mut store = PromptHistoryStore::new_with_path(Some(path.clone()));
        for idx in 0..(MAX_ENTRIES + 10) {
            store.record_submission(&format!("cmd {idx}"));
        }

        let contents = std::fs::read_to_string(&path).expect("read history");
        let lines: Vec<&str> = contents.lines().collect();
        assert_eq!(lines.len(), MAX_ENTRIES);

        let first: HistoryEntry =
            serde_json::from_str(lines.first().unwrap()).expect("decode json");
        assert_eq!(first.text, "cmd 10");

        let last: HistoryEntry = serde_json::from_str(lines.last().unwrap()).expect("decode json");
        assert_eq!(last.text, format!("cmd {}", MAX_ENTRIES + 9));
    }

    #[test]
    fn multi_process_writers_do_not_clobber_each_other() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("history.jsonl");

        let mut store_one = PromptHistoryStore::new_with_path(Some(path.clone()));
        let mut store_two = PromptHistoryStore::new_with_path(Some(path));

        store_one.record_submission("first");
        store_two.record_submission("second");

        let contents =
            std::fs::read_to_string(dir.path().join("history.jsonl")).expect("read history");
        let entries: Vec<HistoryEntry> = contents
            .lines()
            .map(|line| serde_json::from_str(line).expect("decode json"))
            .collect();

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].text, "first");
        assert_eq!(entries[1].text, "second");
    }

    #[test]
    fn dedupe_checks_latest_on_disk_entry() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("history.jsonl");

        let mut store_one = PromptHistoryStore::new_with_path(Some(path.clone()));
        let mut store_two = PromptHistoryStore::new_with_path(Some(path));

        store_one.record_submission("alpha");
        store_two.record_submission("beta");
        store_one.record_submission("alpha");

        let contents =
            std::fs::read_to_string(dir.path().join("history.jsonl")).expect("read history");
        let entries: Vec<HistoryEntry> = contents
            .lines()
            .map(|line| serde_json::from_str(line).expect("decode json"))
            .collect();

        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].text, "alpha");
        assert_eq!(entries[1].text, "beta");
        assert_eq!(entries[2].text, "alpha");
    }

    #[test]
    fn lookup_text_reads_persisted_entries() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("history.jsonl");
        let mut store = PromptHistoryStore::new_with_path(Some(path));

        store.record_submission("one");
        store.record_submission("two");

        let (log_id, entry_count) = store.metadata();
        assert_eq!(entry_count, 2);
        assert_eq!(store.lookup_text(log_id, 0), Some("one".to_string()));
        assert_eq!(store.lookup_text(log_id, 1), Some("two".to_string()));
    }
}
