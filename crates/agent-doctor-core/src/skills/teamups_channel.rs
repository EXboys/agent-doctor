//! Desktop long connection to the TeamUps site.
//!
//! Feishu talks to the website. The website pushes one job over this socket.
//! This computer does the work and sends the reply back. The socket is outbound
//! only: nothing on the computer is opened to the internet.
//!
//! `wss://<site>/api/v1/doctor/channel`
//! `Authorization: Bearer <license>`
//!
//! Client: `{"type":"hello","id":"...","name":"...","os":"...","ip":"...","resume":true,"since":{}}`
//! `since` maps phone conversation ids to the archive update this computer
//! last merged. `{}` means "nothing here yet" and asks the site to replay
//! every conversation it kept — that is how a reinstalled computer catches up.
//! Server: `{"type":"sync","conversation":{"chat":"...","title":"...","updatedAt":...,"rows":[...]}}`
//! replays one archived conversation.
//! Client: `{"type":"heard","id":"..."}` as soon as a job frame arrives
//! Client: `{"type":"reply","id":"...","ok":true,"text":"..."}`
//! `chat` is the Feishu conversation. Empty means one shared Feishu chat.
//! `resume` means the server may send an unfinished job again after a
//! reconnect; the same `id` comes back and the computer must not run it twice.

use anyhow::Result;
use serde_json::{json, Value};

use crate::store::open_settings_store;

use crate::store::default_teamups_base_url;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeamupsChannelFile {
    pub name: String,
    pub mime: String,
    /// `image` or `file`.
    pub kind: String,
    /// Standard base64, no data-URL prefix.
    pub data: String,
}

pub struct TeamupsChannelJob {
    pub id: String,
    pub text: String,
    /// Which Feishu conversation this sentence belongs to. Empty when the site
    /// did not say, so every such job stays in one desktop chat.
    pub chat: String,
    /// Registered project the phone asked this sentence to use. Empty keeps the chat's project.
    pub project: String,
    /// Pictures and files the phone attached. Empty for a text-only sentence.
    pub files: Vec<TeamupsChannelFile>,
    /// Which command line should answer. Empty keeps the chat's current one.
    pub runtime: String,
    /// Which model the phone picked for that command line. Empty keeps the
    /// chat's current model. Only meaningful together with `runtime` or a chat
    /// the owner already pinned.
    pub model: String,
    /// Which saved service serves that model. Empty means the wired one. A
    /// model id alone is ambiguous when the computer holds several providers.
    pub provider: String,
    /// The phone asked this chat how to confirm: `Some(true)` skips the
    /// per-step asks, `Some(false)` asks again even on a computer whose
    /// global default would let go, `None` keeps whatever this chat does.
    pub auto_allow: Option<bool>,
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

const CHANNEL_ENABLED_KEY: &str = "teamups_channel_enabled";
const CHANNEL_CLIENT_ID_KEY: &str = "teamups_channel_client_id";
const CHANNEL_SINCE_KEY: &str = "teamups_channel_since";

/// Whether this computer should keep the line open. Default is on.
/// Logged-out computers still do not connect.
pub fn teamups_channel_enabled() -> Result<bool> {
    let store = open_settings_store()?;
    Ok(store.meta_get(CHANNEL_ENABLED_KEY)?.as_deref() != Some("0"))
}

pub fn set_teamups_channel_enabled(on: bool) -> Result<()> {
    let store = open_settings_store()?;
    store.meta_set(CHANNEL_ENABLED_KEY, if on { "1" } else { "0" })
}

/// `None` when nobody is signed in, or when the person turned the line off.
/// Callers must not open a socket in that case.
pub fn teamups_channel_auth() -> Result<Option<(String, String)>> {
    if !teamups_channel_enabled()? {
        return Ok(None);
    }
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

pub fn teamups_channel_signed_in() -> Result<bool> {
    let store = open_settings_store()?;
    Ok(store
        .get_teamups_license()?
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false))
}

pub fn teamups_channel_hello() -> String {
    let since = teamups_channel_since().unwrap_or_else(|| json!({}));
    channel_hello_frame(
        &channel_client_id(),
        &computer_name(),
        &computer_os(),
        &computer_lan_ip(),
        &since,
    )
}

/// The per-conversation cursors the chat window pushed: `{conversationId:
/// updatedAt}`. `None` means it never pushed one and the hello carries an
/// empty map, which asks the site to replay everything it kept.
pub fn teamups_channel_since() -> Option<Value> {
    let store = open_settings_store().ok()?;
    let raw = store.meta_get(CHANNEL_SINCE_KEY).ok()??;
    clean_since(serde_json::from_str(&raw).ok()?)
}

pub fn set_teamups_channel_since(since: Value) -> Result<()> {
    let store = open_settings_store()?;
    let cleaned = clean_since(since).unwrap_or_else(|| json!({}));
    store.meta_set(CHANNEL_SINCE_KEY, &cleaned.to_string())
}

/// Keep only entries the site can act on: a chat id and a real stamp, at
/// most 100 chats. Anything malformed is dropped, not fatal.
fn clean_since(value: Value) -> Option<Value> {
    let map = value.as_object()?;
    let mut clean = serde_json::Map::new();
    for (id, stamp) in map.iter().take(100) {
        let chat = clip_field(id, 80);
        let at = match stamp.as_f64() {
            Some(at) if at.is_finite() && at >= 0.0 => at as u64,
            _ => continue,
        };
        if chat.is_empty() {
            continue;
        }
        clean.insert(chat, json!(at));
    }
    Some(Value::Object(clean))
}

pub(crate) fn channel_client_id() -> String {
    let Ok(store) = open_settings_store() else {
        return new_client_id();
    };
    if let Ok(Some(existing)) = store.meta_get(CHANNEL_CLIENT_ID_KEY) {
        let id = clip_field(&existing, 64);
        if !id.is_empty() {
            return id;
        }
    }
    let id = new_client_id();
    let _ = store.meta_set(CHANNEL_CLIENT_ID_KEY, &id);
    id
}

fn new_client_id() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    format!("ad-{now:x}")
}

fn channel_hello_frame(id: &str, name: &str, os: &str, ip: &str, since: &Value) -> String {
    json!({
        "type": "hello",
        "id": clip_field(id, 64),
        "name": clip_field(name, 80),
        "os": clip_field(os, 40),
        "ip": clip_field(ip, 64),
        "resume": true,
        "since": since,
    })
    .to_string()
}

/// This computer's channel id, so schedules can bind to it.
pub fn teamups_channel_client_id() -> Result<String> {
    Ok(channel_client_id())
}

pub fn teamups_channel_heard(id: &str) -> String {
    json!({ "type": "heard", "id": id }).to_string()
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

pub(crate) fn computer_name() -> String {
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
    let output = std::process::Command::new("scutil") // raw-spawn-ok
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

pub fn teamups_channel_reply(id: &str, text: &str, ok: bool, done: bool, chat: &str) -> String {
    let text: String = text.chars().take(6000).collect();
    let mut value = json!({ "type": "reply", "id": id, "ok": ok, "text": text, "done": done });
    let chat = chat.trim();
    // Two chats can be running. The phone uses this to put the result back
    // in the conversation that asked, not the one open on screen.
    if !chat.is_empty() {
        value["chat"] = json!(chat);
    }
    value.to_string()
}

/// How many pictures one answer may carry, and how big each may be once
/// base64-encoded. The phone renders them inline, so the line stays the limit.
pub const REPLY_IMAGE_MAX: usize = 3;
pub const REPLY_IMAGE_DATA_MAX: usize = 900_000;
/// Small icons/diagrams below this size and within [`REPLY_IMAGE_MAX_EDGE`] stay untouched.
const REPLY_IMAGE_SKIP_COMPRESS_BYTES: usize = 80_000;
/// Target raw byte budget (~175 KB raw / ~240 KB base64) so screenshots stay light on mobile data.
const REPLY_IMAGE_SOFT_RAW_BYTES: usize = 180_000;
const REPLY_IMAGE_MAX_EDGE: u32 = 1280;

fn reply_image_mime(path: &str) -> &'static str {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".png") {
        "image/png"
    } else if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        "image/jpeg"
    } else if lower.ends_with(".gif") {
        "image/gif"
    } else if lower.ends_with(".webp") {
        "image/webp"
    } else {
        ""
    }
}

/// A picture the answer wants on the phone: `{ name, mime, data }`, where
/// `data` is raw base64. Anything oversized or not a picture is dropped, so a
/// runaway answer cannot stuff megabytes into one reply.
pub fn clean_reply_images(value: Option<&Value>) -> Vec<Value> {
    let Some(items) = value.and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut kept = Vec::new();
    for item in items.iter().take(REPLY_IMAGE_MAX) {
        let Some(map) = item.as_object() else {
            continue;
        };
        let name = map.get("name").and_then(Value::as_str).unwrap_or("").trim();
        let mime = map.get("mime").and_then(Value::as_str).unwrap_or("").trim();
        let data = map.get("data").and_then(Value::as_str).unwrap_or("").trim();
        let data: String = data.chars().filter(|ch| !ch.is_whitespace()).collect();
        let mime_ok = matches!(
            mime,
            "image/png" | "image/jpeg" | "image/gif" | "image/webp"
        );
        let data_ok = !data.is_empty()
            && data.len() <= REPLY_IMAGE_DATA_MAX
            && data
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '+' || ch == '/' || ch == '=');
        if !mime_ok || !data_ok {
            continue;
        }
        let name = if name.is_empty() { "图片" } else { name };
        kept.push(json!({
            "name": name.chars().take(80).collect::<String>(),
            "mime": mime,
            "data": data,
        }));
    }
    kept
}

/// Read a picture from disk into the frame shape above.
pub fn read_reply_image(path: &str) -> Result<Value, String> {
    read_reply_image_in(path, None)
}

fn resolve_reply_image_path(raw: &str, cwd: Option<&str>) -> Option<std::path::PathBuf> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.to_ascii_lowercase().starts_with("file://") {
        if let Ok(parsed) = url::Url::parse(trimmed) {
            if let Ok(file_path) = parsed.to_file_path() {
                return Some(file_path);
            }
        }
    }
    let path = if trimmed == "~" {
        dirs::home_dir()?
    } else if let Some(rest) = trimmed
        .strip_prefix("~/")
        .or_else(|| trimmed.strip_prefix("~\\"))
    {
        dirs::home_dir()?.join(rest)
    } else {
        std::path::PathBuf::from(trimmed)
    };
    if path.is_relative() {
        if let Some(base) = cwd.map(str::trim).filter(|s| !s.is_empty()) {
            return Some(std::path::Path::new(base).join(path));
        }
    }
    Some(path)
}

fn flatten_to_rgb_on_white(img: &image::DynamicImage) -> image::RgbImage {
    if !img.color().has_alpha() {
        return img.to_rgb8();
    }
    let rgba = img.to_rgba8();
    let mut rgb = image::RgbImage::new(rgba.width(), rgba.height());
    for (x, y, p) in rgba.enumerate_pixels() {
        let a = u16::from(p[3]);
        let inv = 255 - a;
        rgb.put_pixel(
            x,
            y,
            image::Rgb([
                ((u16::from(p[0]) * a + 255 * inv + 127) / 255) as u8,
                ((u16::from(p[1]) * a + 255 * inv + 127) / 255) as u8,
                ((u16::from(p[2]) * a + 255 * inv + 127) / 255) as u8,
            ]),
        );
    }
    rgb
}

fn compress_reply_image_bytes(
    bytes: &[u8],
    soft_bytes: usize,
    hard_bytes: usize,
) -> Option<(Vec<u8>, &'static str)> {
    let img = image::load_from_memory(bytes).ok()?;
    if bytes.len() <= REPLY_IMAGE_SKIP_COMPRESS_BYTES
        && img.width() <= REPLY_IMAGE_MAX_EDGE
        && img.height() <= REPLY_IMAGE_MAX_EDGE
    {
        return None;
    }
    let mut best_under_hard: Option<Vec<u8>> = None;
    for (max_edge, quality) in [
        (REPLY_IMAGE_MAX_EDGE, 74u8),
        (1120, 68),
        (960, 62),
        (800, 56),
    ] {
        let resized = if img.width() > max_edge || img.height() > max_edge {
            img.resize(max_edge, max_edge, image::imageops::FilterType::CatmullRom)
        } else {
            img.clone()
        };
        let rgb = flatten_to_rgb_on_white(&resized);
        let mut out = Vec::new();
        let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality);
        if encoder
            .encode(
                rgb.as_raw(),
                rgb.width(),
                rgb.height(),
                image::ExtendedColorType::Rgb8,
            )
            .is_err()
        {
            continue;
        }
        if out.len() <= soft_bytes {
            return Some((out, "image/jpeg"));
        }
        if out.len() <= hard_bytes
            && best_under_hard
                .as_ref()
                .map_or(true, |prev| out.len() < prev.len())
        {
            best_under_hard = Some(out);
        }
    }
    best_under_hard.map(|out| (out, "image/jpeg"))
}

/// Read a picture from disk (resolving `file://`, `~/`, and relative paths under `cwd`)
/// and compress screenshots so they stay light over the channel and under [`REPLY_IMAGE_DATA_MAX`].
pub fn read_reply_image_in(path: &str, cwd: Option<&str>) -> Result<Value, String> {
    let resolved =
        resolve_reply_image_path(path, cwd).ok_or_else(|| "没有给图片路径。".to_string())?;
    let resolved_str = resolved.to_string_lossy();
    let mut mime = reply_image_mime(&resolved_str);
    if mime.is_empty() {
        return Err("只支持 png / jpg / gif / webp。".to_string());
    }
    let mut bytes = std::fs::read(&resolved).map_err(|err| format!("读不到这张图：{err}"))?;
    let max_raw = REPLY_IMAGE_DATA_MAX / 4 * 3;
    if mime != "image/gif" {
        if let Some((shrunk, next_mime)) =
            compress_reply_image_bytes(&bytes, REPLY_IMAGE_SOFT_RAW_BYTES, max_raw)
        {
            // Keep original only when it already fits `max_raw` and re-encoding barely saves bytes.
            if bytes.len() > max_raw || shrunk.len() + 8_192 < bytes.len() {
                bytes = shrunk;
                mime = next_mime;
            }
        }
    }
    if bytes.len() > max_raw {
        return Err("图片太大了，压到 600KB 以内再发。".to_string());
    }
    let data = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes);
    let name = resolved
        .file_name()
        .and_then(|item| item.to_str())
        .unwrap_or("图片");
    Ok(json!({ "name": name, "mime": mime, "data": data }))
}

pub fn parse_channel_stop(frame: &str) -> Option<String> {
    let value: Value = serde_json::from_str(frame).ok()?;
    if value.get("type").and_then(Value::as_str) != Some("stop") {
        return None;
    }
    let id = value.get("id").and_then(Value::as_str)?.trim();
    if id.is_empty() {
        None
    } else {
        Some(id.to_string())
    }
}

/// A conversation the site archived, replayed after hello. Returns the
/// `conversation` object; the rows inside stay the webview's business.
pub fn parse_teamups_channel_sync(frame: &str) -> Option<Value> {
    let value: Value = serde_json::from_str(frame).ok()?;
    if value.get("type").and_then(Value::as_str) != Some("sync") {
        return None;
    }
    let conversation = value.get("conversation")?;
    if conversation.is_object() {
        Some(conversation.clone())
    } else {
        None
    }
}

pub fn parse_teamups_channel_job(frame: &str) -> Option<TeamupsChannelJob> {
    let value: Value = serde_json::from_str(frame).ok()?;
    if value.get("type").and_then(Value::as_str) != Some("job") {
        return None;
    }
    let id = value.get("id").and_then(Value::as_str)?.trim();
    let text = value
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let files = parse_job_files(value.get("files"));
    if id.is_empty() || (text.is_empty() && files.is_empty()) {
        return None;
    }
    let chat = value
        .get("chat")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let project = value
        .get("project")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let runtime = value
        .get("runtime")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let model = value
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let provider = value
        .get("provider")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    Some(TeamupsChannelJob {
        id: id.to_string(),
        text: text.to_string(),
        chat: chat.chars().take(128).collect(),
        project: project.chars().take(80).collect(),
        files,
        runtime: runtime.chars().take(40).collect(),
        model: model.chars().take(120).collect(),
        provider: provider.chars().take(64).collect(),
        auto_allow: value.get("autoAllow").and_then(Value::as_bool),
    })
}

/// The site asks what this computer can run (`{"type":"choices"}`).
pub fn parse_channel_choices(frame: &str) -> bool {
    let Ok(value) = serde_json::from_str::<Value>(frame) else {
        return false;
    };
    value.get("type").and_then(Value::as_str) == Some("choices")
}

/// The phone sets how one chat confirms tool calls:
/// `{"type":"auto-allow","chat":"phone:...","on":true}`. `on: false` asks
/// every step again; a frame without `on` follows the computer's setting.
/// Returns the chat key and the requested state.
pub fn parse_channel_auto_allow(frame: &str) -> Option<(String, Option<bool>)> {
    let value: Value = serde_json::from_str(frame).ok()?;
    if value.get("type").and_then(Value::as_str) != Some("auto-allow") {
        return None;
    }
    let chat = value.get("chat").and_then(Value::as_str)?.trim();
    if chat.is_empty() {
        return None;
    }
    Some((
        chat.chars().take(128).collect(),
        value.get("on").and_then(Value::as_bool),
    ))
}

fn parse_job_files(value: Option<&Value>) -> Vec<TeamupsChannelFile> {
    let Some(list) = value.and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut files = Vec::new();
    let mut total = 0usize;
    for item in list {
        if files.len() >= 4 {
            break;
        }
        let Some(obj) = item.as_object() else {
            continue;
        };
        let data = obj.get("data").and_then(Value::as_str).unwrap_or("").trim();
        if data.is_empty() || data.len() > 1_400_000 || !is_base64(data) {
            continue;
        }
        if total + data.len() > 3_200_000 {
            break;
        }
        total += data.len();
        let mime: String = obj
            .get("mime")
            .and_then(Value::as_str)
            .unwrap_or("")
            .chars()
            .take(80)
            .collect();
        let kind_raw = obj.get("kind").and_then(Value::as_str).unwrap_or("");
        let kind = if kind_raw == "image" || mime.starts_with("image/") {
            "image"
        } else {
            "file"
        };
        files.push(TeamupsChannelFile {
            name: clip_file_name(obj.get("name").and_then(Value::as_str).unwrap_or("file")),
            mime,
            kind: kind.to_string(),
            data: data.to_string(),
        });
    }
    files
}

fn is_base64(value: &str) -> bool {
    value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'+' || byte == b'/' || byte == b'=')
}

fn clip_file_name(raw: &str) -> String {
    let base = raw.rsplit(['/', '\\']).next().unwrap_or(raw).trim();
    let name: String = base.chars().take(80).collect();
    if name.is_empty() {
        "file".to_string()
    } else {
        name
    }
}

#[cfg(test)]
mod tests {
    use super::{
        channel_hello_frame, parse_channel_auto_allow, parse_channel_choices, parse_channel_stop,
        parse_teamups_channel_job, parse_teamups_channel_sync, teamups_channel_reply,
        teamups_channel_ws_url,
    };

    #[test]
    fn the_official_site_uses_a_secure_socket() {
        assert_eq!(
            teamups_channel_ws_url("https://agentdoctor.cn"),
            "wss://agentdoctor.cn/api/v1/doctor/channel"
        );
        assert_eq!(
            teamups_channel_ws_url("http://127.0.0.1:3000/"),
            "ws://127.0.0.1:3000/api/v1/doctor/channel"
        );
    }

    #[test]
    fn the_site_can_ask_what_this_computer_runs() {
        assert!(parse_channel_choices(r#"{"type":"choices"}"#));
        assert!(!parse_channel_choices(
            r#"{"type":"job","id":"a","text":"t"}"#
        ));
        assert!(!parse_channel_choices("not json"));
    }

    #[test]
    fn a_job_can_name_an_agent_and_a_model() {
        let job = parse_teamups_channel_job(
            r#"{"type":"job","id":"m","text":"跑复盘","chat":"phone:conv-1","runtime":"hermes","model":"deepseek-v4-flash","provider":"prov-2"}"#,
        )
        .unwrap();
        assert_eq!(job.runtime, "hermes");
        assert_eq!(job.model, "deepseek-v4-flash");
        assert_eq!(job.provider, "prov-2");
        let plain = parse_teamups_channel_job(r#"{"type":"job","id":"n","text":"默认"}"#).unwrap();
        assert_eq!(plain.model, "");
        let long = parse_teamups_channel_job(&format!(
            r#"{{"type":"job","id":"o","text":"x","model":"{}"}}"#,
            "m".repeat(400)
        ))
        .unwrap();
        assert_eq!(long.model.chars().count(), 120);
    }

    #[test]
    fn a_job_can_carry_the_auto_allow_request() {
        let job = parse_teamups_channel_job(
            r#"{"type":"job","id":"p","text":"放手做","autoAllow":true}"#,
        )
        .unwrap();
        assert_eq!(job.auto_allow, Some(true));
        let ask =
            parse_teamups_channel_job(r#"{"type":"job","id":"r","text":"问我","autoAllow":false}"#)
                .unwrap();
        assert_eq!(ask.auto_allow, Some(false));
        let plain = parse_teamups_channel_job(r#"{"type":"job","id":"q","text":"照旧"}"#).unwrap();
        assert_eq!(plain.auto_allow, None);
    }

    #[test]
    fn the_phone_can_set_how_a_chat_confirms() {
        let on = parse_channel_auto_allow(r#"{"type":"auto-allow","chat":"phone:c1","on":true}"#);
        assert_eq!(on, Some(("phone:c1".to_string(), Some(true))));
        let off = parse_channel_auto_allow(r#"{"type":"auto-allow","chat":"phone:c1","on":false}"#);
        assert_eq!(off, Some(("phone:c1".to_string(), Some(false))));
        let follow = parse_channel_auto_allow(r#"{"type":"auto-allow","chat":"phone:c1"}"#);
        assert_eq!(follow, Some(("phone:c1".to_string(), None)));
        assert_eq!(parse_channel_auto_allow(r#"{"type":"auto-allow"}"#), None);
        assert_eq!(parse_channel_auto_allow("not json"), None);
        assert_eq!(
            parse_channel_auto_allow(r#"{"type":"job","id":"a","text":"t"}"#),
            None,
        );
    }

    #[test]
    fn a_job_needs_an_id_and_some_text() {
        let job =
            parse_teamups_channel_job(r#"{"type":"job","id":"a","text":"打开项目"}"#).unwrap();
        assert_eq!(job.id, "a");
        assert_eq!(job.text, "打开项目");
        assert_eq!(job.chat, "");
        assert_eq!(job.runtime, "");
        let named = parse_teamups_channel_job(
            r#"{"type":"job","id":"b","text":"下一句","chat":"oc_123","runtime":"qoder"}"#,
        )
        .unwrap();
        assert_eq!(named.chat, "oc_123");
        assert_eq!(named.runtime, "qoder");
        assert!(named.files.is_empty());
        let with_file = parse_teamups_channel_job(
            r#"{"type":"job","id":"c","text":"看看","files":[{"name":"a.png","mime":"image/png","kind":"image","data":"aGVsbG8="}]}"#,
        )
        .unwrap();
        assert_eq!(with_file.files.len(), 1);
        assert_eq!(with_file.files[0].name, "a.png");
        assert_eq!(with_file.files[0].kind, "image");
        assert!(parse_teamups_channel_job(r#"{"type":"job","id":"d","text":""}"#).is_none());
        assert!(parse_channel_stop(r#"{"type":"stop","id":"job-1"}"#).as_deref() == Some("job-1"));
        assert!(parse_channel_stop(r#"{"type":"stop","id":"  "}"#).is_none());
        assert!(parse_teamups_channel_job(r#"{"type":"hello"}"#).is_none());
        assert!(parse_teamups_channel_job(r#"{"type":"job","id":"","text":"hi"}"#).is_none());
    }

    #[test]
    fn hello_names_this_computer() {
        let raw = channel_hello_frame(
            "ad-1",
            "MacBook Pro",
            "macOS",
            "192.168.1.8",
            &serde_json::json!({"chat-1": 1234}),
        );
        let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(value["type"], "hello");
        assert_eq!(value["id"], "ad-1");
        assert_eq!(value["name"], "MacBook Pro");
        assert_eq!(value["os"], "macOS");
        assert_eq!(value["ip"], "192.168.1.8");
        assert_eq!(value["resume"], true);
        assert_eq!(value["since"]["chat-1"], 1234);
        let empty: serde_json::Value = serde_json::from_str(&channel_hello_frame(
            "ad-1",
            "n",
            "os",
            "ip",
            &serde_json::json!({}),
        ))
        .unwrap();
        assert_eq!(empty["since"], serde_json::json!({}));
    }

    #[test]
    fn a_sync_frame_carries_one_conversation() {
        let parsed = parse_teamups_channel_sync(
            r#"{"type":"sync","conversation":{"chat":"chat-9","title":"标题","updatedAt":88,"rows":[{"key":"m:1","kind":"me","text":"你好"}]}}"#,
        )
        .unwrap();
        assert_eq!(parsed["chat"], "chat-9");
        assert_eq!(parsed["updatedAt"], 88);
        assert_eq!(parsed["rows"].as_array().unwrap().len(), 1);
        assert!(parse_teamups_channel_sync(r#"{"type":"job","id":"a","text":"t"}"#).is_none());
        assert!(parse_teamups_channel_sync(r#"{"type":"sync"}"#).is_none());
        assert!(parse_teamups_channel_sync(r#"{"type":"sync","conversation":[]}"#).is_none());
    }

    #[test]
    fn a_reply_names_the_same_job() {
        let raw = teamups_channel_reply("a", "做完了", true, true, "phone:chat-1");
        let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(value["type"], "reply");
        assert_eq!(value["id"], "a");
        assert_eq!(value["ok"], true);
        assert_eq!(value["text"], "做完了");
        assert_eq!(value["chat"], "phone:chat-1");
        let bare: serde_json::Value =
            serde_json::from_str(&teamups_channel_reply("a", "做完了", true, true, "  ")).unwrap();
        assert!(bare.get("chat").is_none());
    }

    #[test]
    fn reply_pictures_are_capped_and_type_checked() {
        use super::{clean_reply_images, REPLY_IMAGE_DATA_MAX};
        let small = serde_json::json!([{ "name": "chart.png", "mime": "image/png", "data": "iVBORw0KGgo=" }]);
        let kept = clean_reply_images(Some(&small));
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0]["mime"], "image/png");

        // A page rendered as PDF, or a data URL wrapper, is not a picture body.
        let bad = serde_json::json!([
            { "name": "a.pdf", "mime": "application/pdf", "data": "iVBORw0KGgo=" },
            { "name": "b.png", "mime": "image/png", "data": "data:image/png;base64,iVBORw0KGgo=" },
            { "name": "c.png", "mime": "image/png", "data": "" },
        ]);
        assert!(clean_reply_images(Some(&bad)).is_empty());

        // One oversized picture does not sink the good one beside it.
        let huge = "A".repeat(REPLY_IMAGE_DATA_MAX + 4);
        let mixed = serde_json::json!([
            { "name": "big.png", "mime": "image/png", "data": huge },
            { "name": "ok.png", "mime": "image/png", "data": "iVBORw0KGgo=" },
        ]);
        let kept = clean_reply_images(Some(&mixed));
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0]["name"], "ok.png");

        // Only three ride along, and a nameless one still gets a label.
        let many = serde_json::json!([
            { "mime": "image/png", "data": "iVBORw0KGgo=" },
            { "mime": "image/png", "data": "iVBORw0KGgo=" },
            { "mime": "image/png", "data": "iVBORw0KGgo=" },
            { "mime": "image/png", "data": "iVBORw0KGgo=" },
        ]);
        let kept = clean_reply_images(Some(&many));
        assert_eq!(kept.len(), 3);
        assert_eq!(kept[0]["name"], "图片");

        assert!(clean_reply_images(None).is_empty());
        assert!(clean_reply_images(Some(&serde_json::json!("x"))).is_empty());
    }

    #[test]
    fn read_reply_image_resolves_relative_paths_and_shrinks_oversized_screenshots() {
        use super::{clean_reply_images, read_reply_image_in, REPLY_IMAGE_DATA_MAX};
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("out");
        std::fs::create_dir_all(&sub).unwrap();

        // Small PNG stays PNG and resolves relative to `cwd`.
        let small_path = sub.join("预览图.png");
        let small_img = image::RgbImage::from_pixel(16, 16, image::Rgb([32, 120, 220]));
        small_img.save(&small_path).unwrap();
        let small =
            read_reply_image_in("out/预览图.png", Some(&dir.path().to_string_lossy())).unwrap();
        assert_eq!(small["name"], "预览图.png");
        assert_eq!(small["mime"], "image/png");
        assert_eq!(
            clean_reply_images(Some(&serde_json::json!([small]))).len(),
            1
        );

        // `file://` URL with percent-encoded CJK name resolves cleanly.
        let file_url = url::Url::from_file_path(&small_path).unwrap();
        let from_url = read_reply_image_in(file_url.as_str(), None).unwrap();
        assert_eq!(from_url["name"], "预览图.png");

        // Medium screenshot (~200 KB PNG, < 675 KB) is ALSO compressed proactively to save bandwidth,
        // and transparent pixels flatten cleanly onto white instead of black.
        let med_path = sub.join("ui-shot.png");
        let mut med_img = image::RgbaImage::new(600, 400);
        for (x, y, pixel) in med_img.enumerate_pixels_mut() {
            let r = ((x * 255) / 600) as u8;
            let g = ((y * 255) / 400) as u8;
            let b = (((x + y) % 211) as u8).wrapping_add(((x / 32) ^ (y / 32)) as u8);
            let a = if x < 24 && y < 24 { 0 } else { 255 };
            *pixel = image::Rgba([r, g, b, a]);
        }
        med_img.save(&med_path).unwrap();
        let med_raw_len = std::fs::metadata(&med_path).unwrap().len() as usize;
        assert!(
            med_raw_len > 80_000 && med_raw_len < REPLY_IMAGE_DATA_MAX / 4 * 3,
            "med_raw_len={med_raw_len}"
        );
        let med = read_reply_image_in(&med_path.to_string_lossy(), None).unwrap();
        assert_eq!(med["mime"], "image/jpeg");
        let med_decoded = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            med["data"].as_str().unwrap(),
        )
        .unwrap();
        assert!(
            med_decoded.len() <= 180_000 && med_decoded.len() * 2 < med_raw_len,
            "med_decoded={} vs med_raw_len={med_raw_len}",
            med_decoded.len()
        );
        let check_white = image::load_from_memory(&med_decoded).unwrap().to_rgb8();
        assert!(check_white.get_pixel(2, 2)[0] > 240);

        // Oversized noisy PNG (> 675 KB raw) is shrunk to JPEG under `REPLY_IMAGE_DATA_MAX`.
        let big_path = sub.join("retina.png");
        let mut big_img = image::RgbImage::new(1400, 1000);
        for (x, y, pixel) in big_img.enumerate_pixels_mut() {
            let v1 = ((x.wrapping_mul(73) ^ y.wrapping_mul(179)) & 0xff) as u8;
            let v2 = ((x.wrapping_mul(199).wrapping_add(y.wrapping_mul(37))) & 0xff) as u8;
            let v3 = ((x ^ y).wrapping_mul(97) & 0xff) as u8;
            *pixel = image::Rgb([v1, v2, v3]);
        }
        big_img.save(&big_path).unwrap();
        assert!(
            std::fs::metadata(&big_path).unwrap().len() as usize > REPLY_IMAGE_DATA_MAX / 4 * 3
        );

        let shrunk = read_reply_image_in(&big_path.to_string_lossy(), None).unwrap();
        assert_eq!(shrunk["name"], "retina.png");
        assert_eq!(shrunk["mime"], "image/jpeg");
        assert!(shrunk["data"].as_str().unwrap().len() <= REPLY_IMAGE_DATA_MAX);
        assert_eq!(
            clean_reply_images(Some(&serde_json::json!([shrunk]))).len(),
            1
        );
    }
}
