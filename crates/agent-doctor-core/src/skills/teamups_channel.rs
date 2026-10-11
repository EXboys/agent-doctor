//! Desktop long connection to the TeamUps site.
//!
//! Feishu talks to the website. The website pushes one job over this socket.
//! This computer does the work and sends the reply back. The socket is outbound
//! only: nothing on the computer is opened to the internet.
//!
//! `wss://<site>/api/v1/doctor/channel`
//! `Authorization: Bearer <license>`
//!
//! Client: `{"type":"hello","name":"...","os":"...","ip":"..."}`
//! Client: `{"type":"reply","id":"...","ok":true,"text":"..."}`
//! Server: `{"type":"job","id":"...","text":"..."}`

use anyhow::Result;
use serde_json::{json, Value};

use crate::store::open_settings_store;

use crate::store::default_teamups_base_url;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeamupsChannelJob {
    pub id: String,
    pub text: String,
}

pub fn teamups_channel_ws_url(http_base: &str) -> String {
    let base = http_base.trim().trim_end_matches('/');
    let (scheme, rest) = if let Some(rest) = base.strip_prefix("https://") {
        ("wss", rest)
    } else if let Some(rest) = base.strip_prefix("http://") {
        ("ws", rest)
    } else if let Some(rest) = base.strip_prefix("wss://") {
        ("wss", rest)
    } else if let Some(rest) = base.strip_prefix("ws://") {
        ("ws", rest)
    } else {
        ("wss", base)
    };
    format!("{scheme}://{rest}/api/v1/doctor/channel")
}

/// Signed-in license and the socket address. `None` when the person is logged out.
pub fn teamups_channel_auth() -> Result<Option<(String, String)>> {
    let store = open_settings_store()?;
    let Some(license) = store
        .get_teamups_license()?
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };
    let base = default_teamups_base_url();
    Ok(Some((teamups_channel_ws_url(&base), license)))
}

pub fn teamups_channel_hello() -> String {
    channel_hello_frame(&computer_name(), &computer_os(), &computer_lan_ip())
}

fn channel_hello_frame(name: &str, os: &str, ip: &str) -> String {
    json!({
        "type": "hello",
        "name": clip_field(name, 80),
        "os": clip_field(os, 40),
        "ip": clip_field(ip, 64),
    })
    .to_string()
}

fn clip_field(value: &str, max: usize) -> String {
    value
        .chars()
        .filter(|ch| !ch.is_control())
        .take(max)
        .collect::<String>()
        .trim()
        .to_string()
}

fn computer_name() -> String {
    #[cfg(target_os = "macos")]
    if let Some(name) = macos_computer_name() {
        return name;
    }
    hostname::get()
        .ok()
        .and_then(|value| value.into_string().ok())
        .map(|value| value.trim_end_matches(".local").trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "这台电脑".to_string())
}

#[cfg(target_os = "macos")]
fn macos_computer_name() -> Option<String> {
    let output = std::process::Command::new("scutil")
        .args(["--get", "ComputerName"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let name = String::from_utf8(output.stdout).ok()?;
    let name = name.trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

fn computer_os() -> String {
    match std::env::consts::OS {
        "macos" => "macOS".to_string(),
        "windows" => "Windows".to_string(),
        "linux" => "Linux".to_string(),
        other => other.to_string(),
    }
}

/// Address this computer uses on the local network. Empty when it cannot be seen.
fn computer_lan_ip() -> String {
    let socket = match std::net::UdpSocket::bind("0.0.0.0:0") {
        Ok(socket) => socket,
        Err(_) => return String::new(),
    };
    if socket.connect("1.1.1.1:80").is_err() {
        return String::new();
    }
    match socket.local_addr() {
        Ok(addr) => {
            let ip = addr.ip();
            if ip.is_loopback() || ip.is_unspecified() {
                String::new()
            } else {
                ip.to_string()
            }
        }
        Err(_) => String::new(),
    }
}

pub fn teamups_channel_reply(id: &str, text: &str, ok: bool) -> String {
    let text: String = text.chars().take(6000).collect();
    json!({ "type": "reply", "id": id, "ok": ok, "text": text }).to_string()
}

pub fn parse_teamups_channel_job(frame: &str) -> Option<TeamupsChannelJob> {
    let value: Value = serde_json::from_str(frame).ok()?;
    if value.get("type").and_then(Value::as_str) != Some("job") {
        return None;
    }
    let id = value.get("id").and_then(Value::as_str)?.trim();
    let text = value.get("text").and_then(Value::as_str)?.trim();
    if id.is_empty() || text.is_empty() {
        return None;
    }
    Some(TeamupsChannelJob {
        id: id.to_string(),
        text: text.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::{
        channel_hello_frame, parse_teamups_channel_job, teamups_channel_reply,
        teamups_channel_ws_url,
    };

    #[test]
    fn the_official_site_uses_a_secure_socket() {
        assert_eq!(
            teamups_channel_ws_url("https://teamups.vip"),
            "wss://teamups.vip/api/v1/doctor/channel"
        );
        assert_eq!(
            teamups_channel_ws_url("http://127.0.0.1:3000/"),
            "ws://127.0.0.1:3000/api/v1/doctor/channel"
        );
    }

    #[test]
    fn a_job_needs_an_id_and_some_text() {
        let job =
            parse_teamups_channel_job(r#"{"type":"job","id":"a","text":"打开项目"}"#).unwrap();
        assert_eq!(job.id, "a");
        assert_eq!(job.text, "打开项目");
        assert!(parse_teamups_channel_job(r#"{"type":"hello"}"#).is_none());
        assert!(parse_teamups_channel_job(r#"{"type":"job","id":"","text":"hi"}"#).is_none());
    }

    #[test]
    fn hello_names_this_computer() {
        let raw = channel_hello_frame("MacBook Pro", "macOS", "192.168.1.8");
        let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(value["type"], "hello");
        assert_eq!(value["name"], "MacBook Pro");
        assert_eq!(value["os"], "macOS");
        assert_eq!(value["ip"], "192.168.1.8");
    }

    #[test]
    fn a_reply_names_the_same_job() {
        let raw = teamups_channel_reply("a", "做完了", true);
        let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(value["type"], "reply");
        assert_eq!(value["id"], "a");
        assert_eq!(value["ok"], true);
        assert_eq!(value["text"], "做完了");
    }
}
