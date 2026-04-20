use std::path::PathBuf;

use crate::engine::CommandShell;
use crate::engine::HooksEngine;
use crate::events::project_stop::ProjectStopOutcome;
use crate::events::project_stop::ProjectStopRequest;

#[derive(Default, Clone)]
pub struct HooksConfig {
    pub cwd: Option<PathBuf>,
    /// Override the Codex home directory used when locating `hooks.json`.
    ///
    /// When unset, hooks discovery follows upstream behavior:
    /// - `$CODEX_HOME/hooks.json` when `CODEX_HOME` is set and non-empty
    /// - otherwise `~/.codex/hooks.json`
    pub codex_home_dir: Option<PathBuf>,
    pub shell_program: Option<String>,
    pub shell_args: Vec<String>,
}

#[derive(Clone)]
pub struct Hooks {
    engine: HooksEngine,
}

impl Default for Hooks {
    fn default() -> Self {
        Self::new(HooksConfig::default())
    }
}

impl Hooks {
    pub fn new(config: HooksConfig) -> Self {
        let engine = HooksEngine::new(
            config.cwd.as_deref(),
            config.codex_home_dir.as_deref(),
            CommandShell {
                program: config.shell_program.unwrap_or_default(),
                args: config.shell_args,
            },
        );
        Self { engine }
    }

    pub fn startup_warnings(&self) -> &[String] {
        self.engine.warnings()
    }

    pub fn preview_project_stop(
        &self,
        request: &ProjectStopRequest,
    ) -> Vec<codex_protocol::protocol::HookRunSummary> {
        self.engine.preview_project_stop(request)
    }

    pub async fn run_project_stop(&self, request: ProjectStopRequest) -> ProjectStopOutcome {
        self.engine.run_project_stop(request).await
    }
}
