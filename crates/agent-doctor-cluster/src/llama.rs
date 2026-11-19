//! llama.cpp runtime for machines that cannot host the current exo build.
//!
//! One computer runs `llama-server` and calls the others' `llama-rpc-server`
//! over the LAN (including a Thunderbolt bridge). The chat API stays on this computer.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use serde::Deserialize;

use agent_doctor_core::exec::{command_for_path, run_output};
use agent_doctor_core::lifecycle::{use_china_mirrors, GITHUB_MIRROR_PREFIX};
use agent_doctor_core::setup::upsert_local_cluster_provider;

use crate::exo::daemon::DaemonOwner;
use crate::namespace::{self, saved_llama_peer_hosts, set_saved_llama_peer_hosts};
use crate::tiers::{self, ClusterModelTier};

const PINNED_LLAMA_CPP: &str = "b12532";
const API_PORT: u16 = 52416;
pub const LLAMA_RPC_PORT: u16 = 50052;
/// Tiny status port so another computer can read this machine's name and free memory.
pub const LLAMA_PEER_INFO_PORT: u16 = 50053;
const API_ORIGIN: &str = "http://127.0.0.1:52416";

static CANCEL_INSTALL: AtomicBool = AtomicBool::new(false);

struct ManagedLlama {
    rpc: Option<Child>,
    server: Option<Child>,
    sleep_guard: Option<Child>,
    model_id: Option<String>,
}

static MANAGED: Mutex<Option<ManagedLlama>> = Mutex::new(None);
static PEER_INFO_STOP: AtomicBool = AtomicBool::new(false);
static PEER_INFO_RUNNING: AtomicBool = AtomicBool::new(false);
static USABLE_CACHE: Mutex<Option<(Instant, u64)>> = Mutex::new(None);

pub fn cancel_llama_install() {
    CANCEL_INSTALL.store(true, Ordering::SeqCst);
}

fn clear_llama_cancel() {
    CANCEL_INSTALL.store(false, Ordering::SeqCst);
}

pub fn llama_api_origin() -> &'static str {
    API_ORIGIN
}

pub fn release_asset_name() -> Option<&'static str> {
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Some("llama-b12532-bin-macos-arm64.tar.gz")
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        Some("llama-b12532-bin-macos-x64.tar.gz")
    } else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        Some("llama-b12532-bin-win-vulkan-x64.zip")
    } else if cfg!(all(target_os = "windows", target_arch = "aarch64")) {
        Some("llama-b12532-bin-win-vulkan-arm64.zip")
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some("llama-b12532-bin-ubuntu-x64.tar.gz")
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        Some("llama-b12532-bin-ubuntu-arm64.tar.gz")
    } else {
        None
    }
}

pub fn llama_tiers(usable_bytes: u64) -> Vec<ClusterModelTier> {
    let gib = 1024 * 1024 * 1024;
    // Names shown in the card live in the desktop translations. Sizes are the
    // Q4_K_M files on ModelScope, rounded to the download the user will see.
    let specs = [
        (
            "qwen25-1.5b",
            "Qwen/Qwen2.5-1.5B-Instruct-GGUF:Q4_K_M",
            (gib * 11) / 10,
            3 * gib,
        ),
        (
            "qwen25-3b",
            "Qwen/Qwen2.5-3B-Instruct-GGUF:Q4_K_M",
            (gib * 20) / 10,
            4 * gib,
        ),
        (
            "qwen25-7b",
            "Qwen/Qwen2.5-7B-Instruct-GGUF:Q4_K_M",
            (gib * 44) / 10,
            6 * gib,
        ),
        (
            "qwen25-coder-7b",
            "Qwen/Qwen2.5-Coder-7B-Instruct-GGUF:Q4_K_M",
            (gib * 44) / 10,
            6 * gib,
        ),
        (
            "deepseek-r1-7b",
            "unsloth/DeepSeek-R1-Distill-Qwen-7B-GGUF:Q4_K_M",
            (gib * 44) / 10,
            6 * gib,
        ),
        (
            "qwen3-8b",
            "Qwen/Qwen3-8B-GGUF:Q4_K_M",
            (gib * 47) / 10,
            7 * gib,
        ),
        (
            "qwen25-14b",
            "Qwen/Qwen2.5-14B-Instruct-GGUF:Q4_K_M",
            (gib * 84) / 10,
            14 * gib,
        ),
        (
            "qwen3-14b",
            "Qwen/Qwen3-14B-GGUF:Q4_K_M",
            (gib * 84) / 10,
            14 * gib,
        ),
        (
            "qwen3-30b",
            "Qwen/Qwen3-30B-A3B-GGUF:Q4_K_M",
            (gib * 173) / 10,
            20 * gib,
        ),
        (
            "qwen25-32b",
            "Qwen/Qwen2.5-32B-Instruct-GGUF:Q4_K_M",
            (gib * 185) / 10,
            22 * gib,
        ),
        (
            "qwen25-72b",
            "Qwen/Qwen2.5-72B-Instruct-GGUF:Q4_K_M",
            41 * gib,
            48 * gib,
        ),
    ];
    let recommended_id = specs
        .iter()
        .rev()
        .find(|(_, _, _, min)| usable_bytes >= *min)
        .map(|(id, _, _, _)| *id)
        .unwrap_or("qwen25-7b");
    specs
        .into_iter()
        .map(
            |(id, model_id, size_bytes, min_usable_bytes)| ClusterModelTier {
                id: id.to_string(),
                model_id: model_id.to_string(),
                min_usable_bytes,
                fits_cluster: usable_bytes == 0 || usable_bytes >= min_usable_bytes,
                recommended: id == recommended_id,
                size_bytes,
                on_disk: crate::modelscope::llama_weights_ready(model_id),
            },
        )
        .collect()
}

pub fn local_usable_memory_bytes() -> u64 {
    let total = local_total_memory_bytes();
    total.saturating_sub(tiers::SYSTEM_MEMORY_HEADROOM_BYTES)
}

/// This computer plus every saved peer that answers with a real memory number.
/// Unknown local memory stays unknown, so a missing reading does not unlock every model.
pub fn cluster_usable_memory_bytes() -> u64 {
    let now = Instant::now();
    if let Some((at, bytes)) = *USABLE_CACHE.lock().unwrap_or_else(|e| e.into_inner()) {
        if now.duration_since(at) < Duration::from_secs(15) {
            return bytes;
        }
    }
    let bytes = uncached_cluster_usable();
    *USABLE_CACHE.lock().unwrap_or_else(|e| e.into_inner()) = Some((now, bytes));
    bytes
}

fn uncached_cluster_usable() -> u64 {
    let local = local_usable_memory_bytes();
    if local == 0 {
        return 0;
    }
    let Ok(peers) = parse_peer_hosts(&saved_peer_text()) else {
        return local;
    };
    let extra: u64 = peers
        .iter()
        .filter_map(|endpoint| read_peer_info(endpoint).map(|info| info.usable))
        .filter(|usable| *usable > 0)
        .sum();
    local.saturating_add(extra)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PeerInfo {
    name: String,
    usable: u64,
}

fn read_peer_info(endpoint: &str) -> Option<PeerInfo> {
    let host = endpoint.split(':').next().unwrap_or("").trim();
    if host.is_empty() {
        return None;
    }
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_millis(250))
        .timeout(Duration::from_millis(400))
        .build()
        .ok()?;
    let text = client
        .get(format!("http://{host}:{LLAMA_PEER_INFO_PORT}/"))
        .send()
        .ok()?
        .text()
        .ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    let name = value.get("name")?.as_str()?.trim().to_string();
    let usable = value.get("usable")?.as_u64()?;
    if name.is_empty() {
        return None;
    }
    Some(PeerInfo { name, usable })
}

/// One address per computer. Thunderbolt (`169.254.`) wins when the same name shows up twice.
pub fn collapse_peer_hosts(found: Vec<(String, Option<String>)>) -> Vec<String> {
    let mut named: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut unnamed = Vec::new();
    for (host, name) in found {
        let host = host.split(':').next().unwrap_or("").trim().to_string();
        if host.is_empty() {
            continue;
        }
        match name {
            Some(name) if !name.is_empty() => named.entry(name).or_default().push(host),
            _ => unnamed.push(host),
        }
    }
    let mut out: Vec<String> = named
        .into_values()
        .map(|hosts| {
            hosts
                .iter()
                .find(|host| host.starts_with("169.254."))
                .cloned()
                .unwrap_or_else(|| hosts[0].clone())
        })
        .collect();
    out.extend(unnamed);
    out.sort();
    out.dedup();
    out
}

pub fn scan_llama_peer_hosts() -> Vec<String> {
    let found = crate::lan_scan::scan_lan_tcp_port(LLAMA_RPC_PORT, true);
    let paired = found
        .into_iter()
        .map(|endpoint| {
            let name = read_peer_info(&endpoint).map(|info| info.name);
            (endpoint, name)
        })
        .collect();
    collapse_peer_hosts(paired)
}

fn local_total_memory_bytes() -> u64 {
    #[cfg(target_os = "macos")]
    {
        let Ok(out) = run_output(
            Path::new("/usr/sbin/sysctl"),
            &["-n", "hw.memsize"],
            Duration::from_secs(2),
        ) else {
            return 0;
        };
        String::from_utf8_lossy(&out.stdout)
            .trim()
            .parse()
            .unwrap_or(0)
    }
    #[cfg(target_os = "linux")]
    {
        let Ok(raw) = fs::read_to_string("/proc/meminfo") else {
            return 0;
        };
        for line in raw.lines() {
            if let Some(rest) = line.strip_prefix("MemTotal:") {
                let kb: u64 = rest
                    .split_whitespace()
                    .next()
                    .unwrap_or("0")
                    .parse()
                    .unwrap_or(0);
                return kb.saturating_mul(1024);
            }
        }
        0
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        0
    }
}

/// Split a beginner-typed list into `host:50052` endpoints. Empty input is fine.
pub fn parse_peer_hosts(raw: &str) -> Result<Vec<String>> {
    let mut peers = Vec::new();
    for part in raw.split(|c: char| {
        c == ',' || c == '，' || c == ';' || c == '；' || c == '\n' || c.is_whitespace()
    }) {
        let part = part.trim().trim_end_matches('/');
        if part.is_empty() {
            continue;
        }
        let part = part
            .trim_start_matches("http://")
            .trim_start_matches("https://");
        if part.contains('/') {
            bail!("地址写成 192.168.1.20 这样，多台用逗号分开。");
        }
        let endpoint = if part.contains(':') {
            part.to_string()
        } else {
            format!("{part}:{LLAMA_RPC_PORT}")
        };
        if !endpoint
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | ':' | '-' | '[' | ']'))
        {
            bail!("地址写成 192.168.1.20 这样，多台用逗号分开。");
        }
        if !peers.iter().any(|existing| existing == &endpoint) {
            peers.push(endpoint);
        }
    }
    Ok(peers)
}

pub fn saved_peer_text() -> String {
    saved_llama_peer_hosts().unwrap_or_default()
}

pub fn runtime_installed() -> bool {
    locate_server().is_some()
}

pub fn server_reachable() -> bool {
    port_open(API_PORT)
}

pub fn current_owner() -> DaemonOwner {
    let mut guard = MANAGED.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(proc) = guard.as_mut() {
        let rpc_dead = proc
            .rpc
            .as_mut()
            .and_then(|child| child.try_wait().ok())
            .is_some_and(|status| status.is_some());
        let server_dead = proc
            .server
            .as_mut()
            .and_then(|child| child.try_wait().ok())
            .is_some_and(|status| status.is_some());
        if proc.rpc.is_some() && !rpc_dead || proc.server.is_some() && !server_dead {
            return DaemonOwner::ManagedByUs;
        }
        *guard = None;
    }
    if port_open(API_PORT) || port_open(LLAMA_RPC_PORT) {
        DaemonOwner::ExternalProcess
    } else {
        DaemonOwner::NotRunning
    }
}

pub fn running_model_id() -> Option<String> {
    let guard = MANAGED.lock().unwrap_or_else(|e| e.into_inner());
    guard.as_ref().and_then(|proc| proc.model_id.clone())
}

pub fn install_and_start_rpc<F>(mut on_progress: F) -> Result<()>
where
    F: FnMut(u64, Option<u64>, &str),
{
    if locate_server().is_none() {
        install_runtime(&mut on_progress)?;
    }
    start_rpc()
}

pub fn set_peer_text(raw: &str) -> Result<()> {
    let _ = parse_peer_hosts(raw)?;
    *USABLE_CACHE.lock().unwrap_or_else(|e| e.into_inner()) = None;
    set_saved_llama_peer_hosts(raw.trim())
}

pub fn activate_model<F>(model_id: &str, on_progress: &mut F) -> Result<()>
where
    F: FnMut(u64, Option<u64>, &str),
{
    let model_id = model_id.trim();
    if model_id.is_empty() {
        bail!("先选一个模型。");
    }
    let peers = parse_peer_hosts(&saved_peer_text())?;
    let server = locate_server().context("请先下载并开启 llama.cpp。")?;
    clear_llama_cancel();
    let source = if use_china_mirrors() {
        let weights = crate::modelscope::ensure_llama_gguf(model_id, on_progress, &|| {
            CANCEL_INSTALL.load(Ordering::SeqCst)
        })?;
        ModelSource::File(weights)
    } else {
        ModelSource::Hub(model_id.to_string())
    };
    start_rpc()?;
    start_model_server(&server, &source, &peers)?;
    let served = served_model_id().unwrap_or_else(|| model_id.to_string());
    upsert_local_cluster_provider(&format!("{API_ORIGIN}/v1"), &served, true)?;
    namespace::set_saved_preferred_cluster_model(model_id)?;
    if let Some(proc) = MANAGED.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        proc.model_id = Some(model_id.to_string());
    }
    Ok(())
}

pub fn stop() {
    stop_peer_info();
    let mut guard = MANAGED.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(mut proc) = guard.take() {
        stop_child(proc.server.as_mut());
        stop_child(proc.rpc.as_mut());
        if let Some(mut sleep_guard) = proc.sleep_guard.take() {
            let _ = sleep_guard.kill();
            let _ = sleep_guard.wait();
        }
    }
    reap_pid_file("llama-rpc.pid");
    reap_pid_file("llama-server.pid");
}

fn install_runtime<F>(on_progress: &mut F) -> Result<PathBuf>
where
    F: FnMut(u64, Option<u64>, &str),
{
    let asset = release_asset_name().context("这台电脑没有对应的 llama.cpp 安装包。")?;
    let root = runtime_root().context("无法定位本机用户主目录")?;
    fs::create_dir_all(root.parent().unwrap_or(&root))?;
    let staging = root
        .parent()
        .unwrap_or(&root)
        .join(format!(".llama-staging-{}", std::process::id()));
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&staging)?;
    CANCEL_INSTALL.store(false, Ordering::SeqCst);

    let archive = staging.join(asset);
    let urls = download_urls(asset);
    let mut last_err = None;
    for url in &urls {
        match download_to_file(url, &archive, on_progress) {
            Ok(()) => {
                last_err = None;
                break;
            }
            Err(err) => {
                let _ = fs::remove_file(&archive);
                if CANCEL_INSTALL.load(Ordering::SeqCst) {
                    let _ = fs::remove_dir_all(&staging);
                    return Err(err);
                }
                last_err = Some(err);
            }
        }
    }
    if let Some(err) = last_err {
        let _ = fs::remove_dir_all(&staging);
        return Err(err);
    }

    on_progress(0, None, "extracting");
    extract_archive(&archive, &staging)?;
    let _ = fs::remove_file(&archive);
    if root.exists() {
        let _ = fs::remove_dir_all(&root);
    }
    fs::rename(&staging, &root).context("安装 llama.cpp 失败")?;
    locate_server().context("llama.cpp 已下载，但没有找到可运行的程序。")
}

fn download_urls(asset: &str) -> Vec<String> {
    let official = format!(
        "https://github.com/ggml-org/llama.cpp/releases/download/{PINNED_LLAMA_CPP}/{asset}"
    );
    if use_china_mirrors() {
        vec![
            format!(
                "{GITHUB_MIRROR_PREFIX}ggml-org/llama.cpp/releases/download/{PINNED_LLAMA_CPP}/{asset}"
            ),
            official,
        ]
    } else {
        vec![official]
    }
}

fn download_to_file<F>(url: &str, dest: &Path, on_progress: &mut F) -> Result<()>
where
    F: FnMut(u64, Option<u64>, &str),
{
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(1800))
        .build()
        .context("创建下载客户端失败")?;
    let mut resp = client
        .get(url)
        .send()
        .with_context(|| "连接 llama.cpp 下载源失败")?;
    if !resp.status().is_success() {
        bail!(
            "llama.cpp 下载失败（HTTP {}）。请检查网络后再试。",
            resp.status().as_u16()
        );
    }
    let total = resp.content_length();
    let mut file = File::create(dest).context("无法保存 llama.cpp 安装包")?;
    let mut downloaded = 0u64;
    let mut buf = [0u8; 64 * 1024];
    on_progress(0, total, "downloading");
    loop {
        if CANCEL_INSTALL.load(Ordering::SeqCst) {
            bail!("已取消下载。");
        }
        let n = resp.read(&mut buf).context("下载中断，请再试一次。")?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).context("写入安装包失败")?;
        downloaded += n as u64;
        on_progress(downloaded, total, "downloading");
    }
    file.flush()?;
    Ok(())
}

fn extract_archive(archive: &Path, dest: &Path) -> Result<()> {
    let name = archive.file_name().and_then(|s| s.to_str()).unwrap_or("");
    let mut cmd = Command::new("tar");
    if name.ends_with(".zip") {
        cmd.arg("-xf");
    } else {
        cmd.arg("-xzf");
    }
    let output = cmd
        .arg(archive)
        .arg("-C")
        .arg(dest)
        .output()
        .context("无法解压 llama.cpp。")?;
    if !output.status.success() {
        bail!("解压 llama.cpp 失败。");
    }
    Ok(())
}

fn start_rpc() -> Result<()> {
    start_peer_info();
    let Some(rpc_bin) = locate_rpc() else {
        return Ok(());
    };
    if port_open(LLAMA_RPC_PORT) {
        return Ok(());
    }
    reap_pid_file("llama-rpc.pid");
    let log = open_log()?;
    let log_err = log.try_clone().context("无法写 llama.cpp 日志")?;
    let mut cmd = command_for_path(&rpc_bin);
    cmd.current_dir(rpc_bin.parent().unwrap_or(Path::new(".")))
        .args(["--host", "0.0.0.0", "--port", &LLAMA_RPC_PORT.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(log_err));
    let mut child = cmd.spawn().context("llama.cpp 没有启动起来。")?;
    write_pid("llama-rpc.pid", child.id());
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        if let Ok(Some(_)) = child.try_wait() {
            bail!("llama.cpp 启动后马上退出了。{}", log_tail());
        }
        if port_open(LLAMA_RPC_PORT) {
            let sleep_guard = spawn_sleep_guard(child.id());
            let mut guard = MANAGED.lock().unwrap_or_else(|e| e.into_inner());
            let slot = guard.get_or_insert_with(|| ManagedLlama {
                rpc: None,
                server: None,
                sleep_guard: None,
                model_id: None,
            });
            slot.rpc = Some(child);
            if slot.sleep_guard.is_none() {
                slot.sleep_guard = sleep_guard;
            }
            return Ok(());
        }
        thread::sleep(Duration::from_millis(300));
    }
    let mut guard = MANAGED.lock().unwrap_or_else(|e| e.into_inner());
    let slot = guard.get_or_insert_with(|| ManagedLlama {
        rpc: None,
        server: None,
        sleep_guard: None,
        model_id: None,
    });
    slot.rpc = Some(child);
    Ok(())
}

enum ModelSource {
    Hub(String),
    File(PathBuf),
}

fn start_model_server(server_bin: &Path, source: &ModelSource, peers: &[String]) -> Result<()> {
    if let Some(proc) = MANAGED.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        stop_child(proc.server.as_mut());
        proc.server = None;
        proc.model_id = None;
    }
    reap_pid_file("llama-server.pid");
    let log = open_log()?;
    let log_err = log.try_clone().context("无法写 llama.cpp 日志")?;
    let mut cmd = command_for_path(server_bin);
    cmd.current_dir(server_bin.parent().unwrap_or(Path::new(".")))
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg(API_PORT.to_string())
        .arg("-ngl")
        .arg("999")
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(log_err));
    match source {
        ModelSource::Hub(id) => {
            cmd.arg("-hf").arg(id);
        }
        ModelSource::File(path) => {
            cmd.arg("-m").arg(path);
        }
    }
    if !peers.is_empty() {
        cmd.arg("--rpc").arg(peers.join(","));
    }
    let mut child = cmd.spawn().context("模型没有启动起来。")?;
    write_pid("llama-server.pid", child.id());
    let deadline = Instant::now() + Duration::from_secs(30 * 60);
    while Instant::now() < deadline {
        if let Ok(Some(_)) = child.try_wait() {
            bail!("模型没有加载起来。{}", log_tail());
        }
        if server_reachable() {
            let mut guard = MANAGED.lock().unwrap_or_else(|e| e.into_inner());
            let slot = guard.get_or_insert_with(|| ManagedLlama {
                rpc: None,
                server: None,
                sleep_guard: None,
                model_id: None,
            });
            slot.server = Some(child);
            slot.model_id = Some(match source {
                ModelSource::Hub(id) => id.clone(),
                ModelSource::File(path) => path.display().to_string(),
            });
            return Ok(());
        }
        thread::sleep(Duration::from_secs(2));
    }
    stop_child(Some(&mut child));
    bail!("模型还没准备好，超过 30 分钟了。可以再点一次。")
}

fn served_model_id() -> Option<String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(4))
        .build()
        .ok()?;
    let resp = client.get(format!("{API_ORIGIN}/v1/models")).send().ok()?;
    let body: ModelsResponse = resp.json().ok()?;
    body.data.into_iter().find_map(|item| item.id)
}

#[derive(Deserialize)]
struct ModelsResponse {
    data: Vec<ModelItem>,
}

#[derive(Deserialize)]
struct ModelItem {
    id: Option<String>,
}

fn locate_server() -> Option<PathBuf> {
    let root = runtime_root()?;
    find_named(&root, &["llama-server", "llama-server.exe"])
}

fn locate_rpc() -> Option<PathBuf> {
    let root = runtime_root()?;
    find_named(
        &root,
        &[
            "llama-rpc-server",
            "llama-rpc-server.exe",
            "ggml-rpc-server",
            "ggml-rpc-server.exe",
            "rpc-server",
            "rpc-server.exe",
        ],
    )
}

fn find_named(root: &Path, names: &[&str]) -> Option<PathBuf> {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let file_name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
            if names.contains(&file_name) {
                return Some(path);
            }
        }
    }
    None
}

fn runtime_root() -> Option<PathBuf> {
    dirs::home_dir().map(|home| {
        home.join(".agent-doctor")
            .join("runtimes")
            .join("llama.cpp")
            .join(PINNED_LLAMA_CPP)
    })
}

fn port_open(port: u16) -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    TcpStream::connect_timeout(&addr, Duration::from_millis(250)).is_ok()
}

fn log_path() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".agent-doctor").join("logs").join("llama.log"))
}

fn open_log() -> Result<File> {
    let path = log_path().context("无法定位日志目录")?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    File::options()
        .create(true)
        .append(true)
        .open(&path)
        .context("无法写 llama.cpp 日志")
}

fn log_tail() -> String {
    let Some(path) = log_path() else {
        return String::new();
    };
    let Ok(raw) = fs::read_to_string(path) else {
        return String::new();
    };
    let tail: Vec<&str> = raw.lines().rev().take(4).collect();
    let text = tail.into_iter().rev().collect::<Vec<_>>().join(" ");
    if text.trim().is_empty() {
        String::new()
    } else {
        format!(" {text}")
    }
}

fn pid_path(name: &str) -> Option<PathBuf> {
    dirs::home_dir().map(|home| {
        home.join(".agent-doctor")
            .join("runtimes")
            .join("llama.cpp")
            .join(name)
    })
}

fn write_pid(name: &str, pid: u32) {
    if let Some(path) = pid_path(name) {
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let _ = fs::write(path, pid.to_string());
    }
}

fn reap_pid_file(name: &str) {
    let Some(path) = pid_path(name) else {
        return;
    };
    let Ok(raw) = fs::read_to_string(&path) else {
        return;
    };
    if let Ok(pid) = raw.trim().parse::<u32>() {
        #[cfg(unix)]
        {
            let _ = run_output(
                Path::new("/bin/kill"),
                &["-TERM", &pid.to_string()],
                Duration::from_secs(2),
            );
        }
        #[cfg(windows)]
        {
            let _ = Command::new("taskkill")
                .args(["/PID", &pid.to_string(), "/T", "/F"])
                .output();
        }
    }
    let _ = fs::remove_file(path);
}

fn stop_child(child: Option<&mut Child>) {
    let Some(child) = child else {
        return;
    };
    let _ = child.kill();
    let _ = child.wait();
}

fn start_peer_info() {
    if PEER_INFO_RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    PEER_INFO_STOP.store(false, Ordering::SeqCst);
    thread::spawn(|| {
        let listener = match TcpListener::bind(("0.0.0.0", LLAMA_PEER_INFO_PORT)) {
            Ok(listener) => listener,
            Err(_) => {
                PEER_INFO_RUNNING.store(false, Ordering::SeqCst);
                return;
            }
        };
        listener.set_nonblocking(true).ok();
        while !PEER_INFO_STOP.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
                    let mut _buf = [0_u8; 512];
                    let _ = stream.read(&mut _buf);
                    let body = peer_info_body();
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.write_all(response.as_bytes());
                }
                Err(err) if err.kind() == ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(200));
                }
                Err(_) => break,
            }
        }
        PEER_INFO_RUNNING.store(false, Ordering::SeqCst);
    });
}

fn stop_peer_info() {
    PEER_INFO_STOP.store(true, Ordering::SeqCst);
}

fn peer_info_body() -> String {
    serde_json::json!({
        "name": machine_name(),
        "usable": local_usable_memory_bytes(),
    })
    .to_string()
}

fn machine_name() -> String {
    let output = Command::new("hostname").stdin(Stdio::null()).output();
    let name = output
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "computer".to_string());
    name.chars().take(64).collect()
}

fn spawn_sleep_guard(pid: u32) -> Option<Child> {
    #[cfg(target_os = "macos")]
    {
        Command::new("/usr/bin/caffeinate")
            .args(["-i", "-w", &pid.to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .ok()
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = pid;
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peer_list_accepts_plain_addresses_and_adds_the_port() {
        let peers = parse_peer_hosts("192.168.1.20，10.0.0.5:50052").unwrap();
        assert_eq!(peers, vec!["192.168.1.20:50052", "10.0.0.5:50052"]);
        assert!(parse_peer_hosts("").unwrap().is_empty());
        assert!(parse_peer_hosts("bad/host").is_err());
    }

    #[test]
    fn same_computer_on_wifi_and_thunderbolt_keeps_the_thunderbolt_address() {
        let chosen = collapse_peer_hosts(vec![
            ("192.168.1.8".into(), Some("mini".into())),
            ("169.254.10.2".into(), Some("mini".into())),
            ("192.168.1.9".into(), Some("book".into())),
        ]);
        assert_eq!(
            chosen,
            vec!["169.254.10.2".to_string(), "192.168.1.9".to_string()]
        );
    }

    #[test]
    fn two_sixteen_gig_macs_unlock_a_model_one_mac_cannot_run() {
        let gib = 1024 * 1024 * 1024;
        let alone = llama_tiers(12 * gib);
        let together = llama_tiers(24 * gib);
        assert!(alone
            .iter()
            .any(|tier| tier.id == "qwen25-14b" && !tier.fits_cluster));
        assert!(together
            .iter()
            .any(|tier| tier.id == "qwen25-14b" && tier.fits_cluster));
    }

    #[test]
    fn this_platform_has_a_release_asset() {
        assert!(release_asset_name().is_some());
    }

    #[test]
    fn sixteen_gig_mac_lists_named_models_and_hides_nothing() {
        let gib = 1024 * 1024 * 1024;
        let tiers = llama_tiers(12 * gib);
        assert!(tiers.len() > 3);
        assert!(tiers
            .iter()
            .any(|tier| tier.id == "qwen25-7b" && tier.fits_cluster));
        assert!(tiers
            .iter()
            .any(|tier| tier.id == "qwen25-72b" && !tier.fits_cluster));
        assert_eq!(
            tiers
                .iter()
                .find(|tier| tier.recommended)
                .map(|tier| tier.id.as_str()),
            Some("qwen3-8b")
        );
    }
}
