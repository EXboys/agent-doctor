//! Ensure selected MCP servers are wired into the Ask runtime before spawn.
//!
//! Ask `@mcp:` used to be prompt-only. For Claude Code / Codex we upsert the
//! Agent Doctor browser MCP entry into the same config paths those CLIs load.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Context;

use agent_doctor_mcp::{
    discover_chrome, wire_browser_mcp, WireBrowserMcpOptions, BROWSER_MCP_WIRE_RUNTIMES,
};

use crate::adapters::util::find_binary;
use crate::workspace::load_workspaces;

use super::PromptSessionOptions;

/// True when this Ask turn should expose browser MCP tools.
pub(crate) fn wants_browser_mcp(options: &PromptSessionOptions) -> bool {
    if options
        .selected_mcps
        .iter()
        .any(|name| is_browser_mcp_name(name))
    {
        return true;
    }
    let prompt = options.prompt.to_ascii_lowercase();
    prompt.contains("browser mcp")
        || prompt.contains("@mcp:browser")
        || prompt.contains("mcp server: browser")
        || (options.prompt.contains("浏览器")
            && options.prompt.to_ascii_lowercase().contains("mcp"))
}

fn is_browser_mcp_name(name: &str) -> bool {
    let trimmed = name.trim().to_ascii_lowercase();
    trimmed == "browser" || trimmed.contains("browser")
}

pub(crate) fn runtime_supports_browser_mcp(runtime: &str) -> bool {
    BROWSER_MCP_WIRE_RUNTIMES
        .iter()
        .any(|id| id.eq_ignore_ascii_case(runtime.trim()))
}

/// Upsert browser MCP into Claude project `.mcp.json` / Codex `$CODEX_HOME`.
///
/// Best-effort: failures are logged via the returned message but do not abort Ask.
pub(crate) fn ensure_browser_mcp_for_ask(
    runtime: &str,
    project_cwd: &Path,
    overlay: &HashMap<String, String>,
) -> Option<String> {
    if !runtime_supports_browser_mcp(runtime) {
        return Some(format!(
            "browser MCP is not wired for runtime '{runtime}' yet (supported: claude-code, codex)"
        ));
    }

    let discovery = match discover_chrome() {
        Ok(d) => d,
        Err(err) => return Some(format!("browser MCP skipped: Chrome not found ({err})")),
    };
    let binary = match resolve_mcp_binary() {
        Ok(path) => path,
        Err(err) => return Some(format!("browser MCP skipped: {err}")),
    };

    let (project_path, codex_home) = resolve_workspace_paths(project_cwd, overlay);
    let mut options = WireBrowserMcpOptions::with_binary(binary);
    options.project_path = project_path;
    options.codex_home = codex_home;
    options.runtimes = vec![runtime.to_string()];

    let report = wire_browser_mcp(&discovery, &options);
    let mut notes = Vec::new();
    for item in report.results {
        if item.ok {
            notes.push(format!(
                "browser MCP ready for {} ({})",
                item.runtime,
                item.config_path.unwrap_or_else(|| "config".into())
            ));
        } else {
            notes.push(format!(
                "browser MCP wire failed for {}: {}",
                item.runtime, item.message
            ));
        }
    }
    if notes.is_empty() {
        None
    } else {
        Some(notes.join("; "))
    }
}

fn resolve_workspace_paths(
    project_cwd: &Path,
    overlay: &HashMap<String, String>,
) -> (Option<PathBuf>, Option<PathBuf>) {
    let doc = load_workspaces().unwrap_or_default();
    let active = doc
        .active
        .as_ref()
        .and_then(|name| doc.workspaces.get(name));

    let project_path = active
        .map(|entry| entry.path.clone())
        .or_else(|| Some(project_cwd.to_path_buf()));

    let codex_home = overlay
        .get("CODEX_HOME")
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| active.map(|entry| entry.codex_home.clone()));

    (project_path, codex_home)
}

fn resolve_mcp_binary() -> anyhow::Result<PathBuf> {
    if let Ok(path) = std::env::var("AGENT_DOCTOR_BIN") {
        let path = PathBuf::from(path.trim());
        if path.as_os_str().is_empty() {
            // fall through
        } else if path.exists() {
            return Ok(path);
        }
    }
    for name in ["agent-doctor-cli", "agent-doctor"] {
        if let Some(path) = find_binary(name) {
            return Ok(path);
        }
    }
    let exe = std::env::current_exe().context("resolve current executable")?;
    Ok(exe)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_browser_selection_and_prompt() {
        let mut opts = PromptSessionOptions {
            runtime: "codex".into(),
            prompt: "hello".into(),
            cwd: None,
            timeout_sec: 30,
            dangerously_skip_permissions: false,
            full_auto: false,
            resume_thread_id: None,
            selected_mcps: vec!["browser".into()],
        };
        assert!(wants_browser_mcp(&opts));
        opts.selected_mcps.clear();
        assert!(!wants_browser_mcp(&opts));
        opts.prompt = "请用浏览器 MCP 打开页面".into();
        assert!(wants_browser_mcp(&opts));
    }
}
