//! Host execution backends for local and remote (SSH) operations.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;

mod local;

pub use local::LocalBackend;

/// Read-only / command execution surface used by remote doctor (and later repair).
pub trait ExecBackend {
    /// Run a command and return stdout (trimmed trailing newlines preserved in raw form).
    fn run(&self, argv: &[&str], cwd: Option<&Path>) -> Result<ExecOutput>;

    /// Read a remote/local file as UTF-8 text.
    fn read_to_string(&self, path: &Path) -> Result<String>;

    /// Whether a path exists (file or directory).
    fn exists(&self, path: &Path) -> Result<bool>;

    /// Whether path is a directory.
    fn is_dir(&self, path: &Path) -> Result<bool>;

    /// Resolve `$HOME` on the target host.
    fn home_dir(&self) -> Result<PathBuf>;
}

#[derive(Debug, Clone)]
pub struct ExecOutput {
    pub status: i32,
    pub stdout: String,
    pub stderr: String,
}

impl ExecOutput {
    pub fn success(&self) -> bool {
        self.status == 0
    }

    pub fn stdout_trim(&self) -> &str {
        self.stdout.trim()
    }
}

/// Default command timeout for remote/local exec helpers that honor it.
pub const DEFAULT_EXEC_TIMEOUT: Duration = Duration::from_secs(30);

/// Escape a path for use inside a single-quoted remote shell argument.
pub fn shell_quote(path: &str) -> String {
    format!("'{}'", path.replace('\'', "'\\''"))
}
