use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::{json, Value as JsonValue};

use crate::adapters::util::home_join;

#[derive(Debug, Clone, serde::Serialize)]
pub struct ClaudeMcpMigrationReport {
    pub applied: bool,
    pub dry_run: bool,
    pub project_mcp_path: PathBuf,
    pub global_sources: Vec<String>,
    pub added_servers: Vec<String>,
    pub skipped_servers: Vec<String>,
    pub conflict_servers: Vec<String>,
}

pub fn migrate_claude_global_mcp_to_project(
    project_path: &Path,
    dry_run: bool,
) -> Result<ClaudeMcpMigrationReport> {
    let (global_servers, sources) = collect_global_mcp_servers();
    let project_mcp = project_path.join(".mcp.json");

    let mut existing = BTreeMap::new();
    if project_mcp.exists() {
        if let Ok(raw) = fs::read_to_string(&project_mcp) {
            if let Ok(value) = serde_json::from_str::<JsonValue>(&raw) {
                if let Some(map) = value.get("mcpServers").and_then(JsonValue::as_object) {
                    for (key, val) in map {
                        existing.insert(key.clone(), val.clone());
                    }
                }
            }
        }
    }

    let mut added = Vec::new();
    let mut skipped = Vec::new();
    let mut conflicts = Vec::new();
    let mut merged = existing.clone();

    for (name, server) in global_servers {
        if existing.contains_key(&name) {
            if existing.get(&name) == Some(&server) {
                skipped.push(name);
            } else {
                conflicts.push(name);
            }
            continue;
        }
        merged.insert(name.clone(), server);
        added.push(name);
    }

    let applied = !dry_run && !added.is_empty();
    if applied {
        let payload = json!({ "mcpServers": merged });
        fs::write(
            &project_mcp,
            format!("{}\n", serde_json::to_string_pretty(&payload)?),
        )
        .with_context(|| format!("write {}", project_mcp.display()))?;
    }

    Ok(ClaudeMcpMigrationReport {
        applied,
        dry_run,
        project_mcp_path: project_mcp,
        global_sources: sources,
        added_servers: added,
        skipped_servers: skipped,
        conflict_servers: conflicts,
    })
}

fn collect_global_mcp_servers() -> (BTreeMap<String, JsonValue>, Vec<String>) {
    let mut servers = BTreeMap::new();
    let mut sources = Vec::new();

    let settings = home_join(".claude/settings.json");
    if settings.exists() {
        if let Ok(raw) = fs::read_to_string(&settings) {
            if let Ok(value) = serde_json::from_str::<JsonValue>(&raw) {
                if let Some(map) = value.get("mcpServers").and_then(JsonValue::as_object) {
                    for (key, val) in map {
                        servers.insert(key.clone(), val.clone());
                    }
                    if !map.is_empty() {
                        sources.push(format!("{} ({} servers)", settings.display(), map.len()));
                    }
                }
            }
        }
    }

    let claude_json = home_join(".claude.json");
    if claude_json.exists() {
        if let Ok(raw) = fs::read_to_string(&claude_json) {
            if let Ok(value) = serde_json::from_str::<JsonValue>(&raw) {
                if let Some(map) = value.get("mcpServers").and_then(JsonValue::as_object) {
                    let before = servers.len();
                    for (key, val) in map {
                        servers.entry(key.clone()).or_insert_with(|| val.clone());
                    }
                    if !map.is_empty() {
                        sources.push(format!(
                            "{} ({} global servers, {} merged)",
                            claude_json.display(),
                            map.len(),
                            servers.len().saturating_sub(before)
                        ));
                    }
                }
            }
        }
    }

    (servers, sources)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn migrate_copies_global_servers_to_project() {
        let temp = env::temp_dir().join(format!("ad-mcp-migrate-{}", std::process::id()));
        let _ = fs::remove_dir_all(&temp);
        fs::create_dir_all(&temp).unwrap();

        let claude_dir = temp.join(".claude");
        fs::create_dir_all(&claude_dir).unwrap();
        fs::write(
            claude_dir.join("settings.json"),
            r#"{"mcpServers":{"global-demo":{"command":"echo","args":["demo"]}}}"#,
        )
        .unwrap();

        let project = temp.join("project");
        fs::create_dir_all(&project).unwrap();

        let previous = env::var_os("HOME");
        // SAFETY: test-only HOME override, restored below.
        unsafe { env::set_var("HOME", &temp) };
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let report = migrate_claude_global_mcp_to_project(&project, false).unwrap();
            assert!(report.applied);
            assert!(report.added_servers.contains(&"global-demo".to_string()));
            assert!(project.join(".mcp.json").exists());
        }));
        match previous {
            Some(value) => unsafe { env::set_var("HOME", value) },
            None => unsafe { env::remove_var("HOME") },
        }
        let _ = fs::remove_dir_all(&temp);
        if let Err(payload) = result {
            std::panic::resume_unwind(payload);
        }
    }
}
