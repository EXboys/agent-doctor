use std::path::Path;
#[cfg(target_os = "windows")]
use std::process::Stdio;

use anyhow::Result;
use url::form_urlencoded;

#[cfg(windows)]
use crate::profile::read_company_profile;
#[cfg(windows)]
use crate::setup::anthropic_gateway_url_from_evotown_base;
use crate::setup::merge::apply_claude_code;

use super::*;

pub fn claude_cli_deep_link(cwd: &Path, prompt: Option<&str>) -> String {
    let mut pairs: Vec<(&str, String)> = Vec::new();
    let cwd_str = cwd.to_string_lossy().into_owned();
    if !cwd_str.is_empty() {
        pairs.push(("cwd", cwd_str));
    }
    if let Some(q) = prompt.map(str::trim).filter(|s| !s.is_empty()) {
        // Keep under Claude's documented 5000-char cap for `q`.
        let clipped: String = q.chars().take(5000).collect();
        pairs.push(("q", clipped));
    }
    let query = form_urlencoded::Serializer::new(String::new())
        .extend_pairs(pairs)
        .finish();
    if query.is_empty() {
        "claude-cli://open".to_string()
    } else {
        format!("claude-cli://open?{query}")
    }
}

pub(crate) fn open_claude_code(
    cwd: &Path,
    prompt: Option<&str>,
    prefer_deep_link: bool,
) -> Result<OpenSessionReport> {
    let refreshed = match resolve_claude_launch_env() {
        Some((url, key)) => apply_claude_code(&url, &key).ok().map(|_| url),
        None => None,
    };

    if prefer_deep_link {
        let link = claude_cli_deep_link(cwd, prompt);
        if open_url(&link).is_ok() {
            let detail = if let Some(url) = refreshed.as_deref() {
                format!(
                    "Opened Claude Code via claude-cli:// deep link after writing ANTHROPIC_BASE_URL={url} to ~/.claude/settings.json (prompt pre-filled, not auto-sent). Restart Claude if it was already running."
                )
            } else {
                "Opened Claude Code via claude-cli:// deep link (prompt pre-filled, not auto-sent)."
                    .into()
            };
            return Ok(OpenSessionReport {
                runtime: "claude-code".into(),
                method: OpenSessionMethod::DeepLink,
                cwd: cwd.display().to_string(),
                target: link,
                detail,
            });
        }
    }

    // Interactive CLI: wrap exports ANTHROPIC_* so the process does not hit api.anthropic.com.
    let _ = prompt;
    open_in_terminal("claude-code", &["claude"], cwd, None)
}
