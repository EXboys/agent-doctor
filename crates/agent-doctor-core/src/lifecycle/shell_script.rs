//! The shared native shell-script installer.
//!
//! Cursor, Qoder and WorkBuddy all install through the same curl-to-temp-file
//! bash one-liner on Unix and `irm | iex` PowerShell one-liner on Windows,
//! with an update path that tries the CLI's own `update` command first. The
//! templates live here once; each runtime only names its URLs and display
//! name, so a fix to the curl flags lands everywhere at the same time.

use anyhow::{Context, Result};

use super::runner::run_shell_command;

/// One native shell-script installer, described by its runtime.
pub(crate) struct ShellScriptLifecycle {
    /// Display name in the failure context, e.g. `"Cursor"`.
    pub(crate) display_name: &'static str,
    /// The CLI that updates itself, tried before the script fallback.
    pub(crate) update_cli: &'static str,
    /// `bash $tmp` argument on the update fallback: `" --force"` or `""`.
    pub(crate) update_bash_args: &'static str,
    /// Unix install script URL. A function because WorkBuddy picks a mirror.
    /// Only read on Unix.
    #[cfg_attr(target_os = "windows", allow(dead_code))]
    pub(crate) unix_script_url: fn() -> &'static str,
    /// The `irm` argument on Windows — a URL, quoted when it needs it
    /// (Cursor's carries a `?win32=true` query). Only read on Windows.
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    pub(crate) windows_irm_arg: fn() -> &'static str,
}

impl ShellScriptLifecycle {
    /// The install one-liner for this platform.
    pub(crate) fn install_command(&self) -> String {
        self.script_command("")
    }

    /// The update one-liner for this platform: the CLI's own `update` first,
    /// the install script as the fallback.
    pub(crate) fn update_command(&self) -> String {
        format!(
            "{} update || {}",
            self.update_cli,
            self.script_command(self.update_bash_args)
        )
    }

    pub(crate) fn run_install(&self) -> Result<()> {
        self.run(self.install_command(), "install")
    }

    pub(crate) fn run_update(&self) -> Result<()> {
        self.run(self.update_command(), "update")
    }

    fn script_command(&self, bash_args: &str) -> String {
        #[cfg(target_os = "windows")]
        {
            let _ = bash_args;
            format!(
                r#"powershell -NoProfile -ExecutionPolicy Bypass -Command "irm {} | iex""#,
                (self.windows_irm_arg)()
            )
        }
        #[cfg(not(target_os = "windows"))]
        {
            format!(
                "bash -c 'tmp=$(mktemp) && curl -fsSL --proto \"=https\" --tlsv1.2 \
                {} -o $tmp && bash $tmp{}; \
                status=$?; rm -f $tmp; exit $status'",
                (self.unix_script_url)(),
                bash_args
            )
        }
    }

    fn run(&self, command: String, action: &str) -> Result<()> {
        run_shell_command(&command)
            .with_context(|| format!("{} {} failed", self.display_name, action))
    }
}
