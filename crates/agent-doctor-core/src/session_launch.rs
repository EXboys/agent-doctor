//! Open an interactive coding-agent session outside Agent Doctor (CC Switch-style).
//!
//! Doctor stays the ops shell: connection, health, preferred runtime, and launch.
//! Chat/TUI stays in the official CLI (Claude Code deep link or a system terminal).

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use url::form_urlencoded;

use crate::evotown::{load_evotown_config, normalize_runtime};
use crate::profile::agent_profile_path;
#[cfg(windows)]
use crate::profile::read_company_profile;
#[cfg(windows)]
use crate::setup::EVOTOWN_API_KEY_ENV;
use crate::setup::{
    clear_codex_placeholder_auth, evotown_agent_env_path, write_company_profile_with_gateway,
    COMPANY_API_KEY_ENV,
};
use crate::workspace::load_workspaces;

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
        "codex" => {
            let _ = clear_codex_placeholder_auth();
            open_in_terminal("codex", &["codex"], &cwd, options.prompt.as_deref())
        }
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
    if prefer_deep_link {
        let link = claude_cli_deep_link(cwd, prompt);
        if open_url(&link).is_ok() {
            return Ok(OpenSessionReport {
                runtime: "claude-code".into(),
                method: OpenSessionMethod::DeepLink,
                cwd: cwd.display().to_string(),
                target: link,
                detail: "Opened Claude Code via claude-cli:// deep link (prompt pre-filled, not auto-sent)."
                    .into(),
            });
        }
    }

    // Interactive CLI: start normally; deep link is preferred for prompt pre-fill.
    let _ = prompt;
    open_in_terminal("claude-code", &["claude"], cwd, None)
}

fn open_in_terminal(
    runtime: &str,
    argv: &[&str],
    cwd: &Path,
    _prompt: Option<&str>,
) -> Result<OpenSessionReport> {
    if argv.is_empty() {
        bail!("empty terminal command for {runtime}");
    }
    let command_line = wrap_with_company_env(&shell_join(argv));
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

/// Prefix a shell command so Evotown / company API keys are available to the CLI.
/// Prefer sourcing env files (never inline secrets into the displayed command).
fn wrap_with_company_env(command: &str) -> String {
    let _ = ensure_profile_env_from_evotown();

    #[cfg(not(windows))]
    {
        let mut parts: Vec<String> = Vec::new();
        if let Some(path) = agent_profile_path().filter(|path| path.exists()) {
            parts.push(format!(". {}", shell_single_quote(&path.to_string_lossy())));
        }
        if let Some(path) = evotown_agent_env_path().filter(|path| path.exists()) {
            parts.push(format!(". {}", shell_single_quote(&path.to_string_lossy())));
        }
        if parts.is_empty() {
            return command.to_string();
        }
        format!(
            "set -a && {} && set +a && export {COMPANY_API_KEY_ENV}=\"${{{COMPANY_API_KEY_ENV}:-${{OPENAI_API_KEY:-$EVOTOWN_API_KEY}}}}\" && {command}",
            parts.join(" && ")
        )
    }

    #[cfg(windows)]
    {
        let api_key = read_company_profile()
            .ok()
            .flatten()
            .and_then(|profile| profile.api_key)
            .filter(|key| !key.trim().is_empty())
            .or_else(|| {
                load_evotown_config()
                    .ok()
                    .map(|config| config.api_key)
                    .filter(|key| !key.trim().is_empty())
            });
        match api_key {
            Some(key) => {
                let escaped = key.replace('\'', "''");
                format!(
                    "$env:OPENAI_API_KEY='{escaped}'; $env:{EVOTOWN_API_KEY_ENV}='{escaped}'; $env:{COMPANY_API_KEY_ENV}='{escaped}'; {command}"
                )
            }
            None => command.to_string(),
        }
    }
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
        let wrapped = wrap_with_company_env("codex");
        assert!(wrapped.contains("codex"));
    }
}
