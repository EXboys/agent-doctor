//! ZCode desktop installer.
//!
//! The published app is the desktop build of https://github.com/zai-org/ZCode.
//! The download page at https://zcode.z.ai/cn lists the current installer.

use anyhow::{Context, Result};

use super::runner::run_shell_command;

pub const ZCODE_DOWNLOAD_PAGE: &str = "https://zcode.z.ai/cn";
pub const ZCODE_CDN_PREFIX: &str = "https://cdn-zcode.z.ai/zcode/electron/releases/";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZcodeLifecycleAction {
    Install,
    Update,
}

/// Folder segment and filename suffix for one OS and CPU.
#[cfg(test)]
fn zcode_download_asset(os: &str, arch: &str) -> Option<(&'static str, &'static str)> {
    let arm = matches!(arch, "arm64" | "aarch64");
    match os {
        "macos" | "darwin" => Some(if arm {
            ("macos-arm64/", "-mac-arm64.dmg")
        } else {
            ("macos-x64/", "-mac-x64.dmg")
        }),
        "windows" => Some(if arm {
            ("windows-arm64/", "-win-arm64.exe")
        } else {
            ("windows-x64/", "-win-x64.exe")
        }),
        "linux" => Some(if arm {
            ("linux-arm64/", "-linux-arm64.AppImage")
        } else {
            ("linux-x64/", "-linux-x64.AppImage")
        }),
        _ => None,
    }
}

/// First official installer URL on the download page for this computer.
#[cfg(test)]
fn zcode_installer_url(page: &str, os: &str, arch: &str) -> Option<String> {
    let (folder, suffix) = zcode_download_asset(os, arch)?;
    let mut rest = page;
    while let Some(start) = rest.find(ZCODE_CDN_PREFIX) {
        let tail = &rest[start..];
        let end = tail
            .find(|ch: char| ch.is_whitespace() || matches!(ch, '"' | '\'' | '<' | '>' | '\\'))
            .unwrap_or(tail.len());
        let url = &tail[..end];
        if url.contains(folder) && url.ends_with(suffix) {
            return Some(url.to_string());
        }
        let step = ZCODE_CDN_PREFIX.len().min(tail.len());
        rest = &tail[step..];
    }
    None
}

pub fn zcode_install_shell_command() -> String {
    #[cfg(windows)]
    {
        super::powershell_encoded(WINDOWS_INSTALL)
    }
    #[cfg(not(windows))]
    {
        UNIX_INSTALL.to_string()
    }
}

pub fn zcode_update_shell_command() -> String {
    zcode_install_shell_command()
}

pub fn run_zcode_lifecycle(action: ZcodeLifecycleAction) -> Result<()> {
    let command = match action {
        ZcodeLifecycleAction::Install | ZcodeLifecycleAction::Update => {
            zcode_install_shell_command()
        }
    };
    run_shell_command(&command).with_context(|| {
        format!(
            "ZCode {} failed",
            match action {
                ZcodeLifecycleAction::Install => "install",
                ZcodeLifecycleAction::Update => "update",
            }
        )
    })
}

#[cfg(windows)]
const WINDOWS_INSTALL: &str = r#"
$ErrorActionPreference = 'Stop'
Write-Output '正在获取 ZCode 安装包地址…'
$page = (Invoke-WebRequest -Uri 'https://zcode.z.ai/cn' -UseBasicParsing).Content
$arm = $env:PROCESSOR_ARCHITECTURE -match 'ARM'
if ($arm) {
  $folder = 'windows-arm64/'
  $suffix = '-win-arm64.exe'
} else {
  $folder = 'windows-x64/'
  $suffix = '-win-x64.exe'
}
$found = $null
foreach ($match in [regex]::Matches($page, 'https://cdn-zcode\.z\.ai/zcode/electron/releases/[^\s"''<>\\]+')) {
  $url = $match.Value
  if ($url.Contains($folder) -and $url.EndsWith($suffix)) { $found = $url; break }
}
if (-not $found -or -not $found.StartsWith('https://cdn-zcode.z.ai/zcode/electron/releases/')) {
  throw '没找到 ZCode 官方安装包。'
}
Write-Output $found
Write-Output '正在下载 ZCode…'
$dest = Join-Path $env:TEMP ('ZCode-setup' + $suffix)
Invoke-WebRequest -Uri $found -OutFile $dest
Write-Output '正在安装 ZCode…'
$proc = Start-Process -FilePath $dest -ArgumentList '/S' -Wait -PassThru
if ($proc.ExitCode -ne 0) { throw 'ZCode 安装没有完成。' }
Write-Output 'ZCode 已装好。'
"#;

#[cfg(not(windows))]
const UNIX_INSTALL: &str = r#"set -euo pipefail
echo "正在获取 ZCode 安装包地址…"
url=$(python3 - <<'PY'
import platform, sys, urllib.request
page_url = "https://zcode.z.ai/cn"
system = platform.system()
arm = platform.machine().lower() in ("arm64", "aarch64")
if system == "Darwin":
    folder, suffix = ("macos-arm64/", "-mac-arm64.dmg") if arm else ("macos-x64/", "-mac-x64.dmg")
elif system == "Linux":
    folder, suffix = ("linux-arm64/", "-linux-arm64.AppImage") if arm else ("linux-x64/", "-linux-x64.AppImage")
else:
    sys.exit("这台电脑暂时不能自动安装 ZCode。")
try:
    req = urllib.request.Request(page_url, headers={"User-Agent": "agent-doctor"})
    with urllib.request.urlopen(req, timeout=60) as resp:
        page = resp.read().decode("utf-8", "replace")
except Exception:
    sys.exit("连不上安装地址。")
prefix = "https://cdn-zcode.z.ai/zcode/electron/releases/"
rest = page
found = None
while prefix in rest:
    start = rest.find(prefix)
    tail = rest[start:]
    end = len(tail)
    for i, ch in enumerate(tail):
        if ch.isspace() or ch in "\"'<>\\":
            end = i
            break
    url = tail[:end]
    if folder in url and url.endswith(suffix):
        found = url
        break
    rest = tail[len(prefix):]
if not found:
    sys.exit("没找到 ZCode 官方安装包。")
print(found)
PY
)
case "$url" in
  https://cdn-zcode.z.ai/zcode/electron/releases/*) ;;
  *) echo "没找到 ZCode 官方安装包。" >&2; exit 1 ;;
esac
echo "$url"
echo "正在下载 ZCode…"
tmp=$(mktemp)
mnt=""
cleanup() {
  rm -f "$tmp"
  if [ -n "$mnt" ]; then
    hdiutil detach "$mnt" >/dev/null 2>&1 || true
    rm -rf "$mnt"
  fi
}
trap cleanup EXIT
curl -fsSL --proto "=https" --tlsv1.2 --progress-bar -o "$tmp" "$url"
if [ "$(uname -s)" = "Darwin" ]; then
  mnt=$(mktemp -d)
  hdiutil attach -nobrowse -readonly -noverify -noautoopen -mountpoint "$mnt" "$tmp" >/dev/null
  app=$(find "$mnt" -maxdepth 3 -name 'ZCode.app' -print -quit)
  if [ -z "$app" ]; then
    echo "安装包里没有 ZCode。" >&2
    exit 1
  fi
  mkdir -p "$HOME/Applications"
  rm -rf "$HOME/Applications/ZCode.app"
  ditto "$app" "$HOME/Applications/ZCode.app"
  xattr -dr com.apple.quarantine "$HOME/Applications/ZCode.app" || true
else
  mkdir -p "$HOME/.local/bin"
  rm -f "$HOME/.local/bin/ZCode.AppImage"
  cp "$tmp" "$HOME/.local/bin/ZCode.AppImage"
  chmod 755 "$HOME/.local/bin/ZCode.AppImage"
fi
echo "ZCode 已装好。"
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installer_url_picks_the_newest_listed_asset() {
        let page = "\
https://cdn-zcode.z.ai/zcode/electron/releases/3.16.1/macos-arm64/ZCode-3.16.1-mac-arm64.dmg
https://cdn-zcode.z.ai/zcode/electron/releases/3.16.1/macos-x64/ZCode-3.16.1-mac-x64.dmg
https://cdn-zcode.z.ai/zcode/electron/releases/3.15.3/macos-arm64/ZCode-3.15.3-mac-arm64.dmg
https://cdn-zcode.z.ai/zcode/electron/releases/3.16.1/windows-x64/ZCode-3.16.1-win-x64.exe
https://cdn-zcode.z.ai/zcode/electron/releases/3.16.1/linux-arm64/ZCode-3.16.1-linux-arm64.AppImage
";
        assert_eq!(
            zcode_installer_url(page, "macos", "arm64").as_deref(),
            Some(
                "https://cdn-zcode.z.ai/zcode/electron/releases/3.16.1/macos-arm64/ZCode-3.16.1-mac-arm64.dmg"
            )
        );
        assert_eq!(
            zcode_installer_url(page, "windows", "x64").as_deref(),
            Some(
                "https://cdn-zcode.z.ai/zcode/electron/releases/3.16.1/windows-x64/ZCode-3.16.1-win-x64.exe"
            )
        );
        assert!(zcode_installer_url(page, "macos", "arm64")
            .unwrap()
            .starts_with(ZCODE_CDN_PREFIX));
    }

    #[test]
    fn install_script_uses_the_official_download_page() {
        let cmd = zcode_install_shell_command();
        #[cfg(not(target_os = "windows"))]
        {
            assert!(cmd.contains(ZCODE_DOWNLOAD_PAGE));
            assert!(cmd.contains("cdn-zcode.z.ai"));
            assert!(!cmd.contains("npm install"));
            assert!(cmd.contains("macos-arm64/"));
            assert!(cmd.contains("-mac-arm64.dmg"));
            assert!(cmd.contains("linux-x64/"));
            assert!(cmd.contains("-linux-x64.AppImage"));
            assert!(cmd.contains("Applications/ZCode.app"));
        }
        #[cfg(target_os = "windows")]
        {
            // The Windows script ships base64-encoded, so its URLs are not
            // visible in the command string; assert the launcher shape.
            assert!(cmd.contains("powershell"));
            assert!(cmd.contains("-EncodedCommand"));
        }
    }
}
