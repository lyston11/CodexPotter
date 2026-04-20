use std::path::Path;
use std::path::PathBuf;

use codex_protocol::protocol::HookRunSummary;

use crate::events::project_stop::ProjectStopOutcome;
use crate::events::project_stop::ProjectStopRequest;

pub(crate) mod command_runner;
mod config;
mod discovery;
pub(crate) mod dispatcher;
pub(crate) mod schema_loader;

#[derive(Debug, Clone)]
pub(crate) struct CommandShell {
    pub program: String,
    pub args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConfiguredHandler {
    pub event_name: codex_protocol::protocol::HookEventName,
    pub matcher: Option<String>,
    pub command: String,
    pub timeout_sec: u64,
    pub status_message: Option<String>,
    pub source_path: PathBuf,
    pub display_order: i64,
}

impl ConfiguredHandler {
    pub fn run_id(&self) -> String {
        format!(
            "{}:{}:{}",
            self.event_name_label(),
            self.display_order,
            self.source_path.display()
        )
    }

    fn event_name_label(&self) -> &'static str {
        match self.event_name {
            codex_protocol::protocol::HookEventName::PreToolUse => "pre-tool-use",
            codex_protocol::protocol::HookEventName::PostToolUse => "post-tool-use",
            codex_protocol::protocol::HookEventName::SessionStart => "session-start",
            codex_protocol::protocol::HookEventName::UserPromptSubmit => "user-prompt-submit",
            codex_protocol::protocol::HookEventName::Stop => "stop",
            codex_protocol::protocol::HookEventName::PotterProjectStop => "potter-project-stop",
        }
    }
}

#[derive(Clone)]
pub(crate) struct HooksEngine {
    handlers: Vec<ConfiguredHandler>,
    warnings: Vec<String>,
    shell: CommandShell,
}

impl HooksEngine {
    pub(crate) fn new(cwd: Option<&Path>, shell: CommandShell) -> Self {
        let Some(cwd) = cwd else {
            return Self {
                handlers: Vec::new(),
                warnings: Vec::new(),
                shell,
            };
        };

        if cfg!(windows) {
            return Self {
                handlers: Vec::new(),
                warnings: vec![
                    "Disabled hooks because hooks.json lifecycle hooks are not supported on Windows yet."
                        .to_string(),
                ],
                shell,
            };
        }

        let _ = schema_loader::generated_hook_schemas();
        let discovered = discovery::discover_handlers(cwd);
        Self {
            handlers: discovered.handlers,
            warnings: discovered.warnings,
            shell,
        }
    }

    pub(crate) fn warnings(&self) -> &[String] {
        &self.warnings
    }

    pub(crate) fn preview_project_stop(&self, request: &ProjectStopRequest) -> Vec<HookRunSummary> {
        crate::events::project_stop::preview(&self.handlers, request)
    }

    pub(crate) async fn run_project_stop(&self, request: ProjectStopRequest) -> ProjectStopOutcome {
        crate::events::project_stop::run(&self.handlers, &self.shell, request).await
    }
}
