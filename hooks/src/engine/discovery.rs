use std::fs;
use std::path::Path;
use std::path::PathBuf;

use codex_protocol::protocol::HookEventName;

use super::ConfiguredHandler;
use super::config::HookHandlerConfig;
use super::config::HooksFile;
use super::config::MatcherGroup;
use crate::events::common::matcher_pattern_for_event;
use crate::events::common::validate_matcher_pattern;

pub(crate) struct DiscoveryResult {
    pub handlers: Vec<ConfiguredHandler>,
    pub warnings: Vec<String>,
}

pub(crate) fn discover_handlers(cwd: &Path, codex_home_dir: Option<&Path>) -> DiscoveryResult {
    let mut handlers = Vec::new();
    let mut warnings = Vec::new();
    let mut display_order = 0_i64;

    for source_path in discover_hooks_json_paths(cwd, codex_home_dir) {
        if !source_path.is_file() {
            continue;
        }

        let contents = match fs::read_to_string(&source_path) {
            Ok(contents) => contents,
            Err(err) => {
                warnings.push(format!(
                    "failed to read hooks config {}: {err}",
                    source_path.display()
                ));
                continue;
            }
        };

        let parsed: HooksFile = match serde_json::from_str(&contents) {
            Ok(parsed) => parsed,
            Err(err) => {
                warnings.push(format!(
                    "failed to parse hooks config {}: {err}",
                    source_path.display()
                ));
                continue;
            }
        };

        let super::config::HookEvents {
            potter_project_stop,
        } = parsed.hooks;
        append_matcher_groups(
            &mut handlers,
            &mut warnings,
            &mut display_order,
            &source_path,
            HookEventName::PotterProjectStop,
            potter_project_stop,
        );
    }

    DiscoveryResult { handlers, warnings }
}

fn append_group_handlers(
    handlers: &mut Vec<ConfiguredHandler>,
    warnings: &mut Vec<String>,
    display_order: &mut i64,
    source_path: &Path,
    event_name: HookEventName,
    matcher: Option<&str>,
    group_handlers: Vec<HookHandlerConfig>,
) {
    if let Some(matcher) = matcher
        && let Err(err) = validate_matcher_pattern(matcher)
    {
        warnings.push(format!(
            "invalid matcher {matcher:?} in {}: {err}",
            source_path.display()
        ));
        return;
    }

    for handler in group_handlers {
        match handler {
            HookHandlerConfig::Command {
                command,
                timeout_sec,
                r#async,
                status_message,
            } => {
                if r#async {
                    warnings.push(format!(
                        "skipping async hook in {}: async hooks are not supported yet",
                        source_path.display()
                    ));
                    continue;
                }
                if command.trim().is_empty() {
                    warnings.push(format!(
                        "skipping empty hook command in {}",
                        source_path.display()
                    ));
                    continue;
                }
                let timeout_sec = timeout_sec.unwrap_or(600).max(1);
                handlers.push(ConfiguredHandler {
                    event_name,
                    matcher: matcher.map(ToOwned::to_owned),
                    command,
                    timeout_sec,
                    status_message,
                    source_path: source_path.to_path_buf(),
                    display_order: *display_order,
                });
                *display_order += 1;
            }
            HookHandlerConfig::Prompt {} => warnings.push(format!(
                "skipping prompt hook in {}: prompt hooks are not supported yet",
                source_path.display()
            )),
            HookHandlerConfig::Agent {} => warnings.push(format!(
                "skipping agent hook in {}: agent hooks are not supported yet",
                source_path.display()
            )),
        }
    }
}

fn append_matcher_groups(
    handlers: &mut Vec<ConfiguredHandler>,
    warnings: &mut Vec<String>,
    display_order: &mut i64,
    source_path: &Path,
    event_name: HookEventName,
    groups: Vec<MatcherGroup>,
) {
    for group in groups {
        append_group_handlers(
            handlers,
            warnings,
            display_order,
            source_path,
            event_name,
            matcher_pattern_for_event(event_name, group.matcher.as_deref()),
            group.hooks,
        );
    }
}

fn discover_hooks_json_paths(cwd: &Path, codex_home_dir_override: Option<&Path>) -> Vec<PathBuf> {
    let mut paths = Vec::new();

    if let Some(dir) = codex_home_dir_override {
        paths.push(dir.join("hooks.json"));
    } else if let Some(path) = codex_home_dir().map(|dir| dir.join("hooks.json")) {
        paths.push(path);
    }

    let repo_root = find_repo_root(cwd);
    paths.push(repo_root.join(".codex").join("hooks.json"));

    paths
}

fn codex_home_dir() -> Option<PathBuf> {
    let home_dir = dirs::home_dir()?;
    let codex_home_env = std::env::var("CODEX_HOME").ok();
    Some(match codex_home_env.filter(|value| !value.is_empty()) {
        Some(val) => expand_home_relative_path(&val, &home_dir),
        None => home_dir.join(".codex"),
    })
}

fn expand_home_relative_path(path_text: &str, home_dir: &Path) -> PathBuf {
    if path_text == "~" {
        return home_dir.to_path_buf();
    }

    let rest = path_text.strip_prefix("~/").or_else(|| {
        if cfg!(windows) {
            path_text.strip_prefix("~\\")
        } else {
            None
        }
    });
    let Some(rest) = rest else {
        return PathBuf::from(path_text);
    };

    home_dir.join(rest)
}

fn find_repo_root(cwd: &Path) -> PathBuf {
    for ancestor in cwd.ancestors() {
        if ancestor.join(".git").exists() {
            return ancestor.to_path_buf();
        }
    }

    cwd.to_path_buf()
}

#[cfg(test)]
mod tests {
    use crate::events::common::matcher_pattern_for_event;

    use codex_protocol::protocol::HookEventName;
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    use super::ConfiguredHandler;
    use super::HookHandlerConfig;
    use super::append_group_handlers;

    #[test]
    fn potter_project_stop_ignores_invalid_matcher_during_discovery() {
        let mut handlers = Vec::new();
        let mut warnings = Vec::new();
        let mut display_order = 0;

        append_group_handlers(
            &mut handlers,
            &mut warnings,
            &mut display_order,
            std::path::Path::new("/tmp/hooks.json"),
            HookEventName::PotterProjectStop,
            matcher_pattern_for_event(HookEventName::PotterProjectStop, Some("[")),
            vec![HookHandlerConfig::Command {
                command: "echo hello".to_string(),
                timeout_sec: None,
                r#async: false,
                status_message: None,
            }],
        );

        // ProjectStop does not support matchers, so any matcher input is ignored.
        assert_eq!(warnings, Vec::<String>::new());
        assert_eq!(
            handlers,
            vec![ConfiguredHandler {
                event_name: HookEventName::PotterProjectStop,
                matcher: None,
                command: "echo hello".to_string(),
                timeout_sec: 600,
                status_message: None,
                source_path: std::path::PathBuf::from("/tmp/hooks.json"),
                display_order: 0,
            }]
        );
    }

    #[test]
    fn discovery_reads_hooks_json_from_repo_root_dot_codex() {
        let temp = TempDir::new().expect("tempdir");
        let repo = temp.path().join("repo");
        let codex_home = temp.path().join("codex-home");
        std::fs::create_dir_all(&codex_home).expect("create codex home");
        std::fs::create_dir_all(repo.join(".git")).expect("create .git");
        std::fs::create_dir_all(repo.join(".codex")).expect("create .codex");
        std::fs::write(
            repo.join(".codex").join("hooks.json"),
            r#"{"hooks":{"Potter.ProjectStop":[{"hooks":[{"type":"command","command":"echo hello"}]}]}}"#,
        )
        .expect("write hooks.json");

        let discovered = super::discover_handlers(&repo, Some(codex_home.as_path()));
        assert_eq!(discovered.warnings, Vec::<String>::new());
        assert_eq!(discovered.handlers.len(), 1);
        assert_eq!(
            discovered.handlers[0].event_name,
            HookEventName::PotterProjectStop
        );
    }

    #[test]
    fn discovery_resolves_repo_root_from_subdir_and_parses_timeout_sec_alias_and_status_message() {
        let temp = TempDir::new().expect("tempdir");
        let repo = temp.path().join("repo");
        let codex_home = temp.path().join("codex-home");
        std::fs::create_dir_all(&codex_home).expect("create codex home");
        std::fs::create_dir_all(repo.join(".git")).expect("create .git");
        std::fs::create_dir_all(repo.join(".codex")).expect("create .codex");
        std::fs::create_dir_all(repo.join("subdir")).expect("create subdir");

        std::fs::write(
            repo.join(".codex").join("hooks.json"),
            r#"{"hooks":{"Potter.ProjectStop":[{"hooks":[{"type":"command","command":"echo hello","timeoutSec":42,"statusMessage":"Setting up environment"}]}]}}"#,
        )
        .expect("write hooks.json");

        let discovered = super::discover_handlers(&repo.join("subdir"), Some(codex_home.as_path()));
        assert_eq!(discovered.warnings, Vec::<String>::new());
        assert_eq!(
            discovered.handlers,
            vec![ConfiguredHandler {
                event_name: HookEventName::PotterProjectStop,
                matcher: None,
                command: "echo hello".to_string(),
                timeout_sec: 42,
                status_message: Some("Setting up environment".to_string()),
                source_path: repo.join(".codex").join("hooks.json"),
                display_order: 0,
            }]
        );
    }
}
