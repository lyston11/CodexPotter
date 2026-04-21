use std::path::Path;

use schemars::JsonSchema;
use schemars::r#gen::SchemaGenerator;
use schemars::r#gen::SchemaSettings;
use schemars::schema::InstanceType;
use schemars::schema::RootSchema;
use schemars::schema::Schema;
use schemars::schema::SchemaObject;
use serde::Serialize;
use serde_json::Map;
use serde_json::Value;

const POTTER_PROJECT_STOP_INPUT_FIXTURE: &str = "potter-project-stop.command.input.schema.json";

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(rename = "potter-project-stop.command.input")]
/// JSON payload written to stdin for `Potter.ProjectStop` command hooks.
///
/// The generated schema fixture is derived from this type, so field names and requiredness define
/// the contract between CodexPotter and external hook commands.
pub struct PotterProjectStopCommandInput {
    /// Absolute path to the project directory containing `MAIN.md`.
    pub project_dir: String,
    /// Absolute path to the project progress file (`MAIN.md`).
    pub project_file_path: String,
    /// Working directory used for hook discovery and as the hook process CWD.
    pub cwd: String,
    /// Stable hook event name. Always `Potter.ProjectStop`.
    #[schemars(schema_with = "potter_project_stop_hook_event_name_schema")]
    pub hook_event_name: String,
    /// Original user prompt captured when the project started.
    pub user_prompt: String,
    /// Session IDs for every round recorded in the project.
    pub all_session_ids: Vec<String>,
    /// Session IDs created since the current resume boundary.
    pub new_session_ids: Vec<String>,
    /// Final assistant messages for every recorded round.
    pub all_assistant_messages: Vec<String>,
    /// Final assistant messages created since the current resume boundary.
    pub new_assistant_messages: Vec<String>,
    /// Stable stop reason code such as `succeeded` or `budget_exhausted`.
    pub stop_reason_code: String,
}

fn potter_project_stop_hook_event_name_schema(_gen: &mut SchemaGenerator) -> Schema {
    string_const_schema("Potter.ProjectStop")
}

/// Regenerate committed JSON schema fixtures under `schema/generated/`.
pub fn write_schema_fixtures(schema_root: &Path) -> anyhow::Result<()> {
    let generated_dir = schema_root.join("generated");
    ensure_empty_dir(&generated_dir)?;

    write_schema(
        &generated_dir.join(POTTER_PROJECT_STOP_INPUT_FIXTURE),
        schema_json::<PotterProjectStopCommandInput>()?,
    )?;

    Ok(())
}

fn write_schema(path: &Path, json: Vec<u8>) -> anyhow::Result<()> {
    std::fs::write(path, json)?;
    Ok(())
}

fn ensure_empty_dir(dir: &Path) -> anyhow::Result<()> {
    if dir.exists() {
        std::fs::remove_dir_all(dir)?;
    }
    std::fs::create_dir_all(dir)?;
    Ok(())
}

fn schema_json<T>() -> anyhow::Result<Vec<u8>>
where
    T: JsonSchema,
{
    let schema = schema_for_type::<T>();
    let value = serde_json::to_value(schema)?;
    let value = canonicalize_json(&value);
    Ok(serde_json::to_vec_pretty(&value)?)
}

fn schema_for_type<T>() -> RootSchema
where
    T: JsonSchema,
{
    SchemaSettings::draft07()
        .with(|settings| {
            settings.option_add_null_type = false;
        })
        .into_generator()
        .into_root_schema_for::<T>()
}

fn canonicalize_json(value: &Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.iter().map(canonicalize_json).collect()),
        Value::Object(map) => {
            let mut entries: Vec<_> = map.iter().collect();
            entries.sort_by(|(left, _), (right, _)| left.cmp(right));
            let mut sorted = Map::with_capacity(map.len());
            for (key, child) in entries {
                sorted.insert(key.clone(), canonicalize_json(child));
            }
            Value::Object(sorted)
        }
        _ => value.clone(),
    }
}

fn string_const_schema(value: &str) -> Schema {
    let mut schema = SchemaObject {
        instance_type: Some(InstanceType::String.into()),
        ..Default::default()
    };
    schema.const_value = Some(Value::String(value.to_string()));
    Schema::Object(schema)
}

#[cfg(test)]
mod tests {
    use super::POTTER_PROJECT_STOP_INPUT_FIXTURE;
    use super::PotterProjectStopCommandInput;
    use super::schema_json;
    use super::write_schema_fixtures;
    use pretty_assertions::assert_eq;
    use serde_json::Value;
    use tempfile::TempDir;

    fn expected_fixture(name: &str) -> &'static str {
        match name {
            POTTER_PROJECT_STOP_INPUT_FIXTURE => {
                include_str!("../schema/generated/potter-project-stop.command.input.schema.json")
            }
            _ => panic!("unexpected fixture name: {name}"),
        }
    }

    fn normalize_newlines(value: &str) -> String {
        value.replace("\r\n", "\n")
    }

    #[test]
    fn generated_hook_schemas_match_fixtures() {
        let temp_dir = TempDir::new().expect("create temp dir");
        let schema_root = temp_dir.path().join("schema");
        write_schema_fixtures(&schema_root).expect("write generated hook schemas");

        let expected = normalize_newlines(expected_fixture(POTTER_PROJECT_STOP_INPUT_FIXTURE));
        let actual = std::fs::read_to_string(
            schema_root
                .join("generated")
                .join(POTTER_PROJECT_STOP_INPUT_FIXTURE),
        )
        .unwrap_or_else(|err| panic!("read generated schema: {err}"));
        let actual = normalize_newlines(&actual);
        assert_eq!(expected, actual, "fixture should match generated schema");
    }

    #[test]
    fn project_stop_hook_schema_includes_potter_event_name() {
        let schema: Value = serde_json::from_slice(
            &schema_json::<PotterProjectStopCommandInput>()
                .expect("serialize project stop input schema"),
        )
        .expect("parse project stop input schema");

        assert_eq!(
            schema["properties"]["hook_event_name"]["const"],
            Value::String("Potter.ProjectStop".to_string())
        );
    }
}
