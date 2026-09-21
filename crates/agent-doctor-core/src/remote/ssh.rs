use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{bail, Context, Result};

use crate::exec::{shell_quote, ExecBackend, ExecOutput, DEFAULT_EXEC_TIMEOUT};

use super::registry::RemoteHostEntry;

/// How to address a remote host over OpenSSH.
#[derive(Debug, Clone)]
pub enum SshTarget {
    /// Legacy / advanced: `Host` alias from ~/.ssh/config.
    ConfigHost { host: String },
    /// Managed bootstrap path: user@hostname + identity file.
    Direct {
        user: String,
        hostname: String,
        port: u16,
        identity_file: PathBuf,
    },
}

impl SshTarget {
    pub fn from_host_entry(entry: &RemoteHostEntry) -> Result<Self> {
        if entry.is_managed() {
            let hostname = entry
                .hostname
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .context("managed host missing hostname")?
                .to_string();
            let identity = entry
                .identity_file
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .context("managed host missing identity_file")?;
            Ok(Self::Direct {
                user: entry.managed_user().to_string(),
                hostname,
                port: entry.managed_port(),
                identity_file: PathBuf::from(identity),
            })
        } else {
            let host = entry.ssh_config_host.trim();
            if host.is_empty() {
                bail!("host entry has neither managed identity nor ssh_config_host");
            }
            Ok(Self::ConfigHost {
                host: host.to_string(),
            })
        }
    }

    pub fn label(&self) -> String {
        match self {
            Self::ConfigHost { host } => host.clone(),
            Self::Direct {
                user,
                hostname,
                port,
                ..
            } => {
                if *port == 22 {
                    format!("{user}@{hostname}")
                } else {
                    format!("{user}@{hostname}:{port}")
                }
            }
        }
    }
}

/// OpenSSH-backed execution (BatchMode; no password prompts).
#[derive(Debug, Clone)]
pub struct SshBackend {
    pub target: SshTarget,
    pub connect_timeout_secs: u64,
    pub command_timeout: Duration,
}

impl SshBackend {
    pub fn new(ssh_config_host: impl Into<String>) -> Self {
        Self {
            target: SshTarget::ConfigHost {
                host: ssh_config_host.into(),
            },
            connect_timeout_secs: 10,
            command_timeout: DEFAULT_EXEC_TIMEOUT,
        }
    }

    pub fn from_host_entry(entry: &RemoteHostEntry) -> Result<Self> {
        Ok(Self {
            target: SshTarget::from_host_entry(entry)?,
            connect_timeout_secs: 10,
            command_timeout: DEFAULT_EXEC_TIMEOUT,
        })
    }

    pub fn direct(
        user: impl Into<String>,
        hostname: impl Into<String>,
        port: u16,
        identity_file: impl Into<PathBuf>,
    ) -> Self {
        Self {
            target: SshTarget::Direct {
                user: user.into(),
                hostname: hostname.into(),
                port,
                identity_file: identity_file.into(),
            },
            connect_timeout_secs: 10,
            command_timeout: DEFAULT_EXEC_TIMEOUT,
        }
    }

    /// Back-compat accessor for callers that only know config-host mode.
    pub fn ssh_config_host(&self) -> String {
        self.target.label()
    }

    fn ssh_base(&self) -> Command {
        let mut cmd = Command::new("ssh");
        cmd.arg("-o")
            .arg("BatchMode=yes")
            .arg("-o")
            .arg(format!("ConnectTimeout={}", self.connect_timeout_secs))
            .arg("-o")
            .arg("ConnectionAttempts=1")
            .arg("-o")
            .arg("GSSAPIAuthentication=no")
            .arg("-o")
            .arg("PreferredAuthentications=publickey")
            .arg("-o")
            .arg("NumberOfPasswordPrompts=0")
            .arg("-o")
            .arg("StrictHostKeyChecking=accept-new");
        match &self.target {
            SshTarget::ConfigHost { host } => {
                cmd.arg(host);
            }
            SshTarget::Direct {
                user,
                hostname,
                port,
                identity_file,
            } => {
                cmd.arg("-i")
                    .arg(identity_file)
                    .arg("-o")
                    .arg("IdentitiesOnly=yes")
                    .arg("-p")
                    .arg(port.to_string())
                    .arg(format!("{user}@{hostname}"));
            }
        }
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

        let label = self.target.label();
        let output = cmd.output().with_context(|| {
            format!("failed to spawn ssh to '{label}'; is OpenSSH client installed?")
        })?;

        let status = output.status.code().unwrap_or(-1);
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();

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
                self.target.label(),
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
                self.target.label(),
                format_ssh_failure(&output)
            );
        }
        let home = output.stdout_trim();
        if home.is_empty() {
            bail!("remote HOME empty on '{}'", self.target.label());
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
        "authentication failed — use key auth (BatchMode; no password)"
    } else if lower.contains("could not resolve hostname")
        || lower.contains("name or service not known")
    {
        "host not found — check hostname / ~/.ssh/config Host alias"
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
    use crate::remote::registry::RemoteHostEntry;
    use std::collections::BTreeMap;

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

    #[test]
    fn target_from_managed_entry() {
        let entry = RemoteHostEntry {
            ssh_config_host: String::new(),
            hostname: Some("10.0.0.1".into()),
            user: Some("ubuntu".into()),
            port: Some(2222),
            identity_file: Some("/keys/box".into()),
            projects: BTreeMap::new(),
        };
        let target = SshTarget::from_host_entry(&entry).unwrap();
        match target {
            SshTarget::Direct {
                user,
                hostname,
                port,
                identity_file,
            } => {
                assert_eq!(user, "ubuntu");
                assert_eq!(hostname, "10.0.0.1");
                assert_eq!(port, 2222);
                assert_eq!(identity_file, PathBuf::from("/keys/box"));
            }
            other => panic!("expected Direct, got {other:?}"),
        }
    }

    #[test]
    fn target_from_legacy_entry() {
        let entry = RemoteHostEntry {
            ssh_config_host: "prod-vps".into(),
            hostname: None,
            user: None,
            port: None,
            identity_file: None,
            projects: BTreeMap::new(),
        };
        let target = SshTarget::from_host_entry(&entry).unwrap();
        match target {
            SshTarget::ConfigHost { host } => assert_eq!(host, "prod-vps"),
            other => panic!("expected ConfigHost, got {other:?}"),
        }
    }
}
