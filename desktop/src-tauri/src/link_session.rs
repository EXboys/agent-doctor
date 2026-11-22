//! Keeps SSH tunnels to linked computers alive for the desktop process.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use agent_doctor_core::{
    connect_nearby, list_nearby, load_remote_hosts, mark_host_linked, open_link_tunnel,
    open_nearby_client, LinkClient, LinkConnectFailure, LinkHello, LinkTunnel, NearbyEndpoint,
    RemoteHostEntry,
};
use serde::Serialize;

#[derive(Clone, Default)]
pub struct LinkSessionState {
    inner: Arc<Mutex<HashMap<String, LiveLink>>>,
}

struct LiveLink {
    tunnel: Option<LinkTunnel>,
    stop_proxy: Option<Arc<AtomicBool>>,
    hello: LinkHello,
    local_port: u16,
    /// Coordinates for opening fresh authenticated sockets per prompt/cancel/permission.
    lan_endpoint: Option<NearbyEndpoint>,
    /// Shared short-RPC socket so workspace/file reads stay fast.
    lan: Option<Arc<Mutex<LinkClient>>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkHostStatus {
    pub host_id: String,
    pub display_name: String,
    pub target: String,
    pub linked_client: bool,
    pub connected: bool,
    pub computer_name: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkConnectReport {
    pub host_id: String,
    pub ok: bool,
    pub connected: bool,
    pub computer_name: String,
    pub message: String,
    pub kind: String,
}

impl LinkSessionState {
    pub fn disconnect(&self, host_id: &str) {
        if let Ok(mut guard) = self.inner.lock() {
            if let Some(live) = guard.remove(host_id) {
                if let Some(tunnel) = live.tunnel {
                    tunnel.stop();
                }
                if let Some(stop) = live.stop_proxy {
                    stop.store(true, Ordering::SeqCst);
                }
            }
        }
    }

    #[allow(dead_code)]
    pub fn disconnect_all(&self) {
        if let Ok(mut guard) = self.inner.lock() {
            for (_, live) in guard.drain() {
                if let Some(tunnel) = live.tunnel {
                    tunnel.stop();
                }
                if let Some(stop) = live.stop_proxy {
                    stop.store(true, Ordering::SeqCst);
                }
            }
        }
    }

    #[allow(dead_code)]
    pub fn is_connected(&self, host_id: &str) -> bool {
        self.inner
            .lock()
            .map(|g| g.contains_key(host_id))
            .unwrap_or(false)
    }

    pub fn local_port(&self, host_id: &str) -> Option<u16> {
        self.inner
            .lock()
            .ok()
            .and_then(|g| g.get(host_id).map(|live| live.local_port))
    }

    pub fn connect(&self, host_id: &str) -> LinkConnectReport {
        self.disconnect(host_id);
        let doc = match load_remote_hosts() {
            Ok(doc) => doc,
            Err(err) => {
                return LinkConnectReport {
                    host_id: host_id.to_string(),
                    ok: false,
                    connected: false,
                    computer_name: String::new(),
                    message: format!("读不到已保存的电脑列表：{err:#}"),
                    kind: "other".into(),
                };
            }
        };
        let Some(entry) = doc.hosts.get(host_id) else {
            return LinkConnectReport {
                host_id: host_id.to_string(),
                ok: false,
                connected: false,
                computer_name: String::new(),
                message: "找不到这台电脑。".into(),
                kind: "other".into(),
            };
        };

        match open_link_tunnel(host_id, entry) {
            Ok((tunnel, hello)) => {
                let computer_name = hello.computer_name.clone();
                let _ = mark_host_linked(host_id, &computer_name, None);
                if let Ok(mut guard) = self.inner.lock() {
                    guard.insert(
                        host_id.to_string(),
                        LiveLink {
                            local_port: tunnel.local_port,
                            tunnel: Some(tunnel),
                            stop_proxy: None,
                            hello: hello.clone(),
                            lan_endpoint: None,
                            lan: None,
                        },
                    );
                }
                LinkConnectReport {
                    host_id: host_id.to_string(),
                    ok: true,
                    connected: true,
                    computer_name,
                    message: "已连上，可以操作那边的项目了。".into(),
                    kind: "ok".into(),
                }
            }
            Err(LinkConnectFailure::SshUnreachable { detail }) => LinkConnectReport {
                host_id: host_id.to_string(),
                ok: false,
                connected: false,
                computer_name: String::new(),
                message: if detail.trim().is_empty() {
                    "连不上这台电脑，请确认它开着机且网络能通。".into()
                } else {
                    format!("连不上这台电脑，请确认它开着机且网络能通。（{detail}）")
                },
                kind: "ssh".into(),
            },
            Err(LinkConnectFailure::ClientNotOpen { .. }) => LinkConnectReport {
                host_id: host_id.to_string(),
                ok: false,
                connected: false,
                computer_name: String::new(),
                message: "已经连上这台电脑，但那边的客户端还没打开。".into(),
                kind: "client".into(),
            },
            Err(LinkConnectFailure::Other { detail }) => LinkConnectReport {
                host_id: host_id.to_string(),
                ok: false,
                connected: false,
                computer_name: String::new(),
                message: detail,
                kind: "other".into(),
            },
        }
    }

    /// SSH to a public address without writing it into the saved server list.
    pub fn connect_direct(&self, host_id: &str, entry: &RemoteHostEntry) -> LinkConnectReport {
        self.disconnect(host_id);
        match open_link_tunnel(host_id, entry) {
            Ok((tunnel, hello)) => {
                let computer_name = hello.computer_name.clone();
                if let Ok(mut guard) = self.inner.lock() {
                    guard.insert(
                        host_id.to_string(),
                        LiveLink {
                            local_port: tunnel.local_port,
                            tunnel: Some(tunnel),
                            stop_proxy: None,
                            hello,
                            lan_endpoint: None,
                            lan: None,
                        },
                    );
                }
                LinkConnectReport {
                    host_id: host_id.to_string(),
                    ok: true,
                    connected: true,
                    computer_name,
                    message: "已连上，可以操作那边的项目了。".into(),
                    kind: "ok".into(),
                }
            }
            Err(LinkConnectFailure::SshUnreachable { .. }) => LinkConnectReport {
                host_id: host_id.to_string(),
                ok: false,
                connected: false,
                computer_name: String::new(),
                message: "连不上。请确认两边都登录了同一个账号，并在那台电脑打开系统的远程登录，再在路由器把外网的 22 转到那台电脑。".into(),
                kind: "ssh".into(),
            },
            Err(LinkConnectFailure::ClientNotOpen { .. }) => LinkConnectReport {
                host_id: host_id.to_string(),
                ok: false,
                connected: false,
                computer_name: String::new(),
                message: "已经连上那台电脑，但那边的客户端还没打开。".into(),
                kind: "client".into(),
            },
            Err(LinkConnectFailure::Other { detail }) => LinkConnectReport {
                host_id: host_id.to_string(),
                ok: false,
                connected: false,
                computer_name: String::new(),
                message: detail,
                kind: "other".into(),
            },
        }
    }

    pub fn connect_nearby(&self, peer_id: &str) -> LinkConnectReport {
        let host_id = format!("lan-{peer_id}");
        self.disconnect(&host_id);
        match connect_nearby(peer_id) {
            Ok((endpoint, client, hello)) => {
                let computer_name = hello.computer_name.clone();
                if let Ok(mut guard) = self.inner.lock() {
                    guard.insert(
                        host_id.clone(),
                        LiveLink {
                            local_port: 0,
                            tunnel: None,
                            stop_proxy: None,
                            hello,
                            lan_endpoint: Some(endpoint),
                            lan: Some(Arc::new(Mutex::new(client))),
                        },
                    );
                }
                LinkConnectReport {
                    host_id,
                    ok: true,
                    connected: true,
                    computer_name,
                    message: "已连上，可以操作那边的项目了。".into(),
                    kind: "ok".into(),
                }
            }
            Err(err) => LinkConnectReport {
                host_id,
                ok: false,
                connected: false,
                computer_name: String::new(),
                message: format!("{err:#}"),
                kind: "other".into(),
            },
        }
    }

    pub fn lan_endpoint(&self, host_id: &str) -> Option<NearbyEndpoint> {
        self.inner.lock().ok().and_then(|guard| {
            guard
                .get(host_id)
                .and_then(|live| live.lan_endpoint.clone())
        })
    }

    pub fn lan_client(&self, host_id: &str) -> Option<Arc<Mutex<LinkClient>>> {
        self.inner
            .lock()
            .ok()
            .and_then(|guard| guard.get(host_id).and_then(|live| live.lan.clone()))
    }

    /// Re-establish a nearby session automatically if this app restarted while
    /// the peer is still online and already trusts us.
    fn ensure_lan_session(&self, host_id: &str) -> Option<NearbyEndpoint> {
        if let Some(ep) = self.lan_endpoint(host_id) {
            return Some(ep);
        }
        let peer_id = host_id.strip_prefix("lan-")?.trim();
        if peer_id.is_empty() {
            return None;
        }
        let peer = list_nearby().into_iter().find(|p| p.id == peer_id)?;
        let endpoint = NearbyEndpoint {
            peer_id: peer.id,
            addr: peer.addr,
            port: peer.port,
        };
        let mut client = open_nearby_client(&endpoint).ok()?;
        let hello = client.ping().ok()?;
        if let Ok(mut guard) = self.inner.lock() {
            guard.insert(
                host_id.to_string(),
                LiveLink {
                    local_port: 0,
                    tunnel: None,
                    stop_proxy: None,
                    hello,
                    lan_endpoint: Some(endpoint.clone()),
                    lan: Some(Arc::new(Mutex::new(client))),
                },
            );
        }
        Some(endpoint)
    }

    /// Open a dedicated WebSocket client for an isolated operation such as
    /// `start_prompt`, `cancel_prompt`, or `resolve_permission`.
    pub fn open_fresh_client(&self, host_id: &str) -> Result<LinkClient, String> {
        if let Some(endpoint) = self.ensure_lan_session(host_id) {
            return open_nearby_client(&endpoint).map_err(|err| format!("{err:#}"));
        }
        if let Some(port) = self.local_port(host_id).filter(|port| *port != 0) {
            return LinkClient::connect_local(port).map_err(|err| format!("{err:#}"));
        }
        if host_id.starts_with("lan-") {
            Err("还没连上这台电脑。先在「客户端互联」里点「在这台电脑上用」。".into())
        } else {
            Err("还没连上这台电脑，请先点连接。".into())
        }
    }

    pub fn with_client<T>(
        &self,
        host_id: &str,
        mut f: impl FnMut(&mut LinkClient) -> anyhow::Result<T>,
    ) -> Result<T, String> {
        let _ = self.ensure_lan_session(host_id);
        if let Some(lan) = self.lan_client(host_id) {
            let mut client = lan.lock().unwrap_or_else(|err| err.into_inner());
            match f(&mut client) {
                Ok(val) => return Ok(val),
                Err(first_err) => {
                    if let Some(endpoint) = self.lan_endpoint(host_id) {
                        if let Ok(fresh) = open_nearby_client(&endpoint) {
                            *client = fresh;
                            return f(&mut client).map_err(|err| format!("{err:#}"));
                        }
                    }
                    return Err(format!("{first_err:#}"));
                }
            }
        }
        let port = self
            .local_port(host_id)
            .filter(|port| *port != 0)
            .ok_or_else(|| "还没连上这台电脑，请先点连接。".to_string())?;
        let mut client = LinkClient::connect_local(port).map_err(|err| format!("{err:#}"))?;
        f(&mut client).map_err(|err| format!("{err:#}"))
    }

    pub fn status_rows(&self) -> Vec<LinkHostStatus> {
        let doc = load_remote_hosts().unwrap_or_default();
        let connected = self.inner.lock().ok();
        let mut rows = Vec::new();
        for (host_id, entry) in &doc.hosts {
            let live = connected.as_ref().and_then(|g| g.get(host_id.as_str()));
            let display_name = if !entry.display_name.trim().is_empty() {
                entry.display_name.clone()
            } else {
                host_id.clone()
            };
            rows.push(LinkHostStatus {
                host_id: host_id.clone(),
                display_name,
                target: entry.display_target(),
                linked_client: entry.linked_client,
                connected: live.is_some(),
                computer_name: live
                    .map(|l| l.hello.computer_name.clone())
                    .unwrap_or_else(|| entry.linked_computer_name.clone()),
                message: if live.is_some() {
                    "已连接".into()
                } else if entry.linked_client {
                    "未连接".into()
                } else {
                    "可做检查；打开那边的客户端后可完整操作".into()
                },
            });
        }
        if let Some(guard) = connected.as_ref() {
            for (host_id, live) in guard.iter() {
                let shared = host_id.starts_with("lan-") || host_id.starts_with("acct-");
                if !shared || rows.iter().any(|row| row.host_id == *host_id) {
                    continue;
                }
                let name = live.hello.computer_name.clone();
                rows.push(LinkHostStatus {
                    host_id: host_id.clone(),
                    display_name: name.clone(),
                    target: String::new(),
                    linked_client: true,
                    connected: true,
                    computer_name: name,
                    message: "已连接".into(),
                });
            }
        }
        rows.sort_by(|a, b| a.host_id.cmp(&b.host_id));
        rows
    }
}
