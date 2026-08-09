use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};

use super::{ExecBackend, ExecOutput};

/// Local filesystem / process backend (tests and future local probe migration).
#[derive(Debug, Default, Clone)]
pub struct LocalBackend;

impl ExecBackend for LocalBackend {
    fn run(&self, argv: &[&str], cwd: Option<&Path>) -> Result<ExecOutput> {
        let Some((program, args)) = argv.split_first() else {
            bail!("empty argv");
        };
        let mut cmd = Command::new(program);
        cmd.args(args);
        if let Some(cwd) = cwd {
            cmd.current_dir(cwd);
        }
        let output = cmd
            .output()
            .with_context(|| format!("failed to run local command: {program}"))?;
        Ok(ExecOutput {
            status: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }

    fn read_to_string(&self, path: &Path) -> Result<String> {
        fs::read_to_string(path).with_context(|| format!("read {}", path.display()))
    }

    fn exists(&self, path: &Path) -> Result<bool> {
        Ok(path.exists())
    }

    fn is_dir(&self, path: &Path) -> Result<bool> {
        Ok(path.is_dir())
    }

    fn home_dir(&self) -> Result<PathBuf> {
        dirs::home_dir().context("home directory not found")
    }
}
