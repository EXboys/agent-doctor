//! Hermes Agent install/update via the official Nous Research installer.
//!
//! macOS, Linux, and WSL use `https://hermes-agent.nousresearch.com/install.sh`.
//! Windows uses `https://hermes-agent.nousresearch.com/install.ps1`.

use anyhow::{Context, Result};
#[cfg(not(target_os = "windows"))]
use base64::Engine;

use super::download_route::use_china_mirrors;
use super::runner::run_shell_command;

pub const HERMES_INSTALL_SCRIPT_URL: &str = "https://hermes-agent.nousresearch.com/install.sh";

#[cfg(target_os = "windows")]
pub const HERMES_INSTALL_PS1_URL: &str = "https://hermes-agent.nousresearch.com/install.ps1";

/// Unix install: curl to temp file, then bash (not `curl | bash` — safer under WSL/sub-shells).
#[cfg(not(target_os = "windows"))]
const HERMES_FETCH_UNIX: &str =
    "curl -fsSL https://hermes-agent.nousresearch.com/install.sh -o $tmp";

#[cfg(not(target_os = "windows"))]
const HERMES_FETCH_UNIX_CHINA: &str =
    "curl -fsSL --max-time 60 https://hermes-agent.nousresearch.com/install.sh -o $tmp";

/// Save the official `install.ps1` to a file (its GitHub copy through the proxy
/// when the official host is unreachable) and run it as a script, so it can
/// exit with a code and be retried.
///
/// In mainland China the installer still reaches GitHub twice on its own: the
/// source clone and uv's Python download. Those two are pointed at the proxy;
/// every other artifact already falls back to `hermes-assets.nousresearch.com`,
/// and npm reads the mirror set by `apply_download_env`. The proxy rate-limits
/// git, so a failed run is repeated once on the original addresses.
const HERMES_WINDOWS_INSTALL_PS1: &str = r#"
$sources = @('https://hermes-agent.nousresearch.com/install.ps1',
             'https://gh-proxy.com/https://raw.githubusercontent.com/NousResearch/hermes-agent/main/scripts/install.ps1')
$script = Join-Path $env:TEMP ('hermes-install-' + [guid]::NewGuid().ToString('N') + '.ps1')
$saved = $false
foreach ($source in $sources) {
    try {
        Invoke-WebRequest -Uri $source -OutFile $script -UseBasicParsing -TimeoutSec 60 -ErrorAction Stop
        $saved = $true
        break
    } catch {
        Write-Output '安装脚本下载不下来，换一个地址再试…'
    }
}
if (-not $saved) { exit 4 }
$china = '__CHINA__' -eq '1'
if ($china) {
    $env:HERMES_REPO_URL = 'https://gh-proxy.com/https://github.com/NousResearch/hermes-agent.git'
    $env:UV_PYTHON_INSTALL_MIRROR = 'https://gh-proxy.com/https://github.com/astral-sh/python-build-standalone/releases/download'
}
& powershell -NoProfile -ExecutionPolicy Bypass -File $script -NonInteractive
$code = $LASTEXITCODE
if ($code -ne 0 -and $china) {
    Write-Output '国内地址装不上，改用原来的地址再试一次…'
    foreach ($name in @('HERMES_REPO_URL', 'UV_PYTHON_INSTALL_MIRROR', 'GIT_CONFIG_COUNT',
                        'GIT_CONFIG_KEY_0', 'GIT_CONFIG_VALUE_0')) {
        Remove-Item -Path "Env:$name" -ErrorAction SilentlyContinue
    }
    & powershell -NoProfile -ExecutionPolicy Bypass -File $script -NonInteractive
    $code = $LASTEXITCODE
}
Remove-Item -LiteralPath $script -Force -ErrorAction SilentlyContinue
exit $code
"#;

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn windows_install_shell_command(china: bool) -> String {
    let script = HERMES_WINDOWS_INSTALL_PS1.replace("__CHINA__", if china { "1" } else { "0" });
    super::powershell_encoded(&script)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HermesLifecycleAction {
    Install,
    Update,
}

/// Shell command line for install or update on the current platform.
pub fn hermes_shell_command(action: HermesLifecycleAction) -> String {
    match action {
        HermesLifecycleAction::Install => hermes_install_shell_command(),
        HermesLifecycleAction::Update => hermes_update_shell_command(),
    }
}

pub fn hermes_install_shell_command() -> String {
    hermes_install_shell_command_routed(use_china_mirrors())
}

pub fn hermes_update_shell_command() -> String {
    hermes_update_shell_command_routed(use_china_mirrors())
}

fn hermes_install_shell_command_routed(china: bool) -> String {
    #[cfg(target_os = "windows")]
    {
        windows_install_shell_command(china)
    }
    #[cfg(not(target_os = "windows"))]
    {
        unix_install_shell_command(china)
    }
}

fn hermes_update_shell_command_routed(china: bool) -> String {
    #[cfg(target_os = "windows")]
    {
        format!("hermes update || {}", windows_install_shell_command(china))
    }
    #[cfg(not(target_os = "windows"))]
    {
        format!("hermes update || {}", unix_install_shell_command(china))
    }
}

/// Download the official script, then run `uv sync` in a terminal so its
/// "12/259" progress is visible. The installer itself stays non-interactive.
#[cfg(not(target_os = "windows"))]
fn unix_install_shell_command(china: bool) -> String {
    let fetch = if china {
        HERMES_FETCH_UNIX_CHINA
    } else {
        HERMES_FETCH_UNIX
    };
    let retry = if china {
        "if [ $status -ne 0 ] && [ -s $tmp ]; then unset GIT_CONFIG_COUNT GIT_CONFIG_KEY_0 GIT_CONFIG_VALUE_0; echo 国内地址下不下来，改用原来的地址再试一次…; bash $tmp; status=$?; fi; "
    } else {
        ""
    };
    format!(
        "bash -c 'tmp=$(mktemp) && {fetch} && {} && {} && bash $tmp; status=$?; {retry}rm -f $tmp $bindir; exit $status'",
        uv_progress_hook(),
        curl_progress_hook()
    )
}

#[cfg(not(target_os = "windows"))]
fn uv_progress_hook() -> String {
    let encoded = base64::engine::general_purpose::STANDARD.encode(UV_PROGRESS_PY);
    format!(
        "export HERMES_INSTALL_TMP=$tmp; if script -q /dev/null true >/dev/null 2>&1; then printf %s {encoded} | base64 -d | python3; fi"
    )
}

#[cfg(not(target_os = "windows"))]
fn curl_progress_hook() -> String {
    let encoded = base64::engine::general_purpose::STANDARD.encode(CURL_PROGRESS_SH);
    format!(
        "bindir=$(mktemp -d); printf %s {encoded} | base64 -d > $bindir/curl; chmod +x $bindir/curl; export PATH=$bindir:$PATH"
    )
}

#[cfg(not(target_os = "windows"))]
const UV_PROGRESS_PY: &str = r#"from pathlib import Path
import os
p = Path(os.environ["HERMES_INSTALL_TMP"])
text = p.read_text()
old = "$UV_CMD sync --extra all --locked"
new = "script -q /dev/null $UV_CMD sync --extra all --locked"
if old in text:
    p.write_text(text.replace(old, new, 1))
"#;

#[cfg(not(target_os = "windows"))]
const CURL_PROGRESS_SH: &str = r#"#!/bin/bash
real=$(PATH=/usr/bin:/bin command -v curl)
args=()
for arg in "$@"; do
  if [ "${arg#-}" != "$arg" ] && [ "${arg#--}" = "$arg" ]; then
    stripped=$(printf '%s' "$arg" | tr -d 's')
    if [ "$stripped" = "-" ]; then
      continue
    fi
    args+=("$stripped")
  else
    args+=("$arg")
  fi
done
exec "$real" --progress-bar "${args[@]}"
"#;

/// Run the official Hermes install or update script and return on success.
pub fn run_hermes_lifecycle(action: HermesLifecycleAction) -> Result<()> {
    let command_line = hermes_shell_command(action);
    run_shell_command(&command_line).with_context(|| {
        format!(
            "Hermes {} failed",
            match action {
                HermesLifecycleAction::Install => "install",
                HermesLifecycleAction::Update => "update",
            }
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn unix_install_uses_temp_file_not_pipe() {
        let cmd = hermes_install_shell_command();
        assert!(cmd.contains("mktemp"));
        assert!(cmd.contains("install.sh"));
        assert!(!cmd.contains("curl -fsSL") || cmd.contains("-o $tmp"));
    }

    #[test]
    fn unix_update_tries_cli_first() {
        let cmd = hermes_update_shell_command();
        assert!(cmd.starts_with("hermes update"));
        assert!(cmd.contains("||"));
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn install_command_uses_official_nousresearch_script() {
        let cmd = hermes_install_shell_command_routed(false);
        assert!(cmd.contains(HERMES_INSTALL_SCRIPT_URL));
        assert!(cmd.contains("script -q /dev/null"));
        assert!(!cmd.contains("raw.githubusercontent.com"));
        let decoded = decoded_payloads(&cmd);
        assert!(decoded.iter().any(|text| {
            text.contains("script -q /dev/null $UV_CMD sync --extra all --locked")
        }));
        assert!(decoded.iter().any(|text| text.contains("--progress-bar")));
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn china_install_uses_official_script_and_retries_github_clone() {
        let cmd = hermes_install_shell_command_routed(true);
        assert!(cmd.contains(HERMES_INSTALL_SCRIPT_URL));
        assert!(!cmd.contains("raw.githubusercontent.com"));
        assert!(!cmd.contains("jsdelivr.net"));
        assert!(cmd.contains("改用原来的地址再试一次"));
        assert!(cmd.contains("-o $tmp"));
    }

    fn decoded_windows_script(command: &str) -> String {
        use base64::Engine as _;
        let encoded = command.rsplit(' ').next().unwrap();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .unwrap();
        let units: Vec<u16> = bytes
            .chunks(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        String::from_utf16(&units).unwrap()
    }

    #[test]
    fn windows_install_in_china_proxies_github_and_retries_on_the_original_addresses() {
        let script = decoded_windows_script(&windows_install_shell_command(true));
        assert!(!script.contains("__CHINA__"));
        assert!(script.contains("$china = '1' -eq '1'"));
        assert!(script.contains("HERMES_REPO_URL = 'https://gh-proxy.com/"));
        assert!(script.contains("UV_PYTHON_INSTALL_MIRROR"));
        assert!(script.contains("改用原来的地址再试一次"));
        assert!(script.contains(
            "raw.githubusercontent.com/NousResearch/hermes-agent/main/scripts/install.ps1"
        ));
        assert!(script.contains("-File $script -NonInteractive"));
        assert!(
            !script.contains("| iex"),
            "a piped script cannot report failure"
        );
    }

    #[test]
    fn windows_install_elsewhere_uses_the_original_addresses() {
        let script = decoded_windows_script(&windows_install_shell_command(false));
        assert!(script.contains("$china = '0' -eq '1'"));
    }

    #[cfg(not(target_os = "windows"))]
    fn decoded_payloads(command: &str) -> Vec<String> {
        command
            .split("printf %s ")
            .skip(1)
            .filter_map(|rest| rest.split(" | base64").next())
            .filter_map(|encoded| {
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(encoded)
                    .ok()?;
                String::from_utf8(bytes).ok()
            })
            .collect()
    }
}
