use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::backup_manifest::{
    file_name_of, plan_restore_from_manifest, plan_restore_legacy, read_manifest,
    validate_backup_id, validate_snapshot_name, PlannedRestore, MANIFEST_FILE_NAME,
};
use crate::repair::{backups_root, BackupSnapshot, SensitivityLevel, SnapshotFile};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestoreReport {
    pub runtime_id: String,
    pub backup_id: String,
    pub backup_root: String,
    pub restored_files: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notices: Vec<String>,
}

pub fn list_runtime_backup_ids(runtime_id: &str) -> Result<Vec<String>> {
    let root = backups_root()?;
    if !root.exists() {
        return Ok(Vec::new());
    }
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
        if validate_backup_id(runtime_id, &name).is_err() {
            continue;
        }
        if !snapshot_has_restorable_files(&entry.path()) {
            continue;
        }
        ids.push(name);
    }

    ids.sort();
    ids.reverse();
    Ok(ids)
}

fn snapshot_has_restorable_files(root: &Path) -> bool {
    let Ok(entries) = fs::read_dir(root) else {
        return false;
    };
    entries.flatten().any(|entry| {
        entry
            .file_type()
            .map(|kind| kind.is_file())
            .unwrap_or(false)
            && entry.file_name().to_str() != Some(MANIFEST_FILE_NAME)
    })
}

pub fn load_backup_snapshot(runtime_id: &str, backup_id: &str) -> Result<BackupSnapshot> {
    let (snapshot, planned, _) = load_planned_restore(runtime_id, backup_id)?;
    let files = planned
        .into_iter()
        .map(|item| SnapshotFile {
            original_path: item.dest.display().to_string(),
            snapshot_path: item.snapshot_path.display().to_string(),
            sensitivity: SensitivityLevel::LocalPath,
        })
        .collect();
    Ok(BackupSnapshot {
        id: snapshot.id,
        runtime_id: snapshot.runtime_id,
        root: snapshot.root,
        files,
    })
}

fn load_planned_restore(
    runtime_id: &str,
    backup_id: &str,
) -> Result<(BackupSnapshot, Vec<PlannedRestore>, Vec<String>)> {
    validate_backup_id(runtime_id, backup_id)?;
    let root = backups_root()?.join(backup_id);
    if !root.is_dir() {
        anyhow::bail!("backup not found at {}", root.display());
    }

    let (planned, notices) = match read_manifest(&root)? {
        Some(manifest) => plan_restore_from_manifest(runtime_id, backup_id, &root, &manifest)?,
        None => plan_restore_legacy(runtime_id, &root)?,
    };

    let files = planned
        .iter()
        .map(|item| SnapshotFile {
            original_path: item.dest.display().to_string(),
            snapshot_path: item.snapshot_path.display().to_string(),
            sensitivity: SensitivityLevel::LocalPath,
        })
        .collect();

    let snapshot = BackupSnapshot {
        id: backup_id.to_string(),
        runtime_id: runtime_id.to_string(),
        root: root.display().to_string(),
        files,
    };
    Ok((snapshot, planned, notices))
}

pub fn restore_backup_snapshot(backup: &BackupSnapshot) -> Result<RestoreReport> {
    // Callers that build a snapshot by hand (tests, or an already-planned
    // in-memory snapshot) still go through the copy path. Production rollback
    // uses [`restore_runtime_backup`], which validates destinations first.
    restore_planned(
        &backup.runtime_id,
        &backup.id,
        &backup.root,
        backup
            .files
            .iter()
            .map(|file| PlannedRestore {
                file_name: file_name_of(Path::new(&file.snapshot_path)),
                snapshot_path: PathBuf::from(&file.snapshot_path),
                dest: PathBuf::from(&file.original_path),
                notice: None,
            })
            .collect(),
        Vec::new(),
    )
}

fn restore_planned(
    runtime_id: &str,
    backup_id: &str,
    backup_root: &str,
    planned: Vec<PlannedRestore>,
    notices: Vec<String>,
) -> Result<RestoreReport> {
    for item in &planned {
        if validate_snapshot_name(&item.file_name).is_err() {
            validate_snapshot_name(&file_name_of(&item.snapshot_path))?;
        }
        if !item.snapshot_path.is_file() {
            anyhow::bail!("backup file is missing at {}", item.snapshot_path.display());
        }
    }

    let mut restored_files = Vec::new();
    for item in planned {
        if let Some(parent) = item.dest.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(&item.snapshot_path, &item.dest).with_context(|| {
            format!(
                "failed to restore {} from {}",
                item.dest.display(),
                item.snapshot_path.display()
            )
        })?;
        restored_files.push(item.dest.display().to_string());
    }

    Ok(RestoreReport {
        runtime_id: runtime_id.to_string(),
        backup_id: backup_id.to_string(),
        backup_root: backup_root.to_string(),
        restored_files,
        notices,
    })
}

pub fn restore_runtime_backup(runtime_id: &str, backup_id: Option<&str>) -> Result<RestoreReport> {
    let backup_id = match backup_id {
        Some(id) => {
            validate_backup_id(runtime_id, id)?;
            id.to_string()
        }
        None => list_runtime_backup_ids(runtime_id)?
            .into_iter()
            .next()
            .with_context(|| format!("no backups found for runtime '{runtime_id}'"))?,
    };
    let (snapshot, planned, notices) = load_planned_restore(runtime_id, &backup_id)?;
    restore_planned(
        &snapshot.runtime_id,
        &snapshot.id,
        &snapshot.root,
        planned,
        notices,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repair::backup_manifest::{
        write_manifest, BackupManifest, ConfigIdentity, ManifestFile, MANIFEST_FILE_NAME,
        MANIFEST_VERSION,
    };
    use crate::repair::execute::{create_runtime_backup_snapshot, snapshot_config_files};
    use agent_doctor_core::adapters::util::with_test_home;
    use agent_doctor_core::workspace::{save_workspaces, WorkspaceEntry, WorkspacesDocument};
    use std::collections::BTreeMap;
    use std::io::Write;

    fn workspace_entry(home: &Path, name: &str) -> WorkspaceEntry {
        let project = home.join("projects").join(name);
        fs::create_dir_all(&project).expect("project");
        let codex_home = home.join("workspaces").join(name).join("codex-home");
        fs::create_dir_all(&codex_home).expect("codex home");
        WorkspaceEntry {
            path: project,
            hermes_profile: "default".to_string(),
            codex_home,
            openclaw_agent_id: "main".to_string(),
            openclaw_workspace: home.join("workspaces").join(name).join("openclaw"),
            title: String::new(),
        }
    }

    fn save_doc(home: &Path, active: &str, entries: &[(&str, WorkspaceEntry)]) {
        let _ = home;
        let mut workspaces = BTreeMap::new();
        for (name, entry) in entries {
            workspaces.insert((*name).to_string(), entry.clone());
        }
        let doc = WorkspacesDocument {
            active: Some(active.to_string()),
            workspaces,
        };
        save_workspaces(&doc).expect("save workspaces");
    }

    fn with_isolated_home<T>(f: impl FnOnce(&Path) -> T) -> T {
        let temp = tempfile::tempdir().expect("tempdir");
        with_test_home(temp.path(), || f(temp.path()))
    }

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
        assert!(report.notices.is_empty());
    }

    #[test]
    fn restore_same_path_happy_path_through_manifest() {
        with_isolated_home(|home| {
            let config = home.join(".hermes/config.yaml");
            fs::create_dir_all(config.parent().unwrap()).unwrap();
            fs::write(&config, "model: before\n").unwrap();

            let snapshot = create_runtime_backup_snapshot("hermes").expect("backup");
            fs::write(&config, "model: dirty\n").unwrap();

            let report = restore_runtime_backup("hermes", Some(&snapshot.id)).expect("restore");
            assert_eq!(report.restored_files.len(), 1);
            assert!(report.notices.is_empty());
            assert_eq!(fs::read_to_string(&config).unwrap(), "model: before\n");
        });
    }

    #[test]
    fn restore_refuses_when_active_workspace_changes() {
        with_isolated_home(|home| {
            let entry_a = workspace_entry(home, "alpha");
            let entry_b = workspace_entry(home, "beta");
            fs::write(entry_a.codex_home.join("config.toml"), "from = \"a\"\n").unwrap();
            fs::write(entry_b.codex_home.join("config.toml"), "from = \"b\"\n").unwrap();
            fs::create_dir_all(home.join(".codex")).unwrap();
            save_doc(
                home,
                "alpha",
                &[("alpha", entry_a.clone()), ("beta", entry_b.clone())],
            );

            let snapshot = create_runtime_backup_snapshot("codex").expect("backup");
            save_doc(
                home,
                "beta",
                &[("alpha", entry_a.clone()), ("beta", entry_b.clone())],
            );

            let before_b = fs::read_to_string(entry_b.codex_home.join("config.toml")).unwrap();
            let before_a = fs::read_to_string(entry_a.codex_home.join("config.toml")).unwrap();
            let error = restore_runtime_backup("codex", Some(&snapshot.id))
                .expect_err("must refuse cross-workspace restore");
            let message = format!("{error:#}");
            assert!(
                message.contains("alpha") && message.contains("beta"),
                "error should name both workspaces: {message}"
            );
            assert_eq!(
                fs::read_to_string(entry_b.codex_home.join("config.toml")).unwrap(),
                before_b
            );
            assert_eq!(
                fs::read_to_string(entry_a.codex_home.join("config.toml")).unwrap(),
                before_a
            );
        });
    }

    #[test]
    fn restore_retargets_global_to_isolated_same_workspace() {
        with_isolated_home(|home| {
            // Isolated home is recorded but does not exist yet, so backup captures
            // the global config and tags it with the active workspace identity.
            let isolated_home = home.join("workspaces/alpha/codex-home");
            let entry = WorkspaceEntry {
                path: home.join("projects/alpha"),
                hermes_profile: "default".to_string(),
                codex_home: isolated_home.clone(),
                openclaw_agent_id: "main".to_string(),
                openclaw_workspace: home.join("workspaces/alpha/openclaw"),
                title: String::new(),
            };
            fs::create_dir_all(&entry.path).unwrap();
            let global = home.join(".codex/config.toml");
            fs::create_dir_all(global.parent().unwrap()).unwrap();
            fs::write(&global, "from = \"global\"\n").unwrap();
            save_doc(home, "alpha", &[("alpha", entry)]);

            let snapshot = create_runtime_backup_snapshot("codex").expect("backup");
            assert!(
                snapshot
                    .files
                    .iter()
                    .any(|file| file.original_path == global.display().to_string()),
                "expected the global config to be snapshotted, got {:?}",
                snapshot.files
            );

            fs::create_dir_all(&isolated_home).unwrap();
            let isolated = isolated_home.join("config.toml");
            fs::write(&isolated, "from = \"placeholder\"\n").unwrap();
            let report = restore_runtime_backup("codex", Some(&snapshot.id)).expect("retarget");
            assert_eq!(
                fs::read_to_string(&isolated).unwrap(),
                "from = \"global\"\n"
            );
            assert_eq!(fs::read_to_string(&global).unwrap(), "from = \"global\"\n");
            assert!(
                report
                    .notices
                    .iter()
                    .any(|notice| notice.contains("current config path")),
                "expected retarget notice, got {:?}",
                report.notices
            );
            assert!(report
                .restored_files
                .iter()
                .any(|path| Path::new(path) == isolated));
        });
    }

    #[test]
    fn snapshot_rejects_duplicate_file_names() {
        let temp = tempfile::tempdir().expect("tempdir");
        let first = temp.path().join("one/.env");
        let second = temp.path().join("two/.env");
        fs::create_dir_all(first.parent().unwrap()).unwrap();
        fs::create_dir_all(second.parent().unwrap()).unwrap();
        fs::write(&first, "A=1\n").unwrap();
        fs::write(&second, "B=2\n").unwrap();
        let snapshot_root = temp.path().join("snap");
        fs::create_dir_all(&snapshot_root).unwrap();
        let error = snapshot_config_files("hermes", &[first, second], &snapshot_root)
            .expect_err("duplicate names");
        assert!(format!("{error:#}").contains("same name"));
        assert!(!snapshot_root.join(".env").exists());
    }

    #[test]
    fn illegal_backup_id_fails_before_writes() {
        with_isolated_home(|home| {
            let marker = home.join("should-not-change.txt");
            fs::write(&marker, "keep\n").unwrap();
            let before = fs::read_to_string(&marker).unwrap();
            for id in [
                "codex-../x",
                "codex-foo/bar",
                "hermes-1700000000",
                "/tmp/codex-1",
            ] {
                let error = restore_runtime_backup("codex", Some(id)).expect_err(id);
                let message = format!("{error:#}");
                assert!(
                    message.contains("invalid")
                        || message.contains("does not belong")
                        || message.contains("not found"),
                    "unexpected error for {id}: {message}"
                );
            }
            assert_eq!(fs::read_to_string(&marker).unwrap(), before);
        });
    }

    #[test]
    fn legacy_snapshot_restores_unique_global_path_with_warning() {
        with_isolated_home(|home| {
            let config = home.join(".hermes/config.yaml");
            fs::create_dir_all(config.parent().unwrap()).unwrap();
            fs::write(&config, "model: dirty\n").unwrap();

            let snapshot_id = "hermes-1700000002";
            let snapshot_root = backups_root().unwrap().join(snapshot_id);
            fs::create_dir_all(&snapshot_root).unwrap();
            fs::write(snapshot_root.join("config.yaml"), "model: legacy\n").unwrap();

            let report = restore_runtime_backup("hermes", Some(snapshot_id)).expect("legacy");
            assert_eq!(fs::read_to_string(&config).unwrap(), "model: legacy\n");
            assert!(
                report
                    .notices
                    .iter()
                    .any(|notice| notice.contains("legacy")),
                "expected legacy warning, got {:?}",
                report.notices
            );
        });
    }

    #[test]
    fn legacy_snapshot_refuses_isolated_workspace_target() {
        with_isolated_home(|home| {
            let entry = workspace_entry(home, "alpha");
            fs::write(entry.codex_home.join("config.toml"), "from = \"keep\"\n").unwrap();
            fs::create_dir_all(home.join(".codex")).unwrap();
            save_doc(home, "alpha", &[("alpha", entry.clone())]);

            let snapshot_id = "codex-1700000003";
            let snapshot_root = backups_root().unwrap().join(snapshot_id);
            fs::create_dir_all(&snapshot_root).unwrap();
            fs::write(snapshot_root.join("config.toml"), "from = \"legacy\"\n").unwrap();

            let error = restore_runtime_backup("codex", Some(snapshot_id))
                .expect_err("legacy must not enter isolated workspace");
            assert!(format!("{error:#}").contains("no manifest"));
            assert_eq!(
                fs::read_to_string(entry.codex_home.join("config.toml")).unwrap(),
                "from = \"keep\"\n"
            );
        });
    }

    #[test]
    fn tampered_manifest_target_cannot_escape_allowlist() {
        with_isolated_home(|home| {
            let config = home.join(".hermes/config.yaml");
            fs::create_dir_all(config.parent().unwrap()).unwrap();
            fs::write(&config, "model: keep\n").unwrap();
            let evil = home.join("evil.yaml");
            fs::write(&evil, "hacked: no\n").unwrap();

            let snapshot_id = "hermes-1700000004";
            let snapshot_root = backups_root().unwrap().join(snapshot_id);
            fs::create_dir_all(&snapshot_root).unwrap();
            fs::write(snapshot_root.join("config.yaml"), "model: snapshot\n").unwrap();
            write_manifest(
                &snapshot_root,
                &BackupManifest {
                    version: MANIFEST_VERSION,
                    runtime_id: "hermes".into(),
                    id: snapshot_id.into(),
                    files: vec![ManifestFile {
                        file_name: "config.yaml".into(),
                        original_path: evil.display().to_string(),
                        snapshot_name: "config.yaml".into(),
                        identity: ConfigIdentity::default(),
                    }],
                },
            )
            .unwrap();

            let error = restore_runtime_backup("hermes", Some(snapshot_id))
                .expect_err("tampered dest must fail");
            let message = format!("{error:#}");
            assert!(
                message.contains("refusing") || message.contains("not belong"),
                "unexpected error: {message}"
            );
            assert_eq!(fs::read_to_string(&evil).unwrap(), "hacked: no\n");
            assert_eq!(fs::read_to_string(&config).unwrap(), "model: keep\n");
        });
    }

    #[test]
    fn manifest_is_not_restored_as_a_config_file() {
        with_isolated_home(|home| {
            let config = home.join(".hermes/config.yaml");
            fs::create_dir_all(config.parent().unwrap()).unwrap();
            fs::write(&config, "model: before\n").unwrap();
            let snapshot = create_runtime_backup_snapshot("hermes").expect("backup");
            let manifest = PathBuf::from(&snapshot.root).join(MANIFEST_FILE_NAME);
            assert!(manifest.is_file());
            restore_runtime_backup("hermes", Some(&snapshot.id)).expect("restore");
            assert!(!home.join(".hermes/manifest.json").exists());
        });
    }

    #[test]
    fn failed_backup_does_not_leave_a_latest_partial_snapshot() {
        with_isolated_home(|home| {
            let good = home.join(".hermes/config.yaml");
            fs::create_dir_all(good.parent().unwrap()).unwrap();
            fs::write(&good, "model: good\n").unwrap();
            let good_snapshot = create_runtime_backup_snapshot("hermes").expect("good backup");

            // A directory at the config path makes fs::copy fail after the
            // snapshot folder has already been created.
            fs::remove_file(&good).unwrap();
            fs::create_dir_all(&good).unwrap();
            create_runtime_backup_snapshot("hermes").expect_err("copying a directory must fail");

            let ids = list_runtime_backup_ids("hermes").expect("list");
            assert_eq!(ids, vec![good_snapshot.id.clone()]);
            let leftover: Vec<_> = fs::read_dir(backups_root().unwrap())
                .unwrap()
                .flatten()
                .filter(|entry| {
                    entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false)
                        && entry.file_name() != *std::ffi::OsStr::new(&good_snapshot.id)
                })
                .collect();
            assert!(
                leftover.is_empty(),
                "failed backup left unexpected dirs: {:?}",
                leftover
                    .iter()
                    .map(|entry| entry.file_name())
                    .collect::<Vec<_>>()
            );
        });
    }
}
