//! Potter xmodel schedule and completion gates.
//!
//! When xmodel is enabled (either via runtime `--xmodel` or persisted `potter.xmodel`), CodexPotter
//! runs early rounds on GPT 5.2 xhigh and later rounds on GPT 5.5 xhigh. It also ensures at least
//! one GPT 5.5 round is executed before treating a project as succeeded.

use crate::app_server::UpstreamCodexCliArgs;

pub const POTTER_XMODEL_REASONING_EFFORT: &str = "xhigh";
pub const POTTER_XMODEL_REVIEW_MODEL: &str = "gpt-5.5";
pub const POTTER_XMODEL_GPT_5_2_MODEL: &str = "gpt-5.2";

/// Apply xmodel per-round model overrides to the upstream codex CLI args.
///
/// Schedule:
/// - Round 1~3: GPT 5.2 xhigh
/// - Round 4+: GPT 5.5 xhigh
/// - `force_review_model` pins to GPT 5.5 xhigh regardless of round number.
pub fn apply_potter_xmodel_overrides(
    upstream_cli_args: &mut UpstreamCodexCliArgs,
    round_current: u32,
    force_review_model: bool,
) {
    let model = if force_review_model || round_current >= 4 {
        POTTER_XMODEL_REVIEW_MODEL
    } else {
        POTTER_XMODEL_GPT_5_2_MODEL
    };

    upstream_cli_args.model = Some(model.to_string());
    upstream_cli_args.config_overrides.push(format!(
        "model_reasoning_effort=\"{POTTER_XMODEL_REASONING_EFFORT}\""
    ));
}

/// Whether a completed round should result in a project-succeeded marker when `finite_incantatem`
/// is set.
pub fn should_emit_project_succeeded(xmodel_enabled: bool, session_model: Option<&str>) -> bool {
    !xmodel_enabled || session_model == Some(POTTER_XMODEL_REVIEW_MODEL)
}

/// Whether xmodel should ignore a `finite_incantatem: true` stop signal for this round.
///
/// This is the inverse of [`should_emit_project_succeeded`]: when xmodel is enabled, we only allow
/// success to finalize on a GPT 5.5 round to ensure cross-model review runs at least once.
pub fn should_ignore_finite_incantatem(xmodel_enabled: bool, session_model: Option<&str>) -> bool {
    xmodel_enabled && session_model != Some(POTTER_XMODEL_REVIEW_MODEL)
}
