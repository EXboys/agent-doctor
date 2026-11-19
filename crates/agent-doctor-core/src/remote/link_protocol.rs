//! Shared JSON protocol for computer-to-computer client control (Paseo-style).
//!
//! A listens only on loopback. B reaches it through an SSH tunnel, never by
//! opening the client port on the LAN.

use serde::{Deserialize, Serialize};

/// Default loopback port for the local client service.
pub const LINK_DAEMON_PORT: u16 = 18765;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkRequest {
    pub id: String,
    pub method: String,
    #[serde(default)]
    pub params: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkResponse {
    pub id: String,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Server-pushed frame (events while a long RPC such as start_prompt runs).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkPush {
    #[serde(rename = "type")]
    pub kind: String,
    pub channel: String,
    pub payload: serde_json::Value,
}

impl LinkPush {
    pub fn prompt_event(payload: serde_json::Value) -> Self {
        Self {
            kind: "push".into(),
            channel: "prompt".into(),
            payload,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkHello {
    pub computer_name: String,
    pub version: String,
    pub daemon_port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkWorkspaceRow {
    pub name: String,
    pub path: String,
    #[serde(default)]
    pub title: String,
    pub active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkDirEntry {
    pub name: String,
    pub relative_path: String,
    pub is_dir: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkFilePayload {
    pub relative_path: String,
    pub name: String,
    pub content: String,
    pub language: String,
    pub editable: bool,
    pub is_image: bool,
    pub absolute_path: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkInitResult {
    pub name: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkDoctorSummary {
    pub pass: usize,
    pub warn: usize,
    pub fail: usize,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkFixSummary {
    pub applied: usize,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkImageBlob {
    pub name: String,
    pub base64: String,
}

pub mod methods {
    pub const PING: &str = "ping";
    pub const LIST_WORKSPACES: &str = "list_workspaces";
    pub const INIT_WORKSPACE: &str = "init_workspace";
    pub const LIST_DIR: &str = "list_dir";
    pub const READ_FILE: &str = "read_file";
    pub const WRITE_FILE: &str = "write_file";
    pub const WORKSPACE_DOCTOR: &str = "workspace_doctor";
    pub const WORKSPACE_FIX: &str = "workspace_fix";
    pub const START_PROMPT: &str = "start_prompt";
    pub const CANCEL_PROMPT: &str = "cancel_prompt";
    pub const RESOLVE_PERMISSION: &str = "resolve_permission";
}
