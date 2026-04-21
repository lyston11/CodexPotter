use serde::Deserialize;

#[derive(Debug, Default, Deserialize)]
pub(super) struct HooksFile {
    #[serde(default)]
    pub(super) hooks: HookEvents,
}

#[derive(Debug, Default, Deserialize)]
pub(super) struct HookEvents {
    #[serde(rename = "Potter.ProjectStop", default)]
    pub(super) potter_project_stop: Vec<MatcherGroup>,
}

#[derive(Debug, Default, Deserialize)]
pub(super) struct MatcherGroup {
    #[serde(default)]
    pub(super) matcher: Option<String>,
    #[serde(default)]
    pub(super) hooks: Vec<HookHandlerConfig>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
pub(super) enum HookHandlerConfig {
    #[serde(rename = "command")]
    Command {
        command: String,
        #[serde(default, rename = "timeout", alias = "timeoutSec")]
        timeout_sec: Option<u64>,
        #[serde(default)]
        r#async: bool,
        #[serde(default, rename = "statusMessage")]
        status_message: Option<String>,
    },
    #[serde(rename = "prompt")]
    Prompt {},
    #[serde(rename = "agent")]
    Agent {},
}
