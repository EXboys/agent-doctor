//! Ask/prompt sessions on top of the agent runtimes discovered by
//! [`agent_doctor_core`]: one-shot `ask`, streaming events, warm sessions,
//! and the per-runtime `AskBackend` implementations.

pub mod prompt_session;

pub use prompt_session::{
    ask_image_support, ask_runtime_ids, enable_warm_sessions, installed_ask_agents,
    retire_warm_sessions, run_prompt_session, run_prompt_session_with_cancel,
    shutdown_warm_sessions, AskImageSupport, PlanStep, PlanStepState, PromptSessionCancel,
    PromptSessionControl, PromptSessionEvent, PromptSessionOptions, PromptSessionReport,
    PromptSessionStatus,
};
