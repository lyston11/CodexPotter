//! Headless round UI for `codex-potter exec` human-readable output.
//!
//! This renderer is append-only and non-interactive:
//! - it emits plain text blocks to stdout
//! - interactive requests are treated as fatal
//! - round lifecycle still follows `PotterRoundUi`, so project/round orchestration stays shared

use std::io::Write;
use std::time::Duration;
use std::time::Instant;

use codex_protocol::ThreadId;
use codex_protocol::protocol::Event;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::Op;
use codex_protocol::protocol::PotterRoundOutcome;
use codex_protocol::protocol::TokenUsage;
use codex_protocol::user_input::UserInput;
use codex_tui::AppExitInfo;
use codex_tui::ExecHumanRenderer;
use codex_tui::ExitReason;
use codex_tui::Verbosity;

const PENDING_AGENT_MESSAGE_IDLE_FLUSH_DELAY: Duration = Duration::from_millis(250);

/// Append-only `exec` round renderer that writes human-readable text blocks.
pub struct ExecHumanRoundUi<W: Write> {
    output: W,
    renderer: ExecHumanRenderer,
    token_usage: TokenUsage,
    thread_id: Option<ThreadId>,
    needs_spacing_between_blocks: bool,
}

impl<W: Write> ExecHumanRoundUi<W> {
    /// Create a new renderer.
    pub fn new(output: W, verbosity: Verbosity, width: Option<u16>, color_enabled: bool) -> Self {
        Self {
            output,
            renderer: ExecHumanRenderer::new(verbosity, width, color_enabled),
            token_usage: TokenUsage::default(),
            thread_id: None,
            needs_spacing_between_blocks: false,
        }
    }

    fn write_block(&mut self, block: &str) -> anyhow::Result<()> {
        if block.is_empty() {
            return Ok(());
        }
        if self.needs_spacing_between_blocks {
            self.output.write_all(b"\n")?;
        }
        self.output.write_all(block.as_bytes())?;
        self.output.write_all(b"\n")?;
        self.output.flush()?;
        self.needs_spacing_between_blocks = true;
        Ok(())
    }

    fn write_blocks(&mut self, blocks: Vec<String>) -> anyhow::Result<()> {
        for block in blocks {
            self.write_block(&block)?;
        }
        Ok(())
    }

    fn refresh_idle_flush_timer(
        &self,
        idle_flush_armed: &mut bool,
        idle_flush_sleep: std::pin::Pin<&mut tokio::time::Sleep>,
    ) {
        if self.renderer.needs_idle_agent_message_flush() {
            idle_flush_sleep
                .reset(tokio::time::Instant::now() + PENDING_AGENT_MESSAGE_IDLE_FLUSH_DELAY);
            *idle_flush_armed = true;
        } else {
            *idle_flush_armed = false;
        }
    }

    fn process_event(&mut self, event: &Event) -> anyhow::Result<Option<AppExitInfo>> {
        if let EventMsg::TokenCount(ev) = &event.msg
            && let Some(info) = &ev.info
        {
            self.token_usage = info.total_token_usage.clone();
        }
        if let EventMsg::SessionConfigured(cfg) = &event.msg {
            self.thread_id = Some(cfg.session_id);
        }

        match &event.msg {
            EventMsg::RequestUserInput(ev) => {
                let message = format!(
                    "unsupported interactive request: RequestUserInput call_id={}",
                    ev.call_id
                );
                let block = self.renderer.render_error_block(message.clone())?;
                self.write_block(&block)?;
                return Ok(Some(AppExitInfo {
                    token_usage: self.token_usage.clone(),
                    thread_id: self.thread_id,
                    exit_reason: ExitReason::Fatal(message),
                }));
            }
            EventMsg::ElicitationRequest(ev) => {
                let message = format!(
                    "unsupported interactive request: ElicitationRequest server_name={} request_id={}",
                    ev.server_name, ev.id
                );
                let block = self.renderer.render_error_block(message.clone())?;
                self.write_block(&block)?;
                return Ok(Some(AppExitInfo {
                    token_usage: self.token_usage.clone(),
                    thread_id: self.thread_id,
                    exit_reason: ExitReason::Fatal(message),
                }));
            }
            _ => {}
        }

        let blocks = self.renderer.handle_event(&event.msg)?;
        self.write_blocks(blocks)?;

        let exit_reason = match &event.msg {
            EventMsg::PotterRoundFinished { outcome, .. } => {
                Some(exit_reason_from_outcome(outcome))
            }
            _ => None,
        };

        Ok(exit_reason.map(|exit_reason| AppExitInfo {
            token_usage: self.token_usage.clone(),
            thread_id: self.thread_id,
            exit_reason,
        }))
    }
}

impl<W: Write> crate::workflow::round_runner::PotterRoundUi for ExecHumanRoundUi<W> {
    fn set_project_started_at(&mut self, started_at: Instant) {
        self.renderer.set_project_started_at(started_at);
    }

    fn render_round<'a>(
        &'a mut self,
        params: codex_tui::RenderRoundParams,
    ) -> crate::workflow::round_runner::UiFuture<'a, AppExitInfo> {
        Box::pin(async move {
            let codex_tui::RenderRoundParams {
                prompt,
                codex_op_tx,
                mut codex_event_rx,
                mut fatal_exit_rx,
                ..
            } = params;

            self.token_usage = TokenUsage::default();
            self.thread_id = None;

            codex_op_tx
                .send(Op::UserInput {
                    items: vec![UserInput::Text {
                        text: prompt,
                        text_elements: Vec::new(),
                    }],
                    final_output_json_schema: None,
                })
                .map_err(|_| anyhow::anyhow!("codex op channel closed"))?;

            let idle_flush_sleep = tokio::time::sleep(PENDING_AGENT_MESSAGE_IDLE_FLUSH_DELAY);
            tokio::pin!(idle_flush_sleep);
            let mut idle_flush_armed = false;

            loop {
                while let Ok(event) = codex_event_rx.try_recv() {
                    if let Some(exit_info) = self.process_event(&event)? {
                        return Ok(exit_info);
                    }
                    self.refresh_idle_flush_timer(&mut idle_flush_armed, idle_flush_sleep.as_mut());
                }

                if let Ok(message) = fatal_exit_rx.try_recv() {
                    let blocks = self.renderer.flush_for_exit()?;
                    self.write_blocks(blocks)?;
                    let block = self.renderer.render_error_block(message.clone())?;
                    self.write_block(&block)?;
                    return Ok(AppExitInfo {
                        token_usage: self.token_usage.clone(),
                        thread_id: self.thread_id,
                        exit_reason: ExitReason::Fatal(message),
                    });
                }

                tokio::select! {
                    Some(message) = fatal_exit_rx.recv() => {
                        while let Ok(event) = codex_event_rx.try_recv() {
                            if let Some(exit_info) = self.process_event(&event)? {
                                return Ok(exit_info);
                            }
                        }

                        let blocks = self.renderer.flush_for_exit()?;
                        self.write_blocks(blocks)?;
                        let block = self.renderer.render_error_block(message.clone())?;
                        self.write_block(&block)?;
                        return Ok(AppExitInfo {
                            token_usage: self.token_usage.clone(),
                            thread_id: self.thread_id,
                            exit_reason: ExitReason::Fatal(message),
                        });
                    }
                    _ = &mut idle_flush_sleep, if idle_flush_armed => {
                        if let Some(block) = self.renderer.flush_idle_agent_message()? {
                            self.write_block(&block)?;
                        }
                        self.refresh_idle_flush_timer(&mut idle_flush_armed, idle_flush_sleep.as_mut());
                    }
                    maybe_event = codex_event_rx.recv() => {
                        let Some(event) = maybe_event else {
                            let message = "codex event stream closed unexpectedly".to_string();
                            let blocks = self.renderer.flush_for_exit()?;
                            self.write_blocks(blocks)?;
                            let block = self.renderer.render_error_block(message.clone())?;
                            self.write_block(&block)?;
                            return Ok(AppExitInfo {
                                token_usage: self.token_usage.clone(),
                                thread_id: self.thread_id,
                                exit_reason: ExitReason::Fatal(message),
                            });
                        };

                        if let Some(exit_info) = self.process_event(&event)? {
                            return Ok(exit_info);
                        }
                        self.refresh_idle_flush_timer(&mut idle_flush_armed, idle_flush_sleep.as_mut());
                    }
                }
            }
        })
    }
}

fn exit_reason_from_outcome(outcome: &PotterRoundOutcome) -> ExitReason {
    match outcome {
        PotterRoundOutcome::Completed => ExitReason::Completed,
        PotterRoundOutcome::Interrupted => ExitReason::Interrupted,
        PotterRoundOutcome::UserRequested => ExitReason::UserRequested,
        PotterRoundOutcome::TaskFailed { message } => ExitReason::TaskFailed(message.clone()),
        PotterRoundOutcome::Fatal { message } => ExitReason::Fatal(message.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::round_runner::PotterRoundUi;
    use codex_protocol::approvals::ElicitationRequestEvent;
    use codex_protocol::mcp::RequestId;
    use codex_protocol::protocol::AgentMessageEvent;
    use codex_protocol::protocol::PotterRoundOutcome;
    use codex_protocol::protocol::TurnStartedEvent;
    use pretty_assertions::assert_eq;
    use std::io;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::Mutex;
    use tokio::sync::mpsc::unbounded_channel;
    use tokio::time;

    #[derive(Clone, Default)]
    struct SharedOutput {
        inner: Arc<Mutex<Vec<u8>>>,
    }

    impl SharedOutput {
        fn contents(&self) -> String {
            String::from_utf8(self.inner.lock().expect("lock output").clone()).expect("utf8 output")
        }
    }

    impl Write for SharedOutput {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.inner
                .lock()
                .expect("lock output")
                .extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn elicitation_request_fails_fast_with_error_block() {
        let (codex_op_tx, mut codex_op_rx) = unbounded_channel::<Op>();
        let (codex_event_tx, codex_event_rx) = unbounded_channel::<Event>();
        let (_fatal_exit_tx, fatal_exit_rx) = unbounded_channel::<String>();

        codex_event_tx
            .send(Event {
                id: "round-start".to_string(),
                msg: EventMsg::PotterRoundStarted {
                    current: 1,
                    total: 3,
                },
            })
            .expect("send PotterRoundStarted");
        codex_event_tx
            .send(Event {
                id: "turn-start".to_string(),
                msg: EventMsg::TurnStarted(TurnStartedEvent {
                    turn_id: "turn-1".to_string(),
                    model_context_window: None,
                }),
            })
            .expect("send TurnStarted");
        codex_event_tx
            .send(Event {
                id: "elicitation".to_string(),
                msg: EventMsg::ElicitationRequest(ElicitationRequestEvent {
                    turn_id: None,
                    server_name: "mcp-server".to_string(),
                    id: RequestId::String("req-1".to_string()),
                    request: None,
                    message: Some("need input".to_string()),
                }),
            })
            .expect("send ElicitationRequest");
        drop(codex_event_tx);

        let mut ui = ExecHumanRoundUi::new(Vec::new(), Verbosity::Minimal, Some(120), false);
        let exit_info = ui
            .render_round(codex_tui::RenderRoundParams {
                prompt: "Continue working according to the WORKFLOW_INSTRUCTIONS".to_string(),
                pad_before_first_cell: false,
                status_header_prefix: None,
                prompt_footer: codex_tui::PromptFooterContext::new(PathBuf::from("."), None),
                codex_op_tx,
                codex_event_rx,
                fatal_exit_rx,
                projects_overlay_provider: None,
            })
            .await
            .expect("render_round");

        let op = codex_op_rx.try_recv().expect("expected Op::UserInput");
        assert!(matches!(op, Op::UserInput { .. }));

        let ExitReason::Fatal(message) = &exit_info.exit_reason else {
            panic!("expected fatal exit, got: {:?}", exit_info.exit_reason);
        };
        assert_eq!(
            message,
            "unsupported interactive request: ElicitationRequest server_name=mcp-server request_id=req-1"
        );
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn idle_flush_makes_latest_agent_message_visible_before_round_exit() {
        let (codex_op_tx, mut codex_op_rx) = unbounded_channel::<Op>();
        let (codex_event_tx, codex_event_rx) = unbounded_channel::<Event>();
        let (_fatal_exit_tx, fatal_exit_rx) = unbounded_channel::<String>();

        codex_event_tx
            .send(Event {
                id: "agent-message".to_string(),
                msg: EventMsg::AgentMessage(AgentMessageEvent {
                    message: "latest commentary".to_string(),
                    phase: None,
                }),
            })
            .expect("send agent message");

        let output = SharedOutput::default();
        let output_reader = output.clone();

        let local = tokio::task::LocalSet::new();
        local
            .run_until(async move {
                let render = tokio::task::spawn_local(async move {
                    let mut ui =
                        ExecHumanRoundUi::new(output, Verbosity::Minimal, Some(120), false);
                    ui.render_round(codex_tui::RenderRoundParams {
                        prompt: "Continue".to_string(),
                        pad_before_first_cell: false,
                        status_header_prefix: None,
                        prompt_footer: codex_tui::PromptFooterContext::new(
                            PathBuf::from("."),
                            None,
                        ),
                        codex_op_tx,
                        codex_event_rx,
                        fatal_exit_rx,
                        projects_overlay_provider: None,
                    })
                    .await
                });

                let op = codex_op_rx.recv().await.expect("expected Op::UserInput");
                assert!(matches!(op, Op::UserInput { .. }));
                assert!(!output_reader.contents().contains("latest commentary"));

                time::advance(PENDING_AGENT_MESSAGE_IDLE_FLUSH_DELAY + Duration::from_millis(1))
                    .await;
                tokio::task::yield_now().await;

                let visible_output = output_reader.contents();
                assert!(visible_output.contains("latest commentary"));

                codex_event_tx
                    .send(Event {
                        id: "round-finished".to_string(),
                        msg: EventMsg::PotterRoundFinished {
                            outcome: PotterRoundOutcome::Completed,
                            duration_secs: 0,
                        },
                    })
                    .expect("send PotterRoundFinished");
                drop(codex_event_tx);

                let exit_info = render
                    .await
                    .expect("join render task")
                    .expect("render_round");
                assert!(matches!(exit_info.exit_reason, ExitReason::Completed));

                let final_output = output_reader.contents();
                assert_eq!(final_output.matches("latest commentary").count(), 1);
            })
            .await;
    }

    #[tokio::test]
    async fn spacing_between_rounds_is_preserved_across_render_round_calls() {
        let output = SharedOutput::default();
        let output_reader = output.clone();
        let mut ui = ExecHumanRoundUi::new(output, Verbosity::Simple, Some(120), false);

        let (codex_op_tx, mut codex_op_rx) = unbounded_channel::<Op>();
        let (codex_event_tx, codex_event_rx) = unbounded_channel::<Event>();
        let (_fatal_exit_tx, fatal_exit_rx) = unbounded_channel::<String>();

        codex_event_tx
            .send(Event {
                id: "agent-message".to_string(),
                msg: EventMsg::AgentMessage(AgentMessageEvent {
                    message: "Workspace is clean.".to_string(),
                    phase: None,
                }),
            })
            .expect("send agent message");
        codex_event_tx
            .send(Event {
                id: "round-finished".to_string(),
                msg: EventMsg::PotterRoundFinished {
                    outcome: PotterRoundOutcome::Completed,
                    duration_secs: 0,
                },
            })
            .expect("send PotterRoundFinished");
        drop(codex_event_tx);

        let exit_info = ui
            .render_round(codex_tui::RenderRoundParams {
                prompt: "Continue".to_string(),
                pad_before_first_cell: false,
                status_header_prefix: None,
                prompt_footer: codex_tui::PromptFooterContext::new(PathBuf::from("."), None),
                codex_op_tx,
                codex_event_rx,
                fatal_exit_rx,
                projects_overlay_provider: None,
            })
            .await
            .expect("first render_round");
        assert!(matches!(exit_info.exit_reason, ExitReason::Completed));
        let op = codex_op_rx.try_recv().expect("expected Op::UserInput");
        assert!(matches!(op, Op::UserInput { .. }));

        let (codex_op_tx, mut codex_op_rx) = unbounded_channel::<Op>();
        let (codex_event_tx, codex_event_rx) = unbounded_channel::<Event>();
        let (_fatal_exit_tx, fatal_exit_rx) = unbounded_channel::<String>();

        codex_event_tx
            .send(Event {
                id: "round-start".to_string(),
                msg: EventMsg::PotterRoundStarted {
                    current: 2,
                    total: 10,
                },
            })
            .expect("send PotterRoundStarted");
        codex_event_tx
            .send(Event {
                id: "session-configured".to_string(),
                msg: EventMsg::SessionConfigured(
                    codex_protocol::protocol::SessionConfiguredEvent {
                        session_id: ThreadId::from_string("019ca423-63d9-7641-ae83-db060ad3c000")
                            .expect("thread id"),
                        forked_from_id: None,
                        model: "gpt-5.2".to_string(),
                        model_provider_id: "openai".to_string(),
                        service_tier: None,
                        cwd: PathBuf::from("."),
                        reasoning_effort: Some(
                            codex_protocol::openai_models::ReasoningEffort::XHigh,
                        ),
                        history_log_id: 0,
                        history_entry_count: 0,
                        initial_messages: None,
                        rollout_path: PathBuf::from("rollout.jsonl"),
                    },
                ),
            })
            .expect("send SessionConfigured");
        codex_event_tx
            .send(Event {
                id: "round-finished".to_string(),
                msg: EventMsg::PotterRoundFinished {
                    outcome: PotterRoundOutcome::Completed,
                    duration_secs: 0,
                },
            })
            .expect("send PotterRoundFinished");
        drop(codex_event_tx);

        let exit_info = ui
            .render_round(codex_tui::RenderRoundParams {
                prompt: "Continue".to_string(),
                pad_before_first_cell: false,
                status_header_prefix: None,
                prompt_footer: codex_tui::PromptFooterContext::new(PathBuf::from("."), None),
                codex_op_tx,
                codex_event_rx,
                fatal_exit_rx,
                projects_overlay_provider: None,
            })
            .await
            .expect("second render_round");
        assert!(matches!(exit_info.exit_reason, ExitReason::Completed));
        let op = codex_op_rx.try_recv().expect("expected Op::UserInput");
        assert!(matches!(op, Op::UserInput { .. }));

        assert!(
            output_reader.contents().contains(
                "Workspace is clean.\n\nCodexPotter: iteration round 2/10 (gpt-5.2 xhigh)"
            )
        );
    }
}
