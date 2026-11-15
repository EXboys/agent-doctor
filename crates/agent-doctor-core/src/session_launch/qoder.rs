use std::path::Path;

use anyhow::Result;

use crate::adapters::{locate_qoder, QoderInstall};

use super::terminal::open_in_terminal;
use super::OpenSessionReport;

pub(crate) const QODER_CLI_MISSING: &str =
    "桌面里的 Qoder 留在那边。这里要用它的命令行，这台电脑还没装。";

pub(crate) fn open_qoder(cwd: &Path, prompt: Option<&str>) -> Result<OpenSessionReport> {
    open_qoder_install(locate_qoder(), cwd, prompt)
}

pub(crate) fn open_qoder_install(
    found: Option<QoderInstall>,
    cwd: &Path,
    prompt: Option<&str>,
) -> Result<OpenSessionReport> {
    match found {
        Some(QoderInstall::Cli(path)) => {
            let owned = path.to_string_lossy().into_owned();
            open_in_terminal("qoder", &[&owned], cwd, prompt)
        }
        Some(QoderInstall::App(_)) | None => anyhow::bail!(QODER_CLI_MISSING),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn desktop_app_is_not_opened() {
        let err = open_qoder_install(
            Some(QoderInstall::App(PathBuf::from("/Applications/Qoder.app"))),
            Path::new("."),
            None,
        )
        .unwrap_err();
        assert!(err.to_string().contains("命令行"));
    }
}
