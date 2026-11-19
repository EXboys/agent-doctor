//! Automatic LAN `/24` scanner and manual IP normalizer for Windows / consumer devices.

use std::collections::BTreeSet;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpStream, UdpSocket};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

use agent_doctor_core::exec::run_output;

use crate::exo::orchestrator::probe_exo_origin;
use crate::exo::EXO_DEFAULT_PORT;
use crate::ClusterBackendKind;

const SCAN_CONCURRENCY: usize = 32;
const TCP_PROBE_TIMEOUT: Duration = Duration::from_millis(320);
const HTTP_PROBE_TIMEOUT: Duration = Duration::from_millis(850);

/// Discovered cluster snapshots: matched device labels paired with the candidate.
type FoundCandidates = Vec<(BTreeSet<String>, LanClusterCandidate)>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LanClusterCandidate {
    pub backend: ClusterBackendKind,
    /// Origin without `/v1`, e.g. `http://192.168.1.23:52415`.
    pub origin: String,
    pub host: String,
    pub node_count: usize,
    pub device_labels: Vec<String>,
    pub usable_memory_bytes: u64,
    pub active_models: Vec<String>,
}

/// Normalize a user-typed host or URL (`192.168.1.23`, `http://192.168.1.23:52415/v1`)
/// into a clean origin (`http://192.168.1.23:52415`).
pub fn normalize_cluster_origin(raw: &str, default_port: u16) -> Result<String> {
    let trimmed = raw.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        bail!("请输入算力电脑的局域网地址（例如 192.168.1.23）。");
    }
    let with_scheme = if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        trimmed.to_string()
    } else {
        format!("http://{trimmed}")
    };
    let parsed = url::Url::parse(&with_scheme)
        .map_err(|_| anyhow::anyhow!("局域网地址格式不正确，请填写如 192.168.1.23 的 IP 地址。"))?;
    let Some(host) = parsed.host_str() else {
        bail!("未识别到有效的主机地址。");
    };
    let port = parsed.port().unwrap_or(default_port);
    Ok(format!("{}://{}:{}", parsed.scheme(), host, port))
}

/// Scan active local IPv4 subnets for hosts with `port` open.
/// `include_link_local` covers a Thunderbolt bridge (`169.254.x.x`).
pub fn scan_lan_tcp_port(port: u16, include_link_local: bool) -> Vec<String> {
    let local_ips = discover_local_ipv4s(include_link_local);
    let mut targets = Vec::new();
    let mut seen_subnets = BTreeSet::new();

    for ip in &local_ips {
        let octets = ip.octets();
        let prefix = [octets[0], octets[1], octets[2]];
        if !seen_subnets.insert(prefix) {
            continue;
        }
        for host_byte in 1..=254_u8 {
            let candidate = Ipv4Addr::new(prefix[0], prefix[1], prefix[2], host_byte);
            if !local_ips.contains(&candidate) {
                targets.push(candidate);
            }
        }
    }

    if targets.is_empty() {
        return Vec::new();
    }

    let queue = Arc::new(Mutex::new(targets));
    let found: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let worker_count = SCAN_CONCURRENCY.min(queue.lock().unwrap().len().max(1));
    let mut handles = Vec::with_capacity(worker_count);

    for _ in 0..worker_count {
        let q = Arc::clone(&queue);
        let out = Arc::clone(&found);
        handles.push(thread::spawn(move || loop {
            let next_ip = {
                let mut guard = q.lock().unwrap_or_else(|e| e.into_inner());
                guard.pop()
            };
            let Some(ip) = next_ip else {
                break;
            };
            let addr = SocketAddr::V4(SocketAddrV4::new(ip, port));
            if TcpStream::connect_timeout(&addr, TCP_PROBE_TIMEOUT).is_err() {
                continue;
            }
            let mut guard = out.lock().unwrap_or_else(|e| e.into_inner());
            let endpoint = format!("{ip}:{port}");
            if !guard.iter().any(|existing| existing == &endpoint) {
                guard.push(endpoint);
            }
        }));
    }

    for handle in handles {
        let _ = handle.join();
    }

    let mut list = found.lock().unwrap_or_else(|e| e.into_inner()).clone();
    list.sort();
    list
}

/// Scan active local IPv4 `/24` subnets for reachable cluster nodes (`:52415` exo).
pub fn scan_lan_clusters() -> Vec<LanClusterCandidate> {
    let local_ips = discover_local_ipv4s(false);
    let mut targets = Vec::new();
    let mut seen_subnets = BTreeSet::new();

    for ip in &local_ips {
        let octets = ip.octets();
        let prefix = [octets[0], octets[1], octets[2]];
        if !seen_subnets.insert(prefix) {
            continue;
        }
        for host_byte in 1..=254_u8 {
            let candidate = Ipv4Addr::new(prefix[0], prefix[1], prefix[2], host_byte);
            if !local_ips.contains(&candidate) {
                targets.push(candidate);
            }
        }
    }

    if targets.is_empty() {
        return Vec::new();
    }

    let queue = Arc::new(Mutex::new(targets));
    let found: Arc<Mutex<FoundCandidates>> = Arc::new(Mutex::new(Vec::new()));

    let worker_count = SCAN_CONCURRENCY.min(queue.lock().unwrap().len().max(1));
    let mut handles = Vec::with_capacity(worker_count);

    for _ in 0..worker_count {
        let q = Arc::clone(&queue);
        let out = Arc::clone(&found);
        handles.push(thread::spawn(move || loop {
            let next_ip = {
                let mut guard = q.lock().unwrap_or_else(|e| e.into_inner());
                guard.pop()
            };
            let Some(ip) = next_ip else {
                break;
            };
            let addr = SocketAddr::V4(SocketAddrV4::new(ip, EXO_DEFAULT_PORT));
            if TcpStream::connect_timeout(&addr, TCP_PROBE_TIMEOUT).is_err() {
                continue;
            }
            let origin = format!("http://{ip}:{EXO_DEFAULT_PORT}");
            let Ok(snap) = probe_exo_origin(&origin, HTTP_PROBE_TIMEOUT) else {
                continue;
            };
            let mut signature: BTreeSet<String> =
                snap.nodes.iter().map(|n| n.node_id.clone()).collect();
            signature.insert(snap.self_node_id.clone());

            let candidate = LanClusterCandidate {
                backend: ClusterBackendKind::Exo,
                origin,
                host: ip.to_string(),
                node_count: snap.nodes.len().max(1),
                device_labels: snap.nodes.iter().map(|n| n.label.clone()).collect(),
                usable_memory_bytes: snap.usable_memory_bytes,
                active_models: snap.active_models,
            };

            let mut guard = out.lock().unwrap_or_else(|e| e.into_inner());
            // Deduplicate multi-node exo clusters where several Macs respond on :52415.
            if !guard.iter().any(|(sig, _)| !sig.is_disjoint(&signature)) {
                guard.push((signature, candidate));
            }
        }));
    }

    for h in handles {
        let _ = h.join();
    }

    let mut list: Vec<LanClusterCandidate> = found
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .drain(..)
        .map(|(_, c)| c)
        .collect();
    list.sort_by(|a, b| b.node_count.cmp(&a.node_count).then(a.host.cmp(&b.host)));
    list
}

fn discover_local_ipv4s(include_link_local: bool) -> Vec<Ipv4Addr> {
    let mut ips = BTreeSet::new();

    // Fast primary route check via non-transmitting UDP bind/connect.
    for probe_target in ["223.5.5.5:80", "192.168.1.1:80", "10.0.0.1:80"] {
        if let Ok(sock) = UdpSocket::bind("0.0.0.0:0") {
            if sock.connect(probe_target).is_ok() {
                if let Ok(SocketAddr::V4(local)) = sock.local_addr() {
                    let ip = *local.ip();
                    if is_scannable_lan_ipv4(ip, include_link_local) {
                        ips.insert(ip);
                    }
                }
            }
        }
    }

    // Also parse active secondary NICs (e.g. Ethernet + Wi-Fi + Thunderbolt) without C FFI crates.
    #[cfg(target_os = "windows")]
    {
        if let Ok(out) = run_output(Path::new("ipconfig"), &[], Duration::from_secs(2)) {
            for ip in parse_ipv4_tokens(&String::from_utf8_lossy(&out.stdout)) {
                if is_scannable_lan_ipv4(ip, include_link_local) {
                    ips.insert(ip);
                }
            }
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        if let Ok(out) = run_output(Path::new("/sbin/ifconfig"), &[], Duration::from_secs(2)) {
            for ip in parse_ipv4_tokens(&String::from_utf8_lossy(&out.stdout)) {
                if is_scannable_lan_ipv4(ip, include_link_local) {
                    ips.insert(ip);
                }
            }
        }
    }

    let limit = if include_link_local { 4 } else { 3 };
    ips.into_iter().take(limit).collect()
}

fn parse_ipv4_tokens(text: &str) -> Vec<Ipv4Addr> {
    text.split(|c: char| !c.is_ascii_digit() && c != '.')
        .filter(|tok| tok.matches('.').count() == 3)
        .filter_map(|tok| tok.parse::<Ipv4Addr>().ok())
        .collect()
}

fn is_scannable_lan_ipv4(ip: Ipv4Addr, include_link_local: bool) -> bool {
    let [a, b, _, _] = ip.octets();
    if ip.is_loopback() || ip.is_unspecified() {
        return false;
    }
    if ip.is_link_local() {
        return include_link_local;
    }
    // Skip common Docker / WSL default bridge 172.17.0.x unless nothing else exists.
    if a == 172 && b == 17 {
        return false;
    }
    a == 192 && b == 168 || a == 10 || (a == 172 && (16..=31).contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_bare_ip_and_full_v1_url_to_origin() {
        assert_eq!(
            normalize_cluster_origin("192.168.1.23", 52415).unwrap(),
            "http://192.168.1.23:52415"
        );
        assert_eq!(
            normalize_cluster_origin("http://10.0.0.8:52415/v1/", 52415).unwrap(),
            "http://10.0.0.8:52415"
        );
    }

    #[test]
    fn link_local_is_only_scanned_for_direct_cables() {
        let thunderbolt = Ipv4Addr::new(169, 254, 10, 20);
        let wifi = Ipv4Addr::new(192, 168, 1, 8);
        assert!(!is_scannable_lan_ipv4(thunderbolt, false));
        assert!(is_scannable_lan_ipv4(thunderbolt, true));
        assert!(is_scannable_lan_ipv4(wifi, false));
    }
}
