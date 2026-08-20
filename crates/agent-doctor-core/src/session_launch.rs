//! Open an interactive coding-agent session outside Agent Doctor (CC Switch-style).
//!
//! Doctor stays the ops shell: connection, health, preferred runtime, and launch.
//! Chat/TUI stays in the official CLI (Claude Code deep link or a system terminal).

use std::collections::HashMap;
#[cfg(target_os = "macos")]
use std::fs::{self, OpenOptions};
#[cfg(target_os = "macos")]
use std::io::Write;
#[cfg(target_os = "macos")]
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
#[cfg(target_os = "macos")]
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use url::form_urlencoded;

use crate::evotown::{load_evotown_config, normalize_runtime};
#[cfg(windows)]
use crate::profile::read_company_profile;
use crate::profile::{
    agent_profile_path, read_env_map, GATEWAY_URL_ENV, PROVIDER_KIND_COMPANY, PROVIDER_KIND_ENV,
    PROVIDER_KIND_PERSONAL,
};
use crate::setup::merge::{
    apply_claude_code, apply_codex_slot, codex_slot_display_name, codex_slot_env_key,
    CODEX_PERSONAL_SLOT, CODEX_TEAM_SLOT,
};
use crate::setup::{
    anthropic_gateway_url_from_evotown_base, clear_codex_chatgpt_auth_for_gateway,
    clear_codex_placeholder_auth, evotown_agent_env_path, normalize_protocol,
    write_company_profile_with_gateway, COMPANY_API_KEY_ENV, EVOTOWN_API_KEY_ENV, EVOTOWN_URL_ENV,
    MODEL_ENV, PROTOCOL_ANTHROPIC, PROVIDER_PROTOCOL_ENV,
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
        "deepseek-harness" => open_in_terminal("deepseek-harness", &["dsh", "web"], &cwd, None),
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
    // ChatGPT login tokens in auth.json make Codex ignore gateway keys and hit api.openai.com.
    if launch.is_some() {
        let _ = clear_codex_chatgpt_auth_for_gateway();
    }
    let refreshed = launch.as_ref().and_then(|(url, key, model, slot)| {
        apply_codex_slot(url, key, model.as_deref(), Some(slot))
            .ok()
            .filter(|r| r.applied)
            .map(|_| url.clone())
    });

    // Also pass -c overrides so this process cannot fall back to built-in openai
    // even if ~/.codex was stale, CODEX_HOME pointed elsewhere, or the user never
    // re-ran mode switch. The launched command line itself proves the gateway.
    let argv = codex_launch_argv(launch.as_ref());
    let argv_refs: Vec<&str> = argv.iter().map(String::as_str).collect();
    let mut report = open_in_terminal("codex", &argv_refs, cwd, prompt)?;
    if let Some(url) = refreshed {
        report.detail = format!(
            "Opened Codex after writing model_provider + openai_base_url={url} to ~/.codex/config.toml. {}",
            report.detail
        );
    }
    Ok(report)
}

/// Build a self-contained `codex -c …` invocation that defines the whole slot
/// provider table inline, so launch does not depend on `~/.codex/config.toml`
/// being present or current.
///
/// Do NOT route through Codex's built-in `openai` provider: it carries
/// `supports_websockets` and `requires_openai_auth`, which make third-party
/// gateways reject the handshake with 401 on `wss://<host>/v1/responses`.
/// The inline table mirrors exactly what `write_codex_provider_config` writes.
fn codex_launch_argv(launch: Option<&(String, String, Option<String>, String)>) -> Vec<String> {
    let Some((url, _, model, slot)) = launch else {
        return vec!["codex".to_string()];
    };
    let mut argv = vec!["codex".to_string()];
    let mut set = |key: &str, value: String| {
        argv.push("-c".to_string());
        argv.push(format!("{key}={value}"));
    };
    let prefix = format!("model_providers.{slot}");
    set(
        &format!("{prefix}.name"),
        toml_string(codex_slot_display_name(slot)),
    );
    set(&format!("{prefix}.base_url"), toml_string(url));
    set(
        &format!("{prefix}.env_key"),
        toml_string(codex_slot_env_key(slot)),
    );
    set(&format!("{prefix}.wire_api"), toml_string("responses"));
    set(
        &format!("{prefix}.requires_openai_auth"),
        "false".to_string(),
    );
    set(
        &format!("{prefix}.supports_websockets"),
        "false".to_string(),
    );
    set("model_provider", toml_string(slot));
    // Only pin the model when the active profile names one; otherwise leave the
    // user's own `model` in config.toml alone.
    if let Some(model_id) = model.as_deref().map(str::trim).filter(|m| !m.is_empty()) {
        set("model", toml_string(model_id));
    }
    argv
}

/// Quote a value as a TOML basic string so `codex -c key=value` parses it as a string.
fn toml_string(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Resolve OpenAI-compatible gateway + key (+ optional model/slot) from active overlays.
fn resolve_codex_launch_env() -> Option<(String, String, Option<String>, String)> {
    codex_launch_from_env(&collect_launch_env_map())
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
    _prompt: Option<&str>,
) -> Result<OpenSessionReport> {
    if argv.is_empty() {
        bail!("empty terminal command for {runtime}");
    }
    let resolved = resolve_launch_argv(argv);
    let refs: Vec<&str> = resolved.iter().map(String::as_str).collect();
    let command_line = wrap_with_company_env(&shell_join(&refs));
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

fn resolve_launch_argv(argv: &[&str]) -> Vec<String> {
    let mut resolved: Vec<String> = argv.iter().map(|part| (*part).to_string()).collect();
    if let Some(bin) = resolved.first_mut() {
        if let Some(path) = crate::adapters::util::find_binary(bin) {
            *bin = path.to_string_lossy().into_owned();
        }
    }
    resolved
}

/// Prefix a shell command so Evotown / company API keys and workspace env are available.
/// Prefer sourcing env files (never inline secrets into the displayed command).
fn wrap_with_company_env(command: &str) -> String {
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
    #[cfg(target_os = "macos")]
    {
        // Terminal.app's AppleScript `do script` truncates long command strings
        // (commonly at 1024 bytes). Codex provider overrides can exceed that, so
        // put the full command in a private temporary script and only send its
        // short path through AppleScript. The script removes itself immediately.
        let launch_script = write_macos_terminal_launch_script(cwd, command_line)?;
        let invoke_script = shell_single_quote(&launch_script.to_string_lossy());
        let script = format!(
            "tell application \"Terminal\" to do script \"{cmd}\"",
            cmd = escape_applescript(&invoke_script),
        );
        let status = match Command::new("osascript").args(["-e", &script]).status() {
            Ok(status) => status,
            Err(error) => {
                let _ = fs::remove_file(&launch_script);
                return Err(error).context("failed to launch Terminal.app");
            }
        };
        if !status.success() {
            let _ = fs::remove_file(&launch_script);
            bail!("osascript exited with {status}");
        }
        Ok(())
    }
    #[cfg(target_os = "linux")]
    {
        let cwd_str = cwd.to_string_lossy();
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
        let cwd_str = cwd.to_string_lossy();
        let cd = cwd_str.replace('\'', "''");
        let cmd = command_line.replace('\'', "''");
        let ps = format!("Set-Location -LiteralPath '{cd}'; {cmd}");
        // `--` stops Windows Terminal from treating `;` inside -Command as
        // pane/tab separators (which surfaces as 0x80070002 / 找不到指定的文件).
        let status = Command::new("wt")
            .args([
                "-d",
                cwd_str.as_ref(),
                "--",
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

#[cfg(target_os = "macos")]
fn write_macos_terminal_launch_script(cwd: &Path, command_line: &str) -> Result<PathBuf> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let dir = std::env::temp_dir();

    for attempt in 0..16 {
        let path = dir.join(format!(
            "agent-doctor-terminal-{}-{nonce}-{attempt}.sh",
            std::process::id()
        ));
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o700)
            .open(&path);
        let mut file = match file {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(error).context("failed to create terminal launch script");
            }
        };

        let script_path = shell_single_quote(&path.to_string_lossy());
        let cwd = shell_single_quote(&cwd.to_string_lossy());
        if let Err(error) = writeln!(
            file,
            "#!/bin/sh\nrm -f -- {script_path}\ncd {cwd} && {command_line}"
        ) {
            let _ = fs::remove_file(&path);
            return Err(error).context("failed to write terminal launch script");
        }
        return Ok(path);
    }

    bail!("failed to allocate a unique terminal launch script")
}

fn shell_join(argv: &[&str]) -> String {
    #[cfg(windows)]
    {
        return argv
            .iter()
            .enumerate()
            .map(|(index, part)| {
                if index == 0 {
                    let lower = part.to_ascii_lowercase();
                    if lower.ends_with(".cmd")
                        || lower.ends_with(".bat")
                        || part.contains(char::is_whitespace)
                    {
                        return format!("& '{}'", part.replace('\'', "''"));
                    }
                    return (*part).to_string();
                }
                if part.is_empty() || part.contains(char::is_whitespace) || part.contains('\'') {
                    format!("'{}'", part.replace('\'', "''"))
                } else {
                    (*part).to_string()
                }
            })
            .collect::<Vec<_>>()
            .join(" ");
    }
    #[cfg(not(windows))]
    {
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
    fn shell_join_preserves_official_dsh_web_entrypoint() {
        assert_eq!(shell_join(&["dsh", "web"]), "dsh web");
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
