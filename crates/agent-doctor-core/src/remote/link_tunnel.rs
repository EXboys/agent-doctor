//! SSH local-forward tunnel to a remote loopback client service (Paseo-style).

use std::io;
use std::net::TcpStream;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};

use super::link_daemon::{probe_local_daemon, wait_for_local_port};
use super::link_protocol::{LinkHello, LINK_DAEMON_PORT};
use super::registry::RemoteHostEntry;
use super::ssh::SshTarget;

pub struct LinkTunnel {
    pub local_port: u16,
    child: Child,
    host_id: String,
}

impl LinkTunnel {
    pub fn host_id(&self) -> &str {
        &self.host_id
    }

    pub fn stop(mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for LinkTunnel {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn pick_free_local_port() -> Result<u16> {
    let listener =
        std::net::TcpListener::bind("127.0.0.1:0").context("无法在本机选一个空闲端口")?;
    let port = listener.local_addr()?.port();
    drop(listener);
    Ok(port)
}

fn ssh_forward_command(
    entry: &RemoteHostEntry,
    local_port: u16,
    remote_port: u16,
) -> Result<Command> {
    let target = SshTarget::from_host_entry(entry)?;
    let mut cmd = crate::exec::command_for_path(Path::new("ssh"));
    cmd.arg("-N")
        .arg("-o")
        .arg("BatchMode=yes")
        .arg("-o")
        .arg("ConnectTimeout=10")
        .arg("-o")
        .arg("ConnectionAttempts=1")
        .arg("-o")
        .arg("GSSAPIAuthentication=no")
        .arg("-o")
        .arg("PreferredAuthentications=publickey")
        .arg("-o")
        .arg("NumberOfPasswordPrompts=0")
        .arg("-o")
        .arg("StrictHostKeyChecking=accept-new")
        .arg("-o")
        .arg("ExitOnForwardFailure=yes")
        .arg("-o")
        .arg("ClearAllForwardings=yes")
        .arg("-o")
        .arg("ServerAliveInterval=20")
        .arg("-o")
        .arg("ServerAliveCountMax=3")
        .arg("-L")
        .arg(format!("127.0.0.1:{local_port}:127.0.0.1:{remote_port}"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());

    match target {
        SshTarget::ConfigHost { host } => {
            cmd.arg(host);
        }
        SshTarget::Direct {
            user,
            hostname,
            port,
            identity_file,
        } => {
            cmd.arg("-i")
                .arg(identity_file)
                .arg("-o")
                .arg("IdentitiesOnly=yes")
                .arg("-p")
                .arg(port.to_string())
                .arg(format!("{user}@{hostname}"));
        }
    }
    Ok(cmd)
}

#[derive(Debug, Clone)]
pub enum LinkConnectFailure {
    /// SSH itself could not log in.
    SshUnreachable {
        detail: String,
    },
    /// SSH worked, but the remote client service is not listening.
    ClientNotOpen {
        detail: String,
    },
    Other {
        detail: String,
    },
}

impl std::fmt::Display for LinkConnectFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SshUnreachable { .. } => {
                write!(f, "连不上这台电脑，请确认它开着机且网络能通。")
            }
            Self::ClientNotOpen { .. } => {
                write!(f, "已经连上这台电脑，但那边的客户端还没打开。")
            }
            Self::Other { detail } => write!(f, "{detail}"),
        }
    }
}

/// Open an SSH local forward and confirm the remote client answers `ping`.
pub fn open_link_tunnel(
    host_id: &str,
    entry: &RemoteHostEntry,
) -> Result<(LinkTunnel, LinkHello), LinkConnectFailure> {
    let local_port = pick_free_local_port().map_err(|err| LinkConnectFailure::Other {
        detail: format!("{err:#}"),
    })?;
    let mut cmd = ssh_forward_command(entry, local_port, LINK_DAEMON_PORT).map_err(|err| {
        LinkConnectFailure::Other {
            detail: format!("{err:#}"),
        }
    })?;

    let mut child = cmd
        .spawn()
        .map_err(|err| LinkConnectFailure::SshUnreachable {
            detail: format!("打不开 SSH：{err}"),
        })?;

    // Wait until the local forward port accepts TCP, or ssh exits.
    let ready = wait_until_forward_ready(&mut child, local_port, Duration::from_secs(12));
    if !ready {
        let detail = read_child_stderr(&mut child);
        let _ = child.kill();
        let _ = child.wait();
        if detail.to_ascii_lowercase().contains("permission denied")
            || detail.to_ascii_lowercase().contains("connection timed out")
            || detail.to_ascii_lowercase().contains("could not resolve")
            || detail.to_ascii_lowercase().contains("no route to host")
        {
            return Err(LinkConnectFailure::SshUnreachable { detail });
        }
        return Err(LinkConnectFailure::SshUnreachable {
            detail: if detail.is_empty() {
                "SSH 没有建立起来。".into()
            } else {
                detail
            },
        });
    }

    // SSH is up. If the client service is closed on B, probe fails with a clear message.
    match probe_local_daemon(local_port) {
        Ok(hello) => Ok((
            LinkTunnel {
                local_port,
                child,
                host_id: host_id.to_string(),
            },
            hello,
        )),
        Err(err) => {
            let _ = child.kill();
            let _ = child.wait();
            Err(LinkConnectFailure::ClientNotOpen {
                detail: format!("{err:#}"),
            })
        }
    }
}

fn wait_until_forward_ready(child: &mut Child, port: u16, timeout: Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if let Ok(Some(status)) = child.try_wait() {
            let _ = status;
            return false;
        }
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return true;
        }
        // Also accept "port is listening" via the shared wait helper.
        if wait_for_local_port(port, Duration::from_millis(50)) {
            return true;
        }
        thread::sleep(Duration::from_millis(80));
    }
    matches!(child.try_wait(), Ok(None)) && TcpStream::connect(("127.0.0.1", port)).is_ok()
}

fn read_child_stderr(child: &mut Child) -> String {
    let Some(mut stderr) = child.stderr.take() else {
        return String::new();
    };
    // Non-blocking-ish: ssh usually exits quickly on auth failure.
    let mut buf = String::new();
    let _ = io::Read::read_to_string(&mut stderr, &mut buf);
    buf.trim().to_string()
}
