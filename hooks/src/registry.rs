use std::path::PathBuf;

use crate::engine::CommandShell;
use crate::engine::HooksEngine;
use crate::events::project_stop::ProjectStopOutcome;
use crate::events::project_stop::ProjectStopRequest;

/// Configuration for hook discovery and execution.
///
/// CodexPotter currently supports only the `Potter.ProjectStop` hook event.
#[derive(Default, Clone)]
pub struct HooksConfig {
    /// Working directory used for locating repo-level `.codex/hooks.json`.
    pub cwd: Option<PathBuf>,
    /// Override the Codex home directory used when locating `hooks.json`.
    ///
    /// When unset, hooks discovery follows upstream behavior:
    /// - `$CODEX_HOME/hooks.json` when `CODEX_HOME` is set and non-empty
    /// - otherwise `~/.codex/hooks.json`
    pub codex_home_dir: Option<PathBuf>,
    /// Override the shell program used to execute command hooks.
    ///
    /// When unset, the engine uses an OS-appropriate default.
    pub shell_program: Option<String>,
    /// Extra args forwarded to the hook shell program.
    pub shell_args: Vec<String>,
}

/// Hook discovery and execution entrypoint.
///
/// This type performs `hooks.json` discovery on construction, then can preview/run supported
/// hook events.
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
    /// Create a new hooks registry from configuration.
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

    /// Non-fatal warnings captured while discovering hook configuration.
    pub fn startup_warnings(&self) -> &[String] {
        self.engine.warnings()
    }

    /// Preview the `Potter.ProjectStop` hook runs that would execute for `request`.
    pub fn preview_project_stop(
        &self,
        request: &ProjectStopRequest,
    ) -> Vec<codex_protocol::protocol::HookRunSummary> {
        self.engine.preview_project_stop(request)
    }

    /// Execute all configured `Potter.ProjectStop` hooks and return their completion events.
    pub async fn run_project_stop(&self, request: ProjectStopRequest) -> ProjectStopOutcome {
        self.engine.run_project_stop(request).await
    }
}
