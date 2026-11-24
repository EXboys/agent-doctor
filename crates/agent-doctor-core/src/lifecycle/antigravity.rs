//! Antigravity CLI (`agy`) via Google's official installer.
//!
//! The program is a Go binary, not an npm package. macOS and Linux use
//! `https://antigravity.google/cli/install.sh`. Windows uses `install.ps1`.
//! The script is saved to a temp file and then run, so a failed download is
//! not executed.

use anyhow::{Context, Result};

use super::runner::run_shell_command;

#[cfg(not(windows))]
const INSTALL_SH: &str = "https://antigravity.google/cli/install.sh";
#[cfg(windows)]
const INSTALL_PS1: &str = "https://antigravity.google/cli/install.ps1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AntigravityLifecycleAction {
    Install,
    Update,
}

pub fn antigravity_install_shell_command() -> String {
    #[cfg(windows)]
    {
        windows_install_shell_command()
    }
    #[cfg(not(windows))]
    {
        unix_install_shell_command()
    }
}

pub fn antigravity_update_shell_command() -> String {
    antigravity_install_shell_command()
}

pub fn antigravity_uninstall_shell_command() -> String {
    #[cfg(windows)]
    {
        super::powershell_encoded(
            r#"Remove-Item -LiteralPath (Join-Path $env:LOCALAPPDATA 'agy\bin\agy.exe') -Force -ErrorAction SilentlyContinue
Remove-Item -LiteralPath (Join-Path $env:USERPROFILE '.local\bin\agy.exe') -Force -ErrorAction SilentlyContinue
exit 0
"#,
        )
    }
    #[cfg(not(windows))]
    {
        "rm -f \"$HOME/.local/bin/agy\"".to_string()
    }
}

pub fn run_antigravity_lifecycle(action: AntigravityLifecycleAction) -> Result<()> {
    let command = match action {
        AntigravityLifecycleAction::Install | AntigravityLifecycleAction::Update => {
            antigravity_install_shell_command()
        }
    };
    run_shell_command(&command).with_context(|| {
        format!(
            "Antigravity {} failed",
            match action {
                AntigravityLifecycleAction::Install => "install",
                AntigravityLifecycleAction::Update => "update",
            }
        )
    })
}

#[cfg(not(windows))]
fn unix_install_shell_command() -> String {
    format!(
        "bash -c 'tmp=$(mktemp) && curl -fsSL {INSTALL_SH} -o \"$tmp\" && bash \"$tmp\"; status=$?; rm -f \"$tmp\"; exit $status'"
    )
}

#[cfg(windows)]
fn windows_install_shell_command() -> String {
    let script = format!(
        r#"$script = Join-Path $env:TEMP ('agy-install-' + [guid]::NewGuid().ToString('N') + '.ps1')
try {{
    Invoke-WebRequest -Uri '{INSTALL_PS1}' -OutFile $script -UseBasicParsing -TimeoutSec 60 -ErrorAction Stop
}} catch {{
    Write-Output '安装脚本下载不下来。'
    exit 4
}}
& powershell -NoProfile -ExecutionPolicy Bypass -File $script
$code = $LASTEXITCODE
Remove-Item -LiteralPath $script -Force -ErrorAction SilentlyContinue
if ($null -eq $code) {{ exit 0 }} else {{ exit $code }}
"#
    );
    super::powershell_encoded(&script)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_uses_the_official_script() {
        let cmd = antigravity_install_shell_command();
        let script = install_script_text(&cmd);
        assert!(
            script.contains("https://antigravity.google/cli/install."),
            "{script}"
        );
        #[cfg(not(windows))]
        {
            assert!(script.contains("mktemp"));
            assert!(script.contains("-o \"$tmp\""));
            assert!(!script.contains("| bash"));
        }
        #[cfg(windows)]
        {
            assert!(script.contains("install.ps1"));
            assert!(script.contains("Invoke-WebRequest"));
        }
    }

    fn install_script_text(command: &str) -> String {
        #[cfg(windows)]
        {
            decoded_windows_script(command)
        }
        #[cfg(not(windows))]
        {
            command.to_string()
        }
    }

    /// Windows wraps the installer in `-EncodedCommand`, so the URL is not plaintext.
    #[cfg(windows)]
    fn decoded_windows_script(command: &str) -> String {
        use base64::Engine as _;
        let encoded = command.rsplit(' ').next().unwrap();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .unwrap();
        let units: Vec<u16> = bytes
            .chunks(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        String::from_utf16(&units).unwrap()
    }

    #[test]
    fn update_reinstalls_from_the_same_script() {
        assert_eq!(
            antigravity_update_shell_command(),
            antigravity_install_shell_command()
        );
    }

    #[test]
    #[cfg(not(windows))]
    fn uninstall_removes_the_user_binary() {
        assert_eq!(
            antigravity_uninstall_shell_command(),
            "rm -f \"$HOME/.local/bin/agy\""
        );
    }
}
