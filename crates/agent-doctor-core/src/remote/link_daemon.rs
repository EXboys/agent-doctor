//! Loopback client service. Other computers reach it only through SSH.

use std::collections::HashMap;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use tungstenite::{accept, Message, WebSocket};

use super::link_files::{list_absolute_dir, list_dir, read_file, write_file};
use super::link_protocol::{
    methods, LinkDoctorSummary, LinkFixSummary, LinkHello, LinkInitResult, LinkPush, LinkRequest,
    LinkResponse, LinkWorkspaceRow, LINK_DAEMON_PORT,
};
use crate::workspace::{
    ensure_default_workspace, init_workspace, load_workspaces, workspace_doctor, workspace_fix,
    WorkspaceCheckStatus, WorkspaceFixOptions,
};

static RUNNING: AtomicBool = AtomicBool::new(false);

type Ws = WebSocket<TcpStream>;

/// How a custom method finishes.
pub enum LinkMethodOutcome {
    /// Read loop sends this response immediately.
    Reply(Result<serde_json::Value>),
    /// Handler owns the response; it will call `LinkSocket::send_response` later.
    Deferred,
}

/// Shared writer for one accepted WebSocket client.
#[derive(Clone)]
pub struct LinkSocket {
    inner: Arc<Mutex<Ws>>,
}

impl LinkSocket {
    fn new(ws: Ws) -> Self {
        Self {
            inner: Arc::new(Mutex::new(ws)),
        }
    }

    pub fn send_json(&self, value: &serde_json::Value) -> Result<()> {
        let body = serde_json::to_string(value).context("encode link frame")?;
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("link socket lock poisoned"))?;
        guard
            .send(Message::Text(body.into()))
            .context("write link frame")?;
        Ok(())
    }

    pub fn send_response(&self, response: &LinkResponse) -> Result<()> {
        let value = serde_json::to_value(response).context("encode link response")?;
        self.send_json(&value)
    }

    pub fn send_push(&self, push: &LinkPush) -> Result<()> {
        let value = serde_json::to_value(push).context("encode link push")?;
        self.send_json(&value)
    }

    pub fn emit_prompt_event(&self, payload: serde_json::Value) -> Result<()> {
        self.send_push(&LinkPush::prompt_event(payload))
    }

    fn read_message(&self) -> Result<Message> {
        // Short read timeouts so a prompt worker can take the same mutex to
        // push events while this loop waits for the next client RPC.
        loop {
            let mut guard = self
                .inner
                .lock()
                .map_err(|_| anyhow::anyhow!("link socket lock poisoned"))?;
            let _ = guard
                .get_mut()
                .set_read_timeout(Some(Duration::from_millis(200)));
            match guard.read() {
                Ok(msg) => return Ok(msg),
                Err(tungstenite::Error::Io(err))
                    if err.kind() == std::io::ErrorKind::WouldBlock
                        || err.kind() == std::io::ErrorKind::TimedOut =>
                {
                    drop(guard);
                    continue;
                }
                Err(err) => return Err(err).context("read link frame"),
            }
        }
    }

    fn send_pong(&self, data: Vec<u8>) {
        if let Ok(mut guard) = self.inner.lock() {
            let _ = guard.send(Message::Pong(data.into()));
        }
    }
}

/// Context passed to a registered custom method.
pub struct LinkRpcContext {
    pub request: LinkRequest,
    pub socket: LinkSocket,
}

pub type LinkMethodHandler = Arc<dyn Fn(LinkRpcContext) -> LinkMethodOutcome + Send + Sync>;

static EXTRA_METHODS: Mutex<Option<HashMap<String, LinkMethodHandler>>> = Mutex::new(None);

/// Register or replace a custom RPC (e.g. start_prompt from the desktop crate).
pub fn register_link_method(method: &str, handler: LinkMethodHandler) {
    let Ok(mut guard) = EXTRA_METHODS.lock() else {
        return;
    };
    let map = guard.get_or_insert_with(HashMap::new);
    map.insert(method.to_string(), handler);
}

fn lookup_extra(method: &str) -> Option<LinkMethodHandler> {
    EXTRA_METHODS
        .lock()
        .ok()
        .and_then(|guard| guard.as_ref()?.get(method).cloned())
}

pub fn is_link_daemon_running() -> bool {
    RUNNING.load(Ordering::SeqCst)
}

pub fn default_link_bind_addr() -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], LINK_DAEMON_PORT))
}

/// Start the loopback service in a background thread. Safe to call more than once.
pub fn spawn_link_daemon() -> Result<()> {
    if RUNNING.swap(true, Ordering::SeqCst) {
        return Ok(());
    }
    let addr = default_link_bind_addr();
    let listener = match TcpListener::bind(addr) {
        Ok(listener) => listener,
        Err(err) => {
            RUNNING.store(false, Ordering::SeqCst);
            return Err(err)
                .with_context(|| format!("无法在本机打开共享口 {addr}（可能已经开着）"));
        }
    };
    listener.set_nonblocking(false).context("配置共享口失败")?;
    thread::Builder::new()
        .name("ad-link-daemon".into())
        .spawn(move || {
            for stream in listener.incoming() {
                match stream {
                    Ok(stream) => {
                        let _ = stream.set_nodelay(true);
                        thread::spawn(move || {
                            if let Err(err) = handle_client(stream) {
                                eprintln!("[link-daemon] client ended: {err:#}");
                            }
                        });
                    }
                    Err(err) => {
                        eprintln!("[link-daemon] accept failed: {err}");
                        thread::sleep(Duration::from_millis(200));
                    }
                }
            }
            RUNNING.store(false, Ordering::SeqCst);
        })
        .context("无法启动本机共享服务")?;
    Ok(())
}

fn handle_client(stream: TcpStream) -> Result<()> {
    let ws = accept(stream).context("WebSocket 握手失败")?;
    let socket = LinkSocket::new(ws);
    loop {
        let msg = match socket.read_message() {
            Ok(msg) => msg,
            Err(_) => break,
        };
        match msg {
            Message::Text(text) => {
                if let Err(err) = dispatch_text(&socket, &text) {
                    eprintln!("[link-daemon] dispatch: {err:#}");
                }
            }
            Message::Ping(data) => {
                socket.send_pong(data.to_vec());
            }
            Message::Close(_) | Message::Binary(_) => break,
            Message::Pong(_) | Message::Frame(_) => {}
        }
    }
    Ok(())
}

fn dispatch_text(socket: &LinkSocket, text: &str) -> Result<()> {
    let req: LinkRequest = match serde_json::from_str(text) {
        Ok(req) => req,
        Err(_) => {
            let _ = socket.send_response(&LinkResponse {
                id: String::new(),
                ok: false,
                result: None,
                error: Some("请求格式不对".into()),
            });
            return Ok(());
        }
    };

    if let Some(handler) = lookup_extra(&req.method) {
        let outcome = handler(LinkRpcContext {
            request: req.clone(),
            socket: socket.clone(),
        });
        match outcome {
            LinkMethodOutcome::Deferred => {}
            LinkMethodOutcome::Reply(Ok(result)) => {
                socket.send_response(&LinkResponse {
                    id: req.id,
                    ok: true,
                    result: Some(result),
                    error: None,
                })?;
            }
            LinkMethodOutcome::Reply(Err(err)) => {
                socket.send_response(&LinkResponse {
                    id: req.id,
                    ok: false,
                    result: None,
                    error: Some(format!("{err:#}")),
                })?;
            }
        }
        return Ok(());
    }

    let response = match handle_builtin(&req) {
        Ok(result) => LinkResponse {
            id: req.id,
            ok: true,
            result: Some(result),
            error: None,
        },
        Err(err) => LinkResponse {
            id: req.id,
            ok: false,
            result: None,
            error: Some(format!("{err:#}")),
        },
    };
    socket.send_response(&response)?;
    Ok(())
}

fn handle_builtin(req: &LinkRequest) -> Result<serde_json::Value> {
    match req.method.as_str() {
        methods::PING => {
            let hello = LinkHello {
                computer_name: hostname::get()
                    .ok()
                    .and_then(|s| s.into_string().ok())
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or_else(|| "这台电脑".into()),
                version: env!("CARGO_PKG_VERSION").to_string(),
                daemon_port: LINK_DAEMON_PORT,
            };
            Ok(serde_json::to_value(hello)?)
        }
        methods::LIST_WORKSPACES => {
            let _ = ensure_default_workspace();
            let doc = load_workspaces().unwrap_or_default();
            let rows: Vec<LinkWorkspaceRow> = doc
                .workspaces
                .iter()
                .map(|(name, entry)| LinkWorkspaceRow {
                    name: name.clone(),
                    path: entry.path.to_string_lossy().to_string(),
                    title: entry.title.clone(),
                    active: doc.active.as_deref() == Some(name.as_str()),
                })
                .collect();
            Ok(serde_json::to_value(rows)?)
        }
        methods::INIT_WORKSPACE => {
            let path = req
                .params
                .get("path")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .context("请选一个文件夹")?;
            let name = req
                .params
                .get("name")
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            let report = init_workspace(Some(PathBuf::from(path)), name, false)?;
            Ok(serde_json::to_value(LinkInitResult {
                name: report.name,
                path: report.path.to_string_lossy().to_string(),
            })?)
        }
        methods::LIST_DIR => {
            let absolute = req
                .params
                .get("absolute")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if absolute {
                let path = req
                    .params
                    .get("path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                Ok(serde_json::to_value(list_absolute_dir(path)?)?)
            } else {
                let root = req
                    .params
                    .get("root")
                    .and_then(|v| v.as_str())
                    .context("缺少项目路径")?;
                let relative = req
                    .params
                    .get("relative")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                Ok(serde_json::to_value(list_dir(root, relative)?)?)
            }
        }
        methods::READ_FILE => {
            let root = req
                .params
                .get("root")
                .and_then(|v| v.as_str())
                .context("缺少项目路径")?;
            let relative = req
                .params
                .get("relative")
                .and_then(|v| v.as_str())
                .context("缺少文件路径")?;
            Ok(serde_json::to_value(read_file(root, relative)?)?)
        }
        methods::WRITE_FILE => {
            let root = req
                .params
                .get("root")
                .and_then(|v| v.as_str())
                .context("缺少项目路径")?;
            let relative = req
                .params
                .get("relative")
                .and_then(|v| v.as_str())
                .context("缺少文件路径")?;
            let content = req
                .params
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            write_file(root, relative, content)?;
            Ok(serde_json::json!({ "saved": true }))
        }
        methods::WORKSPACE_DOCTOR => {
            let report = workspace_doctor()?;
            let pass = report
                .checks
                .iter()
                .filter(|c| c.status == WorkspaceCheckStatus::Pass)
                .count();
            let warn = report
                .checks
                .iter()
                .filter(|c| c.status == WorkspaceCheckStatus::Warn)
                .count();
            let fail = report
                .checks
                .iter()
                .filter(|c| c.status == WorkspaceCheckStatus::Fail)
                .count();
            let message = if fail == 0 && warn == 0 {
                "这台电脑上的项目已经整理好了。".to_string()
            } else if fail > 0 {
                format!("还有 {fail} 项需要处理。")
            } else {
                format!("有 {warn} 项提醒。")
            };
            Ok(serde_json::to_value(LinkDoctorSummary {
                pass,
                warn,
                fail,
                message,
            })?)
        }
        methods::WORKSPACE_FIX => {
            let report = workspace_fix(&WorkspaceFixOptions {
                dry_run: false,
                restart_gateways: false,
                migrate_claude_mcp: req
                    .params
                    .get("migrate_claude_mcp")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
            })?;
            let applied = report.actions.iter().filter(|a| a.applied).count();
            Ok(serde_json::to_value(LinkFixSummary {
                applied,
                message: format!("已整理 {applied} 项。"),
            })?)
        }
        other => bail!("还不支持这个操作：{other}"),
    }
}

/// One-shot TCP probe used by the tunnel side after SSH is up.
pub fn wait_for_local_port(port: u16, timeout: Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return true;
        }
        thread::sleep(Duration::from_millis(80));
    }
    false
}

/// Cheap check that a WebSocket hello answers on this loopback port.
pub fn probe_local_daemon(port: u16) -> Result<LinkHello> {
    use super::link_client::LinkClient;
    let mut client = LinkClient::connect_local(port)?;
    client.ping()
}

/// Shared handle so callers can stop accepting (process exit is enough for now).
pub struct LinkDaemonGuard {
    _private: Arc<()>,
}

impl LinkDaemonGuard {
    pub fn start() -> Result<Self> {
        spawn_link_daemon()?;
        Ok(Self {
            _private: Arc::new(()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::link_client::LinkClient;

    #[test]
    fn ping_roundtrip_on_ephemeral_is_out_of_scope_here() {
        let raw = r#"{"id":"1","ok":true,"result":{"computer_name":"box","version":"0.1.0","daemon_port":18765}}"#;
        let response: LinkResponse = serde_json::from_str(raw).unwrap();
        assert!(response.ok);
        let _ = LinkClient::decode_hello(response.result.unwrap());
    }
}
