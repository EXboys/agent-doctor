use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

const REMOTE_DIR: &str = "remote";
const HOSTS_FILE: &str = "hosts.yaml";
const KEYS_DIR: &str = "keys";

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct RemoteHostsDocument {
    #[serde(default)]
    pub hosts: BTreeMap<String, RemoteHostEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemoteHostEntry {
    /// OpenSSH config Host alias (legacy / advanced). Empty when managed-only.
    #[serde(default)]
    pub ssh_config_host: String,
    /// Managed host: remote hostname or IP.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hostname: Option<String>,
    /// Managed host: SSH user.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    /// Managed host: SSH port (default 22 when absent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    /// Managed host: local private key path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity_file: Option<String>,
    #[serde(default)]
    pub projects: BTreeMap<String, RemoteProjectEntry>,
}

impl RemoteHostEntry {
    /// True when this entry uses direct hostname + identity_file (bootstrap path).
    pub fn is_managed(&self) -> bool {
        self.hostname.as_ref().is_some_and(|h| !h.trim().is_empty())
            && self
                .identity_file
                .as_ref()
                .is_some_and(|p| !p.trim().is_empty())
    }

    /// Human-readable connection label for CLI/UI lists.
    pub fn display_target(&self) -> String {
        if self.is_managed() {
            let user = self.user.as_deref().unwrap_or("root");
            let host = self.hostname.as_deref().unwrap_or("?");
            let port = self.port.unwrap_or(22);
            if port == 22 {
                format!("{user}@{host}")
            } else {
                format!("{user}@{host}:{port}")
            }
        } else if !self.ssh_config_host.is_empty() {
            self.ssh_config_host.clone()
        } else {
            "(incomplete)".into()
        }
    }

    pub fn managed_user(&self) -> &str {
        self.user.as_deref().filter(|u| !u.is_empty()).unwrap_or("root")
    }

    pub fn managed_port(&self) -> u16 {
        self.port.unwrap_or(22)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemoteProjectEntry {
    /// Absolute path on the remote host.
    pub path: String,
    /// Runtime ids to check; empty means all registered runtimes.
    #[serde(default)]
    pub runtimes: Vec<String>,
}

pub fn remote_root_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("agent-doctor").join(REMOTE_DIR))
}

pub fn remote_hosts_path() -> Option<PathBuf> {
    remote_root_dir().map(|dir| dir.join(HOSTS_FILE))
}

pub fn remote_keys_dir() -> Option<PathBuf> {
    remote_root_dir().map(|dir| dir.join(KEYS_DIR))
}

/// Private key path for a managed host id: `{config}/agent-doctor/remote/keys/<id>`
pub fn remote_key_path_for(host_id: &str) -> Option<PathBuf> {
    remote_keys_dir().map(|dir| dir.join(host_id))
}

pub fn load_remote_hosts() -> Result<RemoteHostsDocument> {
    let Some(path) = remote_hosts_path() else {
        return Ok(RemoteHostsDocument::default());
    };
    load_remote_hosts_from(&path)
}

pub fn load_remote_hosts_from(path: &Path) -> Result<RemoteHostsDocument> {
    if !path.exists() {
        return Ok(RemoteHostsDocument::default());
    }
    let raw = fs::read_to_string(path)
        .with_context(|| format!("read remote hosts {}", path.display()))?;
    let doc: RemoteHostsDocument =
        serde_yaml::from_str(&raw).with_context(|| format!("parse {}", path.display()))?;
    Ok(doc)
}

pub fn save_remote_hosts(doc: &RemoteHostsDocument) -> Result<PathBuf> {
    let path = remote_hosts_path().context("config directory not found")?;
    save_remote_hosts_to(&path, doc)?;
    Ok(path)
}

pub fn save_remote_hosts_to(path: &Path, doc: &RemoteHostsDocument) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let raw = serde_yaml::to_string(doc).context("serialize remote hosts")?;
    fs::write(path, raw).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

/// Advanced path: register an existing OpenSSH config Host alias.
pub fn add_host(id: &str, ssh_config_host: &str) -> Result<RemoteHostsDocument> {
    validate_id(id, "host")?;
    if ssh_config_host.trim().is_empty() {
        bail!("ssh_config_host must not be empty");
    }
    let mut doc = load_remote_hosts()?;
    if doc.hosts.contains_key(id) {
        bail!("host '{id}' already exists");
    }
    doc.hosts.insert(
        id.to_string(),
        RemoteHostEntry {
            ssh_config_host: ssh_config_host.trim().to_string(),
            hostname: None,
            user: None,
            port: None,
            identity_file: None,
            projects: BTreeMap::new(),
        },
    );
    save_remote_hosts(&doc)?;
    Ok(doc)
}

/// Upsert a managed host entry (after successful key bootstrap).
pub fn upsert_managed_host(
    id: &str,
    hostname: &str,
    user: &str,
    port: u16,
    identity_file: &Path,
    ssh_config_host: Option<&str>,
) -> Result<RemoteHostsDocument> {
    validate_id(id, "host")?;
    if hostname.trim().is_empty() {
        bail!("hostname must not be empty");
    }
    if user.trim().is_empty() {
        bail!("user must not be empty");
    }
    if port == 0 {
        bail!("port must be non-zero");
    }
    let mut doc = load_remote_hosts()?;
    let label = ssh_config_host
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("")
        .to_string();
    let entry = RemoteHostEntry {
        ssh_config_host: label,
        hostname: Some(hostname.trim().to_string()),
        user: Some(user.trim().to_string()),
        port: Some(port),
        identity_file: Some(identity_file.display().to_string()),
        projects: doc
            .hosts
            .get(id)
            .map(|h| h.projects.clone())
            .unwrap_or_default(),
    };
    doc.hosts.insert(id.to_string(), entry);
    save_remote_hosts(&doc)?;
    Ok(doc)
}

pub fn remove_host(id: &str) -> Result<RemoteHostsDocument> {
    let mut doc = load_remote_hosts()?;
    if doc.hosts.remove(id).is_none() {
        bail!("unknown host '{id}'");
    }
    save_remote_hosts(&doc)?;
    // Best-effort cleanup of managed key material.
    if let Some(key) = remote_key_path_for(id) {
        let _ = fs::remove_file(&key);
        let pub_path = key.with_extension("pub");
        let _ = fs::remove_file(pub_path);
    }
    Ok(doc)
}

pub fn list_hosts() -> Result<Vec<(String, RemoteHostEntry)>> {
    let doc = load_remote_hosts()?;
    Ok(doc.hosts.into_iter().collect())
}

pub fn add_project(
    host_id: &str,
    name: &str,
    path: &str,
    runtimes: Vec<String>,
) -> Result<RemoteHostsDocument> {
    validate_id(name, "project")?;
    let path = path.trim();
    if path.is_empty() {
        bail!("project path must not be empty");
    }
    if !path.starts_with('/') {
        bail!("project path must be absolute on the remote host (got '{path}')");
    }
    let mut doc = load_remote_hosts()?;
    let host = doc
        .hosts
        .get_mut(host_id)
        .with_context(|| format!("unknown host '{host_id}'"))?;
    if host.projects.contains_key(name) {
        bail!("project '{name}' already exists on host '{host_id}'");
    }
    host.projects.insert(
        name.to_string(),
        RemoteProjectEntry {
            path: path.to_string(),
            runtimes,
        },
    );
    save_remote_hosts(&doc)?;
    Ok(doc)
}

pub fn remove_project(host_id: &str, name: &str) -> Result<RemoteHostsDocument> {
    let mut doc = load_remote_hosts()?;
    let host = doc
        .hosts
        .get_mut(host_id)
        .with_context(|| format!("unknown host '{host_id}'"))?;
    if host.projects.remove(name).is_none() {
        bail!("unknown project '{name}' on host '{host_id}'");
    }
    save_remote_hosts(&doc)?;
    Ok(doc)
}

pub fn list_projects(host_id: Option<&str>) -> Result<Vec<(String, String, RemoteProjectEntry)>> {
    let doc = load_remote_hosts()?;
    let mut out = Vec::new();
    for (hid, host) in doc.hosts {
        if let Some(filter) = host_id {
            if hid != filter {
                continue;
            }
        }
        for (pname, project) in host.projects {
            out.push((hid.clone(), pname, project));
        }
    }
    Ok(out)
}

pub(crate) fn validate_id(id: &str, kind: &str) -> Result<()> {
    if id.is_empty() {
        bail!("{kind} id must not be empty");
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        bail!("{kind} id may only contain ASCII letters, digits, '-' and '_'");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn roundtrip_hosts_yaml() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("hosts.yaml");
        let mut doc = RemoteHostsDocument::default();
        doc.hosts.insert(
            "prod".into(),
            RemoteHostEntry {
                ssh_config_host: "prod-vps".into(),
                hostname: None,
                user: None,
                port: None,
                identity_file: None,
                projects: BTreeMap::from([(
                    "api".into(),
                    RemoteProjectEntry {
                        path: "/srv/api".into(),
                        runtimes: vec!["hermes".into()],
                    },
                )]),
            },
        );
        save_remote_hosts_to(&path, &doc).unwrap();
        let loaded = load_remote_hosts_from(&path).unwrap();
        assert_eq!(loaded, doc);
    }

    #[test]
    fn roundtrip_managed_host() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("hosts.yaml");
        let mut doc = RemoteHostsDocument::default();
        doc.hosts.insert(
            "box".into(),
            RemoteHostEntry {
                ssh_config_host: String::new(),
                hostname: Some("1.2.3.4".into()),
                user: Some("ubuntu".into()),
                port: Some(22),
                identity_file: Some("/tmp/keys/box".into()),
                projects: BTreeMap::new(),
            },
        );
        save_remote_hosts_to(&path, &doc).unwrap();
        let loaded = load_remote_hosts_from(&path).unwrap();
        assert_eq!(loaded, doc);
        assert!(loaded.hosts["box"].is_managed());
        assert_eq!(loaded.hosts["box"].display_target(), "ubuntu@1.2.3.4");
    }

    #[test]
    fn legacy_yaml_without_managed_fields() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("hosts.yaml");
        fs::write(
            &path,
            "hosts:\n  prod:\n    ssh_config_host: prod-vps\n    projects: {}\n",
        )
        .unwrap();
        let loaded = load_remote_hosts_from(&path).unwrap();
        assert!(!loaded.hosts["prod"].is_managed());
        assert_eq!(loaded.hosts["prod"].ssh_config_host, "prod-vps");
    }
}
