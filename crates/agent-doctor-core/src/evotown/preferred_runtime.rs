//! Local preferred dispatch runtime (`EVOTOWN_RUNTIME`) — aligned with Evotown inventory.

use std::fs;
use std::path::Path;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::doctor::run_doctor;
use crate::evotown::jobs::{is_known_dispatch_runtime, known_dispatch_runtimes, normalize_runtime};
use crate::setup::{evotown_agent_env_path, EVOTOWN_RUNTIME_ENV};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreferredRuntimeStatus {
    pub runtime: Option<String>,
    pub installed: bool,
    pub env_path: Option<String>,
    pub known_runtimes: Vec<String>,
}

pub fn read_preferred_runtime_from_env_file(path: &Path) -> Option<String> {
    let raw = fs::read_to_string(path).ok()?;
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || !line.contains('=') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        if k.trim() == EVOTOWN_RUNTIME_ENV {
            let value = v.trim().trim_matches('"').trim();
            if !value.is_empty() {
                return Some(normalize_runtime(value));
            }
        }
    }
    None
}

pub fn resolve_preferred_runtime() -> Option<String> {
    if let Ok(v) = std::env::var(EVOTOWN_RUNTIME_ENV) {
        let t = v.trim();
        if !t.is_empty() {
            return Some(normalize_runtime(t));
        }
    }
    let path = evotown_agent_env_path()?;
    if path.exists() {
        return read_preferred_runtime_from_env_file(&path);
    }
    None
}

pub fn preferred_runtime_status() -> PreferredRuntimeStatus {
    let env_path = evotown_agent_env_path().map(|p| p.display().to_string());
    let runtime = resolve_preferred_runtime();
    let installed = runtime
        .as_ref()
        .map(|r| {
            run_doctor()
                .runtimes
                .iter()
                .any(|rt| rt.id == *r && rt.installed)
        })
        .unwrap_or(false);
    PreferredRuntimeStatus {
        runtime,
        installed,
        env_path,
        known_runtimes: known_dispatch_runtimes()
            .iter()
            .map(|s| (*s).to_string())
            .collect(),
    }
}

/// Upsert `EVOTOWN_RUNTIME` in evotown.agent.env without wiping other keys.
pub fn set_preferred_runtime(runtime: &str) -> Result<PreferredRuntimeStatus> {
    let normalized = normalize_runtime(runtime);
    if !is_known_dispatch_runtime(&normalized) {
        bail!(
            "unknown runtime '{runtime}' — expected one of: {}",
            known_dispatch_runtimes().join(", ")
        );
    }

    let path = evotown_agent_env_path().context("could not resolve evotown.agent.env path")?;
    crate::setup::upsert_evotown_agent_env_key(&path, EVOTOWN_RUNTIME_ENV, &normalized)?;

    let mut status = preferred_runtime_status();
    // Ensure we reflect the value just written even if process env overrides.
    status.runtime = Some(normalized.clone());
    status.installed = run_doctor()
        .runtimes
        .iter()
        .any(|rt| rt.id == normalized && rt.installed);
    status.env_path = Some(path.display().to_string());
    Ok(status)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn upsert_preserves_other_keys() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("evotown.agent.env");
        fs::write(
            &path,
            "EVOTOWN_URL=https://www.skilllite.ai\nEVOTOWN_ENGINE_ID=doctor-1\nEVOTOWN_RUNTIME=openclaw\n",
        )
        .unwrap();
        crate::setup::upsert_evotown_agent_env_key(&path, EVOTOWN_RUNTIME_ENV, "claude-code")
            .unwrap();
        let raw = fs::read_to_string(&path).unwrap();
        assert!(raw.contains("EVOTOWN_URL=https://www.skilllite.ai"));
        assert!(raw.contains("EVOTOWN_ENGINE_ID=doctor-1"));
        assert!(raw.contains("EVOTOWN_RUNTIME=claude-code"));
        assert!(!raw.contains("EVOTOWN_RUNTIME=openclaw"));
    }

    #[test]
    fn read_preferred_normalizes() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("evotown.agent.env");
        fs::write(&path, "EVOTOWN_RUNTIME=claude\n").unwrap();
        assert_eq!(
            read_preferred_runtime_from_env_file(&path).as_deref(),
            Some("claude-code")
        );
    }
}
