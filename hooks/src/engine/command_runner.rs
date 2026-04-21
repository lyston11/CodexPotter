use std::path::Path;
use std::process::Stdio;
use std::time::Duration;
use std::time::Instant;

use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::time::timeout;

use super::CommandShell;
use super::ConfiguredHandler;

#[derive(Debug)]
#[allow(dead_code)]
pub(super) struct CommandRunResult {
    pub started_at: i64,
    pub completed_at: i64,
    pub duration_ms: i64,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub error: Option<String>,
}

pub(super) async fn run_command(
    shell: &CommandShell,
    handler: &ConfiguredHandler,
    input_json: &str,
    cwd: &Path,
) -> CommandRunResult {
    let started_at = chrono::Utc::now().timestamp();
    let started = Instant::now();

    let mut command = build_command(shell, handler);
    command
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(err) => {
            return CommandRunResult {
                started_at,
                completed_at: chrono::Utc::now().timestamp(),
                duration_ms: started.elapsed().as_millis().try_into().unwrap_or(i64::MAX),
                exit_code: None,
                stdout: String::new(),
                stderr: String::new(),
                error: Some(err.to_string()),
            };
        }
    };

    if let Some(mut stdin) = child.stdin.take()
        && let Err(err) = stdin.write_all(input_json.as_bytes()).await
    {
        let _ = child.kill().await;
        return CommandRunResult {
            started_at,
            completed_at: chrono::Utc::now().timestamp(),
            duration_ms: started.elapsed().as_millis().try_into().unwrap_or(i64::MAX),
            exit_code: None,
            stdout: String::new(),
            stderr: String::new(),
            error: Some(format!("failed to write hook stdin: {err}")),
        };
    }

    let timeout_duration = Duration::from_secs(handler.timeout_sec);
    match timeout(timeout_duration, child.wait_with_output()).await {
        Ok(Ok(output)) => CommandRunResult {
            started_at,
            completed_at: chrono::Utc::now().timestamp(),
            duration_ms: started.elapsed().as_millis().try_into().unwrap_or(i64::MAX),
            exit_code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            error: None,
        },
        Ok(Err(err)) => CommandRunResult {
            started_at,
            completed_at: chrono::Utc::now().timestamp(),
            duration_ms: started.elapsed().as_millis().try_into().unwrap_or(i64::MAX),
            exit_code: None,
            stdout: String::new(),
            stderr: String::new(),
            error: Some(err.to_string()),
        },
        Err(_) => CommandRunResult {
            started_at,
            completed_at: chrono::Utc::now().timestamp(),
            duration_ms: started.elapsed().as_millis().try_into().unwrap_or(i64::MAX),
            exit_code: None,
            stdout: String::new(),
            stderr: String::new(),
            error: Some(format!("hook timed out after {}s", handler.timeout_sec)),
        },
    }
}

fn build_command(shell: &CommandShell, handler: &ConfiguredHandler) -> Command {
    let mut command = if shell.program.is_empty() {
        default_shell_command()
    } else {
        Command::new(&shell.program)
    };
    if shell.program.is_empty() {
        command.arg(&handler.command);
        command
    } else {
        command.args(&shell.args);
        command.arg(&handler.command);
        command
    }
}

fn default_shell_command() -> Command {
    #[cfg(windows)]
    {
        let comspec = std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string());
        let mut command = Command::new(comspec);
        command.arg("/C");
        command
    }

    #[cfg(not(windows))]
    {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        let mut command = Command::new(shell);
        command.arg("-lc");
        command
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use codex_protocol::protocol::HookEventName;
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    use super::run_command;
    use crate::engine::CommandShell;
    use crate::engine::ConfiguredHandler;

    #[cfg(not(windows))]
    #[tokio::test]
    async fn run_command_uses_provided_cwd_as_working_directory() {
        let temp_dir = TempDir::new().expect("tempdir");
        let cwd = temp_dir.path().canonicalize().expect("canonicalize cwd");

        let shell = CommandShell {
            program: String::new(),
            args: Vec::new(),
        };

        let handler = ConfiguredHandler {
            event_name: HookEventName::PotterProjectStop,
            matcher: None,
            command: "cat >/dev/null; pwd".to_string(),
            timeout_sec: 5,
            status_message: None,
            source_path: PathBuf::from("/tmp/hooks.json"),
            display_order: 0,
        };

        let result = run_command(&shell, &handler, "{}", cwd.as_path()).await;
        assert_eq!(result.error, None);
        assert_eq!(result.exit_code, Some(0));

        let reported = PathBuf::from(result.stdout.trim())
            .canonicalize()
            .expect("canonicalize reported cwd");
        assert_eq!(reported, cwd);
    }
}
