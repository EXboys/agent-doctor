pub mod hermes;
pub mod npm_cli;
pub mod openclaw;

pub use hermes::PlaybookApplyResult;
pub use hermes::{apply_hermes_playbook, apply_hermes_playbook_filtered, suggest_hermes_repairs};
pub use npm_cli::{
    apply_claude_code_playbook, apply_claude_code_playbook_filtered, apply_codex_playbook,
    apply_codex_playbook_filtered, suggest_claude_code_repairs, suggest_codex_repairs,
};
pub use openclaw::{
    apply_openclaw_playbook, apply_openclaw_playbook_filtered, suggest_openclaw_repairs,
};

pub(crate) fn should_run(action_id: &str, only_ids: Option<&[String]>) -> bool {
    only_ids
        .map(|ids| ids.iter().any(|id| id == action_id))
        .unwrap_or(true)
}
