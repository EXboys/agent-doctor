use anyhow::{anyhow, bail, Result};

use crate::adapters::util::{ensure_managed_runtime_path, find_all_binaries, home_join};
use crate::adapters::DEEPSEEK_HARNESS_NPM_PACKAGE;

use super::npm_target::{
    leftover_npm_cli_binaries, npm_uninstall_global_command, resolve_active_npm_prefix,
};
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
                "rm -f \"$HOME/.local/bin/qoder\" \"$HOME/.qoder/bin/qoder\"; npm uninstall -g @qoder-ai/qodercli 2>/dev/null || true"
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

fn uninstall_npm_package(package: &str, binary_name: &str) -> Result<()> {
    crate::lifecycle::nodejs::ensure_npm().map_err(|_| anyhow!("卸掉需要本机已安装 npm。"))?;

    let active = resolve_active_npm_prefix(binary_name)?;
    let removed_path = active.detected_binary.clone();
    run_shell_command(&npm_uninstall_global_command(package, &active.prefix))?;

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

pub fn uninstall_runtime(runtime_id: &str) -> Result<()> {
    match runtime_id {
        "qoder" => uninstall_native_cli(&["qoder"], "@qoder-ai/qodercli", "qoder"),
        "workbuddy" => uninstall_native_cli(
            &["codebuddy", "workbuddy"],
            "@tencent-ai/codebuddy-code",
            "codebuddy",
        ),
        "cursor" => uninstall_cursor_cli(),
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
    fn leftover_message_is_beginner_friendly() {
        // Keep the copy stable — desktop surfaces this string to beginners.
        let msg = "这一份已经卸掉了，但本机还有别处的一份。再点一次卸载，继续卸掉剩下的。";
        assert!(msg.contains("还有别处的一份"));
    }
}
