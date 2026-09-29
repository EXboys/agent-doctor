//! Open an interactive coding-agent session outside Agent Doctor (CC Switch-style).
//!
//! Doctor stays the ops shell: connection, health, preferred runtime, and launch.
//! Chat/TUI stays in the official CLI (Claude Code deep link or a system terminal).

use std::path::{Path, PathBuf};
#[cfg(target_os = "windows")]
use std::process::Stdio;

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

use crate::evotown::normalize_runtime;
#[cfg(windows)]
use crate::profile::read_company_profile;
#[cfg(windows)]
use crate::setup::anthropic_gateway_url_from_evotown_base;
#[cfg(test)]
use crate::setup::{PROTOCOL_ANTHROPIC, PROVIDER_PROTOCOL_ENV};
use crate::workspace::{ensure_default_workspace, load_workspaces};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenSessionOptions {
    pub runtime: String,
    pub cwd: Option<PathBuf>,
    pub prompt: Option<String>,
    /// Prefer opening via deep link when available (Claude Code).
    #[serde(default = "default_true")]
    pub prefer_deep_link: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum OpenSessionMethod {
    DeepLink,
    Terminal,
    App,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenSessionReport {
    pub runtime: String,
    pub method: OpenSessionMethod,
    pub cwd: String,
    pub target: String,
    pub detail: String,
}

fn existing_dir(path: &Path) -> Option<PathBuf> {
    path.is_dir().then(|| path.to_path_buf())
}

/// Resolve cwd: existing explicit dir → existing workspace → home → process cwd.
/// Missing folders (deleted projects / leftover temp paths) never fail Ask.
pub fn resolve_session_cwd(explicit: Option<&Path>) -> PathBuf {
    if let Some(path) = explicit {
        if let Some(dir) = existing_dir(path) {
            return dir;
        }
    }
    if let Ok(doc) = ensure_default_workspace().or_else(|_| load_workspaces()) {
        if let Some(active) = doc.active.as_deref() {
            if let Some(entry) = doc.workspaces.get(active) {
                if let Some(dir) = existing_dir(&entry.path) {
                    return dir;
                }
            }
        }
    }
    dirs::home_dir()
        .and_then(|home| existing_dir(&home))
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Open an interactive session for a known runtime.
pub fn open_interactive_session(options: &OpenSessionOptions) -> Result<OpenSessionReport> {
    let runtime = normalize_runtime(&options.runtime);
    let cwd = resolve_session_cwd(options.cwd.as_deref());
    if !cwd.exists() {
        bail!("session cwd does not exist: {}", cwd.display());
    }

    let Some(open) = crate::runtime::open_session(&runtime) else {
        bail!("opening interactive sessions is not supported for runtime '{runtime}'");
    };
    open(&cwd, options.prompt.as_deref(), options.prefer_deep_link)
}

mod claude;
mod codex;
mod cursor;
mod terminal;

pub use claude::*;
pub(crate) use codex::*;
pub(crate) use cursor::*;
pub(crate) use terminal::*;

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::fs;
    use std::path::{Path, PathBuf};

    use crate::profile::{
        GATEWAY_URL_ENV, PROVIDER_KIND_COMPANY, PROVIDER_KIND_ENV, PROVIDER_KIND_PERSONAL,
    };
    use crate::setup::merge::{CODEX_PERSONAL_SLOT, CODEX_TEAM_SLOT};
    use crate::setup::{COMPANY_API_KEY_ENV, EVOTOWN_API_KEY_ENV, MODEL_ENV};

    #[test]
    fn resolve_session_cwd_skips_missing_explicit_dir() {
        let missing = PathBuf::from("/this/path/should/not/exist/agent-doctor-cwd-test");
        assert!(!missing.exists());
        let cwd = resolve_session_cwd(Some(&missing));
        assert!(
            cwd.is_dir(),
            "fallback must be an existing directory: {}",
            cwd.display()
        );
    }

    #[test]
    fn builds_claude_deep_link_with_cwd_and_prompt() {
        let link = claude_cli_deep_link(Path::new("/tmp/demo project"), Some("review PRs\nplease"));
        assert!(link.starts_with("claude-cli://open?"));
        assert!(link.contains("cwd="));
        assert!(link.contains("q="));
        assert!(link.contains("review"));
    }

    #[test]
    fn builds_bare_claude_deep_link() {
        assert_eq!(
            claude_cli_deep_link(Path::new(""), None),
            "claude-cli://open"
        );
    }

    #[test]
    fn shell_join_preserves_official_dsh_web_entrypoint() {
        assert_eq!(shell_join(&["dsh", "web"]), "dsh web");
    }

    #[test]
    fn windows_terminal_script_keeps_console_visible() {
        let body = render_windows_terminal_script(
            r"C:\Temp\agent-doctor-terminal.ps1",
            r"C:\Users\zhang\work",
            "& 'C:\\Users\\zhang\\AppData\\Roaming\\npm\\claude.cmd'",
        );
        assert!(body.contains("Set-Location -LiteralPath 'C:\\Users\\zhang\\work'"));
        assert!(body.contains("claude.cmd"));
        assert!(body.contains("[System.Console]::ReadLine()"));
        assert!(body.contains("starting session"));
    }

    #[cfg(windows)]
    #[test]
    fn shell_join_calls_windows_cmd_shims() {
        assert_eq!(
            shell_join(&[r"C:\Users\zhang\AppData\Roaming\npm\claude.cmd"]),
            r"& 'C:\Users\zhang\AppData\Roaming\npm\claude.cmd'"
        );
    }

    #[test]
    fn wraps_command_with_exported_key_when_no_profile_env() {
        // Without a profile file, wrap still returns a runnable command.
        let wrapped = wrap_with_company_env("codex");
        assert!(wrapped.contains("codex"));
    }

    #[test]
    fn wrap_exports_openai_base_url_alias() {
        let wrapped = wrap_with_company_env("codex");
        if wrapped != "codex" {
            assert!(
                wrapped.contains("OPENAI_BASE_URL"),
                "expected OPENAI_BASE_URL export, got {wrapped}"
            );
            assert!(
                wrapped.contains("OPENAI_API_KEY"),
                "expected OPENAI_API_KEY export, got {wrapped}"
            );
        }
    }

    #[test]
    fn wrap_exports_anthropic_aliases() {
        let wrapped = wrap_with_company_env("claude");
        if wrapped != "claude" {
            assert!(
                wrapped.contains("ANTHROPIC_BASE_URL"),
                "expected ANTHROPIC_BASE_URL export, got {wrapped}"
            );
            assert!(
                wrapped.contains("ANTHROPIC_API_KEY"),
                "expected ANTHROPIC_API_KEY export, got {wrapped}"
            );
        }
    }

    #[test]
    fn anthropic_launch_prefers_explicit_base_url() {
        let env = HashMap::from([
            (
                "ANTHROPIC_BASE_URL".into(),
                "https://proxy.example/anthropic".into(),
            ),
            (COMPANY_API_KEY_ENV.into(), "sk-company".into()),
            (
                "AGENT_DOCTOR_EVOTOWN_URL".into(),
                "https://www.skilllite.ai".into(),
            ),
        ]);
        let (url, key) = anthropic_launch_from_env(&env).unwrap();
        assert_eq!(url, "https://proxy.example/anthropic");
        assert_eq!(key, "sk-company");
    }

    #[test]
    fn anthropic_launch_derives_team_gateway_from_evotown() {
        let env = HashMap::from([
            (COMPANY_API_KEY_ENV.into(), "sk-team".into()),
            (
                "AGENT_DOCTOR_EVOTOWN_URL".into(),
                "https://www.skilllite.ai".into(),
            ),
        ]);
        let (url, key) = anthropic_launch_from_env(&env).unwrap();
        assert_eq!(url, "https://www.skilllite.ai/api/gateway/anthropic");
        assert_eq!(key, "sk-team");
    }

    #[test]
    fn anthropic_launch_uses_personal_anthropic_gateway() {
        let env = HashMap::from([
            (PROVIDER_PROTOCOL_ENV.into(), PROTOCOL_ANTHROPIC.into()),
            (GATEWAY_URL_ENV.into(), "https://api.anthropic.com".into()),
            (COMPANY_API_KEY_ENV.into(), "sk-ant".into()),
        ]);
        let (url, key) = anthropic_launch_from_env(&env).unwrap();
        assert_eq!(url, "https://api.anthropic.com");
        assert_eq!(key, "sk-ant");
    }

    #[test]
    fn anthropic_launch_maps_deepseek_openai_to_anthropic() {
        let env = HashMap::from([
            (PROVIDER_PROTOCOL_ENV.into(), "openai".into()),
            (GATEWAY_URL_ENV.into(), "https://api.deepseek.com/v1".into()),
            (COMPANY_API_KEY_ENV.into(), "sk-ds".into()),
        ]);
        let (url, key) = anthropic_launch_from_env(&env).unwrap();
        assert_eq!(url, "https://api.deepseek.com/anthropic");
        assert_eq!(key, "sk-ds");
    }

    #[test]
    fn anthropic_launch_skips_openai_without_dual_or_evotown() {
        let env = HashMap::from([
            (PROVIDER_PROTOCOL_ENV.into(), "openai".into()),
            (GATEWAY_URL_ENV.into(), "https://api.openai.com/v1".into()),
            (COMPANY_API_KEY_ENV.into(), "sk-oai".into()),
        ]);
        assert!(anthropic_launch_from_env(&env).is_none());
    }

    #[test]
    fn codex_launch_reads_gateway_key_model_and_company_slot() {
        let env = HashMap::from([
            (
                GATEWAY_URL_ENV.into(),
                "https://www.skilllite.ai/api/gateway/v1".into(),
            ),
            (COMPANY_API_KEY_ENV.into(), "sk-team".into()),
            (MODEL_ENV.into(), "deepseek-v4-flash".into()),
            (PROVIDER_KIND_ENV.into(), PROVIDER_KIND_COMPANY.into()),
        ]);
        let (url, key, model, slot) = codex_launch_from_env(&env).unwrap();
        assert_eq!(url, "https://www.skilllite.ai/api/gateway/v1");
        assert_eq!(key, "sk-team");
        assert_eq!(model.as_deref(), Some("deepseek-v4-flash"));
        assert_eq!(slot, CODEX_TEAM_SLOT);
    }

    #[test]
    fn codex_launch_personal_slot_from_provider_kind() {
        let env = HashMap::from([
            (GATEWAY_URL_ENV.into(), "https://api.deepseek.com/v1".into()),
            (COMPANY_API_KEY_ENV.into(), "sk-ds".into()),
            (PROVIDER_KIND_ENV.into(), PROVIDER_KIND_PERSONAL.into()),
        ]);
        let (_, _, _, slot) = codex_launch_from_env(&env).unwrap();
        assert_eq!(slot, CODEX_PERSONAL_SLOT);
    }

    #[test]
    fn codex_launch_infers_team_slot_without_provider_kind() {
        let env = HashMap::from([
            (
                GATEWAY_URL_ENV.into(),
                "https://www.skilllite.ai/api/gateway/v1".into(),
            ),
            (EVOTOWN_API_KEY_ENV.into(), "evk_team".into()),
        ]);
        let (_, key, model, slot) = codex_launch_from_env(&env).unwrap();
        assert_eq!(key, "evk_team");
        assert!(model.is_none());
        assert_eq!(slot, CODEX_TEAM_SLOT);
    }

    #[test]
    fn codex_launch_requires_gateway_and_key() {
        assert!(codex_launch_from_env(&HashMap::from([(
            GATEWAY_URL_ENV.into(),
            "https://www.skilllite.ai/api/gateway/v1".into(),
        )]))
        .is_none());
        assert!(codex_launch_from_env(&HashMap::from([(
            COMPANY_API_KEY_ENV.into(),
            "sk-team".into(),
        )]))
        .is_none());
    }

    fn has_override(argv: &[String], expected: &str) -> bool {
        argv.windows(2).any(|w| w[0] == "-c" && w[1] == expected)
    }

    #[test]
    fn codex_launch_argv_defines_team_provider_inline() {
        let launch = (
            "https://www.skilllite.ai/api/gateway/v1".to_string(),
            "sk-team".to_string(),
            Some("deepseek-v4-flash".to_string()),
            CODEX_TEAM_SLOT.to_string(),
        );
        let argv = codex_launch_argv(Some(&launch));
        assert_eq!(argv[0], "codex");
        assert!(has_override(&argv, "model_provider=\"company\""));
        assert!(has_override(
            &argv,
            "model_providers.company.base_url=\"https://www.skilllite.ai/api/gateway/v1\""
        ));
        assert!(has_override(
            &argv,
            "model_providers.company.env_key=\"EVOTOWN_API_KEY\""
        ));
        assert!(has_override(&argv, "model=\"deepseek-v4-flash\""));
    }

    #[test]
    fn codex_launch_argv_never_uses_builtin_openai_provider() {
        let launch = (
            "https://api.deepseek.com/v1".to_string(),
            "sk-ds".to_string(),
            Some("deepseek-v4-flash".to_string()),
            CODEX_PERSONAL_SLOT.to_string(),
        );
        let argv = codex_launch_argv(Some(&launch));
        // Built-in `openai` negotiates websockets + OpenAI auth, which third-party
        // gateways reject with 401 on wss://<host>/v1/responses.
        assert!(!has_override(&argv, "model_provider=\"openai\""));
        assert!(has_override(&argv, "model_provider=\"personal\""));
        assert!(has_override(
            &argv,
            "model_providers.personal.supports_websockets=false"
        ));
        assert!(has_override(
            &argv,
            "model_providers.personal.requires_openai_auth=false"
        ));
        assert!(has_override(
            &argv,
            "model_providers.personal.wire_api=\"responses\""
        ));
        assert!(has_override(
            &argv,
            "model_providers.personal.base_url=\"https://api.deepseek.com/v1\""
        ));
    }

    #[test]
    fn codex_launch_argv_omits_model_when_profile_has_none() {
        let launch = (
            "https://api.deepseek.com/v1".to_string(),
            "sk-ds".to_string(),
            None,
            CODEX_PERSONAL_SLOT.to_string(),
        );
        let argv = codex_launch_argv(Some(&launch));
        assert!(!argv.iter().any(|part| part.starts_with("model=")));
        assert_eq!(codex_launch_argv(None), vec!["codex".to_string()]);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_terminal_script_preserves_long_commands_and_removes_itself() {
        let command = format!("codex {}", "x".repeat(2048));
        let path = write_macos_terminal_launch_script(Path::new("/tmp/demo project"), &command)
            .expect("launch script");
        let rendered = fs::read_to_string(&path).expect("read launch script");
        assert!(rendered.contains(&command));
        assert!(rendered.contains("cd '/tmp/demo project' && codex"));
        assert!(rendered.contains("rm -f -- "));
        fs::remove_file(path).expect("remove launch script");
    }
}
