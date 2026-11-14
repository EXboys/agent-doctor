//! Versioned backup manifests and rollback target resolution.
//!
//! The snapshot directory stores copied config files plus `manifest.json`.
//! Restore destinations always come from the live adapter allowlist, never from
//! a raw path recorded in the manifest.

use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use agent_doctor_core::adapters::util::home_join;
use agent_doctor_core::runtime::adapter_by_id;
use agent_doctor_core::workspace::path::paths_equal;
use agent_doctor_core::workspace::{load_workspaces, WorkspaceEntry};

pub const MANIFEST_VERSION: u32 = 1;
pub const MANIFEST_FILE_NAME: &str = "manifest.json";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ConfigScope {
    #[default]
    Unscoped,
    Global,
    Isolated,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ConfigIdentity {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codex_home: Option<String>,
    #[serde(default)]
    pub scope: ConfigScope,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManifestFile {
    pub file_name: String,
    pub original_path: String,
    pub snapshot_name: String,
    #[serde(default)]
    pub identity: ConfigIdentity,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BackupManifest {
    pub version: u32,
    pub runtime_id: String,
    pub id: String,
    pub files: Vec<ManifestFile>,
}

#[derive(Debug, Clone)]
pub struct PlannedRestore {
    pub file_name: String,
    pub snapshot_path: PathBuf,
    pub dest: PathBuf,
    pub notice: Option<String>,
}

pub fn validate_backup_id(runtime_id: &str, backup_id: &str) -> Result<()> {
    if runtime_id.is_empty()
        || runtime_id.contains('/')
        || runtime_id.contains('\\')
        || runtime_id.contains('\0')
        || runtime_id.contains("..")
    {
        bail!("invalid runtime id");
    }
    if backup_id.is_empty()
        || backup_id.contains('/')
        || backup_id.contains('\\')
        || backup_id.contains('\0')
        || backup_id.contains(':')
        || backup_id.contains("..")
        || Path::new(backup_id).is_absolute()
    {
        bail!("invalid backup id '{backup_id}'");
    }
    // A valid id is a single path component: no parent traversal, no separators.
    if Path::new(backup_id)
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        bail!("invalid backup id '{backup_id}'");
    }
    let prefix = format!("{runtime_id}-");
    if !backup_id.starts_with(&prefix) {
        bail!("backup '{backup_id}' does not belong to runtime '{runtime_id}'");
    }
    let rest = &backup_id[prefix.len()..];
    if rest.is_empty()
        || !rest
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '.' || ch == '_' || ch == '-')
    {
        bail!("invalid backup id '{backup_id}'");
    }
    Ok(())
}

pub fn validate_snapshot_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name == MANIFEST_FILE_NAME
        || name.contains('/')
        || name.contains('\\')
        || name.contains('\0')
        || name.contains("..")
        || Path::new(name).is_absolute()
        || Path::new(name)
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        bail!("invalid snapshot file name '{name}'");
    }
    Ok(())
}

pub fn unique_existing_file_names(paths: &[PathBuf]) -> Result<()> {
    let mut seen: Vec<(String, PathBuf)> = Vec::new();
    for path in paths.iter().filter(|path| path.exists()) {
        let name = file_name_of(path);
        if name == MANIFEST_FILE_NAME {
            bail!(
                "refusing to back up {} because '{MANIFEST_FILE_NAME}' is reserved for the snapshot manifest",
                path.display()
            );
        }
        if let Some((_, first)) = seen.iter().find(|(existing, _)| existing == &name) {
            bail!(
                "cannot snapshot two config files with the same name '{name}': {} and {}",
                first.display(),
                path.display()
            );
        }
        seen.push((name, path.clone()));
    }
    Ok(())
}

pub fn file_name_of(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "config".to_string())
}

pub fn identity_for_path(path: &Path) -> ConfigIdentity {
    let global_codex_home = home_join(".codex");
    let Ok(doc) = load_workspaces() else {
        return identity_if_global(path, &global_codex_home, None);
    };

    for (name, entry) in &doc.workspaces {
        if is_isolated_codex_home(&entry.codex_home, &global_codex_home)
            && path_is_within(&entry.codex_home, path)
        {
            return ConfigIdentity {
                workspace_name: Some(name.clone()),
                codex_home: Some(entry.codex_home.display().to_string()),
                scope: ConfigScope::Isolated,
            };
        }
    }

    identity_if_global(path, &global_codex_home, doc.active.clone())
}

fn identity_if_global(
    path: &Path,
    global_codex_home: &Path,
    active: Option<String>,
) -> ConfigIdentity {
    if path_is_within(global_codex_home, path) {
        return ConfigIdentity {
            workspace_name: active,
            codex_home: Some(global_codex_home.display().to_string()),
            scope: ConfigScope::Global,
        };
    }
    ConfigIdentity {
        workspace_name: None,
        codex_home: None,
        scope: ConfigScope::Unscoped,
    }
}

pub fn write_manifest(snapshot_root: &Path, manifest: &BackupManifest) -> Result<()> {
    let path = snapshot_root.join(MANIFEST_FILE_NAME);
    let raw = serde_json::to_string_pretty(manifest)? + "\n";
    fs::write(&path, raw).with_context(|| format!("failed to write {}", path.display()))
}

pub fn read_manifest(snapshot_root: &Path) -> Result<Option<BackupManifest>> {
    let path = snapshot_root.join(MANIFEST_FILE_NAME);
    if !path.exists() {
        return Ok(None);
    }
    let raw =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    let manifest: BackupManifest = serde_json::from_str(&raw)
        .with_context(|| format!("invalid backup manifest at {}", path.display()))?;
    if manifest.version != MANIFEST_VERSION {
        bail!(
            "unsupported backup manifest version {} at {}",
            manifest.version,
            path.display()
        );
    }
    Ok(Some(manifest))
}

pub fn plan_restore_from_manifest(
    runtime_id: &str,
    backup_id: &str,
    snapshot_root: &Path,
    manifest: &BackupManifest,
) -> Result<(Vec<PlannedRestore>, Vec<String>)> {
    if manifest.runtime_id != runtime_id {
        bail!(
            "backup '{backup_id}' belongs to runtime '{}', not '{runtime_id}'",
            manifest.runtime_id
        );
    }
    if manifest.id != backup_id {
        bail!(
            "backup folder '{backup_id}' does not match manifest id '{}'",
            manifest.id
        );
    }
    if manifest.files.is_empty() {
        bail!(
            "backup at {} contains no restorable files",
            snapshot_root.display()
        );
    }

    let legal = legal_config_paths(runtime_id)?;
    let mut planned = Vec::new();
    let mut notices = Vec::new();
    for file in &manifest.files {
        validate_snapshot_name(&file.snapshot_name)?;
        if file.file_name != file.snapshot_name
            && file.file_name != file_name_of(Path::new(&file.snapshot_name))
        {
            // file_name is the logical config basename used for allowlist matching.
            validate_snapshot_name(&file.file_name)?;
        }
        let snapshot_path = snapshot_file(snapshot_root, &file.snapshot_name)?;
        if !snapshot_path.is_file() {
            bail!(
                "backup file '{}' is missing from {}",
                file.snapshot_name,
                snapshot_root.display()
            );
        }
        let current = unique_legal_path(&legal, &file.file_name)?;
        let dest = resolve_destination(
            Path::new(&file.original_path),
            &current,
            &file.identity,
            &identity_for_path(&current),
        )?;
        if !legal.iter().any(|path| path_keys_equal(path, &dest)) {
            bail!(
                "refusing to restore '{}' to {} because it is not a current config path for '{runtime_id}'",
                file.file_name,
                dest.display()
            );
        }
        if let Some(notice) = retarget_notice(Path::new(&file.original_path), &dest) {
            notices.push(notice);
        }
        planned.push(PlannedRestore {
            file_name: file.file_name.clone(),
            snapshot_path,
            dest,
            notice: notices.last().cloned(),
        });
    }
    Ok((planned, notices))
}

pub fn plan_restore_legacy(
    runtime_id: &str,
    snapshot_root: &Path,
) -> Result<(Vec<PlannedRestore>, Vec<String>)> {
    let legal = legal_config_paths(runtime_id)?;
    let mut planned = Vec::new();
    for entry in fs::read_dir(snapshot_root)
        .with_context(|| format!("failed to read {}", snapshot_root.display()))?
    {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == MANIFEST_FILE_NAME {
            continue;
        }
        validate_snapshot_name(&name)?;
        let current = unique_legal_path(&legal, &name)?;
        if isolated_workspace_for(&current).is_some() {
            bail!(
                "refusing to restore legacy backup at {} to {} because the snapshot has no manifest and the current path belongs to an isolated workspace. Re-run a current backup, or restore only when the current config path is the global fallback.",
                snapshot_root.display(),
                current.display()
            );
        }
        planned.push(PlannedRestore {
            file_name: name.clone(),
            snapshot_path: entry.path(),
            dest: current.clone(),
            notice: Some(format!(
                "legacy backup at {} has no manifest; restoring '{name}' to {} after confirming a unique global config path",
                snapshot_root.display(),
                current.display()
            )),
        });
    }
    if planned.is_empty() {
        bail!(
            "backup at {} contains no restorable files",
            snapshot_root.display()
        );
    }
    let notices = planned
        .iter()
        .filter_map(|item| item.notice.clone())
        .collect();
    Ok((planned, notices))
}

fn legal_config_paths(runtime_id: &str) -> Result<Vec<PathBuf>> {
    let adapter =
        adapter_by_id(runtime_id).with_context(|| format!("unknown runtime '{runtime_id}'"))?;
    Ok(adapter.all_config_paths())
}

fn unique_legal_path(legal: &[PathBuf], file_name: &str) -> Result<PathBuf> {
    let matches: Vec<&PathBuf> = legal
        .iter()
        .filter(|path| file_name_of(path) == file_name)
        .collect();
    match matches.as_slice() {
        [only] => Ok((*only).clone()),
        [] => bail!("no current config path matches backup file '{file_name}'"),
        _ => bail!(
            "backup file '{file_name}' matches more than one current config path; refusing to guess"
        ),
    }
}

fn resolve_destination(
    original: &Path,
    current: &Path,
    backup_identity: &ConfigIdentity,
    current_identity: &ConfigIdentity,
) -> Result<PathBuf> {
    if path_keys_equal(original, current) {
        return Ok(current.to_path_buf());
    }
    if same_logical_config(backup_identity, current_identity) {
        return Ok(current.to_path_buf());
    }
    bail!(
        "refusing to restore backup of {} onto {} because they do not belong to the same logical configuration (backup workspace: {}, current workspace: {})",
        original.display(),
        current.display(),
        display_workspace(backup_identity),
        display_workspace(current_identity)
    );
}

fn same_logical_config(backup: &ConfigIdentity, current: &ConfigIdentity) -> bool {
    let Some(backup_ws) = backup
        .workspace_name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
    else {
        return false;
    };
    let Some(current_ws) = current
        .workspace_name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
    else {
        return false;
    };
    if backup_ws != current_ws {
        return false;
    }
    use ConfigScope::*;
    matches!(
        (backup.scope, current.scope),
        (Global, Isolated) | (Isolated, Global) | (Isolated, Isolated) | (Global, Global)
    )
}

fn display_workspace(identity: &ConfigIdentity) -> String {
    identity
        .workspace_name
        .clone()
        .unwrap_or_else(|| match identity.scope {
            ConfigScope::Global => "global".to_string(),
            ConfigScope::Isolated => "isolated".to_string(),
            ConfigScope::Unscoped => "unknown".to_string(),
        })
}

fn retarget_notice(original: &Path, dest: &Path) -> Option<String> {
    if path_keys_equal(original, dest) {
        return None;
    }
    Some(format!(
        "restored {} to the current config path {} (same workspace; path changed since the backup)",
        original.display(),
        dest.display()
    ))
}

fn snapshot_file(snapshot_root: &Path, name: &str) -> Result<PathBuf> {
    validate_snapshot_name(name)?;
    let root = lexical_path(snapshot_root);
    let joined = snapshot_root.join(name);
    let lexical = lexical_path(&joined);
    if !lexical.starts_with(&root) {
        bail!("snapshot file '{name}' escapes the backup directory");
    }
    Ok(joined)
}

fn isolated_workspace_for(path: &Path) -> Option<(String, WorkspaceEntry)> {
    let global_codex_home = home_join(".codex");
    let doc = load_workspaces().ok()?;
    for (name, entry) in doc.workspaces {
        if is_isolated_codex_home(&entry.codex_home, &global_codex_home)
            && path_is_within(&entry.codex_home, path)
        {
            return Some((name, entry));
        }
    }
    None
}

fn is_isolated_codex_home(codex_home: &Path, global_codex_home: &Path) -> bool {
    !codex_home.as_os_str().is_empty() && !path_keys_equal(codex_home, global_codex_home)
}

fn path_is_within(dir: &Path, path: &Path) -> bool {
    if path_keys_equal(dir, path) {
        return true;
    }
    let dir = lexical_path(dir);
    let path = lexical_path(path);
    path.starts_with(&dir)
}

pub fn path_keys_equal(left: &Path, right: &Path) -> bool {
    if paths_equal(left, right) {
        return true;
    }
    lexical_path(left) == lexical_path(right)
}

fn lexical_path(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_backup_id_rejects_path_tricks_and_wrong_runtime() {
        assert!(validate_backup_id("codex", "codex-1700000000").is_ok());
        assert!(validate_backup_id("deepseek-harness", "deepseek-harness-1").is_ok());
        assert!(validate_backup_id("codex", "codex-../x").is_err());
        assert!(validate_backup_id("codex", "codex-foo/bar").is_err());
        assert!(validate_backup_id("codex", "codex-foo\\bar").is_err());
        assert!(validate_backup_id("codex", "/tmp/codex-1").is_err());
        assert!(validate_backup_id("codex", "hermes-1700000000").is_err());
        assert!(validate_backup_id("codex", "codex-").is_err());
        assert!(validate_backup_id("codex", "..").is_err());
    }

    #[test]
    fn same_logical_config_requires_workspace_evidence() {
        let a_global = ConfigIdentity {
            workspace_name: Some("alpha".into()),
            codex_home: Some("/tmp/global".into()),
            scope: ConfigScope::Global,
        };
        let a_isolated = ConfigIdentity {
            workspace_name: Some("alpha".into()),
            codex_home: Some("/tmp/alpha".into()),
            scope: ConfigScope::Isolated,
        };
        let b_isolated = ConfigIdentity {
            workspace_name: Some("beta".into()),
            codex_home: Some("/tmp/beta".into()),
            scope: ConfigScope::Isolated,
        };
        let global_only = ConfigIdentity {
            workspace_name: None,
            codex_home: Some("/tmp/global".into()),
            scope: ConfigScope::Global,
        };
        assert!(same_logical_config(&a_global, &a_isolated));
        assert!(same_logical_config(&a_isolated, &a_global));
        assert!(!same_logical_config(&a_isolated, &b_isolated));
        assert!(!same_logical_config(&global_only, &a_isolated));
        assert!(!same_logical_config(&global_only, &b_isolated));
    }
}
