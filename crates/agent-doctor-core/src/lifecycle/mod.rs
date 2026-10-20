pub mod cursor;
pub mod deepseek_harness;
mod download_route;
pub mod hermes;
pub mod nodejs;
pub mod npm_cli;
mod npm_target;
pub mod openclaw;
pub mod qoder;
mod runner;
mod uninstall;
pub mod workbuddy;
pub mod zcode;

pub use download_route::{use_china_mirrors, NPM_MIRROR};

/// A PowerShell script passed whole, so `cmd /C` never parses its quotes or pipes.
pub(crate) fn powershell_encoded(script: &str) -> String {
    use base64::Engine;
    let utf16: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let encoded = base64::engine::general_purpose::STANDARD.encode(utf16);
    format!(
        "powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -EncodedCommand {encoded}"
    )
}

pub use uninstall::{uninstall_runtime, uninstall_shell_command};

pub use cursor::{cursor_install_shell_command, run_cursor_lifecycle, CursorLifecycleAction};
pub use deepseek_harness::{
    deepseek_harness_install_shell_command, deepseek_harness_is_outdated,
    deepseek_harness_shell_command, deepseek_harness_update_shell_command,
    installed_deepseek_harness_version, run_deepseek_harness_lifecycle,
    DeepSeekHarnessLifecycleAction,
};
pub use hermes::{
    hermes_install_shell_command, hermes_shell_command, hermes_update_shell_command,
    run_hermes_lifecycle, HermesLifecycleAction,
};
pub use npm_cli::{
    claude_code_install_shell_command, claude_code_update_shell_command,
    codex_install_shell_command, codex_update_shell_command, run_claude_code_lifecycle,
    run_codex_lifecycle, NpmCliLifecycleAction,
};
pub use openclaw::{
    openclaw_doctor_fix_shell_command, openclaw_install_shell_command, openclaw_shell_command,
    openclaw_update_shell_command, run_openclaw_doctor_fix, run_openclaw_lifecycle,
    OpenClawLifecycleAction,
};
pub use qoder::{
    qoder_install_shell_command, qoder_update_shell_command, run_qoder_lifecycle,
    QoderLifecycleAction,
};
pub use runner::{
    run_shell_command_capturing, run_shell_command_streaming, submit_install_line,
    waiting_input_kind, write_install_log, ShellCapture,
};
pub use workbuddy::{
    run_workbuddy_lifecycle, workbuddy_install_shell_command, workbuddy_update_shell_command,
    WorkbuddyLifecycleAction,
};
pub use zcode::{
    run_zcode_lifecycle, zcode_install_shell_command, zcode_update_shell_command,
    ZcodeLifecycleAction,
};
