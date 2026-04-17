//! Projects list overlay provider backend.
//!
//! `codex-potter` renders the projects overlay in the TUI, but all filesystem scanning/parsing
//! is owned by the CLI workflow layer. This helper keeps the control-plane logic consistent
//! across the live project render loop and the prompt screen (when no project is running).

use std::path::Path;
use std::path::PathBuf;

use codex_tui::ProjectsOverlayRequest;
use codex_tui::ProjectsOverlayResponse;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::mpsc::unbounded_channel;

/// Spawn a background task that serves projects overlay requests for the given `workdir`.
pub fn spawn_projects_overlay_provider(
    workdir: PathBuf,
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
        request_rx,
        response_tx,
    ));

    codex_tui::ProjectsOverlayProviderChannels {
        request_tx,
        response_rx,
    }
}

async fn serve_projects_overlay_requests(
    workdir: PathBuf,
    mut request_rx: UnboundedReceiver<ProjectsOverlayRequest>,
    response_tx: UnboundedSender<ProjectsOverlayResponse>,
) {
    while let Some(request) = request_rx.recv().await {
        // Discovery/detail parsing is synchronous and can touch the filesystem, so run it on the
        // blocking pool instead of stalling the async runtime.
        let workdir = workdir.clone();
        let response = match tokio::task::spawn_blocking(move || {
            response_for_projects_overlay_request(&workdir, request)
        })
        .await
        {
            Ok(response) => response,
            Err(_) => return,
        };
        if response_tx.send(response).is_err() {
            return;
        }
    }
}

fn response_for_projects_overlay_request(
    workdir: &Path,
    request: ProjectsOverlayRequest,
) -> ProjectsOverlayResponse {
    match request {
        ProjectsOverlayRequest::List => {
            let (projects, error) =
                match super::projects_overlay_index::discover_projects_for_overlay(workdir) {
                    Ok(projects) => (projects, None),
                    Err(err) => (Vec::new(), Some(format!("{err:#}"))),
                };
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
