//! Headless round UI for `codex-potter exec` human-readable output.
//!
//! This renderer is append-only and non-interactive:
//! - it emits plain text blocks to stdout
//! - interactive requests are treated as fatal
//! - round lifecycle still follows `PotterRoundUi`, so project/round orchestration stays shared

use std::io::Write;
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

/// Append-only `exec` round renderer that writes human-readable text blocks.
pub struct ExecHumanRoundUi<W: Write> {
    output: W,
    renderer: ExecHumanRenderer,
    token_usage: TokenUsage,
    thread_id: Option<ThreadId>,
}

impl<W: Write> ExecHumanRoundUi<W> {
    /// Create a new renderer.
    pub fn new(output: W, verbosity: Verbosity, width: Option<u16>, color_enabled: bool) -> Self {
        Self {
            output,
            renderer: ExecHumanRenderer::new(verbosity, width, color_enabled),
            token_usage: TokenUsage::default(),
            thread_id: None,
        }
    }

    fn write_block(&mut self, block: &str, needs_spacing: &mut bool) -> anyhow::Result<()> {
        if block.is_empty() {
            return Ok(());
        }
        if *needs_spacing {
            self.output.write_all(b"\n")?;
        }
        self.output.write_all(block.as_bytes())?;
        self.output.write_all(b"\n")?;
        self.output.flush()?;
        *needs_spacing = true;
        Ok(())
    }

    fn write_blocks(
        &mut self,
        blocks: Vec<String>,
        needs_spacing: &mut bool,
    ) -> anyhow::Result<()> {
        for block in blocks {
            self.write_block(&block, needs_spacing)?;
        }
        Ok(())
    }

    fn process_event(
        &mut self,
        event: &Event,
        needs_spacing: &mut bool,
    ) -> anyhow::Result<Option<AppExitInfo>> {
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
                self.write_block(&block, needs_spacing)?;
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
                self.write_block(&block, needs_spacing)?;
                return Ok(Some(AppExitInfo {
                    token_usage: self.token_usage.clone(),
                    thread_id: self.thread_id,
                    exit_reason: ExitReason::Fatal(message),
                }));
            }
            _ => {}
        }

        let blocks = self.renderer.handle_event(&event.msg)?;
        self.write_blocks(blocks, needs_spacing)?;

        let exit_reason = match &event.msg {
            EventMsg::PotterRoundFinished { outcome } => Some(exit_reason_from_outcome(outcome)),
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
    fn set_project_started_at(&mut self, _started_at: Instant) {}

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

            let mut needs_spacing = false;

            loop {
                while let Ok(event) = codex_event_rx.try_recv() {
                    if let Some(exit_info) = self.process_event(&event, &mut needs_spacing)? {
                        return Ok(exit_info);
                    }
                }

                if let Ok(message) = fatal_exit_rx.try_recv() {
                    let blocks = self.renderer.flush_for_exit()?;
                    self.write_blocks(blocks, &mut needs_spacing)?;
                    let block = self.renderer.render_error_block(message.clone())?;
                    self.write_block(&block, &mut needs_spacing)?;
                    return Ok(AppExitInfo {
                        token_usage: self.token_usage.clone(),
                        thread_id: self.thread_id,
                        exit_reason: ExitReason::Fatal(message),
                    });
                }

                tokio::select! {
                    Some(message) = fatal_exit_rx.recv() => {
                        while let Ok(event) = codex_event_rx.try_recv() {
                            if let Some(exit_info) = self.process_event(&event, &mut needs_spacing)? {
                                return Ok(exit_info);
                            }
                        }

                        let blocks = self.renderer.flush_for_exit()?;
                        self.write_blocks(blocks, &mut needs_spacing)?;
                        let block = self.renderer.render_error_block(message.clone())?;
                        self.write_block(&block, &mut needs_spacing)?;
                        return Ok(AppExitInfo {
                            token_usage: self.token_usage.clone(),
                            thread_id: self.thread_id,
                            exit_reason: ExitReason::Fatal(message),
                        });
                    }
                    maybe_event = codex_event_rx.recv() => {
                        let Some(event) = maybe_event else {
                            let message = "codex event stream closed unexpectedly".to_string();
                            let blocks = self.renderer.flush_for_exit()?;
                            self.write_blocks(blocks, &mut needs_spacing)?;
                            let block = self.renderer.render_error_block(message.clone())?;
                            self.write_block(&block, &mut needs_spacing)?;
                            return Ok(AppExitInfo {
                                token_usage: self.token_usage.clone(),
                                thread_id: self.thread_id,
                                exit_reason: ExitReason::Fatal(message),
                            });
                        };

                        if let Some(exit_info) = self.process_event(&event, &mut needs_spacing)? {
                            return Ok(exit_info);
                        }
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
    use codex_protocol::protocol::TurnStartedEvent;
    use pretty_assertions::assert_eq;
    use std::path::PathBuf;
    use tokio::sync::mpsc::unbounded_channel;

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
}
