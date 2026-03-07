//! Application-level events used to coordinate UI actions.

use codex_protocol::protocol::Event;
use codex_protocol::protocol::Op;

use crate::history_cell::HistoryCell;
use crate::verbosity::Verbosity;
use codex_file_search::FileMatch;

#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
pub enum AppEvent {
    CodexEvent(Event),
    FatalExitRequest(String),

    /// Forward an `Op` to the backend. Using an `AppEvent` for this avoids
    /// bubbling channels through layers of widgets.
    CodexOp(Op),

    /// Kick off an asynchronous file search for the given query (text after
    /// the `@`).
    StartFileSearch(String),

    /// Result of a completed asynchronous file search. The `query` echoes the
    /// original search term so the UI can decide whether the results are still
    /// relevant.
    FileSearchResult {
        query: String,
        matches: Vec<FileMatch>,
    },

    /// Emit a committed transcript cell through the transcript pipeline.
    ///
    /// Prefer this over `InsertHistoryCell` for UI-generated messages so any pending coalescers
    /// (for example `Verbosity::Minimal` compact patch folding) are flushed at the correct
    /// semantic boundary.
    EmitHistoryCell(Box<dyn HistoryCell>),

    /// Insert a committed transcript cell into the history list.
    ///
    /// This is produced by the transcript pipeline. UI widgets should generally emit
    /// `EmitHistoryCell` instead so coalescers can flush deterministically.
    InsertHistoryCell(Box<dyn HistoryCell>),

    StartCommitAnimation,
    StopCommitAnimation,
    CommitTick,

    /// Apply a user-confirmed syntax theme selection.
    SyntaxThemeSelected {
        name: String,
    },

    /// Apply a user-confirmed transcript verbosity selection.
    VerbositySelected {
        verbosity: Verbosity,
    },
}
