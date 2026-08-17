use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::repair::{backups_root, BackupSnapshot, SensitivityLevel, SnapshotFile};
use crate::runtime::adapter_by_id;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestoreReport {
    pub runtime_id: String,
    pub backup_id: String,
    pub backup_root: String,
    pub restored_files: Vec<String>,
}

pub fn list_runtime_backup_ids(runtime_id: &str) -> Result<Vec<String>> {
    let root = backups_root()?;
    let prefix = format!("{runtime_id}-");
    let mut ids = Vec::new();

    for entry in
        fs::read_dir(&root).with_context(|| format!("failed to read {}", root.display()))?
    {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if name.starts_with(&prefix) {
            ids.push(name.to_string());
        }
    }

    ids.sort();
    ids.reverse();
    Ok(ids)
}

pub fn load_backup_snapshot(runtime_id: &str, backup_id: &str) -> Result<BackupSnapshot> {
    let root = backups_root()?.join(backup_id);
    if !root.is_dir() {
        anyhow::bail!("backup not found at {}", root.display());
    }
    if !backup_id.starts_with(&format!("{runtime_id}-")) {
        anyhow::bail!("backup '{backup_id}' does not belong to runtime '{runtime_id}'");
    }

    let adapter =
        adapter_by_id(runtime_id).with_context(|| format!("unknown runtime '{runtime_id}'"))?;
    let config_paths = adapter.all_config_paths();
    let mut files = Vec::new();

    for entry in fs::read_dir(&root)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let snapshot_path = entry.path();
        let file_name = snapshot_path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .context("backup file has no name")?;

        let original = config_paths
            .iter()
            .find(|path| {
                path.file_name()
                    .map(|name| name.to_string_lossy() == file_name)
                    .unwrap_or(false)
            })
            .with_context(|| format!("no config path matches backup file '{file_name}'"))?;

        files.push(SnapshotFile {
            original_path: original.display().to_string(),
            snapshot_path: snapshot_path.display().to_string(),
            sensitivity: SensitivityLevel::LocalPath,
        });
    }

    if files.is_empty() {
        anyhow::bail!("backup at {} contains no restorable files", root.display());
    }

    Ok(BackupSnapshot {
        id: backup_id.to_string(),
        runtime_id: runtime_id.to_string(),
        root: root.display().to_string(),
        files,
    })
}

pub fn restore_backup_snapshot(backup: &BackupSnapshot) -> Result<RestoreReport> {
    let mut restored_files = Vec::new();

    for file in &backup.files {
        let source = Path::new(&file.snapshot_path);
        let dest = Path::new(&file.original_path);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(source, dest).with_context(|| {
            format!(
                "failed to restore {} from {}",
                dest.display(),
                source.display()
            )
        })?;
        restored_files.push(dest.display().to_string());
    }

    Ok(RestoreReport {
        runtime_id: backup.runtime_id.clone(),
        backup_id: backup.id.clone(),
        backup_root: backup.root.clone(),
        restored_files,
    })
}

pub fn restore_runtime_backup(runtime_id: &str, backup_id: Option<&str>) -> Result<RestoreReport> {
    let backup = match backup_id {
        Some(id) => load_backup_snapshot(runtime_id, id)?,
        None => {
            let latest = list_runtime_backup_ids(runtime_id)?
                .into_iter()
                .next()
                .with_context(|| format!("no backups found for runtime '{runtime_id}'"))?;
            load_backup_snapshot(runtime_id, &latest)?
        }
    };
    restore_backup_snapshot(&backup)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn restore_backup_snapshot_copies_files_back() {
        let temp = tempfile::tempdir().expect("tempdir");
        let original = temp.path().join("config.yaml");
        let mut file = fs::File::create(&original).expect("create");
        writeln!(file, "model: old").expect("write");

        let snapshot_root = temp.path().join("hermes-test-1");
        fs::create_dir_all(&snapshot_root).expect("mkdir");
        let snapshot_path = snapshot_root.join("config.yaml");
        fs::copy(&original, &snapshot_path).expect("copy");

        writeln!(file, "model: changed").expect("write");

        let backup = BackupSnapshot {
            id: "hermes-test-1".to_string(),
            runtime_id: "hermes".to_string(),
            root: snapshot_root.display().to_string(),
            files: vec![SnapshotFile {
                original_path: original.display().to_string(),
                snapshot_path: snapshot_path.display().to_string(),
                sensitivity: SensitivityLevel::LocalPath,
            }],
        };

        let report = restore_backup_snapshot(&backup).expect("restore");
        assert_eq!(report.restored_files.len(), 1);
        let restored = fs::read_to_string(&original).expect("read");
        assert!(restored.contains("model: old"));
        assert!(!restored.contains("changed"));
    }
}
