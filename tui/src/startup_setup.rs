/// Shared startup setup step metadata used by onboarding prompts.
///
/// This is currently used to render "Setup X/Y" markers so users understand how many prompts
/// remain during first-run onboarding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StartupSetupStep {
    /// 1-based index of the current step.
    pub index: usize,
    /// Total number of setup steps shown in this startup session.
    pub total: usize,
}

impl StartupSetupStep {
    pub fn new(index: usize, total: usize) -> Self {
        debug_assert!(index > 0, "setup step index must be 1-based");
        debug_assert!(total > 0, "setup step total must be > 0");
        debug_assert!(index <= total, "setup step index must be <= total");
        Self { index, total }
    }

    pub fn should_render(self) -> bool {
        self.total > 1
    }

    pub fn label(self) -> String {
        format!("Setup {}/{}", self.index, self.total)
    }
}
