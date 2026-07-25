use crate::lifecycle::hermes::{hermes_install_shell_command, hermes_update_shell_command};
use crate::lifecycle::npm_cli::{
    claude_code_install_shell_command, claude_code_update_shell_command,
    codex_install_shell_command, codex_update_shell_command,
};
use crate::lifecycle::openclaw::{openclaw_install_shell_command, openclaw_update_shell_command};

/// Install/update shell commands the repair agent may run for a runtime.
pub fn runtime_allowed_bash_commands(runtime_id: &str) -> Vec<String> {
    match runtime_id {
        "hermes" => vec![
            hermes_install_shell_command(),
            hermes_update_shell_command(),
        ],
        "openclaw" => vec![
            openclaw_install_shell_command(),
            openclaw_update_shell_command(),
        ],
        "claude-code" => vec![
            claude_code_install_shell_command(),
            claude_code_update_shell_command(),
        ],
        "codex" => vec![
            codex_install_shell_command(),
            codex_update_shell_command(),
        ],
        _ => Vec::new(),
    }
}

pub fn bash_command_allowed_for_runtime(runtime_id: &str, command: &str) -> bool {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return false;
    }

    if runtime_allowed_bash_commands(runtime_id)
        .iter()
        .any(|allowed| allowed == trimmed)
    {
        return true;
    }

    if trimmed.starts_with("hermes ") {
        let sub = trimmed.trim_start_matches("hermes ").trim();
        return runtime_id == "hermes"
            && (sub.starts_with("update")
                || sub.starts_with("--version")
                || sub.starts_with("-V")
                || sub.starts_with("version"));
    }

    if trimmed.starts_with("openclaw ") {
        let sub = trimmed.trim_start_matches("openclaw ").trim();
        return runtime_id == "openclaw"
            && (sub.starts_with("update")
                || sub.starts_with("--version")
                || sub.starts_with("-V")
                || sub.starts_with("version"));
    }

    if trimmed.starts_with("claude ") && runtime_id == "claude-code" {
        let sub = trimmed.trim_start_matches("claude ").trim();
        return sub.starts_with("--version") || sub.starts_with("-V") || sub.starts_with("version");
    }

    if trimmed.starts_with("codex ") && runtime_id == "codex" {
        let sub = trimmed.trim_start_matches("codex ").trim();
        return sub.starts_with("--version") || sub.starts_with("-V") || sub.starts_with("version");
    }

    trimmed.starts_with("chmod ") && trimmed.contains(".env")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_claude_npm_install_for_claude_runtime() {
        assert!(bash_command_allowed_for_runtime(
            "claude-code",
            "npm install -g @anthropic-ai/claude-code"
        ));
        assert!(!bash_command_allowed_for_runtime(
            "hermes",
            "npm install -g @anthropic-ai/claude-code"
        ));
    }

    #[test]
    fn allows_codex_npm_install() {
        assert!(bash_command_allowed_for_runtime(
            "codex",
            "npm install -g @openai/codex"
        ));
        assert!(bash_command_allowed_for_runtime(
            "codex",
            "npm install -g @openai/codex@latest"
        ));
    }
}
