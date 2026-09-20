//! One-shot password SSH → install ed25519 pubkey → BatchMode key auth thereafter.
//!
//! Password is never written to hosts.yaml or secrets store.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};
use tempfile::TempDir;

use super::registry::{
    remote_key_path_for, remote_keys_dir, upsert_managed_host, validate_id, RemoteHostsDocument,
};
use super::ssh::SshBackend;
use crate::exec::shell_quote;

#[derive(Debug, Clone)]
pub struct BootstrapHostOptions {
    pub id: String,
    pub hostname: String,
    pub user: String,
    pub port: u16,
    /// One-time password; never persisted.
    pub password: String,
    /// Optional display label stored as `ssh_config_host`.
    pub label: Option<String>,
}

/// Generate ed25519 key, install pubkey with password once, verify BatchMode, save registry.
pub fn bootstrap_and_add_host(opts: BootstrapHostOptions) -> Result<RemoteHostsDocument> {
    validate_id(&opts.id, "host")?;
    if opts.hostname.trim().is_empty() {
        bail!("hostname must not be empty");
    }
    if opts.user.trim().is_empty() {
        bail!("user must not be empty");
    }
    if opts.port == 0 {
        bail!("port must be non-zero");
    }
    if opts.password.is_empty() {
        bail!("password must not be empty (use --password-env or --password for one-time auth)");
    }

    let key_path = remote_key_path_for(&opts.id).context("config directory not found")?;
    let keys_dir = remote_keys_dir().context("config directory not found")?;
    fs::create_dir_all(&keys_dir)
        .with_context(|| format!("create keys dir {}", keys_dir.display()))?;

    // Fresh key for this host id.
    cleanup_key_pair(&key_path);
    generate_ed25519_key(&key_path, &opts.id)
        .with_context(|| format!("generate ed25519 key at {}", key_path.display()))?;

    let pubkey_path = key_path.with_extension("pub");
    let pubkey = fs::read_to_string(&pubkey_path)
        .with_context(|| format!("read public key {}", pubkey_path.display()))?;
    let pubkey = pubkey.trim();
    if pubkey.is_empty() {
        cleanup_key_pair(&key_path);
        bail!("generated public key is empty");
    }

    if let Err(err) = install_pubkey_with_password(
        opts.hostname.trim(),
        opts.user.trim(),
        opts.port,
        &opts.password,
        pubkey,
    ) {
        cleanup_key_pair(&key_path);
        return Err(err).context(
            "failed to install public key with password SSH — check host/user/port/password \
             and that password auth is allowed on the VPS",
        );
    }

    if let Err(err) = verify_key_login(
        opts.hostname.trim(),
        opts.user.trim(),
        opts.port,
        &key_path,
    ) {
        cleanup_key_pair(&key_path);
        return Err(err).context(
            "public key installed but BatchMode key login failed — check authorized_keys \
             permissions on the VPS",
        );
    }

    upsert_managed_host(
        &opts.id,
        opts.hostname.trim(),
        opts.user.trim(),
        opts.port,
        &key_path,
        opts.label.as_deref(),
    )
}

/// `ssh-keygen -t ed25519 -N "" -f <path> -C agent-doctor-<id>`
pub fn generate_ed25519_key(path: &Path, host_id: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create {}", parent.display()))?;
    }
    let comment = format!("agent-doctor-{host_id}");
    let status = Command::new("ssh-keygen")
        .arg("-t")
        .arg("ed25519")
        .arg("-N")
        .arg("")
        .arg("-f")
        .arg(path)
        .arg("-C")
        .arg(&comment)
        .arg("-q")
        .stdin(Stdio::null())
        .status()
        .context("failed to spawn ssh-keygen; is OpenSSH installed?")?;
    if !status.success() {
        bail!("ssh-keygen failed with status {status}");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(path)
            .with_context(|| format!("stat {}", path.display()))?
            .permissions();
        perms.set_mode(0o600);
        fs::set_permissions(path, perms)
            .with_context(|| format!("chmod 600 {}", path.display()))?;
    }
    Ok(())
}

/// One-time password SSH that appends `pubkey` to `~/.ssh/authorized_keys`.
pub fn install_pubkey_with_password(
    hostname: &str,
    user: &str,
    port: u16,
    password: &str,
    pubkey: &str,
) -> Result<()> {
    let remote = build_install_pubkey_remote_script(pubkey);
    run_password_ssh(hostname, user, port, password, &remote)
}

/// Build the remote shell script used to install a public key (unit-testable).
pub fn build_install_pubkey_remote_script(pubkey: &str) -> String {
    let quoted = shell_quote(pubkey.trim());
    format!(
        "mkdir -p ~/.ssh && chmod 700 ~/.ssh && \
         touch ~/.ssh/authorized_keys && chmod 600 ~/.ssh/authorized_keys && \
         grep -qxF {quoted} ~/.ssh/authorized_keys || echo {quoted} >> ~/.ssh/authorized_keys"
    )
}

/// `ssh -i … -o BatchMode=yes -o IdentitiesOnly=yes … true`
pub fn verify_key_login(hostname: &str, user: &str, port: u16, identity_file: &Path) -> Result<()> {
    let backend = SshBackend::direct(user, hostname, port, identity_file);
    let mut cmd = Command::new("ssh");
    cmd.arg("-o")
        .arg("BatchMode=yes")
        .arg("-o")
        .arg("IdentitiesOnly=yes")
        .arg("-o")
        .arg("StrictHostKeyChecking=accept-new")
        .arg("-o")
        .arg(format!("ConnectTimeout={}", backend.connect_timeout_secs))
        .arg("-i")
        .arg(identity_file)
        .arg("-p")
        .arg(port.to_string())
        .arg(format!("{user}@{hostname}"))
        .arg("--")
        .arg("true")
        .stdin(Stdio::null());

    let output = cmd
        .output()
        .context("failed to spawn ssh for key verification")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "key login verification failed (exit {}): {}",
            output.status.code().unwrap_or(-1),
            stderr.trim()
        );
    }
    Ok(())
}

fn run_password_ssh(
    hostname: &str,
    user: &str,
    port: u16,
    password: &str,
    remote_script: &str,
) -> Result<()> {
    let askpass = AskPassGuard::create(password)?;
    let mut cmd = Command::new("ssh");
    cmd.arg("-o")
        .arg("PreferredAuthentications=password")
        .arg("-o")
        .arg("PubkeyAuthentication=no")
        .arg("-o")
        .arg("NumberOfPasswordPrompts=1")
        .arg("-o")
        .arg("StrictHostKeyChecking=accept-new")
        .arg("-o")
        .arg("ConnectTimeout=15")
        .arg("-p")
        .arg(port.to_string())
        .arg(format!("{user}@{hostname}"))
        .arg("--")
        .arg("sh")
        .arg("-c")
        .arg(remote_script)
        .stdin(Stdio::null())
        .env("SSH_ASKPASS", &askpass.askpass_path)
        .env("SSH_ASKPASS_REQUIRE", "force")
        // OpenSSH may ignore SSH_ASKPASS without DISPLAY on some platforms.
        .env("DISPLAY", askpass.display.as_str())
        .env_remove("SSH_AUTH_SOCK");

    let output = cmd
        .output()
        .context("failed to spawn ssh for password bootstrap")?;
    // Drop askpass (wipes password file) before interpreting result.
    drop(askpass);

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let detail = if !stderr.trim().is_empty() {
            stderr.trim().to_string()
        } else {
            stdout.trim().to_string()
        };
        bail!(
            "password SSH failed (exit {}): {detail}",
            output.status.code().unwrap_or(-1)
        );
    }
    Ok(())
}

fn cleanup_key_pair(private_key: &Path) {
    let _ = fs::remove_file(private_key);
    let _ = fs::remove_file(private_key.with_extension("pub"));
}

/// Temporary ASKPASS helper that prints the password once; password never stays in argv.
struct AskPassGuard {
    _dir: TempDir,
    askpass_path: PathBuf,
    display: String,
}

impl AskPassGuard {
    fn create(password: &str) -> Result<Self> {
        let dir = tempfile::tempdir().context("create temp dir for SSH_ASKPASS")?;
        let pass_path = dir.path().join("pass");
        {
            let mut f = fs::File::create(&pass_path)
                .with_context(|| format!("create {}", pass_path.display()))?;
            f.write_all(password.as_bytes())
                .context("write password file")?;
            f.write_all(b"\n").ok();
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&pass_path)?.permissions();
            perms.set_mode(0o600);
            fs::set_permissions(&pass_path, perms)?;
        }

        #[cfg(unix)]
        let askpass_path = {
            let script = dir.path().join("askpass.sh");
            let body = format!(
                "#!/bin/sh\nexec cat {}\n",
                shell_quote(&pass_path.display().to_string())
            );
            fs::write(&script, body)?;
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&script)?.permissions();
            perms.set_mode(0o700);
            fs::set_permissions(&script, perms)?;
            script
        };

        #[cfg(windows)]
        let askpass_path = {
            let script = dir.path().join("askpass.cmd");
            // `type` prints the file; quote path for spaces.
            let body = format!("@echo off\r\ntype \"{}\"\r\n", pass_path.display());
            fs::write(&script, body)?;
            script
        };

        let display = std::env::var("DISPLAY").unwrap_or_else(|_| ":0".into());
        Ok(Self {
            _dir: dir,
            askpass_path,
            display,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn install_script_quotes_pubkey() {
        let script = build_install_pubkey_remote_script("ssh-ed25519 AAAAC3 test@host");
        assert!(script.contains("mkdir -p ~/.ssh"));
        assert!(script.contains("authorized_keys"));
        assert!(script.contains("ssh-ed25519 AAAAC3 test@host"));
    }

    #[test]
    fn generate_ed25519_when_ssh_keygen_available() {
        if Command::new("ssh-keygen").arg("-h").output().is_err() {
            return;
        }

        let dir = tempdir().unwrap();
        let key = dir.path().join("testkey");
        if let Err(err) = generate_ed25519_key(&key, "testid") {
            eprintln!("skip generate_ed25519: {err}");
            return;
        }
        assert!(key.exists());
        assert!(key.with_extension("pub").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&key).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "private key must be 0600, got {mode:o}");
        }
    }
}
