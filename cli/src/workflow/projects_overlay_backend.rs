//! Projects list overlay provider backend.
//!
//! `codex-potter` renders the projects overlay in the TUI, but all filesystem scanning/parsing
//! is owned by the CLI workflow layer. This helper keeps the control-plane logic consistent
//! across the live project render loop and the prompt screen (when no project is running).

use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

use codex_tui::ProjectsOverlayRequest;
use codex_tui::ProjectsOverlayResponse;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::mpsc::unbounded_channel;

/// Debounce window for details parsing requests.
///
/// The projects overlay enables "alternate scroll" (mouse wheel emits ↑/↓ escape codes in many
/// terminals). When a user scrolls quickly, the TUI can generate a burst of selection changes.
/// Parsing project details is filesystem-heavy, so we debounce/coalesce `Details` requests to
/// avoid building a long backlog of redundant work.
const DETAILS_DEBOUNCE_WINDOW: Duration = Duration::from_millis(40);

/// Spawn a background task that serves projects overlay requests for the given `workdir`.
pub fn spawn_projects_overlay_provider(
    workdir: PathBuf,
) -> codex_tui::ProjectsOverlayProviderChannels {
    spawn_projects_overlay_provider_with_mode(workdir, OverlayListMode::AllProjects)
}

/// Spawn a background task that serves projects overlay requests for `codex-potter resume`.
///
/// Unlike the normal `/list` overlay provider, this only returns projects that are resumable
/// without missing upstream rollout files.
pub fn spawn_resumable_projects_overlay_provider(
    workdir: PathBuf,
) -> codex_tui::ProjectsOverlayProviderChannels {
    spawn_projects_overlay_provider_with_mode(workdir, OverlayListMode::ResumableProjects)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OverlayListMode {
    AllProjects,
    ResumableProjects,
}

fn spawn_projects_overlay_provider_with_mode(
    workdir: PathBuf,
    mode: OverlayListMode,
) -> codex_tui::ProjectsOverlayProviderChannels {
    let (request_tx, request_rx): (
        UnboundedSender<ProjectsOverlayRequest>,
        UnboundedReceiver<ProjectsOverlayRequest>,
    ) = unbounded_channel();
    let (response_tx, response_rx): (
        UnboundedSender<ProjectsOverlayResponse>,
        UnboundedReceiver<ProjectsOverlayResponse>,
    ) = unbounded_channel();

    tokio::spawn(serve_projects_overlay_requests(
        workdir,
        mode,
        request_rx,
        response_tx,
    ));

    codex_tui::ProjectsOverlayProviderChannels {
        request_tx,
        response_rx,
    }
}

#[derive(Debug)]
enum InFlightKind {
    List,
    Details { project_dir: PathBuf },
}

struct InFlight {
    kind: InFlightKind,
    handle: tokio::task::JoinHandle<ProjectsOverlayResponse>,
}

async fn serve_projects_overlay_requests(
    workdir: PathBuf,
    mode: OverlayListMode,
    mut request_rx: UnboundedReceiver<ProjectsOverlayRequest>,
    response_tx: UnboundedSender<ProjectsOverlayResponse>,
) {
    let mut pending_list = false;
    let mut pending_details: Option<PathBuf> = None;
    let mut details_debounce_until: Option<tokio::time::Instant> = None;
    let mut in_flight: Option<InFlight> = None;

    loop {
        // Start work if we're idle and either:
        // - a list request is pending, or
        // - a details request is pending and past its debounce window.
        if in_flight.is_none() {
            if pending_list {
                pending_list = false;
                pending_details = None;
                details_debounce_until = None;

                let workdir = workdir.clone();
                in_flight = Some(InFlight {
                    kind: InFlightKind::List,
                    handle: tokio::task::spawn_blocking(move || {
                        response_for_projects_overlay_request(
                            &workdir,
                            mode,
                            ProjectsOverlayRequest::List,
                        )
                    }),
                });
            } else if let Some(project_dir) = pending_details.take() {
                let now = tokio::time::Instant::now();
                if details_debounce_until.is_some_and(|deadline| deadline > now) {
                    // Not ready yet; put it back and wait for the debounce timer to fire.
                    pending_details = Some(project_dir);
                } else {
                    details_debounce_until = None;
                    let workdir = workdir.clone();
                    let project_dir_for_kind = project_dir.clone();
                    in_flight = Some(InFlight {
                        kind: InFlightKind::Details {
                            project_dir: project_dir_for_kind,
                        },
                        handle: tokio::task::spawn_blocking(move || {
                            response_for_projects_overlay_request(
                                &workdir,
                                mode,
                                ProjectsOverlayRequest::Details { project_dir },
                            )
                        }),
                    });
                }
            }
        }

        let debounce_deadline = details_debounce_until;
        tokio::select! {
            maybe_request = request_rx.recv() => {
                let Some(request) = maybe_request else {
                    return;
                };

                match request {
                    ProjectsOverlayRequest::List => {
                        pending_list = true;
                        pending_details = None;
                        details_debounce_until = None;
                    }
                    ProjectsOverlayRequest::Details { project_dir } => {
                        pending_details = Some(project_dir);
                        details_debounce_until = Some(tokio::time::Instant::now() + DETAILS_DEBOUNCE_WINDOW);
                    }
                }
            }
            result = async {
                let handle = &mut in_flight.as_mut()?.handle;
                Some(handle.await)
            }, if in_flight.is_some() => {
                let Some(result) = result else {
                    return;
                };
                let response = match result {
                    Ok(response) => response,
                    Err(_) => return,
                };

                let should_send = match &in_flight.as_ref().map(|in_flight| &in_flight.kind) {
                    Some(InFlightKind::List) => true,
                    Some(InFlightKind::Details { project_dir }) => {
                        !pending_list && pending_details.as_ref().is_none_or(|pending| pending == project_dir)
                    }
                    None => false,
                };

                if should_send && response_tx.send(response).is_err() {
                    return;
                }

                // If a duplicate details request arrived while we were parsing, clear it so we do
                // not immediately re-run the same filesystem work.
                if let Some(InFlightKind::Details { project_dir }) =
                    in_flight.as_ref().map(|in_flight| &in_flight.kind)
                    && pending_details
                        .as_ref()
                        .is_some_and(|pending| pending == project_dir)
                {
                    pending_details = None;
                    details_debounce_until = None;
                }

                in_flight = None;
            }
            _ = async {
                let Some(deadline) = debounce_deadline else {
                    return;
                };
                tokio::time::sleep_until(deadline).await;
            }, if in_flight.is_none() && !pending_list && pending_details.is_some() && debounce_deadline.is_some() => {
                // Debounce fired; start the pending details work on the next loop iteration.
                details_debounce_until = None;
            }
        }
    }
}

fn response_for_projects_overlay_request(
    workdir: &Path,
    mode: OverlayListMode,
    request: ProjectsOverlayRequest,
) -> ProjectsOverlayResponse {
    match request {
        ProjectsOverlayRequest::List => {
            let (mut projects, error) =
                match super::projects_overlay_index::discover_projects_for_overlay(workdir) {
                    Ok(projects) => (projects, None),
                    Err(err) => (Vec::new(), Some(format!("{err:#}"))),
                };
            if mode == OverlayListMode::ResumableProjects {
                let resumable = resumable_project_dirs_for_overlay(workdir);
                projects.retain(|project| resumable.contains(&project.project_dir));
            }
            ProjectsOverlayResponse::List { projects, error }
        }
        ProjectsOverlayRequest::Details { project_dir } => {
            let details = super::projects_overlay_details::build_project_details_for_overlay(
                workdir,
                &project_dir,
            );
            ProjectsOverlayResponse::Details { details }
        }
    }
}

fn resumable_project_dirs_for_overlay(workdir: &Path) -> HashSet<PathBuf> {
    let mut out = HashSet::new();
    for progress_file in super::project_progress_files::discover_project_progress_files(workdir) {
        let Some(project_dir_abs) = progress_file.parent() else {
            continue;
        };
        if !project_dir_is_resumable(workdir, project_dir_abs) {
            continue;
        }
        let project_dir = project_dir_abs
            .strip_prefix(workdir)
            .unwrap_or(project_dir_abs)
            .to_path_buf();
        out.insert(project_dir);
    }
    out
}

fn project_dir_is_resumable(workdir: &Path, project_dir_abs: &Path) -> bool {
    let potter_rollout_path = crate::workflow::rollout::potter_rollout_path(project_dir_abs);
    if !potter_rollout_path.is_file() {
        return false;
    }

    let potter_lines = match crate::workflow::rollout::read_lines(&potter_rollout_path) {
        Ok(lines) => lines,
        Err(_) => return false,
    };
    if potter_lines.is_empty() {
        return false;
    }

    let index = match crate::workflow::rollout_resume_index::build_resume_index(&potter_lines) {
        Ok(index) => index,
        Err(_) => return false,
    };

    all_referenced_rollouts_exist(workdir, &index)
}

fn all_referenced_rollouts_exist(
    workdir: &Path,
    index: &crate::workflow::rollout_resume_index::PotterRolloutResumeIndex,
) -> bool {
    let mut all_paths: Vec<&Path> = Vec::new();
    for round in &index.completed_rounds {
        if let Some(configured) = &round.configured {
            all_paths.push(configured.rollout_path.as_path());
        }
    }
    if let Some(unfinished) = &index.unfinished_round {
        all_paths.push(unfinished.rollout_path.as_path());
    }

    all_paths.into_iter().all(|rollout_path| {
        let resolved = crate::workflow::replay_session_config::resolve_rollout_path_for_replay(
            workdir,
            rollout_path,
        );
        resolved.is_file()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use pretty_assertions::assert_eq;

    #[tokio::test]
    async fn coalesces_details_requests_into_latest_after_burst() {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut backend = spawn_projects_overlay_provider(temp.path().to_path_buf());

        let project_dirs = [
            PathBuf::from(".codexpotter/projects/2026/04/16/1"),
            PathBuf::from(".codexpotter/projects/2026/04/16/2"),
            PathBuf::from(".codexpotter/projects/2026/04/16/3"),
        ];
        for project_dir in &project_dirs {
            backend
                .request_tx
                .send(ProjectsOverlayRequest::Details {
                    project_dir: project_dir.clone(),
                })
                .expect("send details request");
        }

        let response = tokio::time::timeout(Duration::from_secs(1), backend.response_rx.recv())
            .await
            .expect("timed out waiting for response")
            .expect("receive response");
        match response {
            ProjectsOverlayResponse::Details { details } => {
                assert_eq!(details.project_dir, project_dirs[2]);
            }
            other => panic!("expected details response, got {other:?}"),
        }

        let followup =
            tokio::time::timeout(Duration::from_millis(100), backend.response_rx.recv()).await;
        assert!(followup.is_err(), "expected no extra details responses");
    }
}
