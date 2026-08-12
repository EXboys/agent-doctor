//! Open an interactive coding-agent session outside Agent Doctor (CC Switch-style).
//!
//! Doctor stays the ops shell: connection, health, preferred runtime, and launch.
//! Chat/TUI stays in the official CLI (Claude Code deep link or a system terminal).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use url::form_urlencoded;

use crate::evotown::{load_evotown_config, normalize_runtime};
#[cfg(windows)]
use crate::profile::read_company_profile;
use crate::profile::{
    agent_profile_path, company_baseline_path, read_env_map, GATEWAY_URL_ENV,
    PROVIDER_KIND_COMPANY, PROVIDER_KIND_ENV, PROVIDER_KIND_PERSONAL,
};
use crate::setup::merge::{
    apply_claude_code, apply_codex_slot, codex_host_supports_responses_api, CODEX_PERSONAL_SLOT,
    CODEX_TEAM_SLOT,
};
use crate::setup::{
    anthropic_gateway_url_from_evotown_base, clear_codex_placeholder_auth, evotown_agent_env_path,
    normalize_protocol, write_company_profile_with_gateway, COMPANY_API_KEY_ENV,
    EVOTOWN_API_KEY_ENV, EVOTOWN_URL_ENV, MODEL_ENV, PROTOCOL_ANTHROPIC, PROVIDER_PROTOCOL_ENV,
};
use crate::workspace::{active_env_path, load_workspaces};

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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenSessionReport {
    pub runtime: String,
    pub method: OpenSessionMethod,
    pub cwd: String,
    pub target: String,
    pub detail: String,
}

/// Resolve cwd: explicit option → active workspace → process cwd.
pub fn resolve_session_cwd(explicit: Option<&Path>) -> PathBuf {
    if let Some(path) = explicit {
        return path.to_path_buf();
    }
    if let Ok(doc) = load_workspaces() {
        if let Some(active) = doc.active.as_deref() {
            if let Some(entry) = doc.workspaces.get(active) {
                return entry.path.clone();
            }
        }
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

/// Build the official Claude Code deep link (`claude-cli://open`).
pub fn claude_cli_deep_link(cwd: &Path, prompt: Option<&str>) -> String {
    let mut pairs: Vec<(&str, String)> = Vec::new();
    let cwd_str = cwd.to_string_lossy().into_owned();
    if !cwd_str.is_empty() {
        pairs.push(("cwd", cwd_str));
    }
    if let Some(q) = prompt.map(str::trim).filter(|s| !s.is_empty()) {
        // Keep under Claude's documented 5000-char cap for `q`.
        let clipped: String = q.chars().take(5000).collect();
        pairs.push(("q", clipped));
    }
    let query = form_urlencoded::Serializer::new(String::new())
        .extend_pairs(pairs)
        .finish();
    if query.is_empty() {
        "claude-cli://open".to_string()
    } else {
        format!("claude-cli://open?{query}")
    }
}

/// Open an interactive session for a known runtime.
pub fn open_interactive_session(options: &OpenSessionOptions) -> Result<OpenSessionReport> {
    let runtime = normalize_runtime(&options.runtime);
    let cwd = resolve_session_cwd(options.cwd.as_deref());
    if !cwd.exists() {
        bail!("session cwd does not exist: {}", cwd.display());
    }

    match runtime.as_str() {
        "claude-code" => {
            open_claude_code(&cwd, options.prompt.as_deref(), options.prefer_deep_link)
        }
        "codex" => open_codex(&cwd, options.prompt.as_deref()),
        "hermes" => open_in_terminal("hermes", &["hermes"], &cwd, options.prompt.as_deref()),
        "openclaw" => open_in_terminal(
            "openclaw",
            &["openclaw", "tui"],
            &cwd,
            options.prompt.as_deref(),
        ),
        other => bail!("opening interactive sessions is not supported for runtime '{other}'"),
    }
}

fn open_claude_code(
    cwd: &Path,
    prompt: Option<&str>,
    prefer_deep_link: bool,
) -> Result<OpenSessionReport> {
    let refreshed = match resolve_claude_launch_env() {
        Some((url, key)) => apply_claude_code(&url, &key).ok().map(|_| url),
        None => None,
    };

    if prefer_deep_link {
        let link = claude_cli_deep_link(cwd, prompt);
        if open_url(&link).is_ok() {
            let detail = if let Some(url) = refreshed.as_deref() {
                format!(
                    "Opened Claude Code via claude-cli:// deep link after writing ANTHROPIC_BASE_URL={url} to ~/.claude/settings.json (prompt pre-filled, not auto-sent). Restart Claude if it was already running."
                )
            } else {
                "Opened Claude Code via claude-cli:// deep link (prompt pre-filled, not auto-sent)."
                    .into()
            };
            return Ok(OpenSessionReport {
                runtime: "claude-code".into(),
                method: OpenSessionMethod::DeepLink,
                cwd: cwd.display().to_string(),
                target: link,
                detail,
            });
        }
    }

    // Interactive CLI: wrap exports ANTHROPIC_* so the process does not hit api.anthropic.com.
    let _ = prompt;
    open_in_terminal("claude-code", &["claude"], cwd, None)
}

fn open_codex(cwd: &Path, prompt: Option<&str>) -> Result<OpenSessionReport> {
    let _ = clear_codex_placeholder_auth();
    // Mirror Claude: rewrite ~/.codex/config.toml before launch. Env OPENAI_BASE_URL alone is
    // not enough — Codex 0.14x still routes via model_provider / openai_base_url in config.toml,
    // and without that it silently hits api.openai.com (401 with company keys).
    let launch = resolve_codex_launch_env();
    let prefer_team_keys = launch
        .as_ref()
        .map(|(_, _, _, slot)| slot == CODEX_TEAM_SLOT)
        .unwrap_or(false);
    let refreshed = launch.and_then(|(url, key, model, slot)| {
        apply_codex_slot(&url, &key, model.as_deref(), Some(&slot))
            .ok()
            .filter(|r| r.applied)
            .map(|_| url)
    });

    let mut report =
        open_in_terminal_with_key_pref("codex", &["codex"], cwd, prompt, prefer_team_keys)?;
    if let Some(url) = refreshed {
        report.detail = format!(
            "Opened Codex after writing model_provider + openai_base_url={url} to ~/.codex/config.toml. {}",
            report.detail
        );
    }
    Ok(report)
}

/// Resolve OpenAI-compatible gateway + key (+ optional model/slot) from active overlays.
///
/// When the active personal host cannot speak Codex Responses API (e.g. DeepSeek),
/// fall back to the durable team baseline so launch does not silently use api.openai.com.
fn resolve_codex_launch_env() -> Option<(String, String, Option<String>, String)> {
    let env = collect_launch_env_map();
    if let Some(launch) = codex_launch_from_env(&env) {
        if launch.3 == CODEX_PERSONAL_SLOT && !codex_host_supports_responses_api(&launch.0) {
            if let Some(team) = codex_launch_from_company_baseline() {
                return Some(team);
            }
        }
        return Some(launch);
    }
    codex_launch_from_company_baseline()
}

fn codex_launch_from_company_baseline() -> Option<(String, String, Option<String>, String)> {
    let path = company_baseline_path().filter(|path| path.exists())?;
    let map = read_env_map(&path).ok()?;
    let (url, key, model, _) = codex_launch_from_env(&map)?;
    Some((url, key, model, CODEX_TEAM_SLOT.to_string()))
}

fn collect_launch_env_map() -> HashMap<String, String> {
    let mut env = HashMap::new();
    if let Some(path) = evotown_agent_env_path().filter(|path| path.exists()) {
        if let Ok(map) = read_env_map(&path) {
            env.extend(map);
        }
    }
    if let Ok(path) = active_env_path() {
        if path.exists() {
            if let Ok(map) = read_env_map(&path) {
                env.extend(map);
            }
        }
    }
    if let Some(path) = agent_profile_path().filter(|path| path.exists()) {
        if let Ok(map) = read_env_map(&path) {
            env.extend(map);
        }
    }
    env
}

fn codex_launch_from_env(
    env: &HashMap<String, String>,
) -> Option<(String, String, Option<String>, String)> {
    let gateway_url = env
        .get(GATEWAY_URL_ENV)
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(str::to_string)?;

    let api_key = [COMPANY_API_KEY_ENV, EVOTOWN_API_KEY_ENV, "OPENAI_API_KEY"]
        .into_iter()
        .find_map(|key| {
            env.get(key)
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        })?;

    let model = env
        .get(MODEL_ENV)
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(str::to_string);

    let slot = match env.get(PROVIDER_KIND_ENV).map(|v| v.trim()) {
        Some(PROVIDER_KIND_PERSONAL) => CODEX_PERSONAL_SLOT.to_string(),
        Some(PROVIDER_KIND_COMPANY) => CODEX_TEAM_SLOT.to_string(),
        _ => {
            // Legacy profiles omit PROVIDER_KIND — infer from gateway host.
            if gateway_url
                .to_ascii_lowercase()
                .contains("api.deepseek.com")
            {
                CODEX_PERSONAL_SLOT.to_string()
            } else {
                CODEX_TEAM_SLOT.to_string()
            }
        }
    };

    Some((gateway_url, api_key, model, slot))
}

/// Resolve Anthropic base URL + key from the active overlay / Evotown env.
fn resolve_claude_launch_env() -> Option<(String, String)> {
    anthropic_launch_from_env(&collect_launch_env_map())
}

fn anthropic_launch_from_env(env: &HashMap<String, String>) -> Option<(String, String)> {
    let api_key = [
        "ANTHROPIC_API_KEY",
        COMPANY_API_KEY_ENV,
        EVOTOWN_API_KEY_ENV,
        "OPENAI_API_KEY",
    ]
    .into_iter()
    .find_map(|key| {
        env.get(key)
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })?;

    if let Some(url) = env
        .get("ANTHROPIC_BASE_URL")
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
    {
        return Some((url.to_string(), api_key));
    }

    let protocol = env
        .get(PROVIDER_PROTOCOL_ENV)
        .map(|value| normalize_protocol(value));
    if protocol.as_deref() == Some(PROTOCOL_ANTHROPIC) {
        if let Some(url) = env
            .get(GATEWAY_URL_ENV)
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
        {
            return Some((url.to_string(), api_key));
        }
    }

    let evotown = env
        .get("AGENT_DOCTOR_EVOTOWN_URL")
        .or_else(|| env.get(EVOTOWN_URL_ENV))
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())?;
    Some((anthropic_gateway_url_from_evotown_base(evotown), api_key))
}

fn open_in_terminal(
    runtime: &str,
    argv: &[&str],
    cwd: &Path,
    prompt: Option<&str>,
) -> Result<OpenSessionReport> {
    open_in_terminal_with_key_pref(runtime, argv, cwd, prompt, false)
}

fn open_in_terminal_with_key_pref(
    runtime: &str,
    argv: &[&str],
    cwd: &Path,
    _prompt: Option<&str>,
    prefer_team_keys: bool,
) -> Result<OpenSessionReport> {
    if argv.is_empty() {
        bail!("empty terminal command for {runtime}");
    }
    let command_line = wrap_with_company_env_pref(&shell_join(argv), prefer_team_keys);
    launch_system_terminal(cwd, &command_line)?;
    Ok(OpenSessionReport {
        runtime: runtime.into(),
        method: OpenSessionMethod::Terminal,
        cwd: cwd.display().to_string(),
        target: command_line.clone(),
        detail: format!(
            "Opened system terminal in {} running `{command_line}`.",
            cwd.display()
        ),
    })
}

/// Prefix a shell command so Evotown / company API keys and workspace env are available.
/// Prefer sourcing env files (never inline secrets into the displayed command).
fn wrap_with_company_env_pref(command: &str, prefer_team_keys: bool) -> String {
    let _ = ensure_profile_env_from_evotown();

    #[cfg(not(windows))]
    {
        let mut parts: Vec<String> = Vec::new();
        // Workspace first so CODEX_HOME / HERMES_HOME isolation applies.
        if let Ok(path) = active_env_path() {
            if path.exists() {
                parts.push(format!(". {}", shell_single_quote(&path.to_string_lossy())));
            }
        }
        if let Some(path) = agent_profile_path().filter(|path| path.exists()) {
            parts.push(format!(". {}", shell_single_quote(&path.to_string_lossy())));
        }
        if let Some(path) = evotown_agent_env_path().filter(|path| path.exists()) {
            parts.push(format!(". {}", shell_single_quote(&path.to_string_lossy())));
        }
        // When Codex is forced onto the team slot (e.g. personal DeepSeek cannot speak
        // Responses API), re-source the durable baseline last so EVOTOWN/COMPANY keys win.
        if prefer_team_keys {
            if let Some(path) = company_baseline_path().filter(|path| path.exists()) {
                parts.push(format!(". {}", shell_single_quote(&path.to_string_lossy())));
            }
        }
        if parts.is_empty() {
            return command.to_string();
        }
        // Codex reads OPENAI_BASE_URL / OPENAI_API_KEY; Claude Code reads
        // ANTHROPIC_BASE_URL / ANTHROPIC_API_KEY. profile.env uses AGENT_DOCTOR_*.
        format!(
            "set -a && {} && set +a && {} && {command}",
            parts.join(" && "),
            unix_key_and_base_exports()
        )
    }

    #[cfg(windows)]
    {
        let _ = prefer_team_keys;
        let profile = read_company_profile().ok().flatten();
        let api_key = profile
            .as_ref()
            .and_then(|p| p.api_key.clone())
            .filter(|key| !key.trim().is_empty())
            .or_else(|| {
                load_evotown_config()
                    .ok()
                    .map(|config| config.api_key)
                    .filter(|key| !key.trim().is_empty())
            });
        let gateway = profile
            .as_ref()
            .and_then(|p| p.gateway_url.clone())
            .filter(|url| !url.trim().is_empty());
        match (api_key, gateway) {
            (Some(key), Some(url)) => {
                let escaped = key.replace('\'', "''");
                let escaped_url = url.replace('\'', "''");
                let anthropic_url = anthropic_gateway_url_from_evotown_base(
                    &crate::setup::evotown_base_from_gateway(&url),
                );
                let escaped_anthropic = anthropic_url.replace('\'', "''");
                format!(
                    "$env:OPENAI_API_KEY='{escaped}'; $env:{EVOTOWN_API_KEY_ENV}='{escaped}'; $env:{COMPANY_API_KEY_ENV}='{escaped}'; $env:OPENAI_BASE_URL='{escaped_url}'; $env:ANTHROPIC_API_KEY='{escaped}'; $env:ANTHROPIC_BASE_URL='{escaped_anthropic}'; {command}"
                )
            }
            (Some(key), None) => {
                let escaped = key.replace('\'', "''");
                format!(
                    "$env:OPENAI_API_KEY='{escaped}'; $env:{EVOTOWN_API_KEY_ENV}='{escaped}'; $env:{COMPANY_API_KEY_ENV}='{escaped}'; $env:ANTHROPIC_API_KEY='{escaped}'; {command}"
                )
            }
            _ => command.to_string(),
        }
    }
}

#[cfg(not(windows))]
fn unix_key_and_base_exports() -> String {
    format!(
        "export {COMPANY_API_KEY_ENV}=\"${{{COMPANY_API_KEY_ENV}:-${{EVOTOWN_API_KEY:-$OPENAI_API_KEY}}}}\" && \
export OPENAI_API_KEY=\"${{{COMPANY_API_KEY_ENV}:-${{EVOTOWN_API_KEY:-$OPENAI_API_KEY}}}}\" && \
export {EVOTOWN_API_KEY_ENV}=\"${{{EVOTOWN_API_KEY_ENV}:-${{{COMPANY_API_KEY_ENV}:-$OPENAI_API_KEY}}}}\" && \
export OPENAI_BASE_URL=\"${{{GATEWAY_URL_ENV}:-$OPENAI_BASE_URL}}\" && \
export ANTHROPIC_API_KEY=\"${{ANTHROPIC_API_KEY:-${{{COMPANY_API_KEY_ENV}:-${{EVOTOWN_API_KEY:-$OPENAI_API_KEY}}}}}}\" && \
_ad_ev=\"${{AGENT_DOCTOR_EVOTOWN_URL:-$EVOTOWN_URL}}\" && \
export ANTHROPIC_BASE_URL=\"${{ANTHROPIC_BASE_URL:-${{_ad_ev:+${{_ad_ev%/}}/api/gateway/anthropic}}}}\" && \
unset _ad_ev"
    )
}

/// If Evotown is configured but profile.env is missing, recreate it so Doctor/Codex share one key.
fn ensure_profile_env_from_evotown() -> anyhow::Result<()> {
    let Some(path) = agent_profile_path() else {
        return Ok(());
    };
    if path.exists() {
        return Ok(());
    }
    let config = load_evotown_config()?;
    let gateway = format!("{}/api/gateway/v1", config.base_url.trim_end_matches('/'));
    write_company_profile_with_gateway(&path, &gateway, &config.api_key, &config.base_url)?;
    Ok(())
}

fn open_url(url: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        let status = Command::new("open")
            .arg(url)
            .status()
            .context("failed to run `open` for deep link")?;
        if !status.success() {
            bail!("`open` exited with {status}");
        }
        Ok(())
    }
    #[cfg(target_os = "linux")]
    {
        let status = Command::new("xdg-open")
            .arg(url)
            .status()
            .context("failed to run `xdg-open` for deep link")?;
        if !status.success() {
            bail!("`xdg-open` exited with {status}");
        }
        Ok(())
    }
    #[cfg(target_os = "windows")]
    {
        let status = Command::new("cmd")
            .args(["/C", "start", "", url])
            .status()
            .context("failed to run `start` for deep link")?;
        if !status.success() {
            bail!("`start` exited with {status}");
        }
        Ok(())
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        bail!("deep link open is not supported on this platform");
    }
}

fn launch_system_terminal(cwd: &Path, command_line: &str) -> Result<()> {
    let cwd_str = cwd.to_string_lossy();
    #[cfg(target_os = "macos")]
    {
        // Prefer Terminal.app script so cwd + command stay interactive.
        let script = format!(
            "tell application \"Terminal\" to do script \"cd {cwd} && {cmd}\"",
            cwd = escape_applescript(&cwd_str),
            cmd = escape_applescript(command_line),
        );
        let status = Command::new("osascript")
            .args(["-e", &script])
            .status()
            .context("failed to launch Terminal.app")?;
        if !status.success() {
            bail!("osascript exited with {status}");
        }
        Ok(())
    }
    #[cfg(target_os = "linux")]
    {
        let shell_cmd = format!(
            "cd {} && exec {}",
            shell_single_quote(&cwd_str),
            command_line
        );
        let candidates: &[(&str, &[&str])] = &[
            ("x-terminal-emulator", &["-e", "bash", "-lc"]),
            ("gnome-terminal", &["--", "bash", "-lc"]),
            ("konsole", &["-e", "bash", "-lc"]),
            ("xterm", &["-e", "bash", "-lc"]),
        ];
        for (bin, prefix) in candidates {
            let mut cmd = Command::new(bin);
            cmd.args(*prefix).arg(&shell_cmd);
            if cmd.spawn().is_ok() {
                return Ok(());
            }
        }
        bail!("no known terminal emulator found (tried x-terminal-emulator, gnome-terminal, konsole, xterm)");
    }
    #[cfg(target_os = "windows")]
    {
        let cd = cwd_str.replace('\'', "''");
        let cmd = command_line.replace('\'', "''");
        let ps = format!("Set-Location -LiteralPath '{cd}'; {cmd}");
        let status = Command::new("wt")
            .args([
                "-d",
                cwd_str.as_ref(),
                "powershell",
                "-NoExit",
                "-Command",
                &ps,
            ])
            .status();
        if status.map(|s| s.success()).unwrap_or(false) {
            return Ok(());
        }
        let status = Command::new("powershell")
            .args([
                "-NoExit",
                "-Command",
                &format!("Set-Location -LiteralPath '{cd}'; {cmd}"),
            ])
            .status()
            .context("failed to launch PowerShell")?;
        if !status.success() {
            bail!("PowerShell exited with {status}");
        }
        Ok(())
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        let _ = (cwd, command_line);
        bail!("system terminal launch is not supported on this platform");
    }
}

fn shell_join(argv: &[&str]) -> String {
    argv.iter()
        .map(|part| {
            if part.is_empty()
                || part.contains(|c: char| c.is_whitespace() || "\"'\\$`".contains(c))
            {
                shell_single_quote(part)
            } else {
                (*part).to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(target_os = "macos")]
fn escape_applescript(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

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
    fn wraps_command_with_exported_key_when_no_profile_env() {
        // Without a profile file, wrap still returns a runnable command.
        let wrapped = wrap_with_company_env_pref("codex", false);
        assert!(wrapped.contains("codex"));
    }

    #[test]
    fn wrap_exports_openai_base_url_alias() {
        let wrapped = wrap_with_company_env_pref("codex", false);
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
        let wrapped = wrap_with_company_env_pref("claude", false);
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
    fn anthropic_launch_skips_personal_openai_without_evotown() {
        let env = HashMap::from([
            (PROVIDER_PROTOCOL_ENV.into(), "openai".into()),
            (GATEWAY_URL_ENV.into(), "https://api.deepseek.com/v1".into()),
            (COMPANY_API_KEY_ENV.into(), "sk-ds".into()),
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
}
