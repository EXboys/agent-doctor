//! Qoder dual installer: CLI (`qodercli`) + desktop client (`Qoder IDE`).
//!
//! Domestic users expect one Install tap to set up both the command line used
//! by in-app Ask (`qodercli`) and the official Qoder desktop window. Both steps
//! run in sequence; if either fails, the command exits non-zero.

use anyhow::{Context, Result};

use super::download_route::use_china_mirrors;
use super::runner::run_shell_command;

pub const QODER_NPM_PACKAGE: &str = "@qoder-ai/qodercli";
pub const QODER_CLI_INSTALL_URL: &str = "https://qoder.com/install";
pub const QODER_CLI_INSTALL_PS1: &str = "https://qoder.com/install.ps1";
pub const QODER_CN_MAC_PREFIX: &str =
    "https://qoder-ide-cn.oss-cn-hangzhou.aliyuncs.com/qoder/release/lastest/Qoder-CN-IDE-darwin-";
pub const QODER_GLOBAL_MAC_PREFIX: &str =
    "https://download.qoder.com/release/latest/Qoder-IDE-darwin-";
pub const QODER_CN_WIN_USER_EXE: &str =
    "https://qoder-ide-cn.oss-cn-hangzhou.aliyuncs.com/qoder/release/lastest/QoderCNIDEUserSetup-x64.exe";
pub const QODER_GLOBAL_WIN_USER_EXE: &str =
    "https://download.qoder.com/release/latest/QoderIDEUserSetup-x64.exe";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QoderLifecycleAction {
    Install,
    Update,
}

pub fn qoder_install_shell_command() -> String {
    qoder_shell_command_routed(false, use_china_mirrors())
}

pub fn qoder_update_shell_command() -> String {
    qoder_shell_command_routed(true, use_china_mirrors())
}

fn qoder_shell_command_routed(update: bool, china: bool) -> String {
    #[cfg(windows)]
    {
        windows_shell_command(update, china)
    }
    #[cfg(not(windows))]
    {
        unix_shell_command(update, china)
    }
}

pub fn run_qoder_lifecycle(action: QoderLifecycleAction) -> Result<()> {
    let command = match action {
        QoderLifecycleAction::Install => qoder_install_shell_command(),
        QoderLifecycleAction::Update => qoder_update_shell_command(),
    };
    run_shell_command(&command).with_context(|| {
        format!(
            "Qoder {} failed",
            match action {
                QoderLifecycleAction::Install => "install",
                QoderLifecycleAction::Update => "update",
            }
        )
    })
}

#[cfg_attr(not(windows), allow(dead_code))]
fn windows_shell_command(update: bool, china: bool) -> String {
    let script = WINDOWS_SCRIPT
        .replace("__UPDATE__", if update { "1" } else { "0" })
        .replace("__CHINA__", if china { "1" } else { "0" });
    super::powershell_encoded(&script)
}

#[cfg_attr(windows, allow(dead_code))]
fn unix_shell_command(update: bool, china: bool) -> String {
    UNIX_SCRIPT
        .replace("__UPDATE__", if update { "1" } else { "0" })
        .replace("__CHINA__", if china { "1" } else { "0" })
}

const WINDOWS_SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
$update = '__UPDATE__' -eq '1'
$china = '__CHINA__' -eq '1'

$cliPresent = (Get-Command qodercli -ErrorAction SilentlyContinue) -or
    (Get-Command qoderclicn -ErrorAction SilentlyContinue) -or
    (Test-Path (Join-Path $env:USERPROFILE '.local\bin\qodercli.exe'))
if (-not $update -and $cliPresent) {
    Write-Output 'Qoder 命令行已在本机。'
} else {
    Write-Output '正在安装 Qoder 命令行…'
    $cliOk = $false
    $cliScript = Join-Path $env:TEMP ('qoder-cli-' + [guid]::NewGuid().ToString('N') + '.ps1')
    try {
        Invoke-WebRequest -Uri 'https://qoder.com/install.ps1' -OutFile $cliScript -UseBasicParsing -TimeoutSec 60
        if ($update) {
            & powershell -NoProfile -ExecutionPolicy Bypass -File $cliScript --force
        } else {
            & powershell -NoProfile -ExecutionPolicy Bypass -File $cliScript
        }
        if ($LASTEXITCODE -eq 0) { $cliOk = $true }
    } catch {
        Write-Output '原生脚本未完成，改用 npm 继续安装命令行…'
    } finally {
        Remove-Item -LiteralPath $cliScript -Force -ErrorAction SilentlyContinue
    }
    if (-not $cliOk) {
        & npm.cmd install -g '@qoder-ai/qodercli@latest'
        if ($LASTEXITCODE -ne 0) { throw 'Qoder 命令行安装没有完成。' }
    }
}

$appCandidates = @(
    (Join-Path $env:LOCALAPPDATA 'Programs\Qoder CN\Qoder CN.exe'),
    (Join-Path $env:LOCALAPPDATA 'Programs\Qoder IDE\Qoder IDE.exe'),
    (Join-Path $env:LOCALAPPDATA 'Programs\Qoder\Qoder.exe'),
    (Join-Path $env:ProgramFiles 'Qoder CN\Qoder CN.exe'),
    (Join-Path $env:ProgramFiles 'Qoder IDE\Qoder IDE.exe'),
    (Join-Path $env:ProgramFiles 'Qoder\Qoder.exe')
)
$appPresent = $false
foreach ($candidate in $appCandidates) {
    if ($candidate -and (Test-Path $candidate)) { $appPresent = $true; break }
}
if (-not $update -and $appPresent) {
    Write-Output 'Qoder 桌面软件已在本机。'
} else {
    $sources = if ($china) {
        @(
            'https://qoder-ide-cn.oss-cn-hangzhou.aliyuncs.com/qoder/release/lastest/QoderCNIDEUserSetup-x64.exe',
            'https://download.qoder.com/release/latest/QoderIDEUserSetup-x64.exe'
        )
    } else {
        @('https://download.qoder.com/release/latest/QoderIDEUserSetup-x64.exe')
    }
    $dest = Join-Path $env:TEMP 'Qoder-setup.exe'
    $downloaded = $false
    try {
        foreach ($src in $sources) {
            try {
                Write-Output '正在下载 Qoder 桌面软件…'
                Invoke-WebRequest -Uri $src -OutFile $dest -TimeoutSec 600
                $downloaded = $true
                break
            } catch {
                Write-Output '当前地址下载未完成，换备用地址再试…'
            }
        }
        if (-not $downloaded) { throw '连不上 Qoder 桌面软件安装地址。' }
        Write-Output '正在安装 Qoder 桌面软件…'
        $proc = Start-Process -FilePath $dest -ArgumentList '/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART','/MERGETASKS=!runcode' -Wait -PassThru
        if ($proc.ExitCode -ne 0) { throw 'Qoder 桌面软件安装没有完成。' }
    } finally {
        Remove-Item -LiteralPath $dest -Force -ErrorAction SilentlyContinue
    }
}
Write-Output 'Qoder 命令行和桌面软件已装好。'
"#;

const UNIX_SCRIPT: &str = r#"set -euo pipefail
update="__UPDATE__"
china="__CHINA__"
export PATH="$HOME/.local/bin:$HOME/.qoder/bin:$PATH"

if [ "$update" != "1" ] && { command -v qodercli >/dev/null 2>&1 || command -v qoderclicn >/dev/null 2>&1; }; then
  echo "Qoder 命令行已在本机。"
else
  echo "正在安装 Qoder 命令行…"
  tmp_cli=$(mktemp)
  cli_ok=0
  cli_args=""
  if [ "$update" = "1" ]; then
    cli_args="--force"
  fi
  if curl -fsSL --proto "=https" --tlsv1.2 --max-time 60 https://qoder.com/install -o "$tmp_cli"; then
    if bash "$tmp_cli" $cli_args; then
      cli_ok=1
    fi
  fi
  rm -f "$tmp_cli"
  if [ "$cli_ok" -ne 1 ]; then
    echo "原生脚本未完成，改用 npm 继续安装命令行…"
    npm install -g @qoder-ai/qodercli@latest
  fi
fi

if [ "$(uname -s)" = "Darwin" ]; then
  has_app=0
  for app_name in "Qoder CN.app" "Qoder IDE.app" "Qoder.app"; do
    if [ -d "$HOME/Applications/$app_name" ] || [ -d "/Applications/$app_name" ]; then
      has_app=1
      break
    fi
  done
  if [ "$update" != "1" ] && [ "$has_app" -eq 1 ]; then
    echo "Qoder 桌面软件已在本机。"
  else
    arch=$(uname -m)
    if [ "$arch" = "x86_64" ] && [ "$(sysctl -n sysctl.proc_translated 2>/dev/null || echo 0)" = "1" ]; then
      arch="arm64"
    fi
    case "$arch" in
      arm64|aarch64) dmg_arch="arm64" ;;
      *) dmg_arch="x64" ;;
    esac
    cn_url="https://qoder-ide-cn.oss-cn-hangzhou.aliyuncs.com/qoder/release/lastest/Qoder-CN-IDE-darwin-${dmg_arch}.dmg"
    global_url="https://download.qoder.com/release/latest/Qoder-IDE-darwin-${dmg_arch}.dmg"
    tmp_dmg=$(mktemp)
    mnt=""
    cleanup() {
      rm -f "$tmp_dmg"
      if [ -n "$mnt" ]; then
        hdiutil detach "$mnt" >/dev/null 2>&1 || true
        rm -rf "$mnt"
      fi
    }
    trap cleanup EXIT
    echo "正在下载 Qoder 桌面软件…"
    if [ "$china" = "1" ]; then
      curl -fL --proto "=https" --tlsv1.2 --progress-bar -o "$tmp_dmg" "$cn_url" \
        || curl -fL --proto "=https" --tlsv1.2 --progress-bar -o "$tmp_dmg" "$global_url"
    else
      curl -fL --proto "=https" --tlsv1.2 --progress-bar -o "$tmp_dmg" "$global_url"
    fi
    mnt=$(mktemp -d)
    hdiutil attach -nobrowse -readonly -noverify -noautoopen -mountpoint "$mnt" "$tmp_dmg" >/dev/null
    app=$(find "$mnt" -maxdepth 3 -name '*.app' -print -quit)
    if [ -z "$app" ]; then
      echo "安装包里没有 Qoder。" >&2
      exit 1
    fi
    app_base=$(basename "$app")
    mkdir -p "$HOME/Applications"
    rm -rf "$HOME/Applications/$app_base"
    ditto "$app" "$HOME/Applications/$app_base"
    xattr -dr com.apple.quarantine "$HOME/Applications/$app_base" || true
  fi
fi
echo "Qoder 命令行和桌面软件已装好。"
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_command_sets_up_both_cli_and_desktop_client() {
        let cn = unix_shell_command(false, true);
        assert!(cn.contains(QODER_CLI_INSTALL_URL));
        assert!(cn.contains(QODER_NPM_PACKAGE));
        assert!(cn.contains(QODER_CN_MAC_PREFIX));
        assert!(cn.contains(QODER_GLOBAL_MAC_PREFIX));
        assert!(cn.contains("china=\"1\""));
        assert!(cn.contains("update=\"0\""));

        let global_update = unix_shell_command(true, false);
        assert!(global_update.contains("china=\"0\""));
        assert!(global_update.contains("update=\"1\""));

        let win = windows_shell_command(false, true);
        assert!(win.contains("powershell"));
        assert!(win.contains("-EncodedCommand"));
    }
}
