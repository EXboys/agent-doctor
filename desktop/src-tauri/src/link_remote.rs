//! A-computer side: drive a prompt on a linked host through the SSH tunnel.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use agent_doctor_core::LinkClient;
use agent_doctor_prompt::{PromptSessionEvent, PromptSessionReport};
use base64::Engine as _;
use serde_json::json;
use tauri::{AppHandle, Emitter};

use crate::link_session::LinkSessionState;

#[derive(Clone, Default)]
pub struct LinkedPromptIndex {
    /// client_run_id → (host_id, local tunnel port)
    by_client: Arc<Mutex<HashMap<String, (String, u16)>>>,
    /// backend session_id → client_run_id
    by_backend: Arc<Mutex<HashMap<String, String>>>,
}

impl LinkedPromptIndex {
    pub fn remember(&self, client_run_id: &str, host_id: &str, port: u16) {
        if let Ok(mut guard) = self.by_client.lock() {
            guard.insert(client_run_id.to_string(), (host_id.to_string(), port));
        }
    }

    pub fn note_backend(&self, session_id: &str, client_run_id: &str) {
        if session_id.is_empty() || client_run_id.is_empty() {
            return;
        }
        if let Ok(mut guard) = self.by_backend.lock() {
            guard.insert(session_id.to_string(), client_run_id.to_string());
        }
    }

    pub fn forget_client(&self, client_run_id: &str) {
        if let Ok(mut guard) = self.by_client.lock() {
            guard.remove(client_run_id);
        }
        if let Ok(mut guard) = self.by_backend.lock() {
            guard.retain(|_, client| client != client_run_id);
        }
    }

    pub fn port_for_client(&self, client_run_id: &str) -> Option<u16> {
        self.by_client
            .lock()
            .ok()
            .and_then(|g| g.get(client_run_id).map(|(_, port)| *port))
    }

    pub fn port_for_backend(&self, session_id: &str) -> Option<u16> {
        let client = self
            .by_backend
            .lock()
            .ok()
            .and_then(|g| g.get(session_id).cloned())?;
        self.port_for_client(&client)
    }

    pub fn is_remote_client(&self, client_run_id: &str) -> bool {
        self.port_for_client(client_run_id).is_some()
    }
}

/// `hostId::projectName` → (hostId, projectName).
pub fn split_linked_workspace(name: Option<&str>) -> Option<(String, String)> {
    let raw = name?.trim();
    let (host, project) = raw.split_once("::")?;
    let host = host.trim();
    let project = project.trim();
    if host.is_empty() || project.is_empty() {
        return None;
    }
    Some((host.to_string(), project.to_string()))
}

fn encode_images(paths: &[PathBuf]) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    for path in paths {
        let Ok(bytes) = fs::read(path) else {
            continue;
        };
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("image.png")
            .to_string();
        let base64 = base64::engine::general_purpose::STANDARD.encode(bytes);
        out.push(json!({ "name": name, "base64": base64 }));
    }
    out
}

fn event_session_id(event: &PromptSessionEvent) -> Option<&str> {
    match event {
        PromptSessionEvent::Started { session_id, .. }
        | PromptSessionEvent::Status { session_id, .. }
        | PromptSessionEvent::Delta { session_id, .. }
        | PromptSessionEvent::Thinking { session_id, .. }
        | PromptSessionEvent::StdoutLine { session_id, .. }
        | PromptSessionEvent::StderrLine { session_id, .. }
        | PromptSessionEvent::TurnError { session_id, .. }
        | PromptSessionEvent::PermissionRequest { session_id, .. }
        | PromptSessionEvent::PermissionResolved { session_id, .. }
        | PromptSessionEvent::Tool { session_id, .. }
        | PromptSessionEvent::Plan { session_id, .. }
        | PromptSessionEvent::Usage { session_id, .. }
        | PromptSessionEvent::Completed { session_id, .. }
        | PromptSessionEvent::ProviderSwitched { session_id, .. } => Some(session_id.as_str()),
    }
}

fn stamp_client_run(event: PromptSessionEvent, client_run_id: &str) -> PromptSessionEvent {
    match event {
        PromptSessionEvent::Started {
            session_id,
            runtime,
            cwd,
            command,
            ..
        } => PromptSessionEvent::Started {
            session_id,
            runtime,
            cwd,
            command,
            client_run_id: client_run_id.to_string(),
        },
        other => other,
    }
}

#[allow(clippy::too_many_arguments)]
pub fn run_linked_prompt(
    app: &AppHandle,
    link_sessions: &LinkSessionState,
    linked_index: &LinkedPromptIndex,
    owner_label: &str,
    client_key: &str,
    host_id: &str,
    remote_workspace_name: &str,
    runtime: String,
    prompt: String,
    cwd: Option<String>,
    timeout_sec: u64,
    dangerously_skip_permissions: bool,
    full_auto: bool,
    resume_thread_id: Option<String>,
    selected_mcps: Vec<String>,
    image_paths: Vec<PathBuf>,
    provider_id: Option<String>,
    model: Option<String>,
    browser_tools_enabled: Option<bool>,
    computer_use_enabled: Option<bool>,
) -> Result<PromptSessionReport, String> {
    let lan = link_sessions.lan_client(host_id);
    let port = link_sessions.local_port(host_id).unwrap_or(0);
    if lan.is_none() && port == 0 {
        return Err("还没连上这台电脑。先在旁边的电脑里点「在这台电脑上用」。".into());
    }

    linked_index.remember(client_key, host_id, port);

    let images = encode_images(&image_paths);
    let params = json!({
        "client_run_id": client_key,
        "runtime": runtime,
        "prompt": prompt,
        "cwd": cwd,
        "timeout_sec": timeout_sec,
        "dangerously_skip_permissions": dangerously_skip_permissions,
        "full_auto": full_auto,
        "resume_thread_id": resume_thread_id,
        "selected_mcps": selected_mcps,
        "workspace_name": remote_workspace_name,
        "provider_id": provider_id,
        "model": model,
        "browser_tools_enabled": browser_tools_enabled,
        "computer_use_enabled": computer_use_enabled,
        "images": images,
    });

    let app_for_emit = app.clone();
    let owner = owner_label.to_string();
    let client_for_events = client_key.to_string();
    let index = linked_index.clone();

    let forward = move |payload: serde_json::Value| {
        let Ok(event) = serde_json::from_value::<PromptSessionEvent>(payload) else {
            return;
        };
        let event = stamp_client_run(event, &client_for_events);
        if let Some(session_id) = event_session_id(&event) {
            index.note_backend(session_id, &client_for_events);
        }
        let _ = app_for_emit.emit_to(owner.as_str(), "prompt-session-event", &event);
    };

    let result = if let Some(lan) = lan {
        let mut client = lan.lock().unwrap_or_else(|err| err.into_inner());
        client.start_prompt(params, forward)
    } else {
        let mut client = LinkClient::connect_local(port).map_err(|err| format!("{err:#}"))?;
        client.start_prompt(params, forward)
    };

    linked_index.forget_client(client_key);

    match result {
        Ok(value) => serde_json::from_value::<PromptSessionReport>(value)
            .map_err(|err| format!("对面回应格式不对：{err}")),
        Err(err) => Err(format!("{err:#}")),
    }
}

pub fn cancel_linked_prompt(
    index: &LinkedPromptIndex,
    client_run_id: &str,
) -> Result<bool, String> {
    let Some(port) = index.port_for_client(client_run_id) else {
        return Ok(false);
    };
    LinkClient::cancel_prompt_on(port, client_run_id).map_err(|err| format!("{err:#}"))
}

pub fn resolve_linked_permission(
    index: &LinkedPromptIndex,
    session_id: &str,
    request_id: &str,
    allow: bool,
    text: Option<&str>,
    updated_permissions_json: Option<&str>,
) -> Result<bool, String> {
    let Some(port) = index.port_for_backend(session_id) else {
        return Ok(false);
    };
    LinkClient::resolve_permission_on(
        port,
        session_id,
        request_id,
        allow,
        text,
        updated_permissions_json,
    )
    .map_err(|err| format!("{err:#}"))
}
