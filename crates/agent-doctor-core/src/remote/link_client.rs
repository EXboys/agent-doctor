//! Blocking WebSocket client for the loopback (or tunneled) client service.

use std::io::ErrorKind;
use std::net::TcpStream;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use tungstenite::{client::client as ws_client, Message, WebSocket};

use super::link_protocol::{
    methods, LinkDirEntry, LinkDoctorSummary, LinkExecResult, LinkFilePayload, LinkFixSummary,
    LinkHello, LinkInitResult, LinkPush, LinkRequest, LinkResponse, LinkWorkspaceRow,
};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

pub struct LinkClient {
    ws: WebSocket<TcpStream>,
    pub local_port: u16,
}

impl LinkClient {
    pub fn connect_local(port: u16) -> Result<Self> {
        let stream = TcpStream::connect(("127.0.0.1", port)).context("连不上那台电脑的客户端")?;
        // Prompt turns can run for a long time; rely on ServerAlive over SSH
        // and application-level cancel instead of a short socket read timeout.
        stream.set_read_timeout(None).ok();
        stream.set_write_timeout(Some(Duration::from_secs(30))).ok();
        let (ws, _) = ws_client(format!("ws://127.0.0.1:{port}/ws"), stream)
            .context("连上了，但客户端握手失败")?;
        Ok(Self {
            ws,
            local_port: port,
        })
    }

    /// Keep using a socket that already finished the hello, instead of opening another one.
    pub fn from_websocket(ws: WebSocket<TcpStream>) -> Self {
        let _ = ws.get_ref().set_nodelay(true);
        Self { ws, local_port: 0 }
    }

    pub fn decode_hello(value: serde_json::Value) -> Result<LinkHello> {
        serde_json::from_value(value).context("电脑信息格式不对")
    }

    fn next_id() -> String {
        NEXT_ID.fetch_add(1, Ordering::SeqCst).to_string()
    }

    fn send_request(&mut self, req: &LinkRequest) -> Result<()> {
        let body = serde_json::to_string(req)?;
        self.ws
            .send(Message::Text(body.into()))
            .context("发送请求失败")?;
        Ok(())
    }

    /// Wait for the response to `id`, forwarding push frames to `on_push`.
    fn wait_response(
        &mut self,
        id: &str,
        keepalive_ping: bool,
        mut on_push: impl FnMut(LinkPush),
    ) -> Result<serde_json::Value> {
        loop {
            let msg = match self.ws.read() {
                Ok(msg) => msg,
                Err(tungstenite::Error::Io(err))
                    if matches!(err.kind(), ErrorKind::TimedOut | ErrorKind::WouldBlock) =>
                {
                    if keepalive_ping {
                        // Application-level ping both verifies the peer loop is
                        // alive and yields the socket lock on older peer builds.
                        let hb = LinkRequest {
                            id: "__hb__".into(),
                            method: methods::PING.into(),
                            params: serde_json::json!({}),
                        };
                        self.send_request(&hb).context("连接已断开")?;
                        continue;
                    }
                    bail!("那边没有回应");
                }
                Err(err) => return Err(err).context("等待回应失败"),
            };
            match msg {
                Message::Text(text) => {
                    if let Ok(push) = serde_json::from_str::<LinkPush>(&text) {
                        if push.kind == "push" {
                            on_push(push);
                            continue;
                        }
                    }
                    let response: LinkResponse =
                        serde_json::from_str(&text).context("回应格式不对")?;
                    if response.id != id {
                        continue;
                    }
                    if !response.ok {
                        bail!("{}", response.error.unwrap_or_else(|| "操作失败".into()));
                    }
                    return Ok(response.result.unwrap_or(serde_json::Value::Null));
                }
                Message::Ping(data) => {
                    let _ = self.ws.send(Message::Pong(data));
                }
                Message::Close(_) => bail!("连接已断开"),
                Message::Binary(_) => bail!("回应格式不对"),
                Message::Pong(_) | Message::Frame(_) => {}
            }
        }
    }

    fn call(&mut self, method: &str, params: serde_json::Value) -> Result<serde_json::Value> {
        // Short questions must not sit forever when the other side never answers.
        let _ = self
            .ws
            .get_mut()
            .set_read_timeout(Some(Duration::from_secs(15)));
        let id = Self::next_id();
        let req = LinkRequest {
            id: id.clone(),
            method: method.to_string(),
            params,
        };
        self.send_request(&req)?;
        self.wait_response(&id, false, |_| {})
    }

    /// Long RPC that may stream push events before the final response.
    pub fn call_with_push(
        &mut self,
        method: &str,
        params: serde_json::Value,
        on_push: impl FnMut(LinkPush),
    ) -> Result<serde_json::Value> {
        let id = Self::next_id();
        let req = LinkRequest {
            id: id.clone(),
            method: method.to_string(),
            params,
        };
        let _ = self
            .ws
            .get_mut()
            .set_read_timeout(Some(Duration::from_millis(200)));
        self.send_request(&req)?;
        self.wait_response(&id, true, on_push)
    }

    /// Fire a short RPC without waiting for an in-flight long call's completion.
    /// Used for cancel / permission while `start_prompt` is still open on another client.
    pub fn call_nowait_separate(
        port: u16,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let mut client = Self::connect_local(port)?;
        client.call(method, params)
    }

    pub fn ping(&mut self) -> Result<LinkHello> {
        let value = self.call(methods::PING, serde_json::json!({}))?;
        Self::decode_hello(value)
    }

    pub fn list_workspaces(&mut self) -> Result<Vec<LinkWorkspaceRow>> {
        let value = self.call(methods::LIST_WORKSPACES, serde_json::json!({}))?;
        serde_json::from_value(value).context("项目列表格式不对")
    }

    pub fn init_workspace(&mut self, path: &str, name: Option<&str>) -> Result<LinkInitResult> {
        let value = self.call(
            methods::INIT_WORKSPACE,
            serde_json::json!({
                "path": path,
                "name": name,
            }),
        )?;
        serde_json::from_value(value).context("添加项目结果格式不对")
    }

    pub fn list_absolute_dir(&mut self, path: &str) -> Result<Vec<LinkDirEntry>> {
        let value = self.call(
            methods::LIST_DIR,
            serde_json::json!({
                "absolute": true,
                "path": path,
            }),
        )?;
        serde_json::from_value(value).context("文件夹列表格式不对")
    }

    pub fn list_project_dir(&mut self, root: &str, relative: &str) -> Result<Vec<LinkDirEntry>> {
        let value = self.call(
            methods::LIST_DIR,
            serde_json::json!({
                "absolute": false,
                "root": root,
                "relative": relative,
            }),
        )?;
        serde_json::from_value(value).context("文件夹列表格式不对")
    }

    pub fn read_file(&mut self, root: &str, relative: &str) -> Result<LinkFilePayload> {
        let value = self.call(
            methods::READ_FILE,
            serde_json::json!({
                "root": root,
                "relative": relative,
            }),
        )?;
        serde_json::from_value(value).context("文件内容格式不对")
    }

    pub fn write_file(&mut self, root: &str, relative: &str, content: &str) -> Result<()> {
        let _ = self.call(
            methods::WRITE_FILE,
            serde_json::json!({
                "root": root,
                "relative": relative,
                "content": content,
            }),
        )?;
        Ok(())
    }

    pub fn workspace_doctor(&mut self) -> Result<LinkDoctorSummary> {
        let value = self.call(methods::WORKSPACE_DOCTOR, serde_json::json!({}))?;
        serde_json::from_value(value).context("检查结果格式不对")
    }

    pub fn workspace_fix(&mut self, migrate_claude_mcp: bool) -> Result<LinkFixSummary> {
        let value = self.call(
            methods::WORKSPACE_FIX,
            serde_json::json!({ "migrate_claude_mcp": migrate_claude_mcp }),
        )?;
        serde_json::from_value(value).context("整理结果格式不对")
    }

    /// Execute a shell command on the linked computer (same contract as `SshBackend`).
    pub fn exec(&mut self, script: &str, cwd: Option<&str>) -> Result<LinkExecResult> {
        let value = self.call_with_push(
            methods::EXEC,
            serde_json::json!({
                "script": script,
                "cwd": cwd,
            }),
            |_| {},
        )?;
        serde_json::from_value(value).context("命令返回格式不对")
    }

    /// Ask the local desktop daemon to run `exec` on an already-connected LAN/SSH host.
    pub fn proxy_exec(
        &mut self,
        host_id: &str,
        script: &str,
        cwd: Option<&str>,
    ) -> Result<LinkExecResult> {
        let value = self.call_with_push(
            methods::PROXY_EXEC,
            serde_json::json!({
                "host_id": host_id,
                "script": script,
                "cwd": cwd,
            }),
            |_| {},
        )?;
        serde_json::from_value(value).context("远程命令返回格式不对")
    }

    pub fn start_prompt(
        &mut self,
        params: serde_json::Value,
        on_event: impl FnMut(serde_json::Value),
    ) -> Result<serde_json::Value> {
        let mut on_event = on_event;
        self.call_with_push(methods::START_PROMPT, params, |push| {
            if push.channel == "prompt" {
                on_event(push.payload);
            }
        })
    }

    pub fn cancel_prompt(&mut self, client_run_id: &str) -> Result<bool> {
        let value = self.call(
            methods::CANCEL_PROMPT,
            serde_json::json!({ "client_run_id": client_run_id }),
        )?;
        Ok(value
            .get("cancelled")
            .and_then(|v| v.as_bool())
            .unwrap_or(true))
    }

    pub fn resolve_permission(
        &mut self,
        session_id: &str,
        request_id: &str,
        allow: bool,
        text: Option<&str>,
        updated_permissions_json: Option<&str>,
    ) -> Result<bool> {
        let value = self.call(
            methods::RESOLVE_PERMISSION,
            serde_json::json!({
                "session_id": session_id,
                "request_id": request_id,
                "allow": allow,
                "text": text,
                "updated_permissions_json": updated_permissions_json,
            }),
        )?;
        Ok(value.get("ok").and_then(|v| v.as_bool()).unwrap_or(true))
    }

    pub fn cancel_prompt_on(port: u16, client_run_id: &str) -> Result<bool> {
        let mut client = Self::connect_local(port)?;
        client.cancel_prompt(client_run_id)
    }

    pub fn resolve_permission_on(
        port: u16,
        session_id: &str,
        request_id: &str,
        allow: bool,
        text: Option<&str>,
        updated_permissions_json: Option<&str>,
    ) -> Result<bool> {
        let mut client = Self::connect_local(port)?;
        client.resolve_permission(
            session_id,
            request_id,
            allow,
            text,
            updated_permissions_json,
        )
    }
}

/// Treat a passwordless LAN-linked computer identically to an `SshBackend` VPS.
pub struct LinkExecBackend {
    client: std::sync::Mutex<LinkClient>,
    label: String,
}

impl LinkExecBackend {
    pub fn new(client: LinkClient, label: impl Into<String>) -> Self {
        Self {
            client: std::sync::Mutex::new(client),
            label: label.into(),
        }
    }

    pub fn run_script(
        &self,
        script: &str,
        cwd: Option<&std::path::Path>,
    ) -> Result<crate::exec::ExecOutput> {
        let cwd_str = cwd.map(|p| p.display().to_string());
        let mut guard = self
            .client
            .lock()
            .map_err(|_| anyhow::anyhow!("link client mutex poisoned"))?;
        let res = guard.exec(script, cwd_str.as_deref())?;
        Ok(crate::exec::ExecOutput {
            status: res.status,
            stdout: res.stdout,
            stderr: res.stderr,
        })
    }
}

impl crate::exec::ExecBackend for LinkExecBackend {
    fn run(&self, argv: &[&str], cwd: Option<&std::path::Path>) -> Result<crate::exec::ExecOutput> {
        let Some((program, args)) = argv.split_first() else {
            bail!("empty argv");
        };
        let mut parts = Vec::with_capacity(1 + args.len());
        parts.push(crate::exec::shell_quote(program));
        for arg in args {
            parts.push(crate::exec::shell_quote(arg));
        }
        self.run_script(&parts.join(" "), cwd)
    }

    fn read_to_string(&self, path: &std::path::Path) -> Result<String> {
        let quoted = crate::exec::shell_quote(&path.display().to_string());
        let out = self.run_script(&format!("cat -- {quoted}"), None)?;
        if !out.success() {
            bail!(
                "read {} on '{}': {}",
                path.display(),
                self.label,
                out.stderr.trim()
            );
        }
        Ok(out.stdout)
    }

    fn exists(&self, path: &std::path::Path) -> Result<bool> {
        let quoted = crate::exec::shell_quote(&path.display().to_string());
        Ok(self
            .run_script(&format!("test -e {quoted}"), None)?
            .success())
    }

    fn is_dir(&self, path: &std::path::Path) -> Result<bool> {
        let quoted = crate::exec::shell_quote(&path.display().to_string());
        Ok(self
            .run_script(&format!("test -d {quoted}"), None)?
            .success())
    }

    fn home_dir(&self) -> Result<std::path::PathBuf> {
        let out = self.run_script("printf %s \"$HOME\"", None)?;
        if !out.success() || out.stdout_trim().is_empty() {
            bail!("resolve HOME on '{}' failed", self.label);
        }
        Ok(std::path::PathBuf::from(out.stdout_trim()))
    }
}
