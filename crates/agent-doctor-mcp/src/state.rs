//! Named browser session state (cookies + storage) on disk.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

/// `~/.config/agent-doctor/browser-sessions` (or platform equivalent).
pub fn browser_sessions_dir() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));
    if cfg!(target_os = "macos") {
        home.join("Library/Application Support/agent-doctor/browser-sessions")
    } else if cfg!(target_os = "windows") {
        home.join(r"AppData\Local\agent-doctor\browser-sessions")
    } else {
        home.join(".config/agent-doctor/browser-sessions")
    }
}

/// Resolve a state file from an explicit path and/or session name.
///
/// - `path` wins when set
/// - else `session` → `{sessions_dir}/{session}.json`
pub fn resolve_state_path(path: Option<&str>, session: Option<&str>) -> Result<PathBuf> {
    if let Some(p) = path.map(str::trim).filter(|s| !s.is_empty()) {
        return Ok(PathBuf::from(p));
    }
    if let Some(name) = session.map(str::trim).filter(|s| !s.is_empty()) {
        if name.contains('/') || name.contains('\\') || name.contains("..") {
            bail!("invalid session name: {name}");
        }
        let dir = browser_sessions_dir();
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("create sessions dir {}", dir.display()))?;
        return Ok(dir.join(format!("{name}.json")));
    }
    bail!("provide path or session name for browser state")
}

pub fn ensure_parent_dir(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create state dir {}", parent.display()))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_name_resolves_under_sessions_dir() {
        let path = resolve_state_path(None, Some("myapp")).unwrap();
        assert!(path.ends_with("myapp.json"));
        assert!(path.to_string_lossy().contains("browser-sessions"));
    }

    #[test]
    fn explicit_path_wins() {
        let path = resolve_state_path(Some("/tmp/auth.json"), Some("ignored")).unwrap();
        assert_eq!(path, PathBuf::from("/tmp/auth.json"));
    }
}
