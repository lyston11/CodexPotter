//! Projects list overlay detail extraction.
//!
//! This module parses a project's `potter-rollout.jsonl` and referenced upstream rollout JSONL
//! files to build the right-pane content for the projects list overlay.

use std::io::BufRead as _;
use std::path::Path;

use anyhow::Context;
use chrono::DateTime;
use codex_protocol::protocol::PotterProjectDetails;
use codex_protocol::protocol::PotterProjectRoundSummary;

pub fn build_project_details_for_overlay(
    workdir: &Path,
    project_dir: &Path,
) -> PotterProjectDetails {
    match build_project_details_for_overlay_inner(workdir, project_dir) {
        Ok(details) => details,
        Err(err) => PotterProjectDetails {
            project_dir: project_dir.to_path_buf(),
            progress_file: project_dir.join("MAIN.md"),
            git_branch: None,
            user_message: None,
            rounds: Vec::new(),
            error: Some(format!("{err:#}")),
        },
    }
}

fn build_project_details_for_overlay_inner(
    workdir: &Path,
    project_dir: &Path,
) -> anyhow::Result<PotterProjectDetails> {
    let project_dir_abs = if project_dir.is_absolute() {
        project_dir.to_path_buf()
    } else {
        workdir.join(project_dir)
    };
    let progress_file = project_dir.join("MAIN.md");
    let progress_file_abs = project_dir_abs.join("MAIN.md");
    anyhow::ensure!(
        progress_file_abs.is_file(),
        "progress file missing: {}",
        progress_file_abs.display()
    );
    let git_branch = crate::workflow::project::progress_file_git_branch(&progress_file_abs)
        .context("read git_branch from progress file")?;

    let potter_rollout_path = crate::workflow::rollout::potter_rollout_path(&project_dir_abs);
    let potter_lines = crate::workflow::rollout::read_lines(&potter_rollout_path)
        .with_context(|| format!("read {}", potter_rollout_path.display()))?;
    let index = crate::workflow::rollout_resume_index::build_resume_index(&potter_lines)
        .with_context(|| format!("parse {}", potter_rollout_path.display()))?;

    let user_message = index.project_started.user_message.clone();
    let mut rounds = Vec::new();

    for round in index.completed_rounds {
        let (final_message_unix_secs, final_message) = match round
            .configured
            .as_ref()
            .map(|cfg| cfg.rollout_path.clone())
        {
            Some(rollout_path) => {
                let abs = crate::workflow::replay_session_config::resolve_rollout_path_for_replay(
                    workdir,
                    &rollout_path,
                );
                read_final_agent_message_from_rollout(&abs).unwrap_or((None, None))
            }
            None => (None, None),
        };

        rounds.push(PotterProjectRoundSummary {
            round_current: round.round_current,
            round_total: round.round_total,
            final_message_unix_secs,
            final_message,
        });
    }

    if let Some(unfinished) = index.unfinished_round {
        let abs = crate::workflow::replay_session_config::resolve_rollout_path_for_replay(
            workdir,
            &unfinished.rollout_path,
        );
        let (final_message_unix_secs, final_message) =
            read_final_agent_message_from_rollout(&abs).unwrap_or((None, None));
        rounds.push(PotterProjectRoundSummary {
            round_current: unfinished.round_current,
            round_total: unfinished.round_total,
            final_message_unix_secs,
            final_message,
        });
    }

    Ok(PotterProjectDetails {
        project_dir: project_dir.to_path_buf(),
        progress_file,
        git_branch,
        user_message,
        rounds,
        error: None,
    })
}

fn read_final_agent_message_from_rollout(
    rollout_path: &Path,
) -> anyhow::Result<(Option<u64>, Option<String>)> {
    let file = std::fs::File::open(rollout_path)
        .with_context(|| format!("open rollout {}", rollout_path.display()))?;
    let reader = std::io::BufReader::new(file);

    let mut last_without_phase: Option<(u64, String)> = None;
    let mut last_final: Option<(u64, String)> = None;
    let mut saw_explicit_phase = false;

    for (idx, line) in reader.lines().enumerate() {
        let line_number = idx + 1;
        let line = line.with_context(|| format!("read rollout line {line_number}"))?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let value: serde_json::Value = serde_json::from_str(trimmed)
            .with_context(|| format!("parse rollout json line {line_number}: {trimmed}"))?;

        let item_type = value
            .get("type")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        if item_type != "event_msg" {
            continue;
        }

        let Some(ts) = value.get("timestamp").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let parsed = DateTime::parse_from_rfc3339(ts)
            .with_context(|| format!("parse rollout timestamp {ts:?}"))?;
        let unix_secs = u64::try_from(parsed.timestamp())
            .context("convert rollout timestamp to unix seconds")?;

        let payload = value
            .get("payload")
            .context("rollout event_msg missing payload")?;
        let payload_type = payload
            .get("type")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        if payload_type != "agent_message" {
            continue;
        }

        let Some(message) = payload.get("message").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let phase = payload.get("phase").and_then(serde_json::Value::as_str);
        let message = message.to_string();

        match phase {
            Some("final_answer") => {
                saw_explicit_phase = true;
                last_final = Some((unix_secs, message));
            }
            Some(_) => {
                saw_explicit_phase = true;
            }
            None => {
                last_without_phase = Some((unix_secs, message));
            }
        }
    }

    // The overlay should show each round's conclusion, not mid-turn commentary. Modern logs mark
    // final answers explicitly; only legacy logs with no `phase` metadata fall back to the last
    // completed agent message for compatibility.
    let (secs, message) = last_final
        .or_else(|| {
            (!saw_explicit_phase)
                .then_some(last_without_phase)
                .flatten()
        })
        .unwrap_or_default();
    Ok((
        (secs != 0).then_some(secs),
        (!message.is_empty()).then_some(message),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::workflow::rollout::PotterRolloutLine;
    use codex_protocol::protocol::PotterRoundOutcome;
    use pretty_assertions::assert_eq;
    use std::path::PathBuf;

    #[test]
    fn final_agent_message_prefers_final_answer_phase() {
        let temp = tempfile::tempdir().expect("tempdir");
        let rollout_path = temp.path().join("rollout.jsonl");
        std::fs::write(
            &rollout_path,
            r#"{"timestamp":"2026-03-01T00:00:00.000Z","type":"event_msg","payload":{"type":"agent_message","message":"commentary","phase":"commentary"}}
{"timestamp":"2026-03-01T00:00:01.000Z","type":"event_msg","payload":{"type":"agent_message","message":"final","phase":"final_answer"}}
"#,
        )
        .expect("write rollout");

        let (secs, message) = read_final_agent_message_from_rollout(&rollout_path).expect("read");
        assert_eq!(secs, Some(1_772_323_201));
        assert_eq!(message.as_deref(), Some("final"));
    }

    #[test]
    fn final_agent_message_falls_back_to_last_message_when_phase_missing() {
        let temp = tempfile::tempdir().expect("tempdir");
        let rollout_path = temp.path().join("rollout.jsonl");
        std::fs::write(
            &rollout_path,
            r#"{"timestamp":"2026-03-01T00:00:00.000Z","type":"event_msg","payload":{"type":"agent_message","message":"first"}}
{"timestamp":"2026-03-01T00:00:01.000Z","type":"event_msg","payload":{"type":"agent_message","message":"second"}}
"#,
        )
        .expect("write rollout");

        let (secs, message) = read_final_agent_message_from_rollout(&rollout_path).expect("read");
        assert_eq!(secs, Some(1_772_323_201));
        assert_eq!(message.as_deref(), Some("second"));
    }

    #[test]
    fn final_agent_message_does_not_fall_back_to_commentary_phase() {
        let temp = tempfile::tempdir().expect("tempdir");
        let rollout_path = temp.path().join("rollout.jsonl");
        std::fs::write(
            &rollout_path,
            r#"{"timestamp":"2026-03-01T00:00:00.000Z","type":"event_msg","payload":{"type":"agent_message","message":"commentary","phase":"commentary"}}
"#,
        )
        .expect("write rollout");

        let (secs, message) = read_final_agent_message_from_rollout(&rollout_path).expect("read");
        assert_eq!(secs, None);
        assert_eq!(message, None);
    }

    #[test]
    fn overlay_details_include_user_task_message_from_potter_rollout() {
        let workdir = tempfile::tempdir().expect("tempdir");
        let project_dir = PathBuf::from(".codexpotter/projects/2026/04/16/1");
        let project_dir_abs = workdir.path().join(&project_dir);
        std::fs::create_dir_all(&project_dir_abs).expect("create project dir");

        let progress_file_abs = project_dir_abs.join("MAIN.md");
        std::fs::write(&progress_file_abs, "---\ngit_branch: \"main\"\n---\n").expect("write MAIN");

        let potter_rollout_path = crate::workflow::rollout::potter_rollout_path(&project_dir_abs);
        crate::workflow::rollout::append_line(
            &potter_rollout_path,
            &PotterRolloutLine::ProjectStarted {
                user_message: Some("hello task".to_string()),
                user_prompt_file: project_dir.join("MAIN.md"),
            },
        )
        .expect("append project_started");
        crate::workflow::rollout::append_line(
            &potter_rollout_path,
            &PotterRolloutLine::RoundStarted {
                current: 1,
                total: 1,
            },
        )
        .expect("append round_started");
        crate::workflow::rollout::append_line(
            &potter_rollout_path,
            &PotterRolloutLine::RoundFinished {
                outcome: PotterRoundOutcome::Interrupted,
            },
        )
        .expect("append round_finished");

        let details = build_project_details_for_overlay(workdir.path(), &project_dir);
        assert_eq!(details.error, None);
        assert_eq!(details.project_dir, project_dir);
        assert_eq!(
            details.user_message.as_deref(),
            Some("hello task"),
            "expected details to surface the original user task message"
        );
        assert_eq!(details.git_branch.as_deref(), Some("main"));
    }
}
