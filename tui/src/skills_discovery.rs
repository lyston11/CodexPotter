use serde::Deserialize;
use std::collections::HashSet;
use std::collections::VecDeque;
use std::path::Path;
use std::path::PathBuf;
use toml_edit::DocumentMut;

const SKILL_FILENAME: &str = "SKILL.md";
const CONFIG_TOML_FILENAME: &str = "config.toml";
const PROJECT_ROOT_MARKERS_KEY: &str = "project_root_markers";
const AGENTS_DIR_NAME: &str = ".agents";
const SKILLS_DIR_NAME: &str = "skills";
const SYSTEM_SKILLS_DIR_NAME: &str = ".system";
const MAX_SCAN_DEPTH: usize = 6;
const MAX_SKILLS_DIRS_PER_ROOT: usize = 2000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SkillScope {
    Repo,
    User,
    System,
    Admin,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkillMetadata {
    pub name: String,
    pub description: String,
    pub short_description: Option<String>,
    pub interface: Option<SkillInterface>,
    pub path: PathBuf,
    pub scope: SkillScope,
}

impl SkillMetadata {
    pub fn display_name(&self) -> &str {
        self.interface
            .as_ref()
            .and_then(|interface| interface.display_name.as_deref())
            .unwrap_or(&self.name)
    }

    pub fn display_description(&self) -> &str {
        self.interface
            .as_ref()
            .and_then(|interface| interface.short_description.as_deref())
            .or(self.short_description.as_deref())
            .unwrap_or(&self.description)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct SkillInterface {
    pub display_name: Option<String>,
    pub short_description: Option<String>,
}

pub fn load_skills(cwd: &Path) -> Vec<SkillMetadata> {
    load_skills_from_roots(skill_roots(cwd))
}

fn load_skills_from_roots(roots: Vec<SkillRoot>) -> Vec<SkillMetadata> {
    let mut out = Vec::new();
    for root in roots {
        discover_skills_under_root(&root, &mut out);
    }

    let mut seen_paths = HashSet::<PathBuf>::new();
    out.retain(|skill| seen_paths.insert(skill.path.clone()));

    fn scope_rank(scope: SkillScope) -> u8 {
        match scope {
            SkillScope::Repo => 0,
            SkillScope::User => 1,
            SkillScope::System => 2,
            SkillScope::Admin => 3,
        }
    }

    out.sort_by(|a, b| {
        scope_rank(a.scope)
            .cmp(&scope_rank(b.scope))
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.path.cmp(&b.path))
    });

    out
}

#[derive(Clone, Debug)]
struct SkillRoot {
    path: PathBuf,
    scope: SkillScope,
    follow_symlinks: bool,
}

fn skill_roots(cwd: &Path) -> Vec<SkillRoot> {
    let home_dir = dirs::home_dir();
    let codex_home = codex_home_from_env_or_home(home_dir.as_deref());
    let project_root_markers = project_root_markers_from_configs(codex_home.as_deref());
    let mut roots = skill_roots_with_dirs(
        cwd,
        codex_home.as_deref(),
        home_dir.as_deref(),
        &project_root_markers,
    );
    if let Some(admin_skills_dir) = system_admin_skills_dir() {
        roots.push(SkillRoot {
            path: admin_skills_dir,
            scope: SkillScope::Admin,
            follow_symlinks: true,
        });
    }

    roots
}

fn codex_home_from_env_or_home(home_dir: Option<&Path>) -> Option<PathBuf> {
    if let Ok(val) = std::env::var("CODEX_HOME")
        && !val.is_empty()
    {
        return Some(PathBuf::from(val));
    }
    home_dir.map(|home| home.join(".codex"))
}

fn skill_roots_with_dirs(
    cwd: &Path,
    codex_home: Option<&Path>,
    home_dir: Option<&Path>,
    project_root_markers: &[String],
) -> Vec<SkillRoot> {
    let mut roots = Vec::new();

    let project_root = find_project_root(cwd, project_root_markers);
    let repo_dirs = repo_dirs_between_project_root_and_cwd(cwd, &project_root);

    for dir in &repo_dirs {
        let skills_dir = dir.join(".codex").join(SKILLS_DIR_NAME);
        if skills_dir.is_dir() {
            roots.push(SkillRoot {
                path: skills_dir,
                scope: SkillScope::Repo,
                follow_symlinks: true,
            });
        }
    }

    if let Some(codex_home) = codex_home {
        let user_skills = codex_home.join(SKILLS_DIR_NAME);
        roots.push(SkillRoot {
            path: user_skills.join(SYSTEM_SKILLS_DIR_NAME),
            scope: SkillScope::System,
            follow_symlinks: false,
        });
        roots.push(SkillRoot {
            path: user_skills,
            scope: SkillScope::User,
            follow_symlinks: true,
        });
    }

    // `$HOME/.agents/skills` (user-installed skills).
    if let Some(home_dir) = home_dir {
        roots.push(SkillRoot {
            path: home_dir.join(AGENTS_DIR_NAME).join(SKILLS_DIR_NAME),
            scope: SkillScope::User,
            follow_symlinks: true,
        });
    }

    // `dir/.agents/skills` (repo-installed skills), for directories between repo root and `cwd`.
    // This mirrors upstream codex's `repo_agents_skill_roots`.
    for &dir in repo_dirs.iter().rev() {
        let agents_skills = dir.join(AGENTS_DIR_NAME).join(SKILLS_DIR_NAME);
        if agents_skills.is_dir() {
            roots.push(SkillRoot {
                path: agents_skills,
                scope: SkillScope::Repo,
                follow_symlinks: true,
            });
        }
    }

    roots
}

const DEFAULT_PROJECT_ROOT_MARKERS: &[&str] = &[".git"];

fn default_project_root_markers() -> Vec<String> {
    DEFAULT_PROJECT_ROOT_MARKERS
        .iter()
        .map(ToString::to_string)
        .collect()
}

enum ProjectRootMarkersValue {
    Absent,
    Valid(Vec<String>),
    Invalid(String),
}

fn project_root_markers_from_configs(codex_home: Option<&Path>) -> Vec<String> {
    project_root_markers_from_config_paths(system_config_toml_path().as_deref(), codex_home)
}

fn project_root_markers_from_config_paths(
    system_config_toml_path: Option<&Path>,
    codex_home: Option<&Path>,
) -> Vec<String> {
    let mut value: Option<Vec<String>> = None;
    let mut invalid: Option<String> = None;

    fn apply_layer(
        layer_name: &str,
        path: &Path,
        value: &mut Option<Vec<String>>,
        invalid: &mut Option<String>,
    ) {
        match read_project_root_markers_from_config_file(path) {
            ProjectRootMarkersValue::Absent => {}
            ProjectRootMarkersValue::Valid(markers) => {
                *value = Some(markers);
                *invalid = None;
            }
            ProjectRootMarkersValue::Invalid(reason) => {
                *value = None;
                *invalid = Some(format!("{layer_name} config ({}) {reason}", path.display()));
            }
        }
    }

    if let Some(system_config_toml_path) = system_config_toml_path {
        apply_layer("system", system_config_toml_path, &mut value, &mut invalid);
    }
    if let Some(codex_home) = codex_home {
        apply_layer(
            "user",
            &codex_home.join(CONFIG_TOML_FILENAME),
            &mut value,
            &mut invalid,
        );
    }

    if let Some(reason) = invalid {
        tracing::warn!("invalid {PROJECT_ROOT_MARKERS_KEY}: {reason}");
        return default_project_root_markers();
    }

    value.unwrap_or_else(default_project_root_markers)
}

fn system_config_toml_path() -> Option<PathBuf> {
    crate::codex_system_paths::system_codex_dir().map(|dir| dir.join(CONFIG_TOML_FILENAME))
}

fn system_admin_skills_dir() -> Option<PathBuf> {
    crate::codex_system_paths::system_codex_dir().map(|dir| dir.join(SKILLS_DIR_NAME))
}

fn read_project_root_markers_from_config_file(path: &Path) -> ProjectRootMarkersValue {
    let contents = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return ProjectRootMarkersValue::Absent;
        }
        Err(err) => {
            return ProjectRootMarkersValue::Invalid(format!("failed to read: {err}"));
        }
    };

    if contents.trim().is_empty() {
        return ProjectRootMarkersValue::Absent;
    }

    let doc = match contents.parse::<DocumentMut>() {
        Ok(doc) => doc,
        Err(err) => {
            return ProjectRootMarkersValue::Invalid(format!("failed to parse TOML: {err}"));
        }
    };

    let Some(item) = doc.get(PROJECT_ROOT_MARKERS_KEY) else {
        return ProjectRootMarkersValue::Absent;
    };

    let Some(arr) = item.as_array() else {
        return ProjectRootMarkersValue::Invalid("must be an array of strings".to_string());
    };

    let mut markers = Vec::new();
    for entry in arr.iter() {
        let Some(marker) = entry.as_str() else {
            return ProjectRootMarkersValue::Invalid("must be an array of strings".to_string());
        };
        markers.push(marker.to_string());
    }

    ProjectRootMarkersValue::Valid(markers)
}

fn find_project_root(cwd: &Path, project_root_markers: &[String]) -> PathBuf {
    if project_root_markers.is_empty() {
        return cwd.to_path_buf();
    }

    for ancestor in cwd.ancestors() {
        for marker in project_root_markers {
            if ancestor.join(marker).exists() {
                return ancestor.to_path_buf();
            }
        }
    }

    cwd.to_path_buf()
}

fn repo_dirs_between_project_root_and_cwd<'a>(cwd: &'a Path, project_root: &Path) -> Vec<&'a Path> {
    // Highest precedence first (closest to cwd).
    cwd.ancestors()
        .scan(false, |done, ancestor| {
            if *done {
                None
            } else {
                if ancestor == project_root {
                    *done = true;
                }
                Some(ancestor)
            }
        })
        .collect::<Vec<_>>()
}

fn discover_skills_under_root(root: &SkillRoot, out: &mut Vec<SkillMetadata>) {
    let Ok(root_dir) = std::fs::canonicalize(&root.path) else {
        return;
    };
    if !root_dir.is_dir() {
        return;
    }

    fn enqueue_dir(
        queue: &mut VecDeque<(PathBuf, usize)>,
        visited: &mut HashSet<PathBuf>,
        truncated_by_dir_limit: &mut bool,
        path: PathBuf,
        depth: usize,
    ) {
        if depth > MAX_SCAN_DEPTH {
            return;
        }
        if visited.len() >= MAX_SKILLS_DIRS_PER_ROOT {
            *truncated_by_dir_limit = true;
            return;
        }
        if visited.insert(path.clone()) {
            queue.push_back((path, depth));
        }
    }

    let mut visited_dirs = HashSet::<PathBuf>::new();
    visited_dirs.insert(root_dir.clone());

    let mut queue = VecDeque::from([(root_dir.clone(), 0)]);
    let mut truncated_by_dir_limit = false;

    while let Some((dir, depth)) = queue.pop_front() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(err) => {
                tracing::warn!("failed to read skills dir {}: {err}", dir.display());
                continue;
            }
        };

        for entry in entries.flatten() {
            let path = entry.path();
            let file_name = match path.file_name().and_then(|name| name.to_str()) {
                Some(name) => name,
                None => continue,
            };

            if file_name.starts_with('.') {
                continue;
            }

            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(_) => continue,
            };

            if file_type.is_symlink() {
                if !root.follow_symlinks {
                    continue;
                }

                let metadata = match std::fs::metadata(&path) {
                    Ok(metadata) => metadata,
                    Err(err) => {
                        tracing::warn!(
                            "failed to stat skills entry {} (symlink): {err}",
                            path.display()
                        );
                        continue;
                    }
                };

                if metadata.is_dir() {
                    let Ok(resolved_dir) = std::fs::canonicalize(&path) else {
                        continue;
                    };
                    enqueue_dir(
                        &mut queue,
                        &mut visited_dirs,
                        &mut truncated_by_dir_limit,
                        resolved_dir,
                        depth + 1,
                    );
                }

                continue;
            }

            if file_type.is_dir() {
                let Ok(resolved_dir) = std::fs::canonicalize(&path) else {
                    continue;
                };
                enqueue_dir(
                    &mut queue,
                    &mut visited_dirs,
                    &mut truncated_by_dir_limit,
                    resolved_dir,
                    depth + 1,
                );
                continue;
            }

            if file_type.is_file() && file_name == SKILL_FILENAME {
                match parse_skill_file(&path, root.scope) {
                    Ok(skill) => out.push(skill),
                    Err(err) => {
                        tracing::warn!("failed to parse {}: {err}", path.display());
                    }
                }
            }
        }
    }

    if truncated_by_dir_limit {
        tracing::warn!(
            "skills scan truncated after {MAX_SKILLS_DIRS_PER_ROOT} directories (root: {})",
            root_dir.display()
        );
    }
}

#[derive(Debug)]
enum SkillParseError {
    Read(std::io::Error),
    MissingFrontmatter,
    InvalidYaml(serde_yaml::Error),
    MissingField(&'static str),
}

impl std::fmt::Display for SkillParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SkillParseError::Read(err) => write!(f, "failed to read file: {err}"),
            SkillParseError::MissingFrontmatter => {
                write!(f, "missing YAML frontmatter delimited by ---")
            }
            SkillParseError::InvalidYaml(err) => write!(f, "invalid YAML: {err}"),
            SkillParseError::MissingField(field) => write!(f, "missing field `{field}`"),
        }
    }
}

impl std::error::Error for SkillParseError {}

#[derive(Debug, Deserialize)]
struct SkillFrontmatter {
    name: String,
    description: String,
    #[serde(default)]
    metadata: SkillFrontmatterMetadata,
}

#[derive(Debug, Default, Deserialize)]
struct SkillFrontmatterMetadata {
    #[serde(default, rename = "short-description")]
    short_description: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct SkillMetadataFile {
    #[serde(default)]
    interface: Option<SkillInterfaceFile>,
}

#[derive(Debug, Default, Deserialize)]
struct SkillInterfaceFile {
    display_name: Option<String>,
    short_description: Option<String>,
}

fn parse_skill_file(path: &Path, scope: SkillScope) -> Result<SkillMetadata, SkillParseError> {
    let contents = std::fs::read_to_string(path).map_err(SkillParseError::Read)?;
    let frontmatter = extract_frontmatter(&contents).ok_or(SkillParseError::MissingFrontmatter)?;

    let parsed: SkillFrontmatter =
        serde_yaml::from_str(&frontmatter).map_err(SkillParseError::InvalidYaml)?;

    if parsed.name.trim().is_empty() {
        return Err(SkillParseError::MissingField("name"));
    }
    if parsed.description.trim().is_empty() {
        return Err(SkillParseError::MissingField("description"));
    }

    let resolved_path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let interface = load_skill_interface(path);

    Ok(SkillMetadata {
        name: parsed.name.trim().to_string(),
        description: parsed.description.trim().to_string(),
        short_description: parsed
            .metadata
            .short_description
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        interface,
        path: resolved_path,
        scope,
    })
}

fn extract_frontmatter(contents: &str) -> Option<String> {
    let mut lines = contents.lines();
    let first = lines.next()?.trim_end_matches('\r');
    if first != "---" {
        return None;
    }

    let mut yaml = String::new();
    for line in lines {
        let trimmed = line.trim_end_matches('\r');
        if trimmed == "---" {
            return Some(yaml);
        }
        yaml.push_str(trimmed);
        yaml.push('\n');
    }

    None
}

fn load_skill_interface(skill_path: &Path) -> Option<SkillInterface> {
    let skill_dir = skill_path.parent()?;

    let metadata_path = skill_dir.join("agents").join("openai.yaml");
    if !metadata_path.exists() {
        return None;
    }

    let contents = match std::fs::read_to_string(&metadata_path) {
        Ok(contents) => contents,
        Err(err) => {
            tracing::warn!(
                "ignoring {}: failed to read openai.yaml: {err}",
                metadata_path.display()
            );
            return None;
        }
    };

    let parsed: SkillMetadataFile = match serde_yaml::from_str(&contents) {
        Ok(parsed) => parsed,
        Err(err) => {
            tracing::warn!(
                "ignoring {}: invalid openai.yaml: {err}",
                metadata_path.display()
            );
            return None;
        }
    };

    let interface = parsed.interface?;
    let display_name = interface
        .display_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let short_description = interface
        .short_description
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);

    if display_name.is_none() && short_description.is_none() {
        None
    } else {
        Some(SkillInterface {
            display_name,
            short_description,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn mark_as_git_repo(dir: &Path) {
        // Repo root discovery only checks for `.git` (file or directory), so avoid shelling out to
        // `git init`.
        std::fs::write(dir.join(".git"), "gitdir: fake\n").expect("write .git marker");
    }

    #[test]
    fn parses_frontmatter_name_description_and_short_description() {
        let dir = tempfile::tempdir().expect("tempdir");
        let skill_dir = dir.path().join("skills").join("my-skill");
        std::fs::create_dir_all(&skill_dir).expect("mkdir skill");
        let skill_path = skill_dir.join("SKILL.md");
        std::fs::write(
            &skill_path,
            r#"---
name: my-skill
description: My test skill.
metadata:
  short-description: Short!
---

# Body
"#,
        )
        .expect("write skill");

        let parsed = parse_skill_file(&skill_path, SkillScope::User).expect("parse");
        assert_eq!(parsed.name, "my-skill");
        assert_eq!(parsed.description, "My test skill.");
        assert_eq!(parsed.short_description.as_deref(), Some("Short!"));
        assert_eq!(parsed.scope, SkillScope::User);
    }

    #[test]
    fn discovers_user_skills_from_home_agents_dir() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let repo_root = tmp.path().join("repo");
        std::fs::create_dir_all(&repo_root).expect("mkdir repo");
        mark_as_git_repo(&repo_root);
        let cwd = repo_root.join("cwd");
        std::fs::create_dir_all(&cwd).expect("mkdir cwd");

        let home = tmp.path().join("home");
        let skill_dir = home
            .join(AGENTS_DIR_NAME)
            .join(SKILLS_DIR_NAME)
            .join("home-skill");
        std::fs::create_dir_all(&skill_dir).expect("mkdir home skill");
        let skill_path = skill_dir.join(SKILL_FILENAME);
        std::fs::write(
            &skill_path,
            r#"---
name: home-skill
description: Installed under $HOME/.agents/skills.
---

# Body
"#,
        )
        .expect("write skill");

        let roots = skill_roots_with_dirs(
            &cwd,
            /*codex_home*/ None,
            Some(&home),
            &default_project_root_markers(),
        );
        let skills = load_skills_from_roots(roots);

        let home_skill = skills
            .iter()
            .find(|skill| skill.name == "home-skill")
            .expect("home-skill should be discovered");
        assert_eq!(home_skill.scope, SkillScope::User);
    }

    #[test]
    fn discovers_repo_skills_from_repo_agents_dir() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let repo_root = tmp.path().join("repo");
        std::fs::create_dir_all(&repo_root).expect("mkdir repo");
        mark_as_git_repo(&repo_root);

        let cwd = repo_root.join("dir_a").join("dir_b");
        std::fs::create_dir_all(&cwd).expect("mkdir cwd");

        let skill_dir = repo_root
            .join("dir_a")
            .join(AGENTS_DIR_NAME)
            .join(SKILLS_DIR_NAME)
            .join("repo-skill");
        std::fs::create_dir_all(&skill_dir).expect("mkdir repo skill");
        let skill_path = skill_dir.join(SKILL_FILENAME);
        std::fs::write(
            &skill_path,
            r#"---
name: repo-skill
description: Installed under dir/.agents/skills.
---

# Body
"#,
        )
        .expect("write skill");

        let roots = skill_roots_with_dirs(
            &cwd,
            /*codex_home*/ None,
            /*home_dir*/ None,
            &default_project_root_markers(),
        );
        let skills = load_skills_from_roots(roots);

        let repo_skill = skills
            .iter()
            .find(|skill| skill.name == "repo-skill")
            .expect("repo-skill should be discovered");
        assert_eq!(repo_skill.scope, SkillScope::Repo);
    }

    #[test]
    fn does_not_scan_ancestors_for_repo_skills_outside_git_repo() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let parent = tmp.path().join("parent");
        let cwd = parent.join("child");
        std::fs::create_dir_all(&cwd).expect("mkdir cwd");

        let parent_skill_dir = parent
            .join(AGENTS_DIR_NAME)
            .join(SKILLS_DIR_NAME)
            .join("parent-skill");
        std::fs::create_dir_all(&parent_skill_dir).expect("mkdir parent skill");
        std::fs::write(
            parent_skill_dir.join(SKILL_FILENAME),
            r#"---
name: parent-skill
description: Should not be treated as a repo skill when cwd is outside a git repo.
---

# Body
"#,
        )
        .expect("write skill");

        let roots = skill_roots_with_dirs(
            &cwd,
            /*codex_home*/ None,
            /*home_dir*/ None,
            &default_project_root_markers(),
        );
        let skills = load_skills_from_roots(roots);

        assert_eq!(
            skills.iter().any(|skill| skill.name == "parent-skill"),
            false,
            "repo skills discovery should not walk past cwd when no .git is present"
        );
    }

    #[test]
    fn configured_project_root_markers_allow_repo_agent_roots_without_git() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let project_root = tmp.path().join("project");
        let cwd = project_root.join("child");
        std::fs::create_dir_all(&cwd).expect("mkdir cwd");

        std::fs::write(project_root.join("MARKER"), "ok\n").expect("write marker");

        let skill_dir = project_root
            .join(AGENTS_DIR_NAME)
            .join(SKILLS_DIR_NAME)
            .join("repo-skill");
        std::fs::create_dir_all(&skill_dir).expect("mkdir repo skill");
        std::fs::write(
            skill_dir.join(SKILL_FILENAME),
            r#"---
name: repo-skill
description: Installed under dir/.agents/skills.
---

# Body
"#,
        )
        .expect("write skill");

        let roots = skill_roots_with_dirs(
            &cwd,
            /*codex_home*/ None,
            /*home_dir*/ None,
            &["MARKER".to_string()],
        );
        let skills = load_skills_from_roots(roots);

        assert_eq!(
            skills.iter().any(|skill| skill.name == "repo-skill"),
            true,
            "repo skills discovery should honor configured project root markers"
        );
    }

    #[test]
    fn empty_project_root_markers_disable_repo_root_traversal() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let repo_root = tmp.path().join("repo");
        std::fs::create_dir_all(&repo_root).expect("mkdir repo");
        mark_as_git_repo(&repo_root);
        let cwd = repo_root.join("child");
        std::fs::create_dir_all(&cwd).expect("mkdir cwd");

        let parent_skill_dir = repo_root
            .join(AGENTS_DIR_NAME)
            .join(SKILLS_DIR_NAME)
            .join("parent-skill");
        std::fs::create_dir_all(&parent_skill_dir).expect("mkdir parent skill");
        std::fs::write(
            parent_skill_dir.join(SKILL_FILENAME),
            r#"---
name: parent-skill
description: Should not be treated as a repo skill when root traversal is disabled.
---

# Body
"#,
        )
        .expect("write skill");

        let roots = skill_roots_with_dirs(
            &cwd,
            /*codex_home*/ None,
            /*home_dir*/ None,
            &Vec::new(),
        );
        let skills = load_skills_from_roots(roots);

        assert_eq!(
            skills.iter().any(|skill| skill.name == "parent-skill"),
            false,
            "repo skills discovery should treat cwd as the project root when root markers are empty"
        );
    }

    #[test]
    fn system_project_root_markers_are_used_when_present() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let system_config = tmp.path().join("system-config.toml");
        std::fs::write(&system_config, "project_root_markers = [\"SYSTEM\"]\n")
            .expect("write system config");

        let markers = project_root_markers_from_config_paths(Some(&system_config), None);

        assert_eq!(markers, vec!["SYSTEM".to_string()]);
    }

    #[test]
    fn user_project_root_markers_override_system_config() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let system_config = tmp.path().join("system-config.toml");
        std::fs::write(&system_config, "project_root_markers = [\"SYSTEM\"]\n")
            .expect("write system config");

        let codex_home = tmp.path().join("codex-home");
        std::fs::create_dir_all(&codex_home).expect("create codex home");
        std::fs::write(
            codex_home.join(CONFIG_TOML_FILENAME),
            "project_root_markers = [\"USER\"]\n",
        )
        .expect("write user config");

        let markers =
            project_root_markers_from_config_paths(Some(&system_config), Some(&codex_home));

        assert_eq!(markers, vec!["USER".to_string()]);
    }

    #[test]
    fn discovers_admin_skills_from_injected_admin_root() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let repo_root = tmp.path().join("repo");
        std::fs::create_dir_all(&repo_root).expect("mkdir repo");
        mark_as_git_repo(&repo_root);

        let cwd = repo_root.join("cwd");
        std::fs::create_dir_all(&cwd).expect("mkdir cwd");

        let admin_root = tmp.path().join("admin-skills");
        let skill_dir = admin_root.join("admin-skill");
        std::fs::create_dir_all(&skill_dir).expect("mkdir admin skill");
        std::fs::write(
            skill_dir.join(SKILL_FILENAME),
            r#"---
name: admin-skill
description: Installed under the admin config root.
---

# Body
"#,
        )
        .expect("write admin skill");

        let mut roots = skill_roots_with_dirs(
            &cwd,
            /*codex_home*/ None,
            /*home_dir*/ None,
            &default_project_root_markers(),
        );
        roots.push(SkillRoot {
            path: admin_root,
            scope: SkillScope::Admin,
            follow_symlinks: true,
        });

        let skills = load_skills_from_roots(roots);
        let admin_skill = skills
            .iter()
            .find(|skill| skill.name == "admin-skill")
            .expect("admin skill should be discovered");

        assert_eq!(admin_skill.scope, SkillScope::Admin);
    }

    #[cfg(windows)]
    #[test]
    fn windows_system_config_toml_path_uses_expected_suffix() {
        let system_dir = crate::codex_system_paths::system_codex_dir().expect("system dir");

        assert_eq!(
            system_config_toml_path().expect("system config path"),
            system_dir.join(CONFIG_TOML_FILENAME),
        );
        assert!(
            system_config_toml_path()
                .expect("system config path")
                .ends_with(Path::new("OpenAI").join("Codex").join(CONFIG_TOML_FILENAME))
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_admin_skills_dir_uses_expected_suffix() {
        let system_dir = crate::codex_system_paths::system_codex_dir().expect("system dir");

        assert_eq!(
            system_admin_skills_dir().expect("admin skills dir"),
            system_dir.join(SKILLS_DIR_NAME)
        );
        assert!(
            system_admin_skills_dir()
                .expect("admin skills dir")
                .ends_with(Path::new("OpenAI").join("Codex").join(SKILLS_DIR_NAME))
        );
    }
}
