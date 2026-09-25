use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::adapters::util::home_join;

use super::shell::{
    bash_hook_file_path, fish_hook_file_path, hook_file_path, powershell_hook_file_path,
};

#[derive(Debug, Clone, serde::Serialize)]
pub struct ShellHookStatus {
    pub shell: &'static str,
    pub hook_path: PathBuf,
    pub hook_installed: bool,
    pub rc_file: Option<PathBuf>,
    pub rc_sources_hook: bool,
}

pub fn workspace_hook_status() -> Result<Vec<ShellHookStatus>> {
    Ok(vec![
        inspect_shell_hook("zsh", &hook_file_path()?, home_join(".zshrc"))?,
        inspect_shell_hook("bash", &bash_hook_file_path()?, home_join(".bashrc"))?,
        inspect_shell_hook(
            "fish",
            &fish_hook_file_path()?,
            home_join(".config/fish/config.fish"),
        )?,
        inspect_shell_hook(
            "powershell",
            &powershell_hook_file_path()?,
            powershell_profile_path(),
        )?,
    ])
}

fn inspect_shell_hook(
    shell: &'static str,
    hook_path: &Path,
    rc_file: PathBuf,
) -> Result<ShellHookStatus> {
    let hook_installed = hook_path.exists();
    let rc_sources_hook = rc_file.exists()
        && fs::read_to_string(&rc_file)
            .map(|contents| rc_references_hook(&contents, hook_path))
            .unwrap_or(false);

    Ok(ShellHookStatus {
        shell,
        hook_path: hook_path.to_path_buf(),
        hook_installed,
        rc_file: rc_file.exists().then_some(rc_file),
        rc_sources_hook,
    })
}

fn rc_references_hook(contents: &str, hook_path: &Path) -> bool {
    let hook = hook_path.display().to_string();
    let hook_name = hook_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    contents.contains(&hook)
        || contents.contains("agent-doctor/hooks/workspace")
        || (hook_name.contains("workspace") && contents.contains(hook_name))
}

fn powershell_profile_path() -> PathBuf {
    // Windows PowerShell lives under Documents. Never probe that path on
    // macOS/Linux — `exists()` on ~/Documents trips the Files-and-Folders prompt.
    #[cfg(windows)]
    {
        if let Ok(profile) = std::env::var("USERPROFILE") {
            return PathBuf::from(profile)
                .join("Documents")
                .join("PowerShell")
                .join("Microsoft.PowerShell_profile.ps1");
        }
        return home_join("Documents/PowerShell/Microsoft.PowerShell_profile.ps1");
    }
    #[cfg(not(windows))]
    {
        home_join(".config/powershell/Microsoft.PowerShell_profile.ps1")
    }
}

#[cfg(test)]
mod tests {
    use super::powershell_profile_path;

    #[test]
    fn unix_powershell_profile_skips_documents_folder() {
        #[cfg(not(windows))]
        {
            let path = powershell_profile_path();
            let rendered = path.to_string_lossy();
            assert!(
                !rendered.contains("/Documents/") && !rendered.contains("/文稿/"),
                "must not stat ~/Documents on Unix: {rendered}"
            );
            assert!(rendered.contains(".config/powershell"));
        }
        #[cfg(windows)]
        {
            let path = powershell_profile_path();
            assert!(path.to_string_lossy().contains("Documents"));
        }
    }
}
