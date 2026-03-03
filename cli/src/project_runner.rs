use std::num::NonZeroUsize;
use std::path::Path;
use std::path::PathBuf;
use std::time::Instant;

use anyhow::Context;
use chrono::DateTime;
use chrono::Local;
use codex_tui::ExitReason;

use crate::round_runner::UiFuture;

/// Runtime configuration for running one or more CodexPotter projects.
#[derive(Debug, Clone)]
pub struct ProjectQueueOptions {
    /// Whether to prompt the user for a project goal when the queue is empty.
    pub allow_prompt_user: bool,
    /// Path to the `codex` binary to launch in app-server mode.
    pub codex_bin: String,
    /// How to launch the upstream app-server.
    pub backend_launch: crate::app_server_backend::AppServerLaunchConfig,
    /// Optional codex-compat home directory to use when launching the app-server.
    pub codex_compat_home: Option<PathBuf>,
    /// Round budget per project.
    pub rounds: NonZeroUsize,
    /// Per-round prompt passed to Codex (workflow driver prompt).
    pub turn_prompt: String,
}

/// Outcome of running the project queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectQueueExit {
    /// The queue was exhausted (or the user cancelled the composer prompt).
    Completed,
    /// The user requested exit while a project was running.
    UserRequestedExit {
        /// The project directory relative path (e.g. `.codexpotter/projects/.../N`).
        project_dir: PathBuf,
    },
    /// Fatal error exit requested.
    FatalExitRequested,
}

/// Run CodexPotter projects until the queue is exhausted.
///
/// When `options.allow_prompt_user` is `false`, this only drains prompts queued via the bottom
/// composer (and exits when the queue is empty).
pub async fn run_project_queue(
    ui: &mut codex_tui::CodexPotterTui,
    workdir: PathBuf,
    options: ProjectQueueOptions,
) -> anyhow::Result<ProjectQueueExit> {
    run_project_queue_with_deps(
        ui,
        workdir,
        options,
        &SystemProjectClock,
        &RealProjectInitializer,
        &RealProjectRoundRunner,
    )
    .await
}

trait ProjectRunnerUi {
    fn clear(&mut self) -> anyhow::Result<()>;

    fn prompt_user<'a>(
        &'a mut self,
        prompt_footer: codex_tui::PromptFooterContext,
    ) -> UiFuture<'a, Option<String>>;

    fn pop_queued_user_prompt(&mut self) -> Option<String>;
}

impl ProjectRunnerUi for codex_tui::CodexPotterTui {
    fn clear(&mut self) -> anyhow::Result<()> {
        codex_tui::CodexPotterTui::clear(self)
    }

    fn prompt_user<'a>(
        &'a mut self,
        prompt_footer: codex_tui::PromptFooterContext,
    ) -> UiFuture<'a, Option<String>> {
        Box::pin(codex_tui::CodexPotterTui::prompt_user(self, prompt_footer))
    }

    fn pop_queued_user_prompt(&mut self) -> Option<String> {
        codex_tui::CodexPotterTui::pop_queued_user_prompt(self)
    }
}

trait ProjectClock {
    fn now_datetime(&self) -> DateTime<Local>;
    fn now_instant(&self) -> Instant;
}

struct SystemProjectClock;

impl ProjectClock for SystemProjectClock {
    fn now_datetime(&self) -> DateTime<Local> {
        Local::now()
    }

    fn now_instant(&self) -> Instant {
        Instant::now()
    }
}

trait ProjectInitializer {
    fn init_project(
        &self,
        workdir: &Path,
        user_prompt: &str,
        now: DateTime<Local>,
    ) -> anyhow::Result<crate::project::ProjectInit>;
}

struct RealProjectInitializer;

impl ProjectInitializer for RealProjectInitializer {
    fn init_project(
        &self,
        workdir: &Path,
        user_prompt: &str,
        now: DateTime<Local>,
    ) -> anyhow::Result<crate::project::ProjectInit> {
        crate::project::init_project(workdir, user_prompt, now)
    }
}

trait ProjectRoundRunner<U> {
    fn run_round<'a>(
        &'a self,
        ui: &'a mut U,
        context: &'a crate::round_runner::PotterRoundContext,
        options: crate::round_runner::PotterRoundOptions,
    ) -> UiFuture<'a, crate::round_runner::PotterRoundResult>;
}

struct RealProjectRoundRunner;

impl<U> ProjectRoundRunner<U> for RealProjectRoundRunner
where
    U: crate::round_runner::PotterRoundUi,
{
    fn run_round<'a>(
        &'a self,
        ui: &'a mut U,
        context: &'a crate::round_runner::PotterRoundContext,
        options: crate::round_runner::PotterRoundOptions,
    ) -> UiFuture<'a, crate::round_runner::PotterRoundResult> {
        Box::pin(crate::round_runner::run_potter_round(ui, context, options))
    }
}

async fn run_project_queue_with_deps<U, C, I, R>(
    ui: &mut U,
    workdir: PathBuf,
    options: ProjectQueueOptions,
    clock: &C,
    initializer: &I,
    round_runner: &R,
) -> anyhow::Result<ProjectQueueExit>
where
    U: ProjectRunnerUi,
    C: ProjectClock,
    I: ProjectInitializer,
    R: ProjectRoundRunner<U>,
{
    let mut pending_user_prompts = crate::prompt_queue::PromptQueue::empty();

    'project: loop {
        let next_prompt = pending_user_prompts.pop_next_prompt(|| ui.pop_queued_user_prompt());

        let next_prompt = if options.allow_prompt_user {
            crate::prompt_queue::next_prompt_or_prompt_user(next_prompt, || {
                let prompt_footer = codex_tui::PromptFooterContext::new(
                    workdir.clone(),
                    crate::project::resolve_git_branch(&workdir),
                );
                ui.prompt_user(prompt_footer)
            })
            .await?
        } else {
            next_prompt.map(crate::prompt_queue::NextPrompt::FromQueue)
        };

        let Some(next_prompt) = next_prompt else {
            break 'project;
        };

        let user_prompt = match next_prompt {
            crate::prompt_queue::NextPrompt::FromQueue(prompt) => prompt,
            crate::prompt_queue::NextPrompt::FromUser(prompt) => {
                // Clear prompt UI remnants before doing any work / streaming output.
                ui.clear()?;
                prompt
            }
        };

        let init = initializer
            .init_project(&workdir, &user_prompt, clock.now_datetime())
            .context("initialize .codexpotter project")?;
        let project_started_at = clock.now_instant();
        let project_dir = init
            .progress_file_rel
            .parent()
            .context("derive CodexPotter project dir from progress file path")?
            .to_path_buf();
        let project_dir_abs = workdir.join(&project_dir);
        let potter_rollout_path = crate::potter_rollout::potter_rollout_path(&project_dir_abs);
        let user_prompt_file = init.progress_file_rel.clone();
        let developer_prompt = crate::project::render_developer_prompt(&init.progress_file_rel);

        let round_context = crate::round_runner::PotterRoundContext {
            codex_bin: options.codex_bin.clone(),
            developer_prompt: developer_prompt.clone(),
            backend_launch: options.backend_launch,
            backend_event_mode: crate::app_server_backend::AppServerEventMode::Interactive,
            codex_compat_home: options.codex_compat_home.clone(),
            thread_cwd: Some(workdir.clone()),
            turn_prompt: options.turn_prompt.clone(),
            workdir: workdir.clone(),
            progress_file_rel: init.progress_file_rel.clone(),
            user_prompt_file: user_prompt_file.clone(),
            git_commit_start: init.git_commit_start.clone(),
            potter_rollout_path: potter_rollout_path.clone(),
            project_started_at,
        };

        for round_index in 0..options.rounds.get() {
            let total_rounds = u32::try_from(options.rounds.get()).unwrap_or(u32::MAX);
            let current_round = u32::try_from(round_index.saturating_add(1)).unwrap_or(u32::MAX);
            let project_started = if round_index == 0 {
                Some(crate::round_runner::PotterProjectStartedInfo {
                    user_message: Some(user_prompt.clone()),
                    working_dir: workdir.clone(),
                    project_dir: project_dir.clone(),
                    user_prompt_file: user_prompt_file.clone(),
                })
            } else {
                None
            };

            let round_result = round_runner
                .run_round(
                    ui,
                    &round_context,
                    crate::round_runner::PotterRoundOptions {
                        pad_before_first_cell: round_index != 0,
                        project_started,
                        round_current: current_round,
                        round_total: total_rounds,
                        project_succeeded_rounds: current_round,
                    },
                )
                .await?;

            match &round_result.exit_reason {
                ExitReason::UserRequested => {
                    return Ok(ProjectQueueExit::UserRequestedExit { project_dir });
                }
                ExitReason::TaskFailed(_) => break,
                ExitReason::Fatal(_) => return Ok(ProjectQueueExit::FatalExitRequested),
                ExitReason::Completed => {}
            }
            if round_result.stop_due_to_finite_incantatem {
                break;
            }
        }
    }

    Ok(ProjectQueueExit::Completed)
}

#[cfg(test)]
mod tests {
    use super::*;

    use pretty_assertions::assert_eq;
    use std::collections::VecDeque;

    #[derive(Debug, Default)]
    struct MockUi {
        queued_prompts: VecDeque<String>,
        prompt_user_calls: usize,
        clear_calls: usize,
    }

    impl MockUi {
        fn new(queued_prompts: Vec<String>) -> Self {
            Self {
                queued_prompts: VecDeque::from(queued_prompts),
                prompt_user_calls: 0,
                clear_calls: 0,
            }
        }
    }

    impl ProjectRunnerUi for MockUi {
        fn clear(&mut self) -> anyhow::Result<()> {
            self.clear_calls += 1;
            Ok(())
        }

        fn prompt_user<'a>(
            &'a mut self,
            _prompt_footer: codex_tui::PromptFooterContext,
        ) -> UiFuture<'a, Option<String>> {
            self.prompt_user_calls += 1;
            Box::pin(async { Ok(None) })
        }

        fn pop_queued_user_prompt(&mut self) -> Option<String> {
            self.queued_prompts.pop_front()
        }
    }

    struct TestClock;

    impl ProjectClock for TestClock {
        fn now_datetime(&self) -> DateTime<Local> {
            Local::now()
        }

        fn now_instant(&self) -> Instant {
            Instant::now()
        }
    }

    #[derive(Debug, Default)]
    struct CapturingInitializer {
        prompts: std::sync::Mutex<Vec<String>>,
    }

    impl CapturingInitializer {
        fn prompts(&self) -> Vec<String> {
            self.prompts.lock().expect("lock").clone()
        }
    }

    impl ProjectInitializer for CapturingInitializer {
        fn init_project(
            &self,
            _workdir: &Path,
            user_prompt: &str,
            _now: DateTime<Local>,
        ) -> anyhow::Result<crate::project::ProjectInit> {
            let mut prompts = self.prompts.lock().expect("lock");
            prompts.push(user_prompt.to_string());
            let idx = prompts.len();
            Ok(crate::project::ProjectInit {
                progress_file_rel: PathBuf::from(format!(
                    ".codexpotter/projects/2026/02/01/{idx}/MAIN.md"
                )),
                git_commit_start: String::new(),
            })
        }
    }

    #[derive(Debug, Default)]
    struct NoopRoundRunner;

    impl ProjectRoundRunner<MockUi> for NoopRoundRunner {
        fn run_round<'a>(
            &'a self,
            _ui: &'a mut MockUi,
            _context: &'a crate::round_runner::PotterRoundContext,
            _options: crate::round_runner::PotterRoundOptions,
        ) -> UiFuture<'a, crate::round_runner::PotterRoundResult> {
            Box::pin(async {
                Ok(crate::round_runner::PotterRoundResult {
                    exit_reason: ExitReason::Completed,
                    stop_due_to_finite_incantatem: false,
                })
            })
        }
    }

    #[tokio::test]
    async fn drains_queued_prompts_without_prompting_user() {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut ui = MockUi::new(vec![String::from("one"), String::from("two")]);
        let initializer = CapturingInitializer::default();
        let clock = TestClock;
        let round_runner = NoopRoundRunner;

        let exit = run_project_queue_with_deps(
            &mut ui,
            temp.path().to_path_buf(),
            ProjectQueueOptions {
                allow_prompt_user: false,
                codex_bin: String::from("codex"),
                backend_launch: crate::app_server_backend::AppServerLaunchConfig {
                    spawn_sandbox: None,
                    thread_sandbox: None,
                    bypass_approvals_and_sandbox: false,
                },
                codex_compat_home: None,
                rounds: NonZeroUsize::new(1).expect("rounds"),
                turn_prompt: String::from("Continue"),
            },
            &clock,
            &initializer,
            &round_runner,
        )
        .await
        .expect("run project queue");

        assert_eq!(exit, ProjectQueueExit::Completed);
        assert_eq!(
            initializer.prompts(),
            vec![String::from("one"), String::from("two")]
        );
        assert_eq!(ui.prompt_user_calls, 0);
        assert_eq!(ui.clear_calls, 0);
        assert_eq!(ui.queued_prompts, VecDeque::<String>::new());
    }
}
