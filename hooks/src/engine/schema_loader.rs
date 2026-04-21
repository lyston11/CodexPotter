use std::sync::OnceLock;

use serde_json::Value;

pub(super) fn validate_generated_hook_schemas_loaded() {
    // Decode the generated schema at startup so invalid checked-in fixtures fail fast.
    let _ = potter_project_stop_command_input_schema();
}

fn potter_project_stop_command_input_schema() -> &'static Value {
    static SCHEMA: OnceLock<Value> = OnceLock::new();
    SCHEMA.get_or_init(|| {
        parse_json_schema(
            "potter-project-stop.command.input",
            include_str!("../../schema/generated/potter-project-stop.command.input.schema.json"),
        )
    })
}

fn parse_json_schema(name: &str, schema: &str) -> Value {
    serde_json::from_str(schema)
        .unwrap_or_else(|err| panic!("invalid generated hooks schema {name}: {err}"))
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::potter_project_stop_command_input_schema;

    #[test]
    fn loads_generated_hook_schemas() {
        let schema = potter_project_stop_command_input_schema();
        assert_eq!(schema["type"], "object");
    }
}
