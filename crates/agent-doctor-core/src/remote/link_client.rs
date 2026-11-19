//! Blocking WebSocket client for the loopback (or tunneled) client service.

use std::net::TcpStream;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use tungstenite::{client::client as ws_client, Message, WebSocket};

use super::link_protocol::{
    methods, LinkDirEntry, LinkDoctorSummary, LinkFilePayload, LinkFixSummary, LinkHello,
    LinkInitResult, LinkPush, LinkRequest, LinkResponse, LinkWorkspaceRow,
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
        mut on_push: impl FnMut(LinkPush),
    ) -> Result<serde_json::Value> {
        loop {
            let msg = self.ws.read().context("等待回应失败")?;
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
        let id = Self::next_id();
        let req = LinkRequest {
            id: id.clone(),
            method: method.to_string(),
            params,
        };
        self.send_request(&req)?;
        self.wait_response(&id, |_| {})
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
        self.send_request(&req)?;
        self.wait_response(&id, on_push)
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

    pub fn cancel_prompt_on(port: u16, client_run_id: &str) -> Result<bool> {
        let value = Self::call_nowait_separate(
            port,
            methods::CANCEL_PROMPT,
            serde_json::json!({ "client_run_id": client_run_id }),
        )?;
        Ok(value
            .get("cancelled")
            .and_then(|v| v.as_bool())
            .unwrap_or(true))
    }

    pub fn resolve_permission_on(
        port: u16,
        session_id: &str,
        request_id: &str,
        allow: bool,
        text: Option<&str>,
        updated_permissions_json: Option<&str>,
    ) -> Result<bool> {
        let value = Self::call_nowait_separate(
            port,
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
}
