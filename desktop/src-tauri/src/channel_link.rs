//! Outbound socket to the TeamUps site. Feishu messages arrive as jobs.

use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use agent_doctor_core::{
    parse_teamups_channel_job, set_teamups_channel_enabled, settings_db_path, teamups_channel_auth,
    teamups_channel_enabled, teamups_channel_heard, teamups_channel_hello, teamups_channel_reply,
    teamups_channel_signed_in,
};
use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use serde_json::json;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::{mpsc, Notify};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, http::HeaderValue, Message},
};

static CONNECTED: AtomicBool = AtomicBool::new(false);
static LOG_LOCK: Mutex<()> = Mutex::new(());

/// Ask the site every so often. A line that answers nothing for `SILENT_LIMIT`
/// is gone, even when the operating system still calls it open.
const BEAT_EVERY: Duration = Duration::from_secs(25);
const SILENT_LIMIT: Duration = Duration::from_secs(70);
const LOG_MAX_BYTES: u64 = 1_000_000;

/// One line per step a Feishu message takes on this computer.
/// `<config>/logs/channel.log`. Starts over past `LOG_MAX_BYTES`.
pub fn note(line: &str) {
    let Some(dir) = settings_db_path().and_then(|p| p.parent().map(|d| d.join("logs"))) else {
        return;
    };
    let _guard = LOG_LOCK.lock();
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let path = dir.join("channel.log");
    let reset = std::fs::metadata(&path)
        .map(|meta| meta.len() > LOG_MAX_BYTES)
        .unwrap_or(false);
    let file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(!reset)
        .truncate(reset)
        .open(&path);
    if let Ok(mut file) = file {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let _ = writeln!(file, "{stamp} {line}");
    }
}

fn clip(text: &str) -> String {
    let flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    flat.chars().take(40).collect()
}

struct Outbound(Mutex<Option<mpsc::UnboundedSender<String>>>);
struct Wake(Arc<Notify>);

#[derive(Clone, Serialize)]
pub struct ChannelPref {
    enabled: bool,
    signed_in: bool,
}

#[derive(Clone, Serialize)]
struct JobPayload {
    id: String,
    text: String,
    chat: String,
}

#[derive(Clone, Serialize)]
struct AnswerPayload {
    #[serde(rename = "requestId")]
    request_id: String,
    text: String,
}

pub fn spawn(app: &AppHandle) {
    app.manage(Outbound(Mutex::new(None)));
    app.manage(Wake(Arc::new(Notify::new())));
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let wake = app.state::<Wake>().0.clone();
        loop {
            // No license, or the person turned the line off: do not dial the site.
            match teamups_channel_auth() {
                Ok(Some((url, license))) => {
                    if let Err(err) = connect(&app, &url, &license).await {
                        if err != "stopped" {
                            eprintln!("[channel] {err}");
                        }
                        note(&format!("line down: {err}"));
                        set_connected(&app, false);
                        tokio::select! {
                            _ = wake.notified() => {}
                            _ = tokio::time::sleep(Duration::from_secs(5)) => {}
                        }
                    }
                }
                _ => {
                    set_connected(&app, false);
                    wake.notified().await;
                }
            }
        }
    });
}

/// Login, logout, or the on/off switch changed. Wakes the loop without opening a socket.
pub fn poke(app: &AppHandle) {
    if let Some(wake) = app.try_state::<Wake>() {
        wake.0.notify_one();
    }
}

fn set_connected(app: &AppHandle, connected: bool) {
    CONNECTED.store(connected, Ordering::SeqCst);
    let _ = app.emit("teamups-channel-status", connected);
}

async fn connect(app: &AppHandle, url: &str, license: &str) -> Result<(), String> {
    let mut request = url.into_client_request().map_err(|err| err.to_string())?;
    let header = format!("Bearer {license}");
    request.headers_mut().insert(
        "Authorization",
        HeaderValue::from_str(&header).map_err(|err| err.to_string())?,
    );
    let connected = tokio::time::timeout(Duration::from_secs(10), connect_async(request))
        .await
        .map_err(|_| "连接超时".to_string())?
        .map_err(|err| err.to_string())?;
    let (socket, _) = connected;
    let (mut write, mut read) = socket.split();
    write
        .send(Message::Text(teamups_channel_hello().into()))
        .await
        .map_err(|err| err.to_string())?;
    let (tx, mut rx) = mpsc::unbounded_channel();
    if let Some(outbound) = app.try_state::<Outbound>() {
        *outbound.0.lock().expect("channel") = Some(tx);
    }
    set_connected(app, true);
    note("line up");
    let wake = app.state::<Wake>().0.clone();
    let mut open = true;
    let mut heard = Instant::now();
    let mut beat = tokio::time::interval(BEAT_EVERY);
    beat.tick().await;
    while open {
        tokio::select! {
            _ = beat.tick() => {
                if heard.elapsed() > SILENT_LIMIT {
                    note("line silent, reconnecting");
                    open = false;
                    continue;
                }
                if write.send(Message::Ping(Vec::new().into())).await.is_err() {
                    open = false;
                }
            }
            outgoing = rx.recv() => {
                let Some(frame) = outgoing else {
                    open = false;
                    continue;
                };
                if write.send(Message::Text(frame.into())).await.is_err() {
                    open = false;
                }
            }
            _ = wake.notified() => {
                // A login poke can arrive while the socket is already up.
                // Close only when the line should actually stop.
                if !matches!(teamups_channel_auth(), Ok(Some(_))) {
                    open = false;
                }
            }
            incoming = read.next() => {
                if matches!(incoming, Some(Ok(_))) {
                    heard = Instant::now();
                }
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        if let Some(job) = parse_teamups_channel_job(&text) {
                            note(&format!(
                                "job {} in chat={} text={}",
                                job.id,
                                if job.chat.is_empty() { "-" } else { job.chat.as_str() },
                                clip(&job.text)
                            ));
                            if write
                                .send(Message::Text(teamups_channel_heard(&job.id).into()))
                                .await
                                .is_err()
                            {
                                open = false;
                            }
                            let _ = app.emit(
                                "teamups-channel-job",
                                JobPayload {
                                    id: job.id,
                                    text: job.text,
                                    chat: job.chat,
                                },
                            );
                        } else if let Some(answer) = parse_channel_answer(&text) {
                            let _ = app.emit("teamups-channel-answer", answer);
                        }
                    }
                    Some(Ok(Message::Ping(data))) => {
                        if write.send(Message::Pong(data)).await.is_err() {
                            open = false;
                        }
                    }
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => open = false,
                    _ => {}
                }
            }
        }
    }
    if let Some(outbound) = app.try_state::<Outbound>() {
        *outbound.0.lock().expect("channel") = None;
    }
    set_connected(app, false);
    Err("stopped".to_string())
}

#[tauri::command]
pub fn teamups_channel_pref_command() -> Result<ChannelPref, String> {
    Ok(ChannelPref {
        enabled: teamups_channel_enabled().map_err(|err| err.to_string())?,
        signed_in: teamups_channel_signed_in().map_err(|err| err.to_string())?,
    })
}

#[tauri::command]
pub fn teamups_channel_set_enabled_command(app: AppHandle, enabled: bool) -> Result<(), String> {
    set_teamups_channel_enabled(enabled).map_err(|err| err.to_string())?;
    poke(&app);
    Ok(())
}

#[tauri::command]
pub fn teamups_channel_connected_command() -> bool {
    CONNECTED.load(Ordering::SeqCst)
}

#[tauri::command]
pub fn teamups_channel_reply_command(
    app: AppHandle,
    id: String,
    text: String,
    ok: bool,
    done: Option<bool>,
) -> Result<(), String> {
    let done = done.unwrap_or(true);
    note(&format!(
        "job {id} reply ok={ok} done={done} text={}",
        clip(&text)
    ));
    let frame = teamups_channel_reply(&id, &text, ok, done);
    let outbound = app
        .try_state::<Outbound>()
        .ok_or_else(|| "飞书这条线还没连上。".to_string())?;
    let guard = outbound.0.lock().expect("channel");
    let tx = guard.as_ref().ok_or_else(|| {
        note(&format!("job {id} reply dropped: line down"));
        "飞书这条线还没连上。".to_string()
    })?;
    tx.send(frame).map_err(|_| "回复没送回官网。".to_string())
}

/// Steps the chat window takes for one Feishu message, for `channel.log`.
#[tauri::command]
pub fn teamups_channel_note_command(id: String, step: String) {
    note(&format!("job {} {}", clip(&id), clip(&step)));
}

#[tauri::command]
pub fn teamups_channel_ask_command(
    app: AppHandle,
    id: String,
    request_id: String,
    text: String,
    wait: Option<bool>,
) -> Result<(), String> {
    let frame = json!({
        "type": "ask",
        "id": id,
        "requestId": request_id,
        "text": text,
        "wait": wait.unwrap_or(true),
    })
    .to_string();
    let outbound = app
        .try_state::<Outbound>()
        .ok_or_else(|| "飞书这条线还没连上。".to_string())?;
    let guard = outbound.0.lock().expect("channel");
    let tx = guard
        .as_ref()
        .ok_or_else(|| "飞书这条线还没连上。".to_string())?;
    tx.send(frame).map_err(|_| "确认没送到飞书。".to_string())
}

fn parse_channel_answer(frame: &str) -> Option<AnswerPayload> {
    let value: serde_json::Value = serde_json::from_str(frame).ok()?;
    if value.get("type").and_then(|item| item.as_str()) != Some("answer") {
        return None;
    }
    let request_id = value
        .get("requestId")
        .and_then(|item| item.as_str())?
        .trim();
    let text = value.get("text").and_then(|item| item.as_str())?.trim();
    if request_id.is_empty() || text.is_empty() {
        return None;
    }
    Some(AnswerPayload {
        request_id: request_id.to_string(),
        text: text.to_string(),
    })
}
