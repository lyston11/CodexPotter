//! Multi-round project rendering loop.
//!
//! A running CodexPotter project is hosted by the long-lived `codex-potter app-server` and emits
//! a single stream of `EventMsg` notifications. This module consumes that stream and drives a
//! per-round UI renderer ([`PotterRoundUi`]), pausing the stream between rounds so each round is
//! rendered as a coherent unit.
//!
//! Notes:
//! - Callers can pass `buffered_events` (notably from `resume`) that must be rendered before
//!   reading from the live server stream.
//! - The project is considered complete only after observing `PotterProjectCompleted`; missing
//!   that marker is treated as a fatal protocol error.

use std::collections::VecDeque;
use std::path::PathBuf;

use anyhow::Context;
use codex_protocol::protocol::Event;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::Op;
use codex_protocol::protocol::PotterProjectOutcome;
use codex_tui::ExitReason;
use tokio::sync::mpsc::unbounded_channel;

use crate::workflow::round_runner::PotterRoundUi;
use crate::workflow::round_runner::UiFuture;

/// Event source for a running Potter project hosted by `codex-potter app-server`.
pub trait PotterEventSource {
    /// Read the next event from the server stream.
    fn read_next_event<'a>(&'a mut self) -> UiFuture<'a, Option<Event>>;
}

/// Control plane for a running Potter project hosted by `codex-potter app-server`.
pub trait PotterProjectController {
    /// Request the active project to interrupt the current round.
    ///
    /// Returns any events that were emitted while awaiting the JSON-RPC response. Callers must
    /// render these before reading from the live event stream to preserve event ordering.
    fn interrupt_project<'a>(&'a mut self, project_id: String) -> UiFuture<'a, Vec<Event>>;
}

/// Options for rendering a running Potter project (multi-round) from an event stream.
#[derive(Debug, Clone)]
pub struct PotterProjectRenderOptions {
    /// Per-round prompt string passed to the round renderer.
    pub turn_prompt: String,
    /// Footer context (working dir + git branch) used by the TUI.
    pub prompt_footer: codex_tui::PromptFooterContext,
    /// Whether to pad the transcript before the first round.
    pub pad_before_first_cell: bool,
    /// Optional status header prefix to use for the first round when no `PotterRoundStarted`
    /// boundary event is expected (for example, continuing an unfinished round on resume).
    pub initial_status_header_prefix: Option<String>,
}

/// Outcome of rendering a Potter project from a server event stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PotterProjectRenderExit {
    /// The project ended normally, and the completion marker was observed.
    Completed { outcome: PotterProjectOutcome },
    /// The project was interrupted and is waiting for user action.
    Interrupted {
        user_prompt_file: PathBuf,
        status_header_prefix: String,
    },
    /// The user requested exit while a round UI was running.
    UserRequested,
    /// The UI requested a fatal exit before the current round or project reached a terminal
    /// marker.
    FatalExitRequested,
}

/// Render a running Potter project by consuming `EventMsg` notifications from a server stream.
pub async fn run_potter_project_render_loop<U, S>(
    ui: &mut U,
    event_source: &mut S,
    project_id: &str,
    options: PotterProjectRenderOptions,
    buffered_events: Vec<Event>,
) -> anyhow::Result<PotterProjectRenderExit>
where
    U: PotterRoundUi,
    S: PotterEventSource + PotterProjectController,
{
    let PotterProjectRenderOptions {
        turn_prompt,
        prompt_footer,
        pad_before_first_cell,
        initial_status_header_prefix,
    } = options;

    let mut pending_events = VecDeque::from(buffered_events);
    let mut project_outcome: Option<PotterProjectOutcome> = None;
    let mut rendered_rounds: u32 = 0;
    let mut pending_initial_status_header_prefix = initial_status_header_prefix;

    loop {
        if let Some(outcome) = project_outcome.clone() {
            return Ok(PotterProjectRenderExit::Completed { outcome });
        }

        let status_header_prefix = if let Some(prefix) = pending_initial_status_header_prefix.take()
        {
            Some(prefix)
        } else {
            wait_for_next_round_prefix(
                project_id,
                event_source,
                &mut pending_events,
                &mut project_outcome,
            )
            .await?
        };

        let Some(status_header_prefix) = status_header_prefix else {
            let outcome = project_outcome
                .clone()
                .unwrap_or(PotterProjectOutcome::Fatal {
                    message: format!(
                        "missing PotterProjectCompleted marker (project_id={project_id})"
                    ),
                });
            return Ok(PotterProjectRenderExit::Completed { outcome });
        };

        let pad_before_first_cell = pad_before_first_cell || rendered_rounds != 0;
        let (op_tx, mut op_rx) = unbounded_channel::<Op>();

        let (event_tx, event_rx) = unbounded_channel::<Event>();
        let (fatal_exit_tx, fatal_exit_rx) = unbounded_channel::<String>();

        let mut render = Box::pin(ui.render_round(codex_tui::RenderRoundParams {
            prompt: turn_prompt.clone(),
            pad_before_first_cell,
            status_header_prefix: Some(status_header_prefix.clone()),
            prompt_footer: prompt_footer.clone(),
            codex_op_tx: op_tx.clone(),
            codex_event_rx: event_rx,
            fatal_exit_rx,
        }));

        let mut waiting_for_render_exit = false;
        let mut interrupt_requested = false;
        let mut saw_round_finished_for_active_render = false;

        loop {
            if waiting_for_render_exit {
                let exit_info = render.await?;
                rendered_rounds = rendered_rounds.saturating_add(1);
                match exit_info.exit_reason {
                    ExitReason::UserRequested => return Ok(PotterProjectRenderExit::UserRequested),
                    ExitReason::Fatal(_) => {
                        if saw_round_finished_for_active_render || project_outcome.is_some() {
                            break;
                        }
                        return Ok(PotterProjectRenderExit::FatalExitRequested);
                    }
                    ExitReason::Interrupted => {
                        match wait_for_project_interrupted_marker(
                            project_id,
                            event_source,
                            &mut pending_events,
                        )
                        .await?
                        {
                            ProjectInterruptedMarkerOutcome::Interrupted { user_prompt_file } => {
                                return Ok(PotterProjectRenderExit::Interrupted {
                                    user_prompt_file,
                                    status_header_prefix,
                                });
                            }
                            ProjectInterruptedMarkerOutcome::Completed { outcome } => {
                                return Ok(PotterProjectRenderExit::Completed { outcome });
                            }
                        }
                    }
                    ExitReason::TaskFailed(_) | ExitReason::Completed => break,
                }
            }

            let next_event = async {
                if let Some(event) = pending_events.pop_front() {
                    return Ok(Some(event));
                }
                event_source.read_next_event().await
            };

            tokio::select! {
                Some(op) = op_rx.recv() => {
                    if matches!(op, Op::Interrupt) && !interrupt_requested {
                        match event_source.interrupt_project(project_id.to_string()).await {
                            Ok(buffered_events) => {
                                let mut buffered_events = VecDeque::from(buffered_events);
                                pending_events.append(&mut buffered_events);
                                interrupt_requested = true;
                            }
                            Err(err) => {
                                let message = format!(
                                    "failed to interrupt project via potter app-server (project_id={project_id}): {err:#}"
                                );
                                project_outcome = Some(PotterProjectOutcome::Fatal { message: message.clone() });
                                let _ = fatal_exit_tx.send(message);
                            }
                        }
                    }
                }
                exit_info = &mut render => {
                    let exit_info = exit_info?;
                    rendered_rounds = rendered_rounds.saturating_add(1);
                    match exit_info.exit_reason {
                        ExitReason::UserRequested => return Ok(PotterProjectRenderExit::UserRequested),
                        ExitReason::Fatal(_) => {
                            if saw_round_finished_for_active_render || project_outcome.is_some() {
                                break;
                            }
                            return Ok(PotterProjectRenderExit::FatalExitRequested);
                        }
                        ExitReason::Interrupted => {
                            match wait_for_project_interrupted_marker(project_id, event_source, &mut pending_events).await? {
                                ProjectInterruptedMarkerOutcome::Interrupted { user_prompt_file } => {
                                    return Ok(PotterProjectRenderExit::Interrupted {
                                        user_prompt_file,
                                        status_header_prefix,
                                    });
                                }
                                ProjectInterruptedMarkerOutcome::Completed { outcome } => {
                                    return Ok(PotterProjectRenderExit::Completed { outcome });
                                }
                            }
                        }
                        ExitReason::TaskFailed(_) | ExitReason::Completed => break,
                    }
                }
                maybe_event = next_event => {
                    let maybe_event = maybe_event.context("read potter app-server event")?;
                    let Some(event) = maybe_event else {
                        let message = format!(
                            "potter app-server event stream closed unexpectedly (project_id={project_id})"
                        );
                        project_outcome = Some(PotterProjectOutcome::Fatal { message: message.clone() });
                        let _ = fatal_exit_tx.send(message);
                        continue;
                    };

                    if let EventMsg::PotterProjectCompleted { outcome } = &event.msg {
                        project_outcome = Some(outcome.clone());
                        continue;
                    }

                    let saw_round_finished =
                        matches!(&event.msg, EventMsg::PotterRoundFinished { .. });
                    if event_tx.send(event).is_err() {
                        continue;
                    }
                    if saw_round_finished {
                        saw_round_finished_for_active_render = true;
                        // Avoid reading events for the next round until the UI exits.
                        waiting_for_render_exit = true;
                    }
                }
            }
        }
    }
}

enum ProjectInterruptedMarkerOutcome {
    Interrupted { user_prompt_file: PathBuf },
    Completed { outcome: PotterProjectOutcome },
}

async fn wait_for_project_interrupted_marker<S>(
    project_id: &str,
    event_source: &mut S,
    pending_events: &mut VecDeque<Event>,
) -> anyhow::Result<ProjectInterruptedMarkerOutcome>
where
    S: PotterEventSource,
{
    loop {
        let event = if let Some(event) = pending_events.pop_front() {
            event
        } else {
            event_source
                .read_next_event()
                .await?
                .context("read potter app-server event while awaiting PotterProjectInterrupted")?
        };

        match event.msg {
            EventMsg::PotterProjectInterrupted {
                project_id: interrupted_project_id,
                user_prompt_file,
            } => {
                anyhow::ensure!(
                    interrupted_project_id == project_id,
                    "unexpected PotterProjectInterrupted marker project_id: expected={project_id} actual={interrupted_project_id}"
                );
                return Ok(ProjectInterruptedMarkerOutcome::Interrupted { user_prompt_file });
            }
            EventMsg::PotterProjectCompleted { outcome } => {
                return Ok(ProjectInterruptedMarkerOutcome::Completed { outcome });
            }
            _ => {}
        }
    }
}

async fn wait_for_next_round_prefix<S>(
    project_id: &str,
    event_source: &mut S,
    pending_events: &mut VecDeque<Event>,
    project_outcome: &mut Option<PotterProjectOutcome>,
) -> anyhow::Result<Option<String>>
where
    S: PotterEventSource,
{
    loop {
        if project_outcome.is_some() {
            return Ok(None);
        }

        for event in pending_events.iter() {
            if let EventMsg::PotterRoundStarted { current, total } = &event.msg {
                return Ok(Some(format!("Round {current}/{total}")));
            }
        }

        if let Some((idx, outcome)) =
            pending_events
                .iter()
                .enumerate()
                .find_map(|(idx, event)| match &event.msg {
                    EventMsg::PotterProjectCompleted { outcome } => Some((idx, outcome.clone())),
                    _ => None,
                })
        {
            let _ = pending_events.remove(idx);
            *project_outcome = Some(outcome);
            continue;
        }

        let Some(event) = event_source.read_next_event().await? else {
            let message = format!(
                "potter app-server event stream closed unexpectedly (project_id={project_id})"
            );
            *project_outcome = Some(PotterProjectOutcome::Fatal { message });
            return Ok(None);
        };

        if let EventMsg::PotterProjectCompleted { outcome } = &event.msg {
            *project_outcome = Some(outcome.clone());
            continue;
        }

        pending_events.push_back(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use codex_protocol::protocol::PotterRoundOutcome;
    use codex_protocol::protocol::TokenUsage;
    use pretty_assertions::assert_eq;

    #[derive(Default)]
    struct InterruptingUi {
        interrupt_sent: bool,
    }

    impl PotterRoundUi for InterruptingUi {
        fn set_project_started_at(&mut self, _started_at: std::time::Instant) {}

        fn render_round<'a>(
            &'a mut self,
            params: codex_tui::RenderRoundParams,
        ) -> UiFuture<'a, codex_tui::AppExitInfo> {
            self.interrupt_sent = true;
            Box::pin(async move {
                let codex_tui::RenderRoundParams {
                    codex_op_tx,
                    mut codex_event_rx,
                    ..
                } = params;

                codex_op_tx
                    .send(Op::Interrupt)
                    .map_err(|_| anyhow::anyhow!("op channel closed"))?;

                while let Some(event) = codex_event_rx.recv().await {
                    if let EventMsg::PotterRoundFinished { outcome } = &event.msg {
                        return Ok(codex_tui::AppExitInfo {
                            token_usage: TokenUsage::default(),
                            thread_id: None,
                            exit_reason: match outcome {
                                PotterRoundOutcome::Completed => codex_tui::ExitReason::Completed,
                                PotterRoundOutcome::Interrupted => {
                                    codex_tui::ExitReason::Interrupted
                                }
                                PotterRoundOutcome::UserRequested => {
                                    codex_tui::ExitReason::UserRequested
                                }
                                PotterRoundOutcome::TaskFailed { message } => {
                                    codex_tui::ExitReason::TaskFailed(message.clone())
                                }
                                PotterRoundOutcome::Fatal { message } => {
                                    codex_tui::ExitReason::Fatal(message.clone())
                                }
                            },
                        });
                    }
                }

                Ok(codex_tui::AppExitInfo {
                    token_usage: TokenUsage::default(),
                    thread_id: None,
                    exit_reason: codex_tui::ExitReason::Fatal(
                        "event stream closed unexpectedly".to_string(),
                    ),
                })
            })
        }
    }

    #[derive(Default)]
    struct MockEventSource {
        interrupt_calls: Vec<String>,
    }

    impl PotterEventSource for MockEventSource {
        fn read_next_event<'a>(&'a mut self) -> UiFuture<'a, Option<Event>> {
            Box::pin(async { Ok(None) })
        }
    }

    impl PotterProjectController for MockEventSource {
        fn interrupt_project<'a>(&'a mut self, project_id: String) -> UiFuture<'a, Vec<Event>> {
            self.interrupt_calls.push(project_id.clone());
            Box::pin(async move {
                Ok(vec![
                    Event {
                        id: "round-finished".to_string(),
                        msg: EventMsg::PotterRoundFinished {
                            outcome: PotterRoundOutcome::Interrupted,
                        },
                    },
                    Event {
                        id: "project-interrupted".to_string(),
                        msg: EventMsg::PotterProjectInterrupted {
                            project_id,
                            user_prompt_file: PathBuf::from(
                                ".codexpotter/projects/2026/03/06/4/MAIN.md",
                            ),
                        },
                    },
                ])
            })
        }
    }

    #[tokio::test]
    async fn interrupt_op_exits_with_interrupted_marker() {
        let mut ui = InterruptingUi::default();
        let mut source = MockEventSource::default();

        let exit = run_potter_project_render_loop(
            &mut ui,
            &mut source,
            "project_1",
            PotterProjectRenderOptions {
                turn_prompt: String::from("Continue"),
                prompt_footer: codex_tui::PromptFooterContext::new(PathBuf::from("/tmp"), None),
                pad_before_first_cell: false,
                initial_status_header_prefix: None,
            },
            vec![Event {
                id: "round-start".to_string(),
                msg: EventMsg::PotterRoundStarted {
                    current: 1,
                    total: 2,
                },
            }],
        )
        .await
        .expect("render loop");

        assert!(ui.interrupt_sent, "expected UI to send Op::Interrupt");
        assert_eq!(source.interrupt_calls, vec![String::from("project_1")]);
        assert_eq!(
            exit,
            PotterProjectRenderExit::Interrupted {
                user_prompt_file: PathBuf::from(".codexpotter/projects/2026/03/06/4/MAIN.md"),
                status_header_prefix: String::from("Round 1/2"),
            }
        );
    }

    #[derive(Default)]
    struct RecordingUi {
        status_header_prefixes: Vec<Option<String>>,
    }

    impl PotterRoundUi for RecordingUi {
        fn set_project_started_at(&mut self, _started_at: std::time::Instant) {}

        fn render_round<'a>(
            &'a mut self,
            params: codex_tui::RenderRoundParams,
        ) -> UiFuture<'a, codex_tui::AppExitInfo> {
            self.status_header_prefixes
                .push(params.status_header_prefix.clone());
            Box::pin(async move {
                let codex_tui::RenderRoundParams {
                    mut codex_event_rx, ..
                } = params;

                while let Some(event) = codex_event_rx.recv().await {
                    if let EventMsg::PotterRoundFinished { outcome } = &event.msg {
                        return Ok(codex_tui::AppExitInfo {
                            token_usage: TokenUsage::default(),
                            thread_id: None,
                            exit_reason: match outcome {
                                PotterRoundOutcome::Completed => codex_tui::ExitReason::Completed,
                                PotterRoundOutcome::Interrupted => {
                                    codex_tui::ExitReason::Interrupted
                                }
                                PotterRoundOutcome::UserRequested => {
                                    codex_tui::ExitReason::UserRequested
                                }
                                PotterRoundOutcome::TaskFailed { message } => {
                                    codex_tui::ExitReason::TaskFailed(message.clone())
                                }
                                PotterRoundOutcome::Fatal { message } => {
                                    codex_tui::ExitReason::Fatal(message.clone())
                                }
                            },
                        });
                    }
                }

                Ok(codex_tui::AppExitInfo {
                    token_usage: TokenUsage::default(),
                    thread_id: None,
                    exit_reason: codex_tui::ExitReason::Fatal(
                        "event stream closed unexpectedly".to_string(),
                    ),
                })
            })
        }
    }

    #[tokio::test]
    async fn fatal_round_continues_when_project_stream_has_more_rounds() {
        let mut ui = RecordingUi::default();
        let mut source = MockEventSource::default();

        let exit = run_potter_project_render_loop(
            &mut ui,
            &mut source,
            "project_1",
            PotterProjectRenderOptions {
                turn_prompt: String::from("Continue"),
                prompt_footer: codex_tui::PromptFooterContext::new(PathBuf::from("/tmp"), None),
                pad_before_first_cell: false,
                initial_status_header_prefix: None,
            },
            vec![
                Event {
                    id: "round-1-start".to_string(),
                    msg: EventMsg::PotterRoundStarted {
                        current: 1,
                        total: 2,
                    },
                },
                Event {
                    id: "round-1-finished".to_string(),
                    msg: EventMsg::PotterRoundFinished {
                        outcome: PotterRoundOutcome::Fatal {
                            message: String::from("access token refresh failed"),
                        },
                    },
                },
                Event {
                    id: "round-2-start".to_string(),
                    msg: EventMsg::PotterRoundStarted {
                        current: 2,
                        total: 2,
                    },
                },
                Event {
                    id: "round-2-finished".to_string(),
                    msg: EventMsg::PotterRoundFinished {
                        outcome: PotterRoundOutcome::Completed,
                    },
                },
                Event {
                    id: "project-completed".to_string(),
                    msg: EventMsg::PotterProjectCompleted {
                        outcome: PotterProjectOutcome::BudgetExhausted,
                    },
                },
            ],
        )
        .await
        .expect("render loop");

        assert_eq!(
            exit,
            PotterProjectRenderExit::Completed {
                outcome: PotterProjectOutcome::BudgetExhausted,
            }
        );
        assert_eq!(
            ui.status_header_prefixes,
            vec![
                Some(String::from("Round 1/2")),
                Some(String::from("Round 2/2")),
            ]
        );
    }
}
