//! Keeps SSH tunnels to linked computers alive for the desktop process.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use agent_doctor_core::{
    load_remote_hosts, mark_host_linked, open_link_tunnel, LinkClient, LinkConnectFailure,
    LinkHello, LinkTunnel,
};
use serde::Serialize;

#[derive(Clone, Default)]
pub struct LinkSessionState {
    inner: Arc<Mutex<HashMap<String, LiveLink>>>,
}

struct LiveLink {
    tunnel: LinkTunnel,
    hello: LinkHello,
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
                live.tunnel.stop();
            }
        }
    }

    #[allow(dead_code)]
    pub fn disconnect_all(&self) {
        if let Ok(mut guard) = self.inner.lock() {
            for (_, live) in guard.drain() {
                live.tunnel.stop();
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
            .and_then(|g| g.get(host_id).map(|live| live.tunnel.local_port))
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
                            tunnel,
                            hello: hello.clone(),
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

    pub fn with_client<T>(
        &self,
        host_id: &str,
        f: impl FnOnce(&mut LinkClient) -> anyhow::Result<T>,
    ) -> Result<T, String> {
        let port = self
            .local_port(host_id)
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
        rows.sort_by(|a, b| a.host_id.cmp(&b.host_id));
        rows
    }
}
