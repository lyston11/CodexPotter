//! Shared progress file discovery for CodexPotter projects.
//!
//! Both the projects list overlay and the resume picker need to scan `.codexpotter/projects/**/MAIN.md`
//! under the current workdir. Keep the filesystem-walk configuration in one place so the two
//! callers stay consistent and we don't have to maintain two slightly different walkers.

use std::ffi::OsStr;
use std::path::Path;
use std::path::PathBuf;

use ignore::WalkBuilder;

const PROJECT_MAIN_FILE: &str = "MAIN.md";

pub(super) fn discover_project_progress_files(workdir: &Path) -> Vec<PathBuf> {
    let projects_root = workdir.join(".codexpotter").join("projects");
    if !projects_root.is_dir() {
        return Vec::new();
    }

    let walker = WalkBuilder::new(&projects_root)
        .hidden(false)
        .ignore(false)
        .git_ignore(false)
        .git_exclude(false)
        .git_global(false)
        .parents(false)
        .follow_links(false)
        .build();

    walker
        .filter_map(|entry| {
            let entry = entry.ok()?;
            if !entry.file_type().is_some_and(|kind| kind.is_file()) {
                return None;
            }
            if entry.path().file_name() != Some(OsStr::new(PROJECT_MAIN_FILE)) {
                return None;
            }
            Some(entry.path().to_path_buf())
        })
        .collect()
}
