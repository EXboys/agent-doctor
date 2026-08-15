//! Ensure selected MCP servers are wired into the Ask runtime before spawn.
//!
//! Ask `@mcp:` used to be prompt-only. For Claude Code / Codex / Hermes / OpenClaw
//! we upsert the Agent Doctor browser MCP entry into the config paths those CLIs load.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Context;

use agent_doctor_mcp::{
    discover_chrome, wire_browser_mcp, WireBrowserMcpOptions, BROWSER_MCP_WIRE_RUNTIMES,
};

use crate::workspace::{load_workspaces, resolve_agent_doctor_binary};

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
    prompt_requests_browser_mcp(last_user_prompt_for_intent(&options.prompt))
}

/// Ask UI may prepend conversation history. Only the current user turn should
/// decide whether this turn needs browser MCP (otherwise a later "你好" still
/// injects BROWSER MCP REQUIRED and can hang on MCP startup).
pub(crate) fn last_user_prompt_for_intent(prompt: &str) -> &str {
    let prompt = prompt.trim();
    let rest = if let Some(idx) = prompt.rfind("\nUser: ") {
        prompt[idx + "\nUser: ".len()..].trim()
    } else if let Some(stripped) = prompt.strip_prefix("User: ") {
        stripped.trim()
    } else {
        prompt
    };
    rest.strip_suffix("\n\nAssistant:")
        .or_else(|| rest.strip_suffix("\nAssistant:"))
        .unwrap_or(rest)
        .trim()
}

pub(crate) fn prompt_requests_browser_mcp(prompt: &str) -> bool {
    let lower = prompt.to_ascii_lowercase();
    if lower.contains("browser mcp")
        || lower.contains("@mcp:browser")
        || lower.contains("mcp server: browser")
        || lower.contains("browser_navigate")
        || lower.contains("mcp__browser")
    {
        return true;
    }
    // Natural-language browser automation (Chinese / English).
    if prompt.contains("浏览器") {
        return true;
    }
    if lower.contains("open browser")
        || lower.contains("launch browser")
        || lower.contains("use the browser")
        || lower.contains("with the browser")
        || lower.contains("in the browser")
        || lower.contains("navigate to")
        || lower.contains("open the page")
        || lower.contains("open webpage")
        || lower.contains("open website")
    {
        return true;
    }
    // "open https://..." / "visit baidu.com" style intents.
    if (lower.contains("open ") || lower.contains("visit ") || lower.contains("go to "))
        && (lower.contains("http://")
            || lower.contains("https://")
            || lower.contains(".com")
            || lower.contains(".cn")
            || lower.contains("baidu")
            || lower.contains("google"))
    {
        return true;
    }
    false
}

/// Extra prompt block so the model prefers MCP browser tools over `open`/`curl`.
pub(crate) fn browser_mcp_tool_instructions() -> &'static str {
    "BROWSER MCP REQUIRED for this turn:\n\
     - Use the configured MCP server named `browser`.\n\
     - Workflow: `browser_navigate` → `browser_snapshot` → act with `@eN` refs → \
`browser_snapshot` again after navigation or DOM changes.\n\
     - Prefer `browser_click` / `browser_fill` with `ref` like `@e1` from snapshot \
(not guessed CSS). CSS `selector` is only a fallback.\n\
     - If refs are awkward, use `browser_find` with strategy \
role|label|text|placeholder|testid (optional action=click|fill).\n\
     - Wait with `browser_wait` load=networkidle when SPAs are still settling.\n\
     - Persist logins via `browser_state_save` / `browser_state_load` (path or session name).\n\
     - Also available: `browser_type`, `browser_screenshot`, `browser_get_text`, \
`browser_list_tabs`.\n\
     - To open a website, call `browser_navigate` with the target URL \
(e.g. https://www.baidu.com), then `browser_snapshot`.\n\
     - NEVER use shell `open`, `xdg-open`, `osascript`, Safari/Chrome CLI, or curl/CDP hacks to browse.\n\
     - Old `@eN` refs become stale after page changes — always re-snapshot before the next click/fill."
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

/// Upsert browser MCP into Claude / Codex / Hermes / OpenClaw config paths.
///
/// Best-effort: failures are logged via the returned message but do not abort Ask.
pub(crate) fn ensure_browser_mcp_for_ask(
    runtime: &str,
    project_cwd: &Path,
    overlay: &HashMap<String, String>,
) -> Option<String> {
    if !runtime_supports_browser_mcp(runtime) {
        return Some(format!(
            "browser MCP is not wired for runtime '{runtime}' yet (supported: {})",
            BROWSER_MCP_WIRE_RUNTIMES.join(", ")
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

    let (project_path, codex_home, hermes_home, openclaw_workspace) =
        resolve_workspace_paths(project_cwd, overlay);
    let mut options = WireBrowserMcpOptions::with_binary(binary);
    options.project_path = project_path;
    options.codex_home = codex_home;
    options.hermes_home = hermes_home;
    options.openclaw_workspace = openclaw_workspace;
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
) -> (
    Option<PathBuf>,
    Option<PathBuf>,
    Option<PathBuf>,
    Option<PathBuf>,
) {
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

    let hermes_home = overlay
        .get("HERMES_HOME")
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            active.map(|entry| {
                dirs_home()
                    .join(".hermes/profiles")
                    .join(&entry.hermes_profile)
            })
        });

    let openclaw_workspace = overlay
        .get("OPENCLAW_WORKSPACE")
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| active.map(|entry| entry.openclaw_workspace.clone()));

    (project_path, codex_home, hermes_home, openclaw_workspace)
}

fn dirs_home() -> PathBuf {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

fn resolve_mcp_binary() -> anyhow::Result<PathBuf> {
    if let Ok(path) = std::env::var("AGENT_DOCTOR_BIN") {
        let path = PathBuf::from(path.trim());
        if !path.as_os_str().is_empty() && path.exists() {
            return Ok(path);
        }
    }
    // Prefer app-bundled / real CLI over a stale PATH shim (common C-end trap).
    resolve_agent_doctor_binary().context("resolve agent-doctor MCP binary")
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
        opts.prompt = "打开浏览器访问百度".into();
        assert!(wants_browser_mcp(&opts));
        opts.prompt = "open https://www.baidu.com".into();
        assert!(wants_browser_mcp(&opts));
        opts.prompt = "Conversation so far:\n\nUser: 打开浏览器访问百度\n\nAssistant: opened\n\nUser: 你好\n\nAssistant:".into();
        assert!(!wants_browser_mcp(&opts));
        assert_eq!(last_user_prompt_for_intent(&opts.prompt), "你好");
    }
}
