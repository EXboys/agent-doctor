//! Managed headless `exo` child process + `caffeinate` idle-sleep guard on Apple Silicon.

use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use agent_doctor_core::exec::{command_for_path, run_output};
use agent_doctor_core::lifecycle::use_china_mirrors;

use crate::exo::lifecycle::{locate_exo_runtime, LocatedExoRuntime};
use crate::exo::{EXO_DEFAULT_PORT, EXO_LOCAL_ORIGIN};
use crate::namespace::resolve_cluster_namespace;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeHostSupport {
    /// Apple Silicon Mac running macOS 26.2 or newer — current exo build can host and share GPU.
    Supported,
    /// Apple Silicon Mac whose macOS cannot load the current exo GPU library.
    MacOsTooOld,
    /// Windows, Linux, or Intel Mac — connects as LAN consumer in v1.
    ConsumeOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DaemonOwner {
    NotRunning,
    ManagedByUs,
    ExternalProcess,
}

struct ManagedExoProcess {
    child: Child,
    sleep_guard: Option<Child>,
    namespace: String,
    /// Started with `EXO_OFFLINE` so model files come from disk, not hf-mirror.
    offline_models: bool,
}

static MANAGED_EXO: Mutex<Option<ManagedExoProcess>> = Mutex::new(None);

pub fn detect_host_support() -> NodeHostSupport {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        if current_macos_supports_exo() {
            NodeHostSupport::Supported
        } else {
            NodeHostSupport::MacOsTooOld
        }
    }
    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    {
        NodeHostSupport::ConsumeOnly
    }
}

/// Current pinned exo ships a Metal 4 library. macOS 15.6 can see the process, but cannot load it.
#[cfg_attr(
    not(all(target_os = "macos", target_arch = "aarch64")),
    allow(dead_code)
)]
pub(crate) fn macos_version_supports_exo(major: u32, minor: u32) -> bool {
    major > 26 || (major == 26 && minor >= 2)
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn current_macos_supports_exo() -> bool {
    let Ok(out) = run_output(
        Path::new("/usr/bin/sw_vers"),
        &["-productVersion"],
        Duration::from_secs(3),
    ) else {
        return false;
    };
    let ver = String::from_utf8_lossy(&out.stdout);
    parse_macos_major_minor(ver.trim())
        .is_some_and(|(maj, min)| macos_version_supports_exo(maj, min))
}

#[cfg_attr(
    not(all(target_os = "macos", target_arch = "aarch64")),
    allow(dead_code)
)]
pub(crate) fn parse_macos_major_minor(raw: &str) -> Option<(u32, u32)> {
    let mut parts = raw.trim().split('.');
    let major: u32 = parts.next()?.parse().ok()?;
    let minor: u32 = parts.next().unwrap_or("0").parse().unwrap_or(0);
    Some((major, minor))
}

pub fn exo_log_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".agent-doctor").join("logs").join("exo.log"))
}

fn pid_file_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| {
        h.join(".agent-doctor")
            .join("runtimes")
            .join("exo")
            .join("managed.pid")
    })
}

pub fn current_daemon_owner(local_api_alive: bool) -> (DaemonOwner, Option<String>) {
    let mut guard = MANAGED_EXO.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(proc) = guard.as_mut() {
        match proc.child.try_wait() {
            Ok(None) => return (DaemonOwner::ManagedByUs, Some(proc.namespace.clone())),
            _ => {
                if let Some(mut caffeinate) = proc.sleep_guard.take() {
                    let _ = caffeinate.kill();
                    let _ = caffeinate.wait();
                }
                *guard = None;
                clear_pid_file();
            }
        }
    }
    if local_api_alive {
        (DaemonOwner::ExternalProcess, None)
    } else {
        (DaemonOwner::NotRunning, None)
    }
}

/// Start the managed headless `exo` daemon (or reuse an already-running local node).
pub fn start_exo_daemon(runtime: &LocatedExoRuntime) -> Result<DaemonOwner> {
    if detect_host_support() != NodeHostSupport::Supported {
        bail!("这台电脑需要 Apple 芯片且系统在 macOS 15.1 以上才能开启本机算力。");
    }

    // Reap any stale PID left behind by an earlier crash before probing port 52415.
    reap_stale_pid_if_needed();

    if local_exo_node_reachable() {
        let (owner, _) = current_daemon_owner(true);
        if owner == DaemonOwner::ManagedByUs {
            return Ok(DaemonOwner::ManagedByUs);
        }
        return Ok(DaemonOwner::ExternalProcess);
    }

    let resolved_ns = resolve_cluster_namespace();
    spawn_managed_exo(runtime, &resolved_ns.namespace)?;
    Ok(DaemonOwner::ManagedByUs)
}

fn exo_cli_supports_namespace_flag(executable: &Path) -> bool {
    let Ok(out) = run_output(executable, &["--help"], Duration::from_secs(8)) else {
        return false;
    };
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    stdout.contains("--namespace") || stderr.contains("--namespace")
}

fn spawn_managed_exo(runtime: &LocatedExoRuntime, namespace: &str) -> Result<()> {
    let exo_home = dirs::home_dir()
        .map(|h| h.join(".exo"))
        .context("无法定位用户目录")?;
    fs::create_dir_all(&exo_home)?;

    let log_file = open_truncated_log()?;
    let log_err = log_file.try_clone().context("复制算力日志句柄失败")?;

    let mut cmd = command_for_path(&runtime.executable);
    cmd.current_dir(&exo_home)
        .arg("--api-port")
        .arg(EXO_DEFAULT_PORT.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::from(log_file))
        .stderr(Stdio::from(log_err));

    // Shipping DMGs through v1.0.71 are still libp2p (`EXO_LIBP2P_NAMESPACE`).
    // Newer mainline builds speak Zenoh (`--namespace` / `EXO_ZENOH_NAMESPACE`).
    // Probe `--help` once so a single daemon path works across both.
    if exo_cli_supports_namespace_flag(&runtime.executable) {
        cmd.arg("--namespace").arg(namespace);
    }

    let base_path = std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".to_string());
    let merged_path = format!("{}:{}", runtime.runtime_dir.display(), base_path);
    cmd.env("PATH", merged_path);
    cmd.env("EXO_RUNTIME_DIR", &runtime.runtime_dir);
    cmd.env("EXO_BUILD_ID", &runtime.build_id);
    // Always set both; the unused var is ignored by the binary that does not read it.
    cmd.env("EXO_LIBP2P_NAMESPACE", namespace);
    cmd.env("EXO_ZENOH_NAMESPACE", namespace);

    if let Some(ref zh_dir) = runtime.dashboard_zh_rel_home {
        cmd.env("EXO_DASHBOARD_DIR", zh_dir);
    }
    // China builds download the model from ModelScope into ~/.exo/models first.
    // Offline stops this process from falling back to an overseas mirror.
    let offline_models = use_china_mirrors();
    if offline_models {
        cmd.env("EXO_OFFLINE", "1");
    }

    let mut child = cmd
        .spawn()
        .with_context(|| format!("启动本机算力服务失败（{}）", runtime.executable.display()))?;
    let pid = child.id();
    write_pid_file(pid);

    let sleep_guard = spawn_sleep_guard(pid);

    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        if let Ok(Some(status)) = child.try_wait() {
            clear_pid_file();
            let tail = read_log_tail(6);
            bail!(
                "本机算力服务启动后立即退出（{status}）。{}",
                if tail.is_empty() {
                    String::new()
                } else {
                    format!("详情：{tail}")
                }
            );
        }
        if local_exo_node_reachable() {
            let mut guard = MANAGED_EXO.lock().unwrap_or_else(|e| e.into_inner());
            *guard = Some(ManagedExoProcess {
                child,
                sleep_guard,
                namespace: namespace.to_string(),
                offline_models,
            });
            return Ok(());
        }
        thread::sleep(Duration::from_millis(350));
    }

    // Still starting on cold Gatekeeper scan; keep handle alive if child hasn't exited.
    let mut guard = MANAGED_EXO.lock().unwrap_or_else(|e| e.into_inner());
    *guard = Some(ManagedExoProcess {
        child,
        sleep_guard,
        namespace: namespace.to_string(),
        offline_models,
    });
    Ok(())
}

/// Prevent macOS from idle-sleeping while sharing unified memory with the cluster.
/// `-w <pid>` ties `caffeinate`'s lifetime strictly to `exo` so it never leaks.
fn spawn_sleep_guard(exo_pid: u32) -> Option<Child> {
    #[cfg(target_os = "macos")]
    {
        let caffeinate = Path::new("/usr/bin/caffeinate");
        if !caffeinate.exists() {
            return None;
        }
        let mut cmd = command_for_path(caffeinate);
        cmd.args(["-i", "-w", &exo_pid.to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        cmd.spawn().ok()
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = exo_pid;
        None
    }
}

/// Stop the managed `exo` daemon (leaves user-launched external `EXO.app` untouched).
pub fn stop_exo_daemon() -> Result<()> {
    let taken = {
        let mut guard = MANAGED_EXO.lock().unwrap_or_else(|e| e.into_inner());
        guard.take()
    };

    if let Some(mut proc) = taken {
        let pid = proc.child.id();
        terminate_pid_gracefully(pid, &mut proc.child);
        if let Some(mut caffeinate) = proc.sleep_guard {
            let _ = caffeinate.kill();
            let _ = caffeinate.wait();
        }
    } else {
        reap_stale_pid_if_needed();
    }
    clear_pid_file();
    Ok(())
}

/// Restart the daemon we started so it reads the ModelScope snapshot from disk.
/// Leaves a separately opened EXO app alone.
pub fn restart_for_offline_models() -> Result<()> {
    if !use_china_mirrors() {
        return Ok(());
    }
    {
        let guard = MANAGED_EXO.lock().unwrap_or_else(|e| e.into_inner());
        if guard.as_ref().is_some_and(|proc| proc.offline_models) {
            return Ok(());
        }
    }
    let foreign = {
        let guard = MANAGED_EXO.lock().unwrap_or_else(|e| e.into_inner());
        guard.is_none() && local_exo_node_reachable() && !pid_file_matches_live_exo()
    };
    if foreign {
        return Ok(());
    }
    stop_exo_daemon()?;
    let deadline = Instant::now() + Duration::from_secs(5);
    while local_exo_node_reachable() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(200));
    }
    if local_exo_node_reachable() {
        return Ok(());
    }
    if let Some(rt) = locate_exo_runtime() {
        start_exo_daemon(&rt)?;
    }
    Ok(())
}

/// Restart managed daemon if its namespace changed (e.g. user changed pairing code).
pub fn restart_exo_daemon_if_managed() -> Result<()> {
    let is_managed = {
        let guard = MANAGED_EXO.lock().unwrap_or_else(|e| e.into_inner());
        guard.is_some()
    };
    if is_managed {
        stop_exo_daemon()?;
        if let Some(rt) = locate_exo_runtime() {
            let _ = start_exo_daemon(&rt)?;
        }
    }
    Ok(())
}

pub fn local_exo_node_reachable() -> bool {
    let Ok(client) = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_millis(350))
        .timeout(Duration::from_millis(700))
        .build()
    else {
        return false;
    };
    client
        .get(format!("{EXO_LOCAL_ORIGIN}/node_id"))
        .send()
        .is_ok_and(|r| r.status().is_success())
}

fn terminate_pid_gracefully(pid: u32, child: &mut Child) {
    #[cfg(unix)]
    {
        let _ = run_output(
            Path::new("/bin/kill"),
            &["-TERM", &pid.to_string()],
            Duration::from_secs(2),
        );
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            if matches!(child.try_wait(), Ok(Some(_))) {
                return;
            }
            thread::sleep(Duration::from_millis(120));
        }
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn reap_stale_pid_if_needed() {
    let Some(path) = pid_file_path() else {
        return;
    };
    let Ok(raw) = fs::read_to_string(&path) else {
        return;
    };
    let Ok(pid) = raw.trim().parse::<u32>() else {
        let _ = fs::remove_file(&path);
        return;
    };
    #[cfg(unix)]
    {
        // Only terminate if `ps` shows this PID is indeed an `exo` process we spawned.
        if let Ok(out) = run_output(
            Path::new("/bin/ps"),
            &["-p", &pid.to_string(), "-o", "comm="],
            Duration::from_secs(2),
        ) {
            let comm = String::from_utf8_lossy(&out.stdout);
            if comm.contains("exo") {
                let _ = run_output(
                    Path::new("/bin/kill"),
                    &["-TERM", &pid.to_string()],
                    Duration::from_secs(2),
                );
                thread::sleep(Duration::from_millis(300));
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
    }
    let _ = fs::remove_file(&path);
}

fn pid_file_matches_live_exo() -> bool {
    let Some(path) = pid_file_path() else {
        return false;
    };
    let Ok(raw) = fs::read_to_string(path) else {
        return false;
    };
    let Ok(pid) = raw.trim().parse::<u32>() else {
        return false;
    };
    #[cfg(unix)]
    {
        let Ok(out) = run_output(
            Path::new("/bin/ps"),
            &["-p", &pid.to_string(), "-o", "comm="],
            Duration::from_secs(2),
        ) else {
            return false;
        };
        String::from_utf8_lossy(&out.stdout).contains("exo")
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        false
    }
}

fn write_pid_file(pid: u32) {
    if let Some(path) = pid_file_path() {
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let _ = fs::write(path, pid.to_string());
    }
}

fn clear_pid_file() {
    if let Some(path) = pid_file_path() {
        let _ = fs::remove_file(path);
    }
}

fn open_truncated_log() -> Result<fs::File> {
    let path = exo_log_path().context("无法创建算力日志目录")?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    // Truncate if previous log exceeded 4 MiB so logs never grow unbounded.
    let truncate = fs::metadata(&path).is_ok_and(|m| m.len() > 4 * 1024 * 1024);
    OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(truncate)
        .append(!truncate)
        .open(&path)
        .with_context(|| format!("打开算力日志文件失败：{}", path.display()))
}

fn read_log_tail(max_lines: usize) -> String {
    let Some(path) = exo_log_path() else {
        return String::new();
    };
    let Ok(raw) = fs::read_to_string(path) else {
        return String::new();
    };
    let lines: Vec<&str> = raw.lines().rev().take(max_lines).collect();
    lines.into_iter().rev().collect::<Vec<_>>().join(" | ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macos_version_check_parses_sequoia_and_tahoe() {
        assert_eq!(parse_macos_major_minor("15.1.1"), Some((15, 1)));
        assert_eq!(parse_macos_major_minor("26.2"), Some((26, 2)));
        assert_eq!(parse_macos_major_minor("14.7"), Some((14, 7)));
        assert!(!macos_version_supports_exo(15, 6));
        assert!(!macos_version_supports_exo(26, 1));
        assert!(macos_version_supports_exo(26, 2));
        assert!(macos_version_supports_exo(27, 0));
    }
}
