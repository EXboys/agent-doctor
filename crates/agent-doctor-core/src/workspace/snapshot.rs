use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::WorkspaceEntry;
use crate::adapters::util::home_join;

const SKILLS_SNAPSHOT: &str = "snapshots/skills";

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct SnapshotReport {
    pub mcp_saved: bool,
    pub mcp_applied: bool,
    pub skills_saved: bool,
    pub skills_applied: bool,
}

pub fn snapshot_dir(data_root: &Path) -> PathBuf {
    data_root.join("snapshots")
}

pub fn save_workspace_snapshot(entry: &WorkspaceEntry, data_root: &Path) -> Result<SnapshotReport> {
    let mut report = SnapshotReport::default();
    let dir = snapshot_dir(data_root);
    fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;

    report.mcp_saved = save_mcp_snapshot(&entry.path, &dir.join("mcp.json"))?;
    report.skills_saved = save_skills_snapshot(&entry.path, &dir.join(SKILLS_SNAPSHOT))?;
    if save_hermes_skills_snapshot(entry, &dir.join("hermes-skills"))? {
        report.skills_saved = true;
    }

    Ok(report)
}

pub fn apply_workspace_snapshot(
    entry: &WorkspaceEntry,
    data_root: &Path,
) -> Result<SnapshotReport> {
    let mut report = SnapshotReport::default();
    let dir = snapshot_dir(data_root);

    let mcp_snapshot = dir.join("mcp.json");
    if mcp_snapshot.exists() {
        report.mcp_applied = apply_mcp_snapshot(&entry.path, &mcp_snapshot)?;
    }

    let skills_snapshot = dir.join(SKILLS_SNAPSHOT);
    if skills_snapshot.is_dir() {
        report.skills_applied = apply_skills_snapshot(&entry.path, &skills_snapshot)?;
    }

    Ok(report)
}

fn save_mcp_snapshot(project_path: &Path, target: &Path) -> Result<bool> {
    let project_mcp = project_path.join(".mcp.json");
    if project_mcp.exists() {
        fs::copy(&project_mcp, target)
            .with_context(|| format!("copy {} to {}", project_mcp.display(), target.display()))?;
        return Ok(true);
    }

    if !target.exists() {
        fs::write(target, "{\n  \"mcpServers\": {}\n}\n")
            .with_context(|| format!("write {}", target.display()))?;
    }
    Ok(false)
}

fn apply_mcp_snapshot(project_path: &Path, snapshot: &Path) -> Result<bool> {
    let project_mcp = project_path.join(".mcp.json");
    if project_mcp.exists() {
        return Ok(false);
    }
    fs::copy(snapshot, &project_mcp).with_context(|| {
        format!(
            "restore {} from {}",
            project_mcp.display(),
            snapshot.display()
        )
    })?;
    Ok(true)
}

fn save_skills_snapshot(project_path: &Path, target_dir: &Path) -> Result<bool> {
    let project_skills = project_path.join(".claude/skills");
    if !project_skills.is_dir() {
        return Ok(false);
    }

    if target_dir.exists() {
        fs::remove_dir_all(target_dir).ok();
    }
    copy_dir_recursive(&project_skills, target_dir)?;
    Ok(true)
}

fn apply_skills_snapshot(project_path: &Path, snapshot_dir: &Path) -> Result<bool> {
    let project_skills = project_path.join(".claude/skills");
    if project_skills.exists() {
        return Ok(false);
    }
    fs::create_dir_all(project_skills.parent().unwrap())?;
    copy_dir_recursive(snapshot_dir, &project_skills)?;
    Ok(true)
}

fn save_hermes_skills_snapshot(entry: &WorkspaceEntry, target_dir: &Path) -> Result<bool> {
    let profile_skills = home_join(".hermes/profiles")
        .join(&entry.hermes_profile)
        .join("skills");
    if !profile_skills.is_dir() {
        return Ok(false);
    }
    if target_dir.exists() {
        fs::remove_dir_all(target_dir).ok();
    }
    copy_dir_recursive(&profile_skills, target_dir)?;
    Ok(true)
}

fn copy_dir_recursive(from: &Path, to: &Path) -> Result<()> {
    fs::create_dir_all(to).with_context(|| format!("create {}", to.display()))?;
    for entry in fs::read_dir(from).with_context(|| format!("read {}", from.display()))? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let dest = to.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_recursive(&entry.path(), &dest)?;
        } else {
            fs::copy(entry.path(), &dest).with_context(|| {
                format!("copy {} to {}", entry.path().display(), dest.display())
            })?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn mcp_snapshot_roundtrip() {
        let temp = env::temp_dir().join(format!("ad-snapshot-{}", std::process::id()));
        let _ = fs::remove_dir_all(&temp);
        fs::create_dir_all(&temp).unwrap();

        let project = temp.join("project");
        fs::create_dir_all(&project).unwrap();
        fs::write(
            project.join(".mcp.json"),
            r#"{"mcpServers":{"demo":{"command":"echo"}}}"#,
        )
        .unwrap();

        let data_root = temp.join("data");
        let entry = WorkspaceEntry {
            title: String::new(),
            path: project.clone(),
            hermes_profile: "demo".into(),
            codex_home: data_root.join("codex"),
            openclaw_agent_id: "demo".into(),
            openclaw_workspace: data_root.join("openclaw"),
        };

        let save = save_workspace_snapshot(&entry, &data_root).unwrap();
        assert!(save.mcp_saved);

        let other = temp.join("other-project");
        fs::create_dir_all(&other).unwrap();
        let mut entry2 = entry.clone();
        entry2.path = other.clone();

        let apply = apply_workspace_snapshot(&entry2, &data_root).unwrap();
        assert!(apply.mcp_applied);
        assert!(other.join(".mcp.json").exists());

        let _ = fs::remove_dir_all(&temp);
    }
}
