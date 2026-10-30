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

pub use claude_code::ClaudeCodeAdapter;
pub use codex::CodexAdapter;
pub(crate) use cursor::cursor_desktop_installs;
pub use cursor::CursorAdapter;
pub use deepseek_harness::{
    DeepSeekHarnessAdapter, DEEPSEEK_API_KEY_ENV, DEEPSEEK_BASE_URL_ENV, DEEPSEEK_HARNESS_CLI,
    DEEPSEEK_HARNESS_NPM_PACKAGE, DEEPSEEK_HARNESS_RUNTIME_ID, DEEPSEEK_HARNESS_VERSION,
};
pub use hermes::{HermesAdapter, HermesSettings};
pub use openclaw::{configured_base_url, OpenClawAdapter};
pub use qoder::QoderAdapter;
pub(crate) use qoder::{locate_qoder, QoderInstall};
pub use util::{hermes_root, refresh_managed_runtime_path};
pub use workbuddy::WorkbuddyAdapter;
pub use zcode::ZcodeAdapter;
pub use zcode::{locate_zcode, locate_zcode_headless, ZcodeHeadless, ZcodeInstall};
