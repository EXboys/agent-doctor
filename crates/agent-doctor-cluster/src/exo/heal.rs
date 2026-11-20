//! When this Mac is alone, join the other Macs on the same account without asking.
//!
//! exo finds peers with mDNS. macOS often drops that (local-network privacy, guest
//! Wi-Fi, a Thunderbolt cable that is not the mDNS interface). The pinned runtime
//! can also dial a fixed port. This module finds those computers and restarts our
//! process onto the account namespace with those addresses.

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpStream};
#[cfg(target_os = "macos")]
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::Result;

#[cfg(target_os = "macos")]
use agent_doctor_core::exec::run_output;

use crate::exo::daemon::{self, NodeHostSupport};
use crate::exo::orchestrator::probe_exo_origin;
use crate::exo::{EXO_DEFAULT_PORT, EXO_LIBP2P_PORT, EXO_LOCAL_ORIGIN};
use crate::lan_scan;
#[cfg(any(target_os = "macos", test))]
use crate::lan_scan::is_scannable_lan_ipv4;
use crate::namespace::resolve_cluster_namespace;

const MAX_BOOTSTRAP_PEERS: usize = 8;
const HEAL_GAP: Duration = Duration::from_secs(15);

struct HealGate {
    running: bool,
    last_finished: Option<Instant>,
}

static HEAL_GATE: Mutex<HealGate> = Mutex::new(HealGate {
    running: false,
    last_finished: None,
});

/// Align the running process with the signed-in account and dial any other
/// Mac already serving compute. No-op when this computer is not hosting,
/// already linked, or a heal just ran.
pub fn heal_lonely_exo() -> Result<()> {
    if daemon::detect_host_support() != NodeHostSupport::Supported {
        return Ok(());
    }
    if !daemon::local_exo_node_reachable() {
        return Ok(());
    }

    {
        let mut gate = HEAL_GATE.lock().unwrap_or_else(|e| e.into_inner());
        if gate.running {
            return Ok(());
        }
        if gate.last_finished.is_some_and(|at| at.elapsed() < HEAL_GAP) {
            return Ok(());
        }
        gate.running = true;
    }

    let outcome = heal_once();

    let mut gate = HEAL_GATE.lock().unwrap_or_else(|e| e.into_inner());
    gate.running = false;
    gate.last_finished = Some(Instant::now());
    outcome
}

fn heal_once() -> Result<()> {
    let desired = resolve_cluster_namespace();
    let applied = daemon::applied_join();
    let snapshot = probe_exo_origin(EXO_LOCAL_ORIGIN, Duration::from_millis(750)).ok();
    let alone = snapshot
        .as_ref()
        .map(|snap| snap.nodes.len() < 2)
        .unwrap_or(true);
    // Already linked on the right account: leave the running process alone.
    if !alone && applied.namespace == desired.namespace {
        return Ok(());
    }

    let found_hosts = if alone {
        discover_peer_hosts()
    } else {
        Vec::new()
    };
    let self_ips = lan_scan::local_scannable_ipv4s(true)
        .into_iter()
        .map(|ip| ip.to_string())
        .collect::<Vec<_>>();
    let Some(peers) = plan_rejoin(
        &applied.namespace,
        &desired.namespace,
        &applied.peers,
        &found_hosts,
        &self_ips,
        applied.port_pinned,
        daemon::runtime_supports_pinned_libp2p(),
    ) else {
        return Ok(());
    };

    daemon::set_pending_bootstrap_peers(&peers);
    daemon::restart_our_exo()?;
    Ok(())
}

/// Two linked Macs with Thunderbolt Bridge on both sides form a loop. macOS
/// will not let an ordinary app change that, so the elected computer asks the
/// system once. The other computer leaves its bridge on.
pub fn close_thunderbolt_loop_if_ours(self_id: String, node_ids: Vec<String>) {
    if !this_mac_closes_the_loop(&self_id, &node_ids) {
        return;
    }
    #[cfg(target_os = "macos")]
    spawn_close_thunderbolt_bridge();
}

/// The computer with the greater id turns its own bridge off. Both sides
/// compute the same answer, so the bridge stays up on one of them.
pub(crate) fn this_mac_closes_the_loop(self_id: &str, node_ids: &[String]) -> bool {
    let self_id = self_id.trim();
    if self_id.is_empty() {
        return false;
    }
    let mut ids: Vec<&str> = node_ids
        .iter()
        .map(|id| id.trim())
        .filter(|id| !id.is_empty())
        .collect();
    if !ids.contains(&self_id) {
        ids.push(self_id);
    }
    ids.sort();
    ids.dedup();
    ids.len() >= 2 && ids.last().copied() == Some(self_id)
}

pub(crate) fn enabled_thunderbolt_bridge(listing: &str) -> Option<String> {
    for line in listing.lines() {
        let Some((disabled, name)) = network_service_line(line) else {
            continue;
        };
        if !disabled && is_thunderbolt_bridge(&name) {
            return Some(name);
        }
    }
    None
}

fn network_service_line(line: &str) -> Option<(bool, String)> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with("An asterisk") {
        return None;
    }
    let disabled = trimmed.starts_with('*');
    let name = trimmed.trim_start_matches('*').trim();
    if name.is_empty() {
        return None;
    }
    Some((disabled, name.to_string()))
}

fn is_thunderbolt_bridge(name: &str) -> bool {
    name.eq_ignore_ascii_case("thunderbolt bridge")
        || name.contains("雷雳桥")
        || name.contains("雷电桥")
        || name.contains("雷雳网桥")
}

#[cfg(target_os = "macos")]
fn spawn_close_thunderbolt_bridge() {
    {
        let mut gate = BRIDGE_GATE.lock().unwrap_or_else(|err| err.into_inner());
        match gate.state {
            BridgeState::Prompting | BridgeState::Closed => return,
            BridgeState::WaitUntil(at) if at > Instant::now() => return,
            BridgeState::Idle | BridgeState::WaitUntil(_) => gate.state = BridgeState::Prompting,
        }
    }
    if std::thread::Builder::new()
        .name("tb-bridge".into())
        .spawn(finish_thunderbolt_bridge)
        .is_err()
    {
        let mut gate = BRIDGE_GATE.lock().unwrap_or_else(|err| err.into_inner());
        gate.state = BridgeState::Idle;
    }
}

#[cfg(target_os = "macos")]
fn finish_thunderbolt_bridge() {
    let closed = close_enabled_thunderbolt_bridge();
    let mut gate = BRIDGE_GATE.lock().unwrap_or_else(|err| err.into_inner());
    gate.state = if closed {
        BridgeState::Closed
    } else {
        BridgeState::WaitUntil(Instant::now() + Duration::from_secs(10 * 60))
    };
}

#[cfg(target_os = "macos")]
fn close_enabled_thunderbolt_bridge() -> bool {
    // raw-spawn-ok: macOS system tool, only compiled on this platform
    let Ok(output) = std::process::Command::new("/usr/sbin/networksetup")
        .arg("-listallnetworkservices")
        .output()
    else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    let listing = String::from_utf8_lossy(&output.stdout);
    let Some(service) = enabled_thunderbolt_bridge(&listing) else {
        return true;
    };
    let script = format!(
        "do shell script \"/usr/sbin/networksetup -setnetworkserviceenabled \" & quoted form of {service} & \" off\" with administrator privileges",
        service = apple_script_string(&service),
    );
    // raw-spawn-ok: macOS asks for the login password in its own dialog
    std::process::Command::new("/usr/bin/osascript")
        .args(["-e", &script])
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[cfg(target_os = "macos")]
fn apple_script_string(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg(target_os = "macos")]
enum BridgeState {
    Idle,
    Prompting,
    Closed,
    WaitUntil(Instant),
}

#[cfg(target_os = "macos")]
struct BridgeGate {
    state: BridgeState,
}

#[cfg(target_os = "macos")]
static BRIDGE_GATE: Mutex<BridgeGate> = Mutex::new(BridgeGate {
    state: BridgeState::Idle,
});

fn discover_peer_hosts() -> Vec<String> {
    let mut hosts: Vec<String> = lan_scan::scan_lan_tcp_port(EXO_DEFAULT_PORT, true)
        .into_iter()
        .filter_map(|endpoint| endpoint.split(':').next().map(|ip| ip.to_string()))
        .collect();

    for ip in arp_neighbor_ipv4s() {
        let host = ip.to_string();
        if hosts.iter().any(|existing| existing == &host) {
            continue;
        }
        if !tcp_open(ip, EXO_DEFAULT_PORT) {
            continue;
        }
        hosts.push(host);
    }

    hosts.retain(|host| exo_api_alive(host));
    hosts.sort();
    hosts.dedup();
    hosts
}

fn tcp_open(ip: Ipv4Addr, port: u16) -> bool {
    let addr = SocketAddr::V4(SocketAddrV4::new(ip, port));
    TcpStream::connect_timeout(&addr, Duration::from_millis(320)).is_ok()
}

fn exo_api_alive(host: &str) -> bool {
    let origin = format!("http://{host}:{EXO_DEFAULT_PORT}");
    probe_exo_origin(&origin, Duration::from_millis(700)).is_ok()
}

fn arp_neighbor_ipv4s() -> Vec<Ipv4Addr> {
    #[cfg(target_os = "macos")]
    {
        let Ok(out) = run_output(Path::new("/usr/sbin/arp"), &["-an"], Duration::from_secs(2))
        else {
            return Vec::new();
        };
        parse_arp_ipv4s(&String::from_utf8_lossy(&out.stdout))
    }
    #[cfg(not(target_os = "macos"))]
    {
        Vec::new()
    }
}

/// Decide whether to restart, and with which dial addresses.
///
/// `None` means the running process is already on the right namespace and
/// already knows every computer we just found.
pub(crate) fn plan_rejoin(
    running_namespace: &str,
    desired_namespace: &str,
    running_peers: &[String],
    found_hosts: &[String],
    self_ips: &[String],
    port_pinned: bool,
    can_pin_port: bool,
) -> Option<Vec<String>> {
    let mut merged = Vec::new();
    for peer in bootstrap_multiaddrs(found_hosts)
        .into_iter()
        .chain(running_peers.iter().cloned())
    {
        if references_self(&peer, self_ips) {
            continue;
        }
        if merged.iter().any(|existing| existing == &peer) {
            continue;
        }
        merged.push(peer);
    }
    merged.sort_by(|a, b| thunderbolt_rank(a).cmp(&thunderbolt_rank(b)).then(a.cmp(b)));
    merged.truncate(MAX_BOOTSTRAP_PEERS);

    let need_pin = can_pin_port && !port_pinned;
    let namespace_changed = running_namespace != desired_namespace;
    let found_new = merged.iter().any(|peer| !running_peers.contains(peer));
    if !need_pin && !namespace_changed && !found_new {
        return None;
    }
    Some(merged)
}

pub(crate) fn bootstrap_multiaddrs(hosts: &[String]) -> Vec<String> {
    let mut peers = Vec::new();
    for host in hosts {
        let ip = host.split(':').next().unwrap_or("").trim();
        if ip.is_empty() {
            continue;
        }
        let peer = format!("/ip4/{ip}/tcp/{EXO_LIBP2P_PORT}");
        if !peers.iter().any(|existing| existing == &peer) {
            peers.push(peer);
        }
    }
    peers
}

fn references_self(multiaddr: &str, self_ips: &[String]) -> bool {
    self_ips
        .iter()
        .any(|ip| !ip.is_empty() && multiaddr.contains(&format!("/ip4/{ip}/")))
}

fn thunderbolt_rank(multiaddr: &str) -> u8 {
    if multiaddr.contains("/ip4/169.254.") {
        0
    } else {
        1
    }
}

#[cfg(any(target_os = "macos", test))]
pub(crate) fn parse_arp_ipv4s(text: &str) -> Vec<Ipv4Addr> {
    let mut ips = Vec::new();
    for line in text.lines() {
        if line.contains("incomplete") {
            continue;
        }
        let Some(start) = line.find('(') else {
            continue;
        };
        let Some(end) = line[start + 1..].find(')') else {
            continue;
        };
        let token = &line[start + 1..start + 1 + end];
        let Ok(ip) = token.parse::<Ipv4Addr>() else {
            continue;
        };
        if !is_scannable_lan_ipv4(ip, true) {
            continue;
        }
        if !ips.contains(&ip) {
            ips.push(ip);
        }
    }
    ips
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejoin_adds_a_newly_found_computer_and_prefers_thunderbolt() {
        let peers = plan_rejoin(
            "ad-acct-same",
            "ad-acct-same",
            &["/ip4/192.168.1.9/tcp/52416".to_string()],
            &["192.168.1.20".to_string(), "169.254.10.8".to_string()],
            &["192.168.1.4".to_string()],
            true,
            true,
        )
        .expect("new computers should be dialed");
        assert_eq!(
            peers,
            vec![
                "/ip4/169.254.10.8/tcp/52416".to_string(),
                "/ip4/192.168.1.20/tcp/52416".to_string(),
                "/ip4/192.168.1.9/tcp/52416".to_string(),
            ]
        );
    }

    #[test]
    fn rejoin_skips_when_nothing_changed() {
        let peers = vec!["/ip4/192.168.1.9/tcp/52416".to_string()];
        assert!(plan_rejoin(
            "ad-acct-same",
            "ad-acct-same",
            &peers,
            &["192.168.1.9".to_string()],
            &[],
            true,
            true,
        )
        .is_none());
    }

    #[test]
    fn rejoin_switches_off_a_stale_code_onto_the_account() {
        let peers = plan_rejoin("ad-code-1111", "ad-acct-same", &[], &[], &[], true, true)
            .expect("login should restart onto the account");
        assert!(peers.is_empty());
    }

    #[test]
    fn rejoin_drops_this_computer_and_pins_the_port_once() {
        let peers = plan_rejoin(
            "ad-acct-same",
            "ad-acct-same",
            &["/ip4/192.168.1.4/tcp/52416".to_string()],
            &["192.168.1.4".to_string(), "192.168.1.9".to_string()],
            &["192.168.1.4".to_string()],
            false,
            true,
        )
        .expect("unpinned port should restart");
        assert_eq!(peers, vec!["/ip4/192.168.1.9/tcp/52416".to_string()]);
    }

    #[test]
    fn the_later_computer_is_the_one_that_closes_the_loop() {
        let nodes = vec!["aa".to_string(), "zz".to_string()];
        assert!(this_mac_closes_the_loop("zz", &nodes));
        assert!(!this_mac_closes_the_loop("aa", &nodes));
        assert!(!this_mac_closes_the_loop("zz", &["zz".to_string()]));
        assert!(!this_mac_closes_the_loop("  ", &nodes));
    }

    #[test]
    fn an_enabled_thunderbolt_bridge_is_the_one_we_can_turn_off() {
        let listing = "\
An asterisk (*) denotes that a network service is disabled.
Wi-Fi
*Thunderbolt Bridge
Ethernet
";
        assert!(enabled_thunderbolt_bridge(listing).is_none());
        let listing = "\
An asterisk (*) denotes that a network service is disabled.
Wi-Fi
Thunderbolt Bridge
";
        assert_eq!(
            enabled_thunderbolt_bridge(listing).as_deref(),
            Some("Thunderbolt Bridge")
        );
        let listing = "雷雳网桥\n";
        assert_eq!(
            enabled_thunderbolt_bridge(listing).as_deref(),
            Some("雷雳网桥")
        );
    }

    #[test]
    fn arp_table_keeps_thunderbolt_neighbors() {
        let text = "\
? (192.168.1.1) at 11:22:33:44:55:66 on en0 ifscope [ethernet]
? (169.254.83.12) at aa:bb:cc:dd:ee:ff on bridge0 ifscope [bridge]
? (192.168.1.23) at (incomplete) on en0 ifscope [ethernet]
? (127.0.0.1) at 127.0.0.1 on lo0
";
        assert_eq!(
            parse_arp_ipv4s(text),
            vec![
                Ipv4Addr::new(192, 168, 1, 1),
                Ipv4Addr::new(169, 254, 83, 12),
            ]
        );
    }
}
