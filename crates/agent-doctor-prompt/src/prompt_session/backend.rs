use anyhow::Result;

use super::{PromptSessionCancel, PromptSessionControl, PromptSessionOptions, PromptSessionReport};

/// Ask-session backend for a single runtime (Claude Code, Codex, …).
///
/// Distinct from Doctor's [`agent_doctor_core::adapter::RuntimeAdapter`] (discover/config).
pub trait AskBackend {
    fn run(
        &self,
        options: &PromptSessionOptions,
        cancel: PromptSessionCancel,
        control: Option<PromptSessionControl>,
        on_event: &mut dyn FnMut(super::PromptSessionEvent),
    ) -> Result<PromptSessionReport>;
}
