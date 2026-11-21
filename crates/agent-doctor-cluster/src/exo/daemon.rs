//! Managed headless `exo` child process + `caffeinate` idle-sleep guard on Apple Silicon.

use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;

use agent_doctor_core::exec::{command_for_path, run_output};
use agent_doctor_core::setup::{load_active_personal_provider, LOCAL_CLUSTER_PROVIDER_ID};

use crate::exo::lifecycle::{locate_exo_runtime, LocatedExoRuntime};
use crate::exo::orchestrator::{prepare_model_and_activate, probe_exo_origin};
use crate::exo::{EXO_DEFAULT_PORT, EXO_LIBP2P_PORT, EXO_LOCAL_ORIGIN};
use crate::namespace::{prefers_modelscope, resolve_cluster_namespace};

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
    bootstrap_peers: Vec<String>,
    /// True when this process was started with `--libp2p-port`.
    port_pinned: bool,
    /// Started with `EXO_OFFLINE` so model files come from disk, not hf-mirror.
    offline_models: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AppliedJoin {
    pub namespace: String,
    pub peers: Vec<String>,
    pub port_pinned: bool,
}

#[derive(Clone, Copy)]
struct ExoCliCaps {
    namespace_flag: bool,
    libp2p_port_flag: bool,
    bootstrap_flag: bool,
}

static MANAGED_EXO: Mutex<Option<ManagedExoProcess>> = Mutex::new(None);
static PENDING_BOOTSTRAP_PEERS: Mutex<Option<Vec<String>>> = Mutex::new(None);

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
        if pid_file_matches_live_exo() {
            let namespace = read_join_file().map(|join| join.namespace);
            return (DaemonOwner::ManagedByUs, namespace);
        }
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

    // Install Agent Doctor's dashboard before spawn so EXO_DASHBOARD_DIR is set.
    let _ = crate::exo::lifecycle::ensure_managed_dashboard_zh();
    let mut runtime = runtime.clone();
    runtime.dashboard_zh_rel_home = crate::exo::lifecycle::pinned_dashboard_zh_rel_home();

    // Reap any stale PID left behind by an earlier crash before probing port 52415.
    reap_stale_pid_if_needed();

    if local_exo_node_reachable() {
        let (owner, _) = current_daemon_owner(true);
        if owner == DaemonOwner::ManagedByUs {
            // Started earlier without the customized board — bounce so the env applies.
            if runtime.dashboard_zh_rel_home.is_some() && !local_dashboard_looks_customized() {
                stop_exo_daemon()?;
                let deadline = Instant::now() + Duration::from_secs(5);
                while local_exo_node_reachable() && Instant::now() < deadline {
                    thread::sleep(Duration::from_millis(200));
                }
                if !local_exo_node_reachable() {
                    let resolved_ns = resolve_cluster_namespace();
                    spawn_managed_exo(&runtime, &resolved_ns.namespace)?;
                    return Ok(DaemonOwner::ManagedByUs);
                }
            }
            return Ok(DaemonOwner::ManagedByUs);
        }
        // Separately opened EXO.app keeps the stock dashboard; leave it alone.
        return Ok(DaemonOwner::ExternalProcess);
    }

    let resolved_ns = resolve_cluster_namespace();
    spawn_managed_exo(&runtime, &resolved_ns.namespace)?;
    Ok(DaemonOwner::ManagedByUs)
}

/// True when the live `:52415` page is Agent Doctor's overlay (not stock exo).
fn local_dashboard_looks_customized() -> bool {
    let Ok(client) = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_millis(400))
        .timeout(Duration::from_millis(1200))
        .build()
    else {
        return false;
    };
    let Ok(resp) = client.get(EXO_LOCAL_ORIGIN).send() else {
        return false;
    };
    if !resp.status().is_success() {
        return false;
    }
    let Ok(text) = resp.text() else {
        return false;
    };
    text.contains("EXO算力管理") || text.contains("适合本机") || text.contains("AGENT_DOCTOR_ZH")
}

fn exo_cli_caps(executable: &Path) -> ExoCliCaps {
    static CACHE: Mutex<Option<(PathBuf, ExoCliCaps)>> = Mutex::new(None);
    {
        let guard = CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((path, caps)) = guard.as_ref() {
            if path == executable {
                return *caps;
            }
        }
    }
    let caps = match run_output(executable, &["--help"], Duration::from_secs(8)) {
        Ok(out) => {
            let text = format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
            ExoCliCaps {
                namespace_flag: text.contains("--namespace"),
                libp2p_port_flag: text.contains("--libp2p-port"),
                bootstrap_flag: text.contains("--bootstrap-peers"),
            }
        }
        Err(_) => ExoCliCaps {
            namespace_flag: false,
            libp2p_port_flag: false,
            bootstrap_flag: false,
        },
    };
    *CACHE.lock().unwrap_or_else(|e| e.into_inner()) = Some((executable.to_path_buf(), caps));
    caps
}

pub(crate) fn runtime_supports_pinned_libp2p() -> bool {
    locate_exo_runtime().is_some_and(|rt| exo_cli_caps(&rt.executable).libp2p_port_flag)
}

fn spawn_managed_exo(runtime: &LocatedExoRuntime, namespace: &str) -> Result<()> {
    let exo_home = dirs::home_dir()
        .map(|h| h.join(".exo"))
        .context("无法定位用户目录")?;
    fs::create_dir_all(&exo_home)?;

    let log_file = open_truncated_log()?;
    let log_err = log_file.try_clone().context("复制算力日志句柄失败")?;

    let peers = peers_for_spawn();
    let caps = exo_cli_caps(&runtime.executable);
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
    if caps.namespace_flag {
        cmd.arg("--namespace").arg(namespace);
    }
    // Fixed port + explicit peers. mDNS often never arrives (local-network privacy,
    // guest Wi-Fi, Thunderbolt). Dialing a known port still joins the same account.
    if caps.libp2p_port_flag {
        cmd.arg("--libp2p-port").arg(EXO_LIBP2P_PORT.to_string());
    }
    if caps.bootstrap_flag && !peers.is_empty() {
        cmd.arg("--bootstrap-peers").arg(peers.join(","));
    }

    let base_path = std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".to_string());
    let merged_path = format!("{}:{}", runtime.runtime_dir.display(), base_path);
    cmd.env("PATH", merged_path);
    cmd.env("EXO_RUNTIME_DIR", &runtime.runtime_dir);
    cmd.env("EXO_BUILD_ID", &runtime.build_id);
    // Always set both; the unused var is ignored by the binary that does not read it.
    cmd.env("EXO_LIBP2P_NAMESPACE", namespace);
    cmd.env("EXO_ZENOH_NAMESPACE", namespace);
    if !peers.is_empty() {
        cmd.env("EXO_BOOTSTRAP_PEERS", peers.join(","));
    }

    if let Some(ref zh_dir) = runtime.dashboard_zh_rel_home {
        cmd.env("EXO_DASHBOARD_DIR", zh_dir);
    }
    // ModelScope downloads land in ~/.exo/models first. Offline stops this
    // process from falling back to Hugging Face and sitting there.
    let offline_models = prefers_modelscope();
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
            remember_managed(
                child,
                sleep_guard,
                namespace,
                peers.clone(),
                caps.libp2p_port_flag,
                offline_models,
            );
            return Ok(());
        }
        thread::sleep(Duration::from_millis(350));
    }

    // Still starting on cold Gatekeeper scan; keep handle alive if child hasn't exited.
    remember_managed(
        child,
        sleep_guard,
        namespace,
        peers,
        caps.libp2p_port_flag,
        offline_models,
    );
    Ok(())
}

fn remember_managed(
    child: Child,
    sleep_guard: Option<Child>,
    namespace: &str,
    peers: Vec<String>,
    port_pinned: bool,
    offline_models: bool,
) {
    let _ = store_join(namespace, &peers, port_pinned);
    let mut guard = MANAGED_EXO.lock().unwrap_or_else(|e| e.into_inner());
    *guard = Some(ManagedExoProcess {
        child,
        sleep_guard,
        namespace: namespace.to_string(),
        bootstrap_peers: peers,
        port_pinned,
        offline_models,
    });
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

/// Restart the daemon we started so it matches the chosen download address.
/// A separately opened EXO window cannot be switched to the China source.
pub fn restart_to_match_model_source() -> Result<()> {
    let offline = prefers_modelscope();
    {
        let guard = MANAGED_EXO.lock().unwrap_or_else(|e| e.into_inner());
        if guard
            .as_ref()
            .is_some_and(|proc| proc.offline_models == offline)
        {
            return Ok(());
        }
    }
    let foreign = {
        let guard = MANAGED_EXO.lock().unwrap_or_else(|e| e.into_inner());
        guard.is_none() && local_exo_node_reachable() && !pid_file_matches_live_exo()
    };
    if foreign {
        if offline {
            bail!("这台电脑上的算力窗口是单独打开的，模型还会从 Hugging Face 下载。先关掉那个窗口，再点一次。");
        }
        return Ok(());
    }
    stop_exo_daemon()?;
    let deadline = Instant::now() + Duration::from_secs(5);
    while local_exo_node_reachable() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(200));
    }
    if local_exo_node_reachable() {
        if offline {
            bail!("算力窗口还开着，没法改用国内源。先关掉那个窗口，再点一次。");
        }
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

pub(crate) fn applied_join() -> AppliedJoin {
    {
        let guard = MANAGED_EXO.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(proc) = guard.as_ref() {
            return AppliedJoin {
                namespace: proc.namespace.clone(),
                peers: proc.bootstrap_peers.clone(),
                port_pinned: proc.port_pinned,
            };
        }
    }
    read_join_file().unwrap_or(AppliedJoin {
        namespace: String::new(),
        peers: Vec::new(),
        port_pinned: false,
    })
}

pub(crate) fn set_pending_bootstrap_peers(peers: &[String]) {
    *PENDING_BOOTSTRAP_PEERS
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = Some(peers.to_vec());
}

fn peers_for_spawn() -> Vec<String> {
    let pending = PENDING_BOOTSTRAP_PEERS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take();
    pending.unwrap_or_else(saved_bootstrap_peers)
}

pub(crate) fn store_join(namespace: &str, peers: &[String], port_pinned: bool) -> Result<()> {
    let path = join_file_path().context("无法定位用户目录")?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let body = json!({
        "namespace": namespace,
        "peers": peers,
        "port_pinned": port_pinned,
    });
    fs::write(&path, serde_json::to_string(&body)?)
        .with_context(|| format!("写入组网记录失败：{}", path.display()))
}

/// Stop our exo (including one left running after the app restarted) and start it again.
/// Leaves a separately opened EXO app alone. Returns false when nothing of ours is running.
///
/// exo 1.0.71 keeps placed models only in memory and does not persist its node id, so the
/// new process comes up empty. Read the ids from `/state` first and place them again
/// once our process is answering. A failed placement does not undo the restart.
pub(crate) fn restart_our_exo() -> Result<bool> {
    let owned = {
        let guard = MANAGED_EXO.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_some() {
            true
        } else if local_exo_node_reachable() {
            pid_file_matches_live_exo()
        } else {
            false
        }
    };
    if !owned {
        return Ok(false);
    }
    let models = models_placed_locally();
    stop_exo_daemon()?;
    let deadline = Instant::now() + Duration::from_secs(5);
    while local_exo_node_reachable() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(200));
    }
    if local_exo_node_reachable() {
        return Ok(false);
    }
    let Some(rt) = locate_exo_runtime() else {
        bail!("本机算力组件不在了，请重新开启。");
    };
    if start_exo_daemon(&rt)? == DaemonOwner::ManagedByUs {
        restore_models_after_restart(&models);
    }
    Ok(true)
}

/// Models exo has placed right now. Empty when the node is down or idle.
/// A miss here is the only chance: after `stop_exo_daemon` the instances are gone.
fn models_placed_locally() -> Vec<String> {
    probe_exo_origin(EXO_LOCAL_ORIGIN, Duration::from_secs(2))
        .map(|snap| snap.active_models)
        .unwrap_or_default()
        .into_iter()
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
        .collect()
}

fn restore_models_after_restart(model_ids: &[String]) {
    let ordered = models_in_restore_order(model_ids, chat_model_on_local_cluster().as_deref());
    if ordered.is_empty() || !local_exo_answers_within(Duration::from_secs(8)) {
        return;
    }
    for model_id in &ordered {
        if let Err(err) = prepare_model_and_activate(EXO_LOCAL_ORIGIN, model_id) {
            note_model_restore_failure(model_id, &err);
        }
    }
}

/// Every placed model goes back. The one chat is using goes last, because each
/// activation rewrites which model the local cluster answers with.
fn models_in_restore_order(placed: &[String], chat_model: Option<&str>) -> Vec<String> {
    let chat = chat_model.map(str::trim).filter(|id| !id.is_empty());
    let mut ordered = Vec::with_capacity(placed.len());
    let mut chat_hit = None;
    for raw in placed {
        let id = raw.trim();
        if id.is_empty() {
            continue;
        }
        if chat == Some(id) {
            chat_hit = Some(id.to_string());
            continue;
        }
        if ordered.iter().any(|existing| existing == id) {
            continue;
        }
        ordered.push(id.to_string());
    }
    if let Some(id) = chat_hit {
        ordered.push(id);
    }
    ordered
}

fn chat_model_on_local_cluster() -> Option<String> {
    let active = load_active_personal_provider().ok().flatten()?;
    if active.id != LOCAL_CLUSTER_PROVIDER_ID {
        return None;
    }
    let model = active.model.trim();
    if model.is_empty() {
        None
    } else {
        Some(model.to_string())
    }
}

fn local_exo_answers_within(limit: Duration) -> bool {
    let deadline = Instant::now() + limit;
    loop {
        if local_exo_node_reachable() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(200));
    }
}

fn note_model_restore_failure(model_id: &str, err: &anyhow::Error) {
    use std::io::Write;
    let Some(path) = exo_log_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) else {
        return;
    };
    let _ = writeln!(file, "重启后没能把模型重新放回去（{model_id}）：{err:#}");
}

fn saved_bootstrap_peers() -> Vec<String> {
    read_join_file().map(|join| join.peers).unwrap_or_default()
}

fn join_file_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| {
        h.join(".agent-doctor")
            .join("runtimes")
            .join("exo")
            .join("join.json")
    })
}

fn read_join_file() -> Option<AppliedJoin> {
    let path = join_file_path()?;
    let raw = fs::read_to_string(path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let namespace = value
        .get("namespace")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let peers = value
        .get("peers")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let port_pinned = value
        .get("port_pinned")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    Some(AppliedJoin {
        namespace,
        peers,
        port_pinned,
    })
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
    fn restore_puts_the_chat_model_last_and_drops_blanks() {
        let placed = vec![
            "  ".to_string(),
            "model-a".to_string(),
            "model-b".to_string(),
            "model-a".to_string(),
        ];
        assert_eq!(
            models_in_restore_order(&placed, Some("model-a")),
            vec!["model-b".to_string(), "model-a".to_string()]
        );
        assert_eq!(
            models_in_restore_order(&placed, Some("model-c")),
            vec!["model-a".to_string(), "model-b".to_string()]
        );
        assert_eq!(
            models_in_restore_order(&placed, None),
            vec!["model-a".to_string(), "model-b".to_string()]
        );
    }

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
