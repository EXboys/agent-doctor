//! Outbound socket to the TeamUps site. Feishu messages arrive as jobs.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use agent_doctor_core::{
    parse_teamups_channel_job, teamups_channel_auth, teamups_channel_hello, teamups_channel_reply,
};
use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::mpsc;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, http::HeaderValue, Message},
};

static CONNECTED: AtomicBool = AtomicBool::new(false);

struct Outbound(Mutex<Option<mpsc::UnboundedSender<String>>>);

#[derive(Clone, Serialize)]
struct JobPayload {
    id: String,
    text: String,
}

pub fn spawn(app: &AppHandle) {
    app.manage(Outbound(Mutex::new(None)));
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            match teamups_channel_auth() {
                Ok(Some((url, _license))) => {
                    if let Err(err) = connect(&app, &url, &_license).await {
                        eprintln!("[channel] {err}");
                        set_connected(&app, false);
                        tokio::time::sleep(Duration::from_secs(5)).await;
                    }
                }
                _ => {
                    set_connected(&app, false);
                    tokio::time::sleep(Duration::from_secs(15)).await;
                }
            }
        }
    });
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
    let mut open = true;
    while open {
        tokio::select! {
            outgoing = rx.recv() => {
                let Some(frame) = outgoing else {
                    open = false;
                    continue;
                };
                if write.send(Message::Text(frame.into())).await.is_err() {
                    open = false;
                }
            }
            incoming = read.next() => {
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        if let Some(job) = parse_teamups_channel_job(&text) {
                            let _ = app.emit(
                                "teamups-channel-job",
                                JobPayload { id: job.id, text: job.text },
                            );
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
    Err("连接已断开".to_string())
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
) -> Result<(), String> {
    let frame = teamups_channel_reply(&id, &text, ok);
    let outbound = app
        .try_state::<Outbound>()
        .ok_or_else(|| "飞书这条线还没连上。".to_string())?;
    let guard = outbound.0.lock().expect("channel");
    let tx = guard
        .as_ref()
        .ok_or_else(|| "飞书这条线还没连上。".to_string())?;
    tx.send(frame).map_err(|_| "回复没送回官网。".to_string())
}
