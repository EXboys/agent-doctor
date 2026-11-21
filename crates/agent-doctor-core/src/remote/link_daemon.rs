//! Loopback client service. Other computers reach it only through SSH.

use std::collections::HashMap;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use tungstenite::{accept, Message, WebSocket};

use super::link_files::{list_absolute_dir, list_dir, read_file, write_file};
use super::link_protocol::{
    methods, LinkDoctorSummary, LinkExecResult, LinkFixSummary, LinkHello, LinkInitResult,
    LinkPush, LinkRequest, LinkResponse, LinkWorkspaceRow, LINK_DAEMON_PORT,
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

enum OutboundFrame {
    Text(String),
    Pong(Vec<u8>),
}

/// Shared writer for one accepted WebSocket client.
///
/// Sending queues onto an `mpsc` channel owned by the socket's single loop
/// thread instead of competing with `ws.read()` for a `Mutex<WebSocket>`. On
/// macOS (`os_unfair_lock`) a tight read-timeout loop re-acquires the mutex in
/// nanoseconds and starves background prompt threads forever.
#[derive(Clone)]
pub struct LinkSocket {
    tx: Sender<OutboundFrame>,
}

impl LinkSocket {
    fn from_sender(tx: Sender<OutboundFrame>) -> Self {
        Self { tx }
    }

    pub fn send_json(&self, value: &serde_json::Value) -> Result<()> {
        let body = serde_json::to_string(value).context("encode link frame")?;
        self.tx
            .send(OutboundFrame::Text(body))
            .map_err(|_| anyhow::anyhow!("link connection closed"))?;
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

    fn send_pong(&self, data: Vec<u8>) {
        let _ = self.tx.send(OutboundFrame::Pong(data));
    }
}

/// Exclusive WebSocket handle used during initial handshake before entering the
/// event loop.
pub(crate) struct AcceptedLinkSocket {
    ws: Ws,
}

impl AcceptedLinkSocket {
    pub(crate) fn next_text(&mut self) -> Result<Option<String>> {
        let _ = self
            .ws
            .get_mut()
            .set_read_timeout(Some(Duration::from_secs(15)));
        loop {
            match self.ws.read() {
                Ok(Message::Text(text)) => return Ok(Some(text.to_string())),
                Ok(Message::Ping(data)) => {
                    let _ = self.ws.send(Message::Pong(data));
                }
                Ok(Message::Close(_) | Message::Binary(_)) => return Ok(None),
                Ok(Message::Pong(_) | Message::Frame(_)) => {}
                Err(_) => return Ok(None),
            }
        }
    }

    pub(crate) fn send_response(&mut self, response: &LinkResponse) -> Result<()> {
        let body = serde_json::to_string(response).context("encode link response")?;
        self.ws
            .send(Message::Text(body.into()))
            .context("write link response")?;
        Ok(())
    }

    pub(crate) fn serve(mut self, first_text: Option<&str>) {
        let (tx, rx) = mpsc::channel::<OutboundFrame>();
        let socket = LinkSocket::from_sender(tx);
        if let Some(text) = first_text {
            if let Err(err) = dispatch_text(&socket, text) {
                eprintln!("[link-daemon] dispatch initial: {err:#}");
            }
        }
        loop {
            if drain_outbound(&mut self.ws, &rx).is_err() {
                break;
            }
            let _ = self
                .ws
                .get_mut()
                .set_read_timeout(Some(Duration::from_millis(15)));
            match self.ws.read() {
                Ok(Message::Text(text)) => {
                    if let Err(err) = dispatch_text(&socket, &text) {
                        eprintln!("[link-daemon] dispatch: {err:#}");
                    }
                }
                Ok(Message::Ping(data)) => {
                    socket.send_pong(data.to_vec());
                }
                Ok(Message::Close(_) | Message::Binary(_)) => {
                    let _ = drain_outbound(&mut self.ws, &rx);
                    break;
                }
                Ok(Message::Pong(_) | Message::Frame(_)) => {}
                Err(tungstenite::Error::Io(err))
                    if matches!(
                        err.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) => {}
                Err(_) => break,
            }
        }
    }
}

fn drain_outbound(ws: &mut Ws, rx: &Receiver<OutboundFrame>) -> Result<()> {
    while let Ok(frame) = rx.try_recv() {
        let msg = match frame {
            OutboundFrame::Text(body) => Message::Text(body.into()),
            OutboundFrame::Pong(data) => Message::Pong(data.into()),
        };
        ws.send(msg).context("write link frame")?;
    }
    Ok(())
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

pub(crate) fn open_link_socket(stream: TcpStream) -> Result<AcceptedLinkSocket> {
    let _ = stream.set_nodelay(true);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(15)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(30)));
    let ws = accept(stream).context("WebSocket 握手失败")?;
    Ok(AcceptedLinkSocket { ws })
}

fn handle_client(stream: TcpStream) -> Result<()> {
    let socket = open_link_socket(stream)?;
    socket.serve(None);
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

    if req.method == methods::EXEC {
        let socket = socket.clone();
        thread::Builder::new()
            .name("ad-link-exec".into())
            .spawn(move || {
                let response = match run_builtin_exec(&req) {
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
                let _ = socket.send_response(&response);
            })
            .ok();
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

fn run_builtin_exec(req: &LinkRequest) -> Result<serde_json::Value> {
    use std::io::Read;
    use std::process::Stdio;

    let script = req
        .params
        .get("script")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .context("缺少要执行的命令")?;
    let cwd = req
        .params
        .get("cwd")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
        .or_else(dirs::home_dir)
        .unwrap_or_else(std::env::temp_dir);

    #[cfg(unix)]
    let mut cmd = {
        let mut c = crate::exec::command_for_path(std::path::Path::new("sh"));
        c.args(["-c", script]);
        c.env("PATH", enriched_exec_path());
        c
    };
    #[cfg(windows)]
    let mut cmd = windows_exec_command(script);

    cmd.current_dir(&cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn().context("无法启动远程命令")?;
    let pid = child.id();
    let mut out_pipe = child.stdout.take();
    let mut err_pipe = child.stderr.take();
    let out_reader = thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(ref mut out) = out_pipe {
            let _ = out.read_to_end(&mut buf);
        }
        buf
    });
    let err_reader = thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(ref mut err) = err_pipe {
            let _ = err.read_to_end(&mut buf);
        }
        buf
    });

    let timeout = Duration::from_secs(120);
    let started = std::time::Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status.code().unwrap_or(-1);
        }
        if started.elapsed() >= timeout {
            crate::exec::kill_process_tree(pid);
            let _ = child.wait();
            bail!("命令执行超过 {} 秒已停止", timeout.as_secs());
        }
        thread::sleep(Duration::from_millis(40));
    };

    let stdout = String::from_utf8_lossy(&out_reader.join().unwrap_or_default()).into_owned();
    let stderr = String::from_utf8_lossy(&err_reader.join().unwrap_or_default()).into_owned();
    Ok(serde_json::to_value(LinkExecResult {
        status,
        stdout,
        stderr,
    })?)
}

/// Scripts sent to a linked computer use the same POSIX quoting as a VPS
/// (`'echo' 'ok'`). `cmd.exe` keeps those quotes as part of the program name,
/// so `'echo'` is not the `echo` builtin and the command fails. Git for
/// Windows (and the Windows CI image) provide `sh.exe`, which runs that
/// script unchanged.
#[cfg(windows)]
fn windows_exec_command(script: &str) -> std::process::Command {
    if let Some(sh) = windows_posix_shell() {
        let mut cmd = crate::exec::command_for_path(&sh);
        cmd.args(["-c", script]);
        cmd.env("MSYS_NO_PATHCONV", "1");
        cmd.env("MSYS2_ARG_CONV_EXCL", "*");
        return cmd;
    }
    let mut cmd = crate::exec::command_for_path(std::path::Path::new("cmd"));
    cmd.args(["/C", script]);
    cmd
}

#[cfg(windows)]
fn windows_posix_shell() -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths).find_map(|dir| {
        let candidate = dir.join("sh.exe");
        candidate.is_file().then_some(candidate)
    })
}

#[cfg(unix)]
fn enriched_exec_path() -> String {
    let mut parts = Vec::new();
    if let Some(home) = dirs::home_dir() {
        for rel in [
            ".local/bin",
            ".cargo/bin",
            ".bun/bin",
            ".npm-global/bin",
            "Library/pnpm",
        ] {
            let p = home.join(rel);
            if p.is_dir() {
                parts.push(p.to_string_lossy().into_owned());
            }
        }
    }
    for fixed in ["/opt/homebrew/bin", "/opt/homebrew/sbin", "/usr/local/bin"] {
        if std::path::Path::new(fixed).is_dir() {
            parts.push(fixed.to_string());
        }
    }
    if let Ok(existing) = std::env::var("PATH") {
        parts.push(existing);
    } else {
        parts.push("/usr/bin:/bin:/usr/sbin:/sbin".to_string());
    }
    parts.join(":")
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
    use serde_json::json;

    #[test]
    fn ping_roundtrip_on_ephemeral_is_out_of_scope_here() {
        let raw = r#"{"id":"1","ok":true,"result":{"computer_name":"box","version":"0.1.0","daemon_port":18765}}"#;
        let response: LinkResponse = serde_json::from_str(raw).unwrap();
        assert!(response.ok);
        let _ = LinkClient::decode_hello(response.result.unwrap());
    }

    #[test]
    fn deferred_worker_streams_push_and_final_reply_without_starvation() {
        register_link_method(
            "test_deferred_stream",
            Arc::new(|ctx: LinkRpcContext| {
                thread::spawn(move || {
                    thread::sleep(Duration::from_millis(25));
                    for idx in 0..4 {
                        let _ = ctx.socket.emit_prompt_event(json!({ "seq": idx }));
                        thread::sleep(Duration::from_millis(5));
                    }
                    let _ = ctx.socket.send_response(&LinkResponse {
                        id: ctx.request.id,
                        ok: true,
                        result: Some(json!({ "done": true })),
                        error: None,
                    });
                });
                LinkMethodOutcome::Deferred
            }),
        );

        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let _ = handle_client(stream);
        });

        let mut client = LinkClient::connect_local(port).unwrap();
        let mut seen = Vec::new();
        let started = std::time::Instant::now();
        let value = client
            .call_with_push("test_deferred_stream", json!({}), |push| {
                if let Some(seq) = push.payload.get("seq").and_then(|v| v.as_i64()) {
                    seen.push(seq);
                }
            })
            .unwrap();
        assert_eq!(seen, vec![0, 1, 2, 3]);
        assert_eq!(value, json!({ "done": true }));
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "deferred reply took {:?}",
            started.elapsed()
        );
        drop(client);
        let _ = server.join();
    }

    #[test]
    fn exec_backend_runs_commands_and_reads_files_like_vps() {
        use crate::exec::ExecBackend;
        use crate::remote::link_client::LinkExecBackend;

        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("probe.txt");
        std::fs::write(&file, "hello-lan-vps\n").unwrap();

        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let _ = handle_client(stream);
        });

        let client = LinkClient::connect_local(port).unwrap();
        let backend = LinkExecBackend::new(client, "lan-test");
        assert!(backend.exists(&file).unwrap());
        assert!(backend.is_dir(dir.path()).unwrap());
        assert_eq!(backend.read_to_string(&file).unwrap(), "hello-lan-vps\n");
        let out = backend
            .run(&["echo", "ok-from-peer"], Some(dir.path()))
            .unwrap();
        assert!(out.success(), "status {} stderr {}", out.status, out.stderr);
        assert_eq!(out.stdout_trim(), "ok-from-peer");
        drop(backend);
        let _ = server.join();
    }
}
