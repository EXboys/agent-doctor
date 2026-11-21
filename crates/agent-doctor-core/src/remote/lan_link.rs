//! Same-LAN discovery for computers that already have this client open.
//!
//! No address, account, or password. The other computer asks once; after
//! that, this computer can open its projects directly.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use tungstenite::{client::client as ws_client, Message};

use super::link_client::LinkClient;
use super::link_daemon::{dispatch_link_text, open_link_socket, serve_link_socket};
use super::link_protocol::{LinkHello, LinkResponse, LINK_DAEMON_PORT};
use super::registry::remote_root_dir;

const DISCOVERY_PORT: u16 = 18766;
const LINK_PORT: u16 = 18767;
const SEEN_FOR: Duration = Duration::from_secs(8);
const ASK_FOR: Duration = Duration::from_secs(45);

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Beacon {
    v: u8,
    id: String,
    name: String,
    port: u16,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NearbyComputer {
    pub id: String,
    pub name: String,
    pub addr: String,
    pub port: u16,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NearbyAsk {
    pub id: String,
    pub name: String,
}

struct SeenPeer {
    name: String,
    addr: IpAddr,
    port: u16,
    seen: Instant,
}

struct AskWait {
    id: String,
    name: String,
    slot: Arc<(Mutex<Option<bool>>, Condvar)>,
}

struct Gate {
    trusted: Vec<String>,
    allowed_ips: HashMap<IpAddr, Instant>,
    asks: Vec<AskWait>,
}

static GATE: Mutex<Option<Gate>> = Mutex::new(None);
static PEERS: Mutex<Option<HashMap<String, SeenPeer>>> = Mutex::new(None);
static SELF_ID: Mutex<Option<String>> = Mutex::new(None);
static STARTED: AtomicBool = AtomicBool::new(false);
static SHARING: AtomicBool = AtomicBool::new(false);

pub fn this_computer_name() -> String {
    computer_name()
}

fn share_path() -> Option<std::path::PathBuf> {
    remote_root_dir().map(|dir| dir.join("lan-sharing"))
}

fn load_sharing() -> bool {
    share_path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .is_some_and(|text| text.trim() == "1")
}

/// Whether this computer is visible to others on the same network.
/// Missing preference means closed.
pub fn nearby_sharing() -> bool {
    SHARING.load(Ordering::SeqCst)
}

pub fn set_nearby_sharing(enabled: bool) {
    SHARING.store(enabled, Ordering::SeqCst);
    let Some(path) = share_path() else {
        return;
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, if enabled { "1" } else { "0" });
}

fn computer_name() -> String {
    hostname::get()
        .ok()
        .and_then(|name| name.into_string().ok())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| "这台电脑".into())
}

fn self_id() -> String {
    if let Ok(guard) = SELF_ID.lock() {
        if let Some(id) = guard.as_ref() {
            return id.clone();
        }
    }
    let fresh = load_or_create_id();
    if let Ok(mut guard) = SELF_ID.lock() {
        if guard.is_none() {
            *guard = Some(fresh.clone());
        }
        return guard.clone().unwrap_or(fresh);
    }
    fresh
}

fn id_path() -> Option<std::path::PathBuf> {
    remote_root_dir().map(|dir| dir.join("lan-id"))
}

fn trust_path() -> Option<std::path::PathBuf> {
    remote_root_dir().map(|dir| dir.join("lan-trusted.json"))
}

fn load_or_create_id() -> String {
    if let Some(path) = id_path() {
        if let Ok(text) = std::fs::read_to_string(&path) {
            let id = text.trim().to_string();
            if !id.is_empty() {
                return id;
            }
        }
        let id = uuid::Uuid::new_v4().to_string();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(path, &id);
        return id;
    }
    uuid::Uuid::new_v4().to_string()
}

fn load_trusted() -> Vec<String> {
    let Some(path) = trust_path() else {
        return Vec::new();
    };
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    serde_json::from_str::<Vec<String>>(&text).unwrap_or_default()
}

fn save_trusted(ids: &[String]) {
    let Some(path) = trust_path() else {
        return;
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(text) = serde_json::to_string(ids) {
        let _ = std::fs::write(path, text);
    }
}

fn with_gate<T>(f: impl FnOnce(&mut Gate) -> T) -> Option<T> {
    let mut guard = GATE.lock().ok()?;
    let gate = guard.get_or_insert_with(|| Gate {
        trusted: load_trusted(),
        allowed_ips: HashMap::new(),
        asks: Vec::new(),
    });
    Some(f(gate))
}

pub fn spawn_lan_link() {
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    SHARING.store(load_sharing(), Ordering::SeqCst);
    let listener = match TcpListener::bind(("0.0.0.0", LINK_PORT)) {
        Ok(listener) => listener,
        Err(err) => {
            STARTED.store(false, Ordering::SeqCst);
            eprintln!("[lan-link] listen failed: {err}");
            return;
        }
    };
    let socket = match UdpSocket::bind(("0.0.0.0", DISCOVERY_PORT)) {
        Ok(socket) => socket,
        Err(err) => {
            STARTED.store(false, Ordering::SeqCst);
            eprintln!("[lan-link] discovery failed: {err}");
            return;
        }
    };
    let _ = socket.set_broadcast(true);
    let _ = socket.set_read_timeout(Some(Duration::from_millis(400)));
    let sender = match socket.try_clone() {
        Ok(sender) => sender,
        Err(err) => {
            eprintln!("[lan-link] discovery clone failed: {err}");
            return;
        }
    };
    let _ = thread::Builder::new()
        .name("ad-lan-listen".into())
        .spawn(move || accept_links(listener));
    let _ = thread::Builder::new()
        .name("ad-lan-beacon".into())
        .spawn(move || announce(sender));
    let _ = thread::Builder::new()
        .name("ad-lan-scan".into())
        .spawn(move || collect(socket));
}

fn announce(socket: UdpSocket) {
    let dest = SocketAddr::from((Ipv4Addr::BROADCAST, DISCOVERY_PORT));
    loop {
        if nearby_sharing() {
            let beacon = Beacon {
                v: 1,
                id: self_id(),
                name: computer_name(),
                port: LINK_PORT,
            };
            if let Ok(bytes) = serde_json::to_vec(&beacon) {
                let _ = socket.send_to(&bytes, dest);
            }
        }
        thread::sleep(Duration::from_secs(2));
    }
}

fn collect(socket: UdpSocket) {
    let mut buf = [0u8; 1024];
    loop {
        let Ok((n, from)) = socket.recv_from(&mut buf) else {
            continue;
        };
        let Ok(beacon) = serde_json::from_slice::<Beacon>(&buf[..n]) else {
            continue;
        };
        if beacon.v != 1 || beacon.id.is_empty() || beacon.id == self_id() {
            continue;
        }
        if let Ok(mut guard) = PEERS.lock() {
            let peers = guard.get_or_insert_with(HashMap::new);
            peers.insert(
                beacon.id,
                SeenPeer {
                    name: beacon.name,
                    addr: from.ip(),
                    port: beacon.port,
                    seen: Instant::now(),
                },
            );
        }
    }
}

pub fn list_nearby() -> Vec<NearbyComputer> {
    let Ok(guard) = PEERS.lock() else {
        return Vec::new();
    };
    let Some(peers) = guard.as_ref() else {
        return Vec::new();
    };
    let now = Instant::now();
    let mut rows: Vec<NearbyComputer> = peers
        .iter()
        .filter(|(_, peer)| now.duration_since(peer.seen) < SEEN_FOR)
        .map(|(id, peer)| NearbyComputer {
            id: id.clone(),
            name: if peer.name.trim().is_empty() {
                "这台电脑".into()
            } else {
                peer.name.clone()
            },
            addr: peer.addr.to_string(),
            port: peer.port,
        })
        .collect();
    rows.sort_by(|a, b| a.name.cmp(&b.name));
    rows
}

pub fn pending_asks() -> Vec<NearbyAsk> {
    with_gate(|gate| {
        gate.asks
            .iter()
            .map(|ask| NearbyAsk {
                id: ask.id.clone(),
                name: ask.name.clone(),
            })
            .collect()
    })
    .unwrap_or_default()
}

pub fn decide_ask(id: &str, allow: bool) {
    let Some(slot) = with_gate(|gate| {
        let index = gate.asks.iter().position(|ask| ask.id == id);
        let slot = index.map(|index| gate.asks.remove(index).slot);
        if allow && slot.is_some() && !gate.trusted.iter().any(|saved| saved == id) {
            gate.trusted.push(id.to_string());
            save_trusted(&gate.trusted);
        }
        slot
    }) else {
        return;
    };
    if let Some(slot) = slot {
        if let Ok(mut decision) = slot.0.lock() {
            *decision = Some(allow);
            slot.1.notify_all();
        }
    }
}

fn accept_links(listener: TcpListener) {
    for stream in listener.incoming() {
        let Ok(stream) = stream else {
            thread::sleep(Duration::from_millis(200));
            continue;
        };
        let ip = stream.peer_addr().ok().map(|addr| addr.ip());
        let _ = stream.set_nodelay(true);
        thread::spawn(move || {
            if let Err(err) = handle_lan_client(stream, ip) {
                eprintln!("[lan-link] client ended: {err:#}");
            }
        });
    }
}

fn handle_lan_client(stream: TcpStream, ip: Option<IpAddr>) -> Result<()> {
    let socket = open_link_socket(stream)?;
    let Some(text) = socket.next_text()? else {
        return Ok(());
    };
    if !nearby_sharing() {
        return refuse_closed(&socket, &text);
    }
    if let Some((id, name)) = lan_hello(&text) {
        if !is_trusted(&id) && !wait_for_allow(&id, &name) {
            let _ = socket.send_response(&LinkResponse {
                id: hello_id(&text),
                ok: false,
                result: None,
                error: Some("那边还没有允许。".into()),
            });
            return Ok(());
        }
        if !nearby_sharing() {
            return refuse_closed(&socket, &text);
        }
        if let Some(ip) = ip {
            allow_ip(ip);
        }
        socket.send_response(&LinkResponse {
            id: hello_id(&text),
            ok: true,
            result: Some(serde_json::to_value(local_hello())?),
            error: None,
        })?;
        serve_link_socket(&socket);
        return Ok(());
    }
    let allowed = ip.map(ip_allowed).unwrap_or(false);
    if !allowed {
        let _ = socket.send_response(&LinkResponse {
            id: hello_id(&text),
            ok: false,
            result: None,
            error: Some("这台电脑还没有允许。".into()),
        });
        return Ok(());
    }
    dispatch_link_text(&socket, &text)?;
    serve_link_socket(&socket);
    Ok(())
}

fn refuse_closed(socket: &super::link_daemon::LinkSocket, text: &str) -> Result<()> {
    let _ = socket.send_response(&LinkResponse {
        id: hello_id(text),
        ok: false,
        result: None,
        error: Some("这台电脑还没有开放。".into()),
    });
    Ok(())
}

fn local_hello() -> LinkHello {
    LinkHello {
        computer_name: computer_name(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        daemon_port: LINK_DAEMON_PORT,
    }
}

fn lan_hello(text: &str) -> Option<(String, String)> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    if value.get("method")?.as_str()? != "lan.hello" {
        return None;
    }
    let params = value.get("params")?;
    let id = params.get("id")?.as_str()?.trim().to_string();
    if id.is_empty() {
        return None;
    }
    let name = params
        .get("name")
        .and_then(|name| name.as_str())
        .unwrap_or("旁边的电脑")
        .to_string();
    Some((id, name))
}

fn hello_id(text: &str) -> String {
    serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .and_then(|value| value.get("id")?.as_str().map(str::to_string))
        .unwrap_or_else(|| "1".into())
}

fn is_trusted(id: &str) -> bool {
    with_gate(|gate| gate.trusted.iter().any(|saved| saved == id)).unwrap_or(false)
}

fn allow_ip(ip: IpAddr) {
    let _ = with_gate(|gate| {
        gate.allowed_ips.insert(ip, Instant::now());
    });
}

fn ip_allowed(ip: IpAddr) -> bool {
    with_gate(|gate| {
        gate.allowed_ips.get(&ip).is_some_and(|seen| {
            Instant::now().duration_since(*seen) < Duration::from_secs(24 * 60 * 60)
        })
    })
    .unwrap_or(false)
}

fn wait_for_allow(id: &str, name: &str) -> bool {
    let slot = Arc::new((Mutex::new(None), Condvar::new()));
    let registered = with_gate(|gate| {
        gate.asks.push(AskWait {
            id: id.to_string(),
            name: name.to_string(),
            slot: slot.clone(),
        });
    });
    if registered.is_none() {
        return false;
    }
    let allowed = {
        let (lock, cv) = &*slot;
        let mut guard = lock.lock().unwrap_or_else(|err| err.into_inner());
        let started = Instant::now();
        while guard.is_none() {
            let left = ASK_FOR.saturating_sub(started.elapsed());
            if left.is_zero() {
                break;
            }
            let (next, timeout) = cv
                .wait_timeout(guard, left)
                .unwrap_or_else(|err| err.into_inner());
            guard = next;
            if timeout.timed_out() {
                break;
            }
        }
        guard.unwrap_or(false)
    };
    let _ = with_gate(|gate| {
        gate.asks.retain(|ask| ask.id != id);
    });
    allowed
}

/// Open the other computer and keep that same connection for later work.
pub fn connect_nearby(id: &str) -> Result<(LinkClient, LinkHello)> {
    let peer = list_nearby()
        .into_iter()
        .find(|peer| peer.id == id)
        .context("这台电脑刚才还在，现在看不到了。请让那边的客户端开着。")?;
    greet(&peer)
}

fn greet(peer: &NearbyComputer) -> Result<(LinkClient, LinkHello)> {
    let addr = format!("{}:{}", peer.addr, peer.port);
    let stream = TcpStream::connect(addr.as_str()).context("连不上旁边这台电脑")?;
    stream.set_nodelay(true).ok();
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok();
    let (mut ws, _) = ws_client(format!("ws://{addr}/ws"), stream).context("连上了，但握手失败")?;
    let body = serde_json::json!({
        "id": "1",
        "method": "lan.hello",
        "params": { "id": self_id(), "name": computer_name() }
    });
    ws.send(Message::Text(body.to_string().into()))
        .context("没能发出连接请求")?;
    let started = Instant::now();
    loop {
        if started.elapsed() > ASK_FOR + Duration::from_secs(5) {
            bail!("那边还没有允许。");
        }
        let msg = match ws.read() {
            Ok(msg) => msg,
            Err(tungstenite::Error::Io(err))
                if matches!(
                    err.kind(),
                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                ) =>
            {
                continue;
            }
            Err(err) => return Err(err).context("等待那边允许时断开了"),
        };
        let Message::Text(text) = msg else {
            if matches!(msg, Message::Close(_)) {
                bail!("那边把连接关掉了");
            }
            continue;
        };
        if let Ok(response) = serde_json::from_str::<LinkResponse>(&text) {
            if response.id != "1" {
                continue;
            }
            if !response.ok {
                bail!(
                    "{}",
                    response.error.unwrap_or_else(|| "那边还没有允许。".into())
                );
            }
            let hello: LinkHello = serde_json::from_value(response.result.unwrap_or_default())
                .context("那边的回应格式不对")?;
            let _ = ws.get_mut().set_read_timeout(None);
            return Ok((LinkClient::from_websocket(ws), hello));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::lan_hello;

    #[test]
    fn reads_a_nearby_hello() {
        let text = r#"{"id":"1","method":"lan.hello","params":{"id":"abc","name":"Mac"}}"#;
        assert_eq!(
            lan_hello(text),
            Some(("abc".to_string(), "Mac".to_string()))
        );
        assert!(lan_hello(r#"{"id":"1","method":"ping"}"#).is_none());
    }
}
