#[cfg(any(target_os = "macos", target_os = "windows"))]
use std::fs;
#[cfg(target_os = "macos")]
use std::fs::OpenOptions;
#[cfg(any(target_os = "macos", target_os = "windows"))]
use std::io::Write;
#[cfg(target_os = "macos")]
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
#[cfg(any(target_os = "macos", target_os = "windows"))]
use std::path::PathBuf;
use std::process::Command;
#[cfg(target_os = "windows")]
use std::process::Stdio;
#[cfg(any(target_os = "macos", target_os = "windows"))]
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};

use crate::evotown::load_evotown_config;
#[cfg(windows)]
use crate::profile::read_company_profile;
use crate::profile::{agent_profile_path, GATEWAY_URL_ENV};
#[cfg(windows)]
use crate::setup::anthropic_gateway_url_from_evotown_base;
use crate::setup::{
    evotown_agent_env_path, write_company_profile_with_gateway, COMPANY_API_KEY_ENV,
    EVOTOWN_API_KEY_ENV,
};
use crate::workspace::active_env_path;

use super::*;

pub(crate) fn open_in_terminal(
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

pub(crate) fn resolve_launch_argv(argv: &[&str]) -> Vec<String> {
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
pub(crate) fn wrap_with_company_env(command: &str) -> String {
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
pub(crate) fn unix_key_and_base_exports() -> String {
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
pub(crate) fn ensure_profile_env_from_evotown() -> anyhow::Result<()> {
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

pub(crate) fn open_url(url: &str) -> Result<()> {
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
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        // `status()` can wait on the "choose an app" dialog if claude-cli://
        // is unregistered, which froze the desktop on "Opening…".
        Command::new("cmd")
            .args(["/C", "start", "", url])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .context("failed to run `start` for deep link")?;
        Ok(())
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        bail!("deep link open is not supported on this platform");
    }
}

pub(crate) fn launch_system_terminal(cwd: &Path, command_line: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        // Prefer `open … .command` over AppleScript:
        // 1) Launch Services brings Terminal.app to the front (Ask window stays on top
        //    otherwise, so users think "打开终端" did nothing).
        // 2) No Automation permission is required to control Terminal.
        // Terminal.app's AppleScript `do script` also truncates long strings (~1024
        // bytes); the self-deleting script keeps Codex overrides intact.
        let launch_script = write_macos_terminal_launch_script(cwd, command_line)?;
        if Command::new("open")
            .arg(&launch_script)
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
        {
            // Nudge Terminal forward in case Ask / another always-focused window
            // kept covering the new tab.
            let _ = Command::new("open").args(["-a", "Terminal"]).status();
            return Ok(());
        }

        // Fallback: AppleScript + activate (needs Automation permission).
        let invoke_script = shell_single_quote(&launch_script.to_string_lossy());
        let script = format!(
            "tell application \"Terminal\"\nactivate\ndo script \"{cmd}\"\nend tell",
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
        let _ = Command::new("open").args(["-a", "Terminal"]).status();
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
        launch_windows_terminal(cwd, command_line)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        let _ = (cwd, command_line);
        bail!("system terminal launch is not supported on this platform");
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn write_macos_terminal_launch_script(
    cwd: &Path,
    command_line: &str,
) -> Result<PathBuf> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let dir = std::env::temp_dir();

    for attempt in 0..16 {
        // `.command` is opened by Terminal.app via Launch Services (`open`), which
        // focuses the terminal — unlike a bare `.sh` handed only to AppleScript.
        let path = dir.join(format!(
            "agent-doctor-terminal-{}-{nonce}-{attempt}.command",
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

/// GUI apps must not wait on a console process, and must not pass `$env:…; cmd`
/// through `wt` / `cmd /C` — both treat `;` as extra windows.
#[cfg(target_os = "windows")]
pub(crate) fn launch_windows_terminal(cwd: &Path, command_line: &str) -> Result<()> {
    use std::os::windows::process::CommandExt;
    const CREATE_NEW_CONSOLE: u32 = 0x00000010;

    let script = write_windows_terminal_launch_script(cwd, command_line)?;
    let script_arg = script.to_string_lossy().into_owned();
    let powershell = windows_powershell_exe();
    let mut cmd = Command::new(&powershell);
    cmd.args([
        "-NoLogo",
        "-NoProfile",
        "-ExecutionPolicy",
        "Bypass",
        "-File",
        &script_arg,
    ])
    .creation_flags(CREATE_NEW_CONSOLE);
    match cmd.spawn() {
        Ok(_) => Ok(()),
        Err(error) => {
            let _ = fs::remove_file(&script);
            Err(error).context("failed to launch PowerShell")
        }
    }
}

#[cfg(target_os = "windows")]
pub(crate) fn windows_powershell_exe() -> PathBuf {
    std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
        .join(r"System32\WindowsPowerShell\v1.0\powershell.exe")
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(crate) fn render_windows_terminal_script(
    script_path: &str,
    cwd: &str,
    command_line: &str,
) -> String {
    format!(
        "Remove-Item -LiteralPath '{script_path}' -Force -ErrorAction SilentlyContinue\r\n\
         Set-Location -LiteralPath '{cwd}'\r\n\
         Write-Host 'Agent Doctor: starting session...'\r\n\
         Write-Host ''\r\n\
         $script:adFailed = $false\r\n\
         try {{\r\n\
         {command_line}\r\n\
         if ($null -ne $LASTEXITCODE -and $LASTEXITCODE -ne 0) {{ $script:adFailed = $true }}\r\n\
         }} catch {{\r\n\
         Write-Host $_\r\n\
         $script:adFailed = $true\r\n\
         }}\r\n\
         Write-Host ''\r\n\
         if ($script:adFailed) {{\r\n\
         Write-Host 'The command exited with an error. Press Enter to close this window.'\r\n\
         }} else {{\r\n\
         Write-Host 'Session ended. Press Enter to close this window.'\r\n\
         }}\r\n\
         [void][System.Console]::ReadLine()\r\n"
    )
}

#[cfg(target_os = "windows")]
pub(crate) fn write_windows_terminal_launch_script(
    cwd: &Path,
    command_line: &str,
) -> Result<PathBuf> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let dir = std::env::temp_dir();
    let cwd = cwd.to_string_lossy().replace('\'', "''");
    for attempt in 0..16 {
        let path = dir.join(format!(
            "agent-doctor-terminal-{}-{nonce}-{attempt}.ps1",
            std::process::id()
        ));
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                let script_path = path.to_string_lossy().replace('\'', "''");
                if let Err(error) = write!(
                    file,
                    "{}",
                    render_windows_terminal_script(&script_path, &cwd, command_line)
                ) {
                    let _ = fs::remove_file(&path);
                    return Err(error).context("failed to write terminal launch script");
                }
                return Ok(path);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(error).context("failed to create terminal launch script");
            }
        }
    }
    bail!("failed to allocate a unique terminal launch script")
}

pub(crate) fn shell_join(argv: &[&str]) -> String {
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

pub(crate) fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(target_os = "macos")]
pub(crate) fn escape_applescript(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}
