mod claude_code;
mod codex;
mod cursor;
mod deepseek_harness;
mod hermes;
mod openclaw;
mod qoder;
pub mod util;
mod workbuddy;
mod zcode;

pub use claude_code::{ClaudeCodeAdapter, CLAUDE_CODE_NPM_PACKAGE};
pub use codex::{CodexAdapter, CODEX_NPM_PACKAGE};
pub(crate) use cursor::cursor_desktop_installs;
pub use cursor::{cursor_cli_path, CursorAdapter};
pub use deepseek_harness::{
    DeepSeekHarnessAdapter, DEEPSEEK_API_KEY_ENV, DEEPSEEK_BASE_URL_ENV, DEEPSEEK_HARNESS_CLI,
    DEEPSEEK_HARNESS_NODE_MIN, DEEPSEEK_HARNESS_NPM_PACKAGE, DEEPSEEK_HARNESS_RUNTIME_ID,
    DEEPSEEK_HARNESS_VERSION, DEEPSEEK_OFFICIAL_BASE_URL, DEEPSEEK_OFFICIAL_PROVIDER_ROUTE,
};
pub use hermes::{HermesAdapter, HermesSettings};
pub use openclaw::{configured_base_url, OpenClawAdapter};
pub(crate) use qoder::{locate_qoder, qoder_desktop_installs, QoderInstall};
pub use qoder::{qoder_cli_path, qoder_desktop_path, qoder_missing_install_part, QoderAdapter};
pub use util::{hermes_root, refresh_managed_runtime_path};
pub(crate) use workbuddy::workbuddy_desktop_installs;
pub use workbuddy::{
    workbuddy_cli_path, workbuddy_desktop_path, workbuddy_missing_install_part, WorkbuddyAdapter,
};
pub use zcode::ZcodeAdapter;
pub use zcode::{
    locate_zcode, locate_zcode_headless, zcode_desktop_path, ZcodeHeadless, ZcodeInstall,
};
