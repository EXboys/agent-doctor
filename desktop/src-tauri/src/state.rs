use std::sync::Mutex;

use agent_doctor_core::{PromptSessionCancel, PromptSessionControl};

/// At most one light ask session at a time (panel UX).
#[derive(Default)]
pub struct PromptSessionState {
    pub cancel: Mutex<Option<PromptSessionCancel>>,
    pub control: Mutex<Option<PromptSessionControl>>,
}
