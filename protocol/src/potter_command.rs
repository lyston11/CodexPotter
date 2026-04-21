use std::ffi::OsStr;
use std::path::Path;

/// Render a shell-safe `codex-potter resume ...` command string.
///
/// This keeps argument ordering aligned across CodexPotter's resume hints:
/// `codex-potter resume <global_args...> <project_path>`.
pub fn render_potter_resume_command(project_path: &str, global_args: &[String]) -> String {
    let mut args: Vec<&str> = Vec::with_capacity(2 + global_args.len() + 1);
    args.push("codex-potter");
    args.push("resume");
    args.extend(global_args.iter().map(String::as_str));
    args.push(project_path);

    shlex::try_join(args.iter().copied()).unwrap_or_else(|err| {
        format!("codex-potter resume <unable to quote args: {err}> {project_path}")
    })
}

/// Derive the project-path argument for `codex-potter resume` when a path points somewhere under
/// `.codexpotter/projects/...`.
///
/// Returns the stable suffix component (e.g. `2026/02/01/11`) so resume hints remain portable
/// across machines and checkouts.
pub fn derive_potter_resume_project_path(project_path: &Path) -> Option<String> {
    let project_dir = if project_path.file_name() == Some(OsStr::new("MAIN.md")) {
        project_path.parent().unwrap_or(project_path)
    } else {
        project_path
    };

    let parts = project_dir
        .components()
        .filter_map(|component| match component {
            std::path::Component::Normal(part) => Some(part.to_string_lossy().to_string()),
            _ => None,
        })
        .collect::<Vec<_>>();

    let codexpotter_idx = parts.iter().rposition(|part| part == ".codexpotter")?;
    if parts.get(codexpotter_idx + 1).map(String::as_str) != Some("projects") {
        return None;
    }

    let remainder = parts.get((codexpotter_idx + 2)..)?;
    if remainder.is_empty() {
        return None;
    }
    Some(remainder.join("/"))
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use std::path::Path;

    use super::*;

    #[test]
    fn derive_potter_resume_project_path_strips_main_md() {
        assert_eq!(
            derive_potter_resume_project_path(Path::new(
                ".codexpotter/projects/2026/02/01/11/MAIN.md"
            )),
            Some("2026/02/01/11".to_string())
        );
    }

    #[test]
    fn derive_potter_resume_project_path_strips_absolute_prefix() {
        assert_eq!(
            derive_potter_resume_project_path(Path::new(
                "/tmp/work/.codexpotter/projects/2026/02/01/11/MAIN.md"
            )),
            Some("2026/02/01/11".to_string())
        );
    }

    #[test]
    fn render_potter_resume_command_orders_global_args_before_project_path() {
        let command = render_potter_resume_command(
            "2026/02/01/11",
            &[
                "--yolo".to_string(),
                "--sandbox".to_string(),
                "read-only".to_string(),
            ],
        );

        assert_eq!(
            command,
            "codex-potter resume --yolo --sandbox read-only 2026/02/01/11"
        );
    }

    #[test]
    fn render_potter_resume_command_quotes_args_with_spaces() {
        let command = render_potter_resume_command(
            "2026/02/01/11",
            &[
                "--codex-bin".to_string(),
                "/tmp/codex bin".to_string(),
                "--config".to_string(),
                "/tmp/with spaces/config.toml".to_string(),
            ],
        );

        assert_eq!(
            command,
            "codex-potter resume --codex-bin '/tmp/codex bin' --config '/tmp/with spaces/config.toml' 2026/02/01/11"
        );
    }
}
