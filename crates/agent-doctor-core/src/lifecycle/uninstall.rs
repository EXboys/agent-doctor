use anyhow::{anyhow, Result};

use crate::adapters::DEEPSEEK_HARNESS_NPM_PACKAGE;

use super::runner::run_shell_command;

pub fn uninstall_shell_command(runtime_id: &str) -> Result<String> {
    let package = match runtime_id {
        "claude-code" => "@anthropic-ai/claude-code",
        "codex" => "@openai/codex",
        "deepseek-harness" => DEEPSEEK_HARNESS_NPM_PACKAGE,
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

pub fn uninstall_runtime(runtime_id: &str) -> Result<()> {
    let command = uninstall_shell_command(runtime_id)?;
    if runtime_id != "hermes" {
        crate::lifecycle::nodejs::ensure_npm().map_err(|_| anyhow!("卸掉需要本机已安装 npm。"))?;
    }
    run_shell_command(&command)
}

#[cfg(test)]
mod tests {
    use super::uninstall_shell_command;

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
}
