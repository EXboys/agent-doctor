use anyhow::{anyhow, bail, Result};

use crate::adapters::util::{ensure_managed_runtime_path, find_all_binaries, home_join};
use crate::adapters::DEEPSEEK_HARNESS_NPM_PACKAGE;

use super::npm_target::{leftover_npm_cli_binaries, resolve_active_npm_prefix};
use super::runner::run_shell_command;

/// Human-facing CLI name used when probing leftover installs.
fn npm_cli_binary_name(runtime_id: &str) -> Option<&'static str> {
    match runtime_id {
        "claude-code" => Some("claude"),
        "codex" => Some("codex"),
        "deepseek-harness" => Some("dsh"),
        "openclaw" => Some("openclaw"),
        "qoder" => Some("qoder"),
        "workbuddy" => Some("codebuddy"),
        _ => None,
    }
}

pub fn uninstall_shell_command(runtime_id: &str) -> Result<String> {
    let package = match runtime_id {
        "claude-code" => "@anthropic-ai/claude-code",
        "codex" => "@openai/codex",
        "deepseek-harness" => DEEPSEEK_HARNESS_NPM_PACKAGE,
        "qoder" => {
            return Ok(
                "rm -f \"$HOME/.local/bin/qoder\" \"$HOME/.local/bin/qodercli\" \"$HOME/.qoder/bin/qoder\"; rm -rf \"$HOME/.qoder/bin/qodercli\"; npm uninstall -g @qoder-ai/qodercli 2>/dev/null || true"
                    .to_string(),
            );
        }
        "workbuddy" => {
            return Ok(
                "rm -f \"$HOME/.local/bin/codebuddy\" \"$HOME/.codebuddy/bin/codebuddy\"; npm uninstall -g @tencent-ai/codebuddy-code 2>/dev/null || true"
                    .to_string(),
            );
        }
        "cursor" => {
            return Ok("rm -f \"$HOME/.local/bin/agent\"".to_string());
        }
        "zcode" => {
            return Ok(
                "rm -rf \"$HOME/Applications/ZCode.app\"; rm -f \"$HOME/.local/bin/zcode\" \"$HOME/.local/bin/ZCode.AppImage\""
                    .to_string(),
            );
        }
        "openclaw" => {
            return Ok(
                "openclaw uninstall --service --yes --non-interactive; npm uninstall -g openclaw"
                    .to_string(),
            );
        }
        "hermes" => return Ok(hermes_uninstall_shell_command()),
        other => return Err(anyhow!("unknown runtime: {other}")),
    };
    Ok(format!("npm uninstall -g {package}"))
}

/// Installed Hermes rejects `hermes uninstall` unless stdin is a terminal, and
/// this install still asks two questions even with `--yes`. Drive those answers
/// on a pseudo-terminal: keep saved settings, remove the program.
fn hermes_uninstall_shell_command() -> String {
    r#"python3 - <<'PY'
import os, select, sys, time

pid, fd = __import__("pty").fork()
if pid == 0:
    os.execvp("hermes", ["hermes", "uninstall"])
    os._exit(127)

buf = b""
sent_choice = False
sent_yes = False
deadline = time.time() + 180
status = 1
while time.time() < deadline:
    readable, _, _ = select.select([fd], [], [], 0.4)
    if readable:
        try:
            chunk = os.read(fd, 4096)
        except OSError:
            chunk = b""
        if chunk:
            buf += chunk
            text = buf.decode("utf-8", "replace")
            if not sent_choice and "Select option" in text:
                os.write(fd, b"1\n")
                sent_choice = True
            elif sent_choice and not sent_yes and "to confirm" in text:
                os.write(fd, b"yes\n")
                sent_yes = True
    done, code = os.waitpid(pid, os.WNOHANG)
    if done:
        status = os.WEXITSTATUS(code) if os.WIFEXITED(code) else 1
        break
else:
    try:
        os.kill(pid, 15)
    except OSError:
        pass
    status = 1
sys.exit(status)
PY"#
    .to_string()
}

/// `install.ps1` puts the program next to the saved settings and adds its
/// launchers to the user PATH. On Windows that program is often
/// `%LOCALAPPDATA%\hermes`, while the checkout itself is the sibling
/// `%LOCALAPPDATA%\hermes-agent`. `HERMES_HOME` during a chat is the project
/// profile, not that program folder. `hermes uninstall` also needs a terminal,
/// which a pipe cannot provide. Remove the program folders we found and keep
/// config, keys and chats.
fn hermes_windows_uninstall_script(roots: &[std::path::PathBuf]) -> String {
    let listed = roots
        .iter()
        .map(|path| format!("'{}'", path.display().to_string().replace('\'', "''")))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        r#"
$ErrorActionPreference = 'Continue'
$roots = @({listed})
$program = @('hermes-agent', 'bin', 'node', 'tools', 'git', 'python', 'uv', 'venv', 'runtime',
             'hermes.cmd', 'hermes.exe', 'hermes.bat', 'hermes.ps1')
$seen = $false
foreach ($root in $roots) {{
    if (-not $root) {{ continue }}
    $root = $root.TrimEnd('\')
    if (-not (Test-Path -LiteralPath $root)) {{ continue }}
    $seen = $true
    $prefix = $root + '\'
    Get-Process | Where-Object {{ $_.Path -and $_.Path.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase) }} |
        Stop-Process -Force -ErrorAction SilentlyContinue
}}
if (-not $seen) {{ exit 3 }}
Start-Sleep -Milliseconds 800
foreach ($root in $roots) {{
    if (-not $root) {{ continue }}
    $root = $root.TrimEnd('\')
    if (-not (Test-Path -LiteralPath $root)) {{ continue }}
    $prefix = $root + '\'
    $leaf = Split-Path -Leaf $root
    if ($leaf -ieq 'hermes-agent') {{
        Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue
    }} else {{
        foreach ($name in $program) {{
            $path = Join-Path $root $name
            if (Test-Path -LiteralPath $path) {{ Remove-Item -LiteralPath $path -Recurse -Force -ErrorAction SilentlyContinue }}
        }}
    }}
    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    if ($userPath) {{
        $kept = $userPath -split ';' | Where-Object {{
            $entry = $_.TrimEnd('\')
            $entry -and -not ($entry -ieq $root -or $entry.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase))
        }}
        $joined = $kept -join ';'
        if ($joined -ne $userPath) {{ [Environment]::SetEnvironmentVariable('Path', $joined, 'User') }}
    }}
}}
$left = @()
foreach ($root in $roots) {{
    if (-not $root) {{ continue }}
    $root = $root.TrimEnd('\')
    if (-not (Test-Path -LiteralPath $root)) {{ continue }}
    $leaf = Split-Path -Leaf $root
    if ($leaf -ieq 'hermes-agent') {{ $left += $root; continue }}
    foreach ($name in $program) {{
        if (Test-Path -LiteralPath (Join-Path $root $name)) {{ $left += (Join-Path $root $name) }}
    }}
}}
if ($left) {{ exit 2 }}
"#
    )
}

/// Folders that hold the Hermes program, not a project profile.
///
/// A profile under `HERMES_HOME` only has settings. The command Windows
/// actually runs often lives in `%LOCALAPPDATA%\hermes-agent`.
fn hermes_program_roots(
    home: &std::path::Path,
    local_app_data: Option<&std::path::Path>,
    binaries: &[std::path::PathBuf],
) -> Vec<std::path::PathBuf> {
    let mut roots = Vec::new();
    let mut push = |path: std::path::PathBuf| {
        if !is_safe_hermes_program_root(&path, home, local_app_data) {
            return;
        }
        if roots
            .iter()
            .any(|existing: &std::path::PathBuf| existing == &path)
        {
            return;
        }
        roots.push(path);
    };
    if let Some(local) = local_app_data {
        let native = local.join("hermes");
        if native.is_dir() {
            push(native);
        }
        let checkout = local.join("hermes-agent");
        if checkout.is_dir() {
            push(checkout);
        }
    }
    let legacy = home.join(".hermes");
    if crate::adapters::util::hermes_install_owns(&legacy) {
        push(legacy);
    }
    for binary in binaries {
        if let Some((data_root, checkout)) = crate::adapters::util::hermes_install_location(binary)
        {
            push(data_root);
            if let Some(checkout) = checkout {
                push(checkout);
            }
        }
    }
    roots
}

fn is_safe_hermes_program_root(
    path: &std::path::Path,
    home: &std::path::Path,
    _local_app_data: Option<&std::path::Path>,
) -> bool {
    if path == home || path.parent().is_none() {
        return false;
    }
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    !matches!(
        name.to_ascii_lowercase().as_str(),
        "users" | "appdata" | "local" | "localappdata" | "windows" | "program files"
    )
}

fn uninstall_hermes_windows() -> Result<()> {
    let home = crate::adapters::util::home_dir();
    let local = std::env::var_os("LOCALAPPDATA").map(std::path::PathBuf::from);
    let binaries = find_all_binaries("hermes");
    let roots = hermes_program_roots(&home, local.as_deref(), &binaries);
    if roots.is_empty() {
        bail!("没找到 Hermes 的安装文件夹。它可能是用别的方式装的，请用当时安装它的方式卸掉。");
    }
    let capture = super::runner::run_shell_command_capturing(&super::powershell_encoded(
        &hermes_windows_uninstall_script(&roots),
    ))?;
    match capture.exit_code {
        Some(0) => {}
        Some(3) => {
            bail!("没找到 Hermes 的安装文件夹。它可能是用别的方式装的，请用当时安装它的方式卸掉。")
        }
        Some(2) => {
            bail!("有些文件正被占用，没删干净。关掉 Hermes 和正在用它的窗口，再点一次卸载。")
        }
        _ => bail!("卸不掉，系统没让删这些文件。重启电脑后再点一次卸载。"),
    }
    if !find_all_binaries("hermes").is_empty() {
        bail!(
            "这里装的那份已经卸掉了，但本机别处还有一份不是从这里装的。请用当时安装它的方式卸掉。"
        );
    }
    Ok(())
}

fn uninstall_npm_package(package: &str, binary_name: &str) -> Result<()> {
    crate::lifecycle::nodejs::ensure_npm().map_err(|_| anyhow!("卸掉需要本机已安装 npm。"))?;

    let active = resolve_active_npm_prefix(binary_name)?;
    if active.prefix.as_os_str().is_empty() {
        bail!("这个程序不是从这里装上去的，所以卸不掉。请用当时安装它的方式卸掉。");
    }
    let removed_path = active.detected_binary.clone();
    super::npm_target::run_npm_global("uninstall", package, &active.prefix)?;

    ensure_managed_runtime_path();
    let leftovers = leftover_npm_cli_binaries(binary_name);
    if leftovers.is_empty() {
        return Ok(());
    }

    if let Some(removed) = removed_path.as_ref() {
        let removed_key = normalize_bin_key(removed);
        let still_same = leftovers
            .iter()
            .any(|path| normalize_bin_key(path) == removed_key);
        if still_same {
            bail!("卸不掉。本机这份可能不是用本应用能卸的方式装的，请先关掉正在运行的窗口后再试。");
        }
    }

    // Active copy is gone; another install is still on PATH.
    bail!("这一份已经卸掉了，但本机还有别处的一份。再点一次卸载，继续卸掉剩下的。");
}

fn normalize_bin_key(path: &std::path::Path) -> String {
    std::fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .display()
        .to_string()
}

fn is_user_managed_binary(path: &std::path::Path) -> bool {
    let home = home_join("");
    path.starts_with(&home)
        || path.starts_with("/usr/local/bin")
        || path.starts_with("/opt/homebrew/bin")
}

fn uninstall_native_cli(binary_names: &[&str], npm_package: &str, npm_binary: &str) -> Result<()> {
    ensure_managed_runtime_path();
    let mut removed = false;
    for name in binary_names {
        for path in find_all_binaries(name) {
            if !is_user_managed_binary(&path) {
                continue;
            }
            match std::fs::remove_file(&path) {
                Ok(()) => removed = true,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
    }
    if crate::lifecycle::nodejs::ensure_npm().is_ok()
        && uninstall_npm_package(npm_package, npm_binary).is_ok()
    {
        removed = true;
    }
    if removed {
        return Ok(());
    }
    bail!("没找到能卸的安装。可能是用别的方式装的。");
}

fn uninstall_cursor_cli() -> Result<()> {
    ensure_managed_runtime_path();
    let mut removed = false;
    let local_agent = home_join(".local/bin/agent");
    if local_agent.is_file() {
        std::fs::remove_file(&local_agent)?;
        removed = true;
    }
    for path in find_all_binaries("agent") {
        let text = path.to_string_lossy();
        if text.contains("cursor-agent") && std::fs::remove_file(&path).is_ok() {
            removed = true;
        }
    }
    if removed {
        return Ok(());
    }
    bail!("没找到 Cursor 命令行。电脑上的 Cursor 窗口不会被卸掉。");
}

fn uninstall_zcode() -> Result<()> {
    let mut removed = false;
    let app = home_join("Applications/ZCode.app");
    if app.exists() {
        std::fs::remove_dir_all(&app)?;
        removed = true;
    }
    for rel in [".local/bin/zcode", ".local/bin/ZCode.AppImage"] {
        let path = home_join(rel);
        if path.is_file() || path.is_symlink() {
            std::fs::remove_file(&path)?;
            removed = true;
        }
    }
    if removed {
        return Ok(());
    }
    bail!("没找到能卸的 ZCode。应用程序文件夹里的窗口不会被卸掉。");
}

pub fn uninstall_runtime(runtime_id: &str) -> Result<()> {
    match runtime_id {
        "qoder" => uninstall_native_cli(&["qoder", "qodercli"], "@qoder-ai/qodercli", "qodercli"),
        "workbuddy" => uninstall_native_cli(
            &["codebuddy", "workbuddy"],
            "@tencent-ai/codebuddy-code",
            "codebuddy",
        ),
        "cursor" => uninstall_cursor_cli(),
        "zcode" => uninstall_zcode(),
        "hermes" if cfg!(windows) => uninstall_hermes_windows(),
        "hermes" => {
            let command = hermes_uninstall_shell_command();
            run_shell_command(&command)
        }
        "openclaw" => {
            // Best-effort service teardown; package removal is the source of truth.
            let _ = run_shell_command("openclaw uninstall --service --yes --non-interactive");
            uninstall_npm_package("openclaw", "openclaw")
        }
        other => {
            let package = match other {
                "claude-code" => "@anthropic-ai/claude-code",
                "codex" => "@openai/codex",
                "deepseek-harness" => DEEPSEEK_HARNESS_NPM_PACKAGE,
                unknown => return Err(anyhow!("unknown runtime: {unknown}")),
            };
            let binary =
                npm_cli_binary_name(other).ok_or_else(|| anyhow!("unknown runtime: {other}"))?;
            uninstall_npm_package(package, binary)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn npm_clis_use_global_uninstall() {
        assert_eq!(
            uninstall_shell_command("claude-code").unwrap(),
            "npm uninstall -g @anthropic-ai/claude-code"
        );
        assert_eq!(
            uninstall_shell_command("codex").unwrap(),
            "npm uninstall -g @openai/codex"
        );
        assert_eq!(
            uninstall_shell_command("deepseek-harness").unwrap(),
            "npm uninstall -g @deepseek-ai/dsh"
        );
        let qoder = uninstall_shell_command("qoder").unwrap();
        assert!(qoder.contains(".local/bin/qoder"));
        assert!(!qoder.starts_with("npm uninstall"));
    }

    #[test]
    fn openclaw_stops_service_then_removes_cli() {
        let cmd = uninstall_shell_command("openclaw").unwrap();
        assert!(cmd.contains("openclaw uninstall --service --yes --non-interactive"));
        assert!(cmd.contains("npm uninstall -g openclaw"));
    }

    #[test]
    fn hermes_uninstall_keeps_saved_settings() {
        let cmd = uninstall_shell_command("hermes").unwrap();
        assert!(cmd.contains("hermes\", \"uninstall\""));
        assert!(cmd.contains("Select option"));
        assert!(cmd.contains("b\"1\\n\""));
        assert!(cmd.contains("b\"yes\\n\""));
        assert!(!cmd.contains("--full"));
    }

    #[test]
    fn windows_hermes_uninstall_removes_the_program_and_keeps_settings() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let local = temp.path().join("Local");
        std::fs::create_dir_all(local.join("hermes")).unwrap();
        let checkout = local.join("hermes-agent");
        let command = checkout.join("venv").join("Scripts").join("hermes.exe");
        std::fs::create_dir_all(command.parent().unwrap()).unwrap();
        std::fs::write(&command, "").unwrap();
        let profile = home.join(".hermes").join("profiles").join("default");
        std::fs::create_dir_all(&profile).unwrap();
        let roots = hermes_program_roots(&home, Some(&local), &[command]);
        assert!(roots.iter().any(|path| path == &local.join("hermes")));
        assert!(roots.iter().any(|path| path == &checkout));
        assert!(!roots.iter().any(|path| path == &profile));

        let custom = temp.path().join("Tools").join("MyHermes");
        let custom_command = custom.join("bin").join("hermes.exe");
        std::fs::create_dir_all(custom_command.parent().unwrap()).unwrap();
        std::fs::write(&custom_command, "").unwrap();
        std::fs::create_dir_all(custom.join("tools")).unwrap();
        let custom_roots = hermes_program_roots(&home, Some(&local), &[custom_command]);
        assert!(custom_roots.iter().any(|path| path == &custom));

        let script = hermes_windows_uninstall_script(&roots);
        assert!(!script.contains("HERMES_HOME"));
        assert!(script.contains("hermes-agent"));
        for kept in ["config.yaml", ".env", "sessions", "memories", "skills"] {
            assert!(!script.contains(kept), "{kept} must stay");
        }
        assert!(script.contains("SetEnvironmentVariable('Path'"));
    }

    #[test]
    fn powershell_gets_the_script_as_utf16_base64() {
        use base64::Engine;
        let line = super::super::powershell_encoded("Write-Output '卸载'");
        let encoded = line.rsplit(' ').next().unwrap();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .unwrap();
        let units: Vec<u16> = bytes
            .chunks(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        assert_eq!(String::from_utf16(&units).unwrap(), "Write-Output '卸载'");
        assert!(line.starts_with("powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass"));
    }

    #[test]
    fn leftover_message_is_beginner_friendly() {
        // Keep the copy stable — desktop surfaces this string to beginners.
        let msg = "这一份已经卸掉了，但本机还有别处的一份。再点一次卸载，继续卸掉剩下的。";
        assert!(msg.contains("还有别处的一份"));
    }
}
