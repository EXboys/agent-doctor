use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{bail, Context, Result};

use crate::exec::{shell_quote, ExecBackend, ExecOutput, DEFAULT_EXEC_TIMEOUT};

/// OpenSSH-backed execution (BatchMode; no password prompts).
#[derive(Debug, Clone)]
pub struct SshBackend {
    pub ssh_config_host: String,
    pub connect_timeout_secs: u64,
    pub command_timeout: Duration,
}

impl SshBackend {
    pub fn new(ssh_config_host: impl Into<String>) -> Self {
        Self {
            ssh_config_host: ssh_config_host.into(),
            connect_timeout_secs: 10,
            command_timeout: DEFAULT_EXEC_TIMEOUT,
        }
    }

    fn ssh_base(&self) -> Command {
        let mut cmd = Command::new("ssh");
        cmd.arg("-o")
            .arg("BatchMode=yes")
            .arg("-o")
            .arg(format!("ConnectTimeout={}", self.connect_timeout_secs))
            .arg("-o")
            .arg("StrictHostKeyChecking=accept-new")
            .arg(&self.ssh_config_host);
        cmd
    }

    fn run_remote_shell(&self, remote_script: &str) -> Result<ExecOutput> {
        let mut cmd = self.ssh_base();
        // Pass script as a single remote argv via `ssh host -- sh -c <script>`
        cmd.arg("--")
            .arg("sh")
            .arg("-c")
            .arg(remote_script)
            .stdin(Stdio::null());

        let output = cmd.output().with_context(|| {
            format!(
                "failed to spawn ssh to '{}'; is OpenSSH client installed?",
                self.ssh_config_host
            )
        })?;

        let status = output.status.code().unwrap_or(-1);
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();

        if status != 0 {
            let hint = classify_ssh_error(&stderr);
            if !hint.is_empty() {
                // Still return ExecOutput for callers that inspect status; context on hard fails
                // is added by higher layers when needed.
                let _ = hint;
            }
        }

        Ok(ExecOutput {
            status,
            stdout,
            stderr,
        })
    }
}

impl ExecBackend for SshBackend {
    fn run(&self, argv: &[&str], cwd: Option<&Path>) -> Result<ExecOutput> {
        let Some((program, args)) = argv.split_first() else {
            bail!("empty argv");
        };
        let mut parts = Vec::with_capacity(1 + args.len());
        parts.push(shell_quote(program));
        for arg in args {
            parts.push(shell_quote(arg));
        }
        let joined = parts.join(" ");
        let script = if let Some(cwd) = cwd {
            format!(
                "cd {} && {}",
                shell_quote(&cwd.display().to_string()),
                joined
            )
        } else {
            joined
        };
        self.run_remote_shell(&script)
    }

    fn read_to_string(&self, path: &Path) -> Result<String> {
        let quoted = shell_quote(&path.display().to_string());
        let output = self.run_remote_shell(&format!("cat -- {quoted}"))?;
        if !output.success() {
            bail!(
                "read {} on '{}': {}",
                path.display(),
                self.ssh_config_host,
                format_ssh_failure(&output)
            );
        }
        Ok(output.stdout)
    }

    fn exists(&self, path: &Path) -> Result<bool> {
        let quoted = shell_quote(&path.display().to_string());
        let output = self.run_remote_shell(&format!("test -e {quoted}"))?;
        Ok(output.success())
    }

    fn is_dir(&self, path: &Path) -> Result<bool> {
        let quoted = shell_quote(&path.display().to_string());
        let output = self.run_remote_shell(&format!("test -d {quoted}"))?;
        Ok(output.success())
    }

    fn home_dir(&self) -> Result<PathBuf> {
        let output = self.run_remote_shell("printf %s \"$HOME\"")?;
        if !output.success() {
            bail!(
                "resolve HOME on '{}': {}",
                self.ssh_config_host,
                format_ssh_failure(&output)
            );
        }
        let home = output.stdout_trim();
        if home.is_empty() {
            bail!("remote HOME empty on '{}'", self.ssh_config_host);
        }
        Ok(PathBuf::from(home))
    }
}

fn format_ssh_failure(output: &ExecOutput) -> String {
    let stderr = output.stderr.trim();
    let hint = classify_ssh_error(stderr);
    if hint.is_empty() {
        if stderr.is_empty() {
            format!("exit {}", output.status)
        } else {
            stderr.to_string()
        }
    } else if stderr.is_empty() {
        format!("exit {}; {hint}", output.status)
    } else {
        format!("{stderr} ({hint})")
    }
}

fn classify_ssh_error(stderr: &str) -> &'static str {
    let lower = stderr.to_ascii_lowercase();
    if lower.contains("permission denied") {
        "authentication failed — use ssh-agent / key auth (BatchMode; no password)"
    } else if lower.contains("could not resolve hostname")
        || lower.contains("name or service not known")
    {
        "host not found — check ~/.ssh/config Host alias"
    } else if lower.contains("connection timed out") || lower.contains("operation timed out") {
        "connection timed out"
    } else if lower.contains("host key verification failed") {
        "host key verification failed"
    } else if lower.contains("no such file") {
        "remote path not found"
    } else {
        ""
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::shell_quote;

    #[test]
    fn shell_quote_escapes_single_quotes() {
        assert_eq!(shell_quote("a'b"), "'a'\\''b'");
    }

    #[test]
    fn classifies_permission_denied() {
        assert!(
            classify_ssh_error("Permission denied (publickey).").contains("authentication failed")
        );
    }
}
