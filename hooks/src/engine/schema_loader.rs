use std::sync::OnceLock;

use serde_json::Value;

#[allow(dead_code)]
pub(crate) struct GeneratedHookSchemas {
    pub potter_project_stop_command_input: Value,
}

pub(crate) fn generated_hook_schemas() -> &'static GeneratedHookSchemas {
    static SCHEMAS: OnceLock<GeneratedHookSchemas> = OnceLock::new();
    SCHEMAS.get_or_init(|| GeneratedHookSchemas {
        potter_project_stop_command_input: parse_json_schema(
            "potter-project-stop.command.input",
            include_str!("../../schema/generated/potter-project-stop.command.input.schema.json"),
        ),
    })
}

fn parse_json_schema(name: &str, schema: &str) -> Value {
    serde_json::from_str(schema)
        .unwrap_or_else(|err| panic!("invalid generated hooks schema {name}: {err}"))
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::generated_hook_schemas;

    #[test]
    fn loads_generated_hook_schemas() {
        let schemas = generated_hook_schemas();
        assert_eq!(schemas.potter_project_stop_command_input["type"], "object");
    }
}
