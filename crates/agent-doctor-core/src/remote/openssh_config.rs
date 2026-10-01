//! Point the computer's normal `ssh user@host` at the key this app just installed.
//!
//! Bootstrap only authorizes Agent Doctor's own key. A later `ssh root@ip` offers
//! the Mac's other keys and is rejected. A marked block at the top of
//! `~/.ssh/config` makes that same key the one OpenSSH uses for this host.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

fn begin_marker(host_id: &str) -> String {
    format!("# BEGIN agent-doctor {host_id}")
}

fn end_marker(host_id: &str) -> String {
    format!("# END agent-doctor {host_id}")
}

pub fn user_ssh_config_path() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".ssh").join("config"))
}

pub fn sync_managed_host(
    config_path: &Path,
    host_id: &str,
    hostname: &str,
    user: &str,
    port: u16,
    identity_file: &Path,
) -> Result<()> {
    validate_token(host_id, "host id")?;
    validate_token(hostname, "hostname")?;
    validate_token(user, "user")?;
    if port == 0 {
        bail!("port must be non-zero");
    }
    let identity = identity_file.display().to_string();
    if identity.contains('\n') || identity.contains('\r') {
        bail!("identity path must be a single line");
    }

    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(parent)
                .with_context(|| format!("stat {}", parent.display()))?
                .permissions();
            perms.set_mode(0o700);
            fs::set_permissions(parent, perms)
                .with_context(|| format!("chmod 700 {}", parent.display()))?;
        }
    }

    let existing = if config_path.exists() {
        fs::read_to_string(config_path)
            .with_context(|| format!("read {}", config_path.display()))?
    } else {
        String::new()
    };
    let rest = strip_block(&existing, host_id);
    let block = render_block(host_id, hostname, user, port, &identity);
    let next = join_block(&block, &rest);
    if next == existing {
        return Ok(());
    }
    fs::write(config_path, &next).with_context(|| format!("write {}", config_path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(config_path)
            .with_context(|| format!("stat {}", config_path.display()))?
            .permissions();
        perms.set_mode(0o600);
        fs::set_permissions(config_path, perms)
            .with_context(|| format!("chmod 600 {}", config_path.display()))?;
    }
    Ok(())
}

pub fn forget_managed_host(config_path: &Path, host_id: &str) -> Result<()> {
    if !config_path.exists() {
        return Ok(());
    }
    let existing = fs::read_to_string(config_path)
        .with_context(|| format!("read {}", config_path.display()))?;
    let next = strip_block(&existing, host_id);
    if next == existing {
        return Ok(());
    }
    fs::write(config_path, next).with_context(|| format!("write {}", config_path.display()))?;
    Ok(())
}

fn validate_token(value: &str, label: &str) -> Result<()> {
    let value = value.trim();
    if value.is_empty() || value.contains(['\n', '\r', ' ', '#']) {
        bail!("{label} is not safe to write into ssh config");
    }
    Ok(())
}

fn join_block(block: &str, rest: &str) -> String {
    let rest = rest.trim_start();
    if rest.is_empty() {
        return block.to_string();
    }
    let mut next = block.to_string();
    if !next.ends_with('\n') {
        next.push('\n');
    }
    next.push('\n');
    next.push_str(rest);
    if !next.ends_with('\n') {
        next.push('\n');
    }
    next
}

fn render_block(host_id: &str, hostname: &str, user: &str, port: u16, identity: &str) -> String {
    let identity = if identity.contains(' ') {
        format!("\"{identity}\"")
    } else {
        identity.to_string()
    };
    format!(
        "{begin}\nHost {hostname}\n  HostName {hostname}\n  User {user}\n  Port {port}\n  IdentityFile {identity}\n  IdentitiesOnly yes\n  PreferredAuthentications publickey\n  PasswordAuthentication no\n{end}\n",
        begin = begin_marker(host_id),
        end = end_marker(host_id),
    )
}

fn strip_block(config: &str, host_id: &str) -> String {
    let begin = begin_marker(host_id);
    let end = end_marker(host_id);
    let mut out = String::new();
    let mut skipping = false;
    for line in config.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\n', '\r']);
        if trimmed == begin {
            skipping = true;
            continue;
        }
        if skipping {
            if trimmed == end {
                skipping = false;
            }
            continue;
        }
        out.push_str(line);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn managed_block_is_first_and_replaces_itself() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config");
        fs::write(&path, "Host *\n  IdentityFile ~/.ssh/id_ed25519\n").unwrap();
        let key = dir.path().join("keys").join("box");
        sync_managed_host(&path, "box", "47.107.251.112", "root", 22, &key).unwrap();
        sync_managed_host(&path, "box", "47.107.251.112", "root", 22, &key).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert_eq!(text.matches("# BEGIN agent-doctor box").count(), 1);
        assert!(text.starts_with("# BEGIN agent-doctor box\n"));
        assert!(text.contains("IdentityFile "));
        assert!(text.contains(key.display().to_string().as_str()));
        assert!(text.contains("IdentitiesOnly yes"));
        assert!(text.contains("PreferredAuthentications publickey"));
        assert!(text.contains("PasswordAuthentication no"));
        assert!(text.contains("Host *"));
        let star = text.find("Host *").unwrap();
        let ours = text.find("Host 47.107.251.112").unwrap();
        assert!(ours < star);
    }

    #[test]
    fn forget_removes_only_that_host() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config");
        let key = dir.path().join("key");
        sync_managed_host(&path, "box", "1.2.3.4", "root", 22, &key).unwrap();
        fs::write(
            &path,
            format!(
                "{}\nHost github.com\n  User git\n",
                fs::read_to_string(&path).unwrap()
            ),
        )
        .unwrap();
        forget_managed_host(&path, "box").unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(!text.contains("agent-doctor"));
        assert!(text.contains("Host github.com"));
    }
}
