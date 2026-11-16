//! WorkBuddy dual installer: CLI (`codebuddy`) + desktop client (`WorkBuddy`).
//!
//! Domestic users expect one Install tap to set up both the command line used
//! by in-app Ask (`codebuddy`) and the official WorkBuddy desktop window. Both
//! steps run in sequence; if either fails, the command exits non-zero.

use anyhow::{Context, Result};

use super::runner::run_shell_command;

pub const WORKBUDDY_NPM_PACKAGE: &str = "@tencent-ai/codebuddy-code";
pub const WORKBUDDY_CLI_INSTALL_SH: &str = "https://www.codebuddy.cn/cli/install.sh";
pub const WORKBUDDY_CLI_INSTALL_PS1: &str = "https://www.codebuddy.cn/cli/install.ps1";
pub const WORKBUDDY_UPDATE_API: &str = "https://www.workbuddy.cn/v2/update";
pub const WORKBUDDY_DOWNLOAD_HISTORY: &str =
    "https://www.workbuddy.cn/docs/workbuddy/Download-History";
pub const TENCENT_NPM_MIRROR: &str = "https://mirrors.cloud.tencent.com/npm/";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkbuddyLifecycleAction {
    Install,
    Update,
}

pub fn workbuddy_install_shell_command() -> String {
    workbuddy_shell_command(false)
}

pub fn workbuddy_update_shell_command() -> String {
    workbuddy_shell_command(true)
}

fn workbuddy_shell_command(update: bool) -> String {
    #[cfg(windows)]
    {
        windows_shell_command(update)
    }
    #[cfg(not(windows))]
    {
        unix_shell_command(update)
    }
}

pub fn run_workbuddy_lifecycle(action: WorkbuddyLifecycleAction) -> Result<()> {
    let command = match action {
        WorkbuddyLifecycleAction::Install => workbuddy_install_shell_command(),
        WorkbuddyLifecycleAction::Update => workbuddy_update_shell_command(),
    };
    run_shell_command(&command).with_context(|| {
        format!(
            "WorkBuddy {} failed",
            match action {
                WorkbuddyLifecycleAction::Install => "install",
                WorkbuddyLifecycleAction::Update => "update",
            }
        )
    })
}

#[cfg_attr(not(windows), allow(dead_code))]
fn windows_shell_command(update: bool) -> String {
    let script = WINDOWS_SCRIPT.replace("__UPDATE__", if update { "1" } else { "0" });
    super::powershell_encoded(&script)
}

#[cfg_attr(windows, allow(dead_code))]
fn unix_shell_command(update: bool) -> String {
    UNIX_SCRIPT.replace("__UPDATE__", if update { "1" } else { "0" })
}

const WINDOWS_SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
$update = '__UPDATE__' -eq '1'

$cliPresent = (Get-Command codebuddy -ErrorAction SilentlyContinue) -or
    (Test-Path (Join-Path $env:USERPROFILE 'AppData\Local\codebuddy\bin\codebuddy.exe'))
if (-not $update -and $cliPresent) {
    Write-Output 'WorkBuddy 命令行已在本机。'
} else {
    Write-Output '正在安装 WorkBuddy 命令行…'
    $cliOk = $false
    $cliScript = Join-Path $env:TEMP ('workbuddy-cli-' + [guid]::NewGuid().ToString('N') + '.ps1')
    try {
        Invoke-WebRequest -Uri 'https://www.codebuddy.cn/cli/install.ps1' -OutFile $cliScript -UseBasicParsing -TimeoutSec 60
        & powershell -NoProfile -ExecutionPolicy Bypass -File $cliScript
        if ($LASTEXITCODE -eq 0) { $cliOk = $true }
    } catch {
        Write-Output '原生脚本未完成，改用 npm 继续安装命令行…'
    } finally {
        Remove-Item -LiteralPath $cliScript -Force -ErrorAction SilentlyContinue
    }
    if (-not $cliOk) {
        & npm.cmd install -g '@tencent-ai/codebuddy-code@latest' --registry 'https://mirrors.cloud.tencent.com/npm/'
        if ($LASTEXITCODE -ne 0) {
            & npm.cmd install -g '@tencent-ai/codebuddy-code@latest'
            if ($LASTEXITCODE -ne 0) { throw 'WorkBuddy 命令行安装没有完成。' }
        }
    }
}

$appCandidates = @(
    (Join-Path $env:LOCALAPPDATA 'Programs\WorkBuddy\WorkBuddy.exe'),
    (Join-Path $env:LOCALAPPDATA 'Programs\workbuddy\WorkBuddy.exe'),
    (Join-Path $env:ProgramFiles 'WorkBuddy\WorkBuddy.exe')
)
$appPresent = $false
foreach ($candidate in $appCandidates) {
    if ($candidate -and (Test-Path $candidate)) { $appPresent = $true; break }
}
if (-not $update -and $appPresent) {
    Write-Output 'WorkBuddy 桌面软件已在本机。'
} else {
    Write-Output '正在获取 WorkBuddy 桌面软件安装包…'
    $exeUrl = $null
    try {
        $meta = Invoke-RestMethod -Uri 'https://www.workbuddy.cn/v2/update?platform=workbuddy-win32-x64-user' -TimeoutSec 30
        if ($meta -and $meta.url) {
            $u = [string]$meta.url
            if ($u.EndsWith('.zip')) { $u = $u.Substring(0, $u.Length - 4) + '.exe' }
            $exeUrl = $u
        }
    } catch {}
    if (-not $exeUrl) {
        $page = (Invoke-WebRequest -Uri 'https://www.workbuddy.cn/docs/workbuddy/Download-History' -UseBasicParsing -TimeoutSec 30).Content
        $m = [regex]::Match($page, 'https://(?:download\.codebuddy\.cn|acc-1258344699\.cos\.[a-z0-9.-]+\.myqcloud\.com)/workbuddy/saas/win32-x64-user/WorkBuddy-win32-x64-user-[^\s"''<>\\)]+\.exe')
        if ($m.Success) { $exeUrl = $m.Value }
    }
    if (-not $exeUrl -or -not ($exeUrl -match '^https://(download\.codebuddy\.cn|acc-1258344699\.cos\.[a-z0-9.-]+\.myqcloud\.com)/')) {
        throw '没找到 WorkBuddy 桌面软件官方安装包。'
    }
    Write-Output '正在下载 WorkBuddy 桌面软件…'
    $dest = Join-Path $env:TEMP 'WorkBuddy-setup.exe'
    try {
        Invoke-WebRequest -Uri $exeUrl -OutFile $dest -TimeoutSec 600
        Write-Output '正在安装 WorkBuddy 桌面软件…'
        $proc = Start-Process -FilePath $dest -ArgumentList '/S' -Wait -PassThru
        if ($proc.ExitCode -ne 0) { throw 'WorkBuddy 桌面软件安装没有完成。' }
    } finally {
        Remove-Item -LiteralPath $dest -Force -ErrorAction SilentlyContinue
    }
}
Write-Output 'WorkBuddy 命令行和桌面软件已装好。'
"#;

const UNIX_SCRIPT: &str = r#"set -euo pipefail
update="__UPDATE__"
export PATH="$HOME/.local/bin:$HOME/.codebuddy/bin:$PATH"

if [ "$update" != "1" ] && command -v codebuddy >/dev/null 2>&1; then
  echo "WorkBuddy 命令行已在本机。"
else
  echo "正在安装 WorkBuddy 命令行…"
  tmp_cli=$(mktemp)
  cli_ok=0
  if curl -fsSL --proto "=https" --tlsv1.2 --max-time 60 https://www.codebuddy.cn/cli/install.sh -o "$tmp_cli"; then
    if bash "$tmp_cli"; then
      cli_ok=1
    fi
  fi
  rm -f "$tmp_cli"
  if [ "$cli_ok" -ne 1 ]; then
    echo "原生脚本未完成，改用 npm 继续安装命令行…"
    npm install -g @tencent-ai/codebuddy-code@latest --registry https://mirrors.cloud.tencent.com/npm/ \
      || npm install -g @tencent-ai/codebuddy-code@latest
  fi
fi

if [ "$(uname -s)" = "Darwin" ]; then
  if [ "$update" != "1" ] && { [ -d "$HOME/Applications/WorkBuddy.app" ] || [ -d "/Applications/WorkBuddy.app" ]; }; then
    echo "WorkBuddy 桌面软件已在本机。"
  else
    echo "正在获取 WorkBuddy 桌面软件安装包…"
    url=$(python3 - <<'PY'
import json, platform, re, subprocess, sys, urllib.request

arch = platform.machine().lower()
if arch == "x86_64":
    try:
        out = subprocess.check_output(["sysctl", "-n", "sysctl.proc_translated"], text=True).strip()
        if out == "1":
            arch = "arm64"
    except Exception:
        pass
arm = arch in ("arm64", "aarch64")
platform_key = "workbuddy-darwin-arm64" if arm else "workbuddy-darwin-x64"
arch_seg = "darwin-arm64" if arm else "darwin-x64"
allowed = (
    "https://download.codebuddy.cn/",
    "https://acc-1258344699.cos.",
)

found = None
try:
    req = urllib.request.Request(
        f"https://www.workbuddy.cn/v2/update?platform={platform_key}",
        headers={"User-Agent": "agent-doctor"},
    )
    with urllib.request.urlopen(req, timeout=30) as resp:
        payload = json.loads(resp.read().decode("utf-8", "replace"))
    raw = str(payload.get("url") or "").strip()
    if raw.endswith(".zip"):
        raw = raw[:-4] + ".dmg"
    if raw.startswith(allowed) and raw.endswith(".dmg"):
        found = raw
except Exception:
    pass

if not found:
    try:
        req = urllib.request.Request(
            "https://www.workbuddy.cn/docs/workbuddy/Download-History",
            headers={"User-Agent": "agent-doctor"},
        )
        with urllib.request.urlopen(req, timeout=30) as resp:
            page = resp.read().decode("utf-8", "replace")
        pattern = rf"https://(?:download\.codebuddy\.cn|acc-1258344699\.cos\.[a-z0-9.-]+\.myqcloud\.com)/workbuddy/saas/{arch_seg}/WorkBuddy-{arch_seg}-[^\s\"'<>\\)]+\.dmg"
        m = re.search(pattern, page)
        if m:
            found = m.group(0)
    except Exception:
        pass

if not found:
    sys.exit("没找到 WorkBuddy 桌面软件官方安装包。")
print(found)
PY
)
    case "$url" in
      https://download.codebuddy.cn/*|https://acc-1258344699.cos.*.myqcloud.com/*) ;;
      *) echo "没找到 WorkBuddy 桌面软件官方安装包。" >&2; exit 1 ;;
    esac
    echo "正在下载 WorkBuddy 桌面软件…"
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
    curl -fL --proto "=https" --tlsv1.2 --progress-bar -o "$tmp_dmg" "$url"
    mnt=$(mktemp -d)
    hdiutil attach -nobrowse -readonly -noverify -noautoopen -mountpoint "$mnt" "$tmp_dmg" >/dev/null
    app=$(find "$mnt" -maxdepth 3 -name 'WorkBuddy.app' -print -quit)
    if [ -z "$app" ]; then
      echo "安装包里没有 WorkBuddy。" >&2
      exit 1
    fi
    mkdir -p "$HOME/Applications"
    rm -rf "$HOME/Applications/WorkBuddy.app"
    ditto "$app" "$HOME/Applications/WorkBuddy.app"
    xattr -dr com.apple.quarantine "$HOME/Applications/WorkBuddy.app" || true
  fi
fi
echo "WorkBuddy 命令行和桌面软件已装好。"
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_command_sets_up_both_cli_and_desktop_client() {
        let unix = unix_shell_command(false);
        assert!(unix.contains(WORKBUDDY_CLI_INSTALL_SH));
        assert!(unix.contains(WORKBUDDY_NPM_PACKAGE));
        assert!(unix.contains(WORKBUDDY_UPDATE_API));
        assert!(unix.contains("Applications/WorkBuddy.app"));
        assert!(unix.contains("update=\"0\""));

        let unix_update = unix_shell_command(true);
        assert!(unix_update.contains("update=\"1\""));

        let win = windows_shell_command(false);
        assert!(win.contains("powershell"));
        assert!(win.contains("-EncodedCommand"));
    }
}
