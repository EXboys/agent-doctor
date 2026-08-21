pub mod deepseek_harness;
pub mod hermes;
pub mod nodejs;
pub mod npm_cli;
pub mod openclaw;
mod runner;

pub use deepseek_harness::{
    deepseek_harness_install_shell_command, deepseek_harness_shell_command,
    deepseek_harness_update_shell_command, run_deepseek_harness_lifecycle,
    DeepSeekHarnessLifecycleAction,
};
pub use hermes::{
    hermes_install_shell_command, hermes_shell_command, run_hermes_lifecycle, HermesLifecycleAction,
};
pub use npm_cli::{
    claude_code_install_shell_command, claude_code_update_shell_command,
    codex_install_shell_command, codex_update_shell_command, run_claude_code_lifecycle,
    run_codex_lifecycle, NpmCliLifecycleAction,
};
pub use openclaw::{
    openclaw_install_shell_command, openclaw_shell_command, run_openclaw_lifecycle,
    OpenClawLifecycleAction,
};
pub use runner::{
    run_shell_command_capturing, run_shell_command_streaming, write_install_log, ShellCapture,
};
