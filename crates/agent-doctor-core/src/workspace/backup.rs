use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};

use crate::adapters::util::home_join;
use crate::repair_types::{backups_root, BackupSnapshot, SensitivityLevel, SnapshotFile};

use super::WorkspaceEntry;

pub fn create_workspace_switch_backup(
    name: &str,
    entry: &WorkspaceEntry,
) -> Result<BackupSnapshot> {
    let snapshot_id = format!("workspace-{name}-{}", unix_seconds());
    let snapshot_root = backups_root()?.join(&snapshot_id);
    fs::create_dir_all(&snapshot_root)?;

    let mut paths = Vec::new();
    let hermes_config = home_join(".hermes/profiles")
        .join(&entry.hermes_profile)
        .join("config.yaml");
    if hermes_config.exists() {
        paths.push(hermes_config);
    }
    let openclaw_config = home_join(".openclaw/openclaw.json");
    if openclaw_config.exists() {
        paths.push(openclaw_config);
    }
    let claude_project_settings = entry.path.join(".claude/settings.json");
    if claude_project_settings.exists() {
        paths.push(claude_project_settings);
    }

    let mut files = snapshot_paths(&paths, &snapshot_root)?;
    files.push(snapshot_metadata(name, entry, &snapshot_root)?);

    Ok(BackupSnapshot {
        id: snapshot_id,
        runtime_id: "workspace".to_string(),
        root: snapshot_root.display().to_string(),
        files,
    })
}

fn snapshot_metadata(
    name: &str,
    entry: &WorkspaceEntry,
    snapshot_root: &Path,
) -> Result<SnapshotFile> {
    let dest = snapshot_root.join("workspace-entry.yaml");
    let raw = format!(
        "name: {name}\npath: {}\nhermes_profile: {}\ncodex_home: {}\nopenclaw_agent_id: {}\nopenclaw_workspace: {}\n",
        entry.path.display(),
        entry.hermes_profile,
        entry.codex_home.display(),
        entry.openclaw_agent_id,
        entry.openclaw_workspace.display(),
    );
    fs::write(&dest, raw).context("write workspace backup metadata")?;
    Ok(SnapshotFile {
        original_path: "workspace-entry".to_string(),
        snapshot_path: dest.display().to_string(),
        sensitivity: SensitivityLevel::ConfigShape,
    })
}

fn snapshot_paths(paths: &[PathBuf], snapshot_root: &Path) -> Result<Vec<SnapshotFile>> {
    let mut files = Vec::new();
    for path in paths {
        if !path.exists() {
            continue;
        }
        let file_name = path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| "config".to_string());
        let dest = snapshot_root.join(&file_name);
        if dest.exists() {
            let stem = path
                .parent()
                .and_then(|parent| parent.file_name())
                .map(|name| format!("{}-", name.to_string_lossy()))
                .unwrap_or_default();
            let dest = snapshot_root.join(format!("{stem}{file_name}"));
            fs::copy(path, &dest).with_context(|| {
                format!("failed to copy {} to {}", path.display(), dest.display())
            })?;
            files.push(SnapshotFile {
                original_path: path.display().to_string(),
                snapshot_path: dest.display().to_string(),
                sensitivity: SensitivityLevel::LocalPath,
            });
            continue;
        }
        fs::copy(path, &dest)
            .with_context(|| format!("failed to copy {} to {}", path.display(), dest.display()))?;
        files.push(SnapshotFile {
            original_path: path.display().to_string(),
            snapshot_path: dest.display().to_string(),
            sensitivity: SensitivityLevel::LocalPath,
        });
    }
    Ok(files)
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}
