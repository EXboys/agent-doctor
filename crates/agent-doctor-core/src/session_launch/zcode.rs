use std::path::Path;

use anyhow::Result;

use crate::adapters::{locate_zcode_headless, ZcodeHeadless};

use super::terminal::open_in_terminal;
use super::OpenSessionReport;

pub(crate) const ZCODE_CLI_MISSING: &str =
    "桌面里的 ZCode 留在那边。这里要用它的命令行，这台电脑还没装。";

pub(crate) fn open_zcode(cwd: &Path) -> Result<OpenSessionReport> {
    open_zcode_launch(locate_zcode_headless(), cwd)
}

pub(crate) fn open_zcode_launch(
    found: Option<ZcodeHeadless>,
    cwd: &Path,
) -> Result<OpenSessionReport> {
    match found {
        Some(launch) if launch.script.is_none() => {
            let owned = launch.program.to_string_lossy().into_owned();
            open_in_terminal("zcode", &[&owned], cwd, None)
        }
        _ => anyhow::bail!(ZCODE_CLI_MISSING),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn desktop_bundle_is_not_opened() {
        let err = open_zcode_launch(
            Some(ZcodeHeadless {
                program: PathBuf::from("/Applications/ZCode.app/Contents/MacOS/ZCode"),
                script: Some(PathBuf::from(
                    "/Applications/ZCode.app/Contents/Resources/glm/zcode.cjs",
                )),
            }),
            Path::new("."),
        )
        .unwrap_err();
        assert!(err.to_string().contains("命令行"));
    }
}
