//! iFlytek cloud ear and voice.
//!
//! One duplex socket turns speech into text and says when a sentence ended.
//! A second synthesis socket reads the assistant's own words. The cloud model's
//! reply and its audio are ignored.

use std::io::ErrorKind;
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::time::Duration;

use agent_doctor_core::teamups_voice_ticket;
use agent_doctor_voice::{self as voice, SpeechError, SpeechErrorCode, SpeechEvent, SpeechResult};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{connect, Error as WsError, Message, WebSocket};

const FRAME_BYTES: usize = 1280;
const LOUD_RMS: f32 = 0.08;
const LOUD_HOLD_MS: u64 = 300;

static CLOUD_SPEAKING: AtomicBool = AtomicBool::new(false);
static CLOUD_BARGE: AtomicBool = AtomicBool::new(false);
static LOUD_MS: AtomicU64 = AtomicU64::new(0);

struct Ticket {
    app_id: String,
    spark_url: String,
    tts_url: String,
}

pub fn credentials_ready() -> bool {
    load_ticket().is_ok()
}

fn load_ticket() -> Result<Ticket, SpeechError> {
    match teamups_voice_ticket() {
        Ok(ticket) => Ok(Ticket {
            app_id: ticket.app_id,
            spark_url: ticket.spark_url,
            tts_url: ticket.tts_url,
        }),
        Err(err) if err.to_string().contains("cloud_unready") => Err(missing()),
        Err(_) => Err(down()),
    }
}

fn missing() -> SpeechError {
    SpeechError::new(SpeechErrorCode::Unavailable, "cloud_unready")
}

fn down() -> SpeechError {
    SpeechError::new(SpeechErrorCode::Failed, "cloud_down")
}

fn emit(app: &AppHandle, event: SpeechEvent) {
    let _ = app.emit("speech-event", event);
}

pub fn listen_once(
    app: &AppHandle,
    should_cancel: &dyn Fn() -> bool,
) -> Result<SpeechResult, SpeechError> {
    let mut heard = None;
    run_ear(app, should_cancel, true, &mut |text| {
        heard = Some(text.to_string())
    })?;
    match heard {
        Some(text) if !text.trim().is_empty() => Ok(SpeechResult {
            text,
            confidence: 1.0,
            is_final: true,
        }),
        _ => Err(SpeechError::new(
            SpeechErrorCode::NoSpeech,
            "no speech detected",
        )),
    }
}

pub fn listen_session(
    app: &AppHandle,
    should_cancel: &dyn Fn() -> bool,
) -> Result<(), SpeechError> {
    run_ear(app, should_cancel, false, &mut |_| {})
}

fn run_ear(
    app: &AppHandle,
    should_cancel: &dyn Fn() -> bool,
    once: bool,
    on_final: &mut dyn FnMut(&str),
) -> Result<(), SpeechError> {
    let ticket = load_ticket()?;
    let (mut socket, _) = connect(ticket.spark_url.as_str()).map_err(|_| down())?;
    set_read_timeout(&mut socket);
    let (audio_tx, audio_rx) = std::sync::mpsc::sync_channel::<(Vec<u8>, f32)>(64);
    let capture_cancel = std::sync::Arc::new(AtomicBool::new(false));
    let capture_flag = capture_cancel.clone();
    let capture_error: std::sync::Arc<std::sync::Mutex<Option<SpeechError>>> =
        std::sync::Arc::new(std::sync::Mutex::new(None));
    let capture_error_thread = capture_error.clone();
    let capture = std::thread::spawn(move || {
        let result = voice::capture_pcm16_mono(
            &|pcm, rms| {
                let _ = audio_tx.try_send((pcm.to_vec(), rms));
            },
            &|| capture_flag.load(Ordering::Relaxed),
        );
        if let Err(err) = result {
            if let Ok(mut slot) = capture_error_thread.lock() {
                *slot = Some(err);
            }
        }
    });

    let mut pending: Vec<u8> = Vec::new();
    let mut started = false;
    let mut latest = String::new();
    let mut quiet_for = 0u64;
    let result = (|| loop {
        if should_cancel() {
            let _ = send_audio(
                &mut socket,
                &ticket,
                &[],
                if started { 2 } else { 2 },
                started,
            );
            return Ok(());
        }
        match audio_rx.recv_timeout(Duration::from_millis(40)) {
            Ok((pcm, rms)) => {
                note_loudness(rms, (pcm.len() / 32) as u64, app);
                pending.extend_from_slice(&pcm);
                quiet_for = 0;
            }
            Err(RecvTimeoutError::Timeout) => {
                quiet_for = quiet_for.saturating_add(40);
            }
            Err(RecvTimeoutError::Disconnected) => {
                let stored = capture_error.lock().ok().and_then(|slot| slot.clone());
                return Err(stored.unwrap_or_else(down));
            }
        }
        if quiet_for >= 80 && pending.is_empty() {
            pending.extend(std::iter::repeat(0).take(FRAME_BYTES));
            quiet_for = 0;
        }
        while pending.len() >= FRAME_BYTES {
            let frame: Vec<u8> = pending.drain(..FRAME_BYTES).collect();
            let status = if started { 1 } else { 0 };
            send_audio(&mut socket, &ticket, &frame, status, started)?;
            started = true;
        }
        match read_message(&mut socket)? {
            Some(text) => {
                if handle_server(app, &text, &mut latest, once, on_final)? && once {
                    return Ok(());
                }
            }
            None => {}
        }
    })();

    capture_cancel.store(true, Ordering::Relaxed);
    let _ = capture.join();
    let _ = socket.close(None);
    result
}

fn note_loudness(rms: f32, frame_ms: u64, app: &AppHandle) {
    if !CLOUD_SPEAKING.load(Ordering::Relaxed) {
        LOUD_MS.store(0, Ordering::Relaxed);
        return;
    }
    let held = if rms >= LOUD_RMS {
        LOUD_MS.fetch_add(frame_ms.max(1), Ordering::Relaxed) + frame_ms.max(1)
    } else {
        let next = LOUD_MS.load(Ordering::Relaxed).saturating_sub(frame_ms / 2);
        LOUD_MS.store(next, Ordering::Relaxed);
        next
    };
    if held >= LOUD_HOLD_MS {
        LOUD_MS.store(0, Ordering::Relaxed);
        CLOUD_BARGE.store(true, Ordering::Relaxed);
        emit(app, SpeechEvent::Barge);
    }
}

fn handle_server(
    app: &AppHandle,
    raw: &str,
    latest: &mut String,
    once: bool,
    on_final: &mut dyn FnMut(&str),
) -> Result<bool, SpeechError> {
    let value: Value = serde_json::from_str(raw).map_err(|_| down())?;
    let code = value
        .pointer("/header/code")
        .and_then(|item| item.as_i64())
        .unwrap_or(0);
    if code != 0 {
        return Err(down());
    }
    if let Some(key) = payload_text(&value, "event").and_then(|text| event_key(&text)) {
        if key == "Eos" && !latest.trim().is_empty() && !CLOUD_SPEAKING.load(Ordering::Relaxed) {
            let text = latest.trim().to_string();
            emit(
                app,
                SpeechEvent::Final {
                    text: text.clone(),
                    confidence: 1.0,
                },
            );
            on_final(&text);
            latest.clear();
            if once {
                return Ok(true);
            }
        }
        if key == "Bos" {
            latest.clear();
        }
    }
    if let Some(text) = payload_text(&value, "iat") {
        if let Some(heard) = iat_text(&text) {
            if heard != *latest {
                *latest = heard;
                emit(
                    app,
                    SpeechEvent::Partial {
                        text: latest.clone(),
                    },
                );
            }
        }
    }
    Ok(false)
}

fn payload_text(value: &Value, field: &str) -> Option<String> {
    let encoded = value.pointer(&format!("/payload/{field}/text"))?.as_str()?;
    let bytes = STANDARD.decode(encoded).ok()?;
    String::from_utf8(bytes).ok()
}

fn event_key(text: &str) -> Option<String> {
    let value: Value = serde_json::from_str(text).ok()?;
    value
        .get("key")
        .and_then(|item| item.as_str())
        .map(str::to_string)
}

fn iat_text(text: &str) -> Option<String> {
    let value: Value = serde_json::from_str(text).ok()?;
    let words = value.get("ws")?.as_array()?;
    let mut heard = String::new();
    for item in words {
        let Some(pieces) = item.get("cw").and_then(|cw| cw.as_array()) else {
            continue;
        };
        for piece in pieces {
            if let Some(word) = piece.get("w").and_then(|item| item.as_str()) {
                heard.push_str(word);
            }
        }
    }
    Some(heard)
}

fn send_audio(
    socket: &mut WebSocket<MaybeTlsStream<TcpStream>>,
    ticket: &Ticket,
    pcm: &[u8],
    audio_status: i32,
    started: bool,
) -> Result<(), SpeechError> {
    let header_status = if !started { 0 } else { audio_status };
    let audio = json!({
        "status": audio_status,
        "audio": STANDARD.encode(pcm),
        "encoding": "raw",
        "sample_rate": 16000,
        "channels": 1,
        "bit_depth": 16,
        "frame_size": 0
    });
    let body = if started {
        json!({
            "header": {
                "app_id": ticket.app_id,
                "uid": "agent-doctor",
                "status": header_status,
                "stmid": "0",
                "scene": "sos_app"
            },
            "payload": { "audio": audio }
        })
    } else {
        json!({
            "header": {
                "app_id": ticket.app_id,
                "uid": "agent-doctor",
                "status": 0,
                "stmid": "0",
                "scene": "sos_app",
                "interact_mode": "continuous"
            },
            "parameter": {
                "iat": {
                    "vgap": 60,
                    "dwa": "wpgs",
                    "iat": { "encoding": "utf8", "compress": "raw", "format": "json" }
                },
                "nlp": {
                    "nlp": { "encoding": "utf8", "compress": "raw", "format": "json" },
                    "new_session": "true"
                },
                "tts": {
                    "vcn": "x5_lingxiaoyue_flow",
                    "speed": 50,
                    "volume": 50,
                    "pitch": 50,
                    "tts": { "encoding": "raw", "sample_rate": 16000, "channels": 1, "bit_depth": 16, "frame_size": 0 }
                }
            },
            "payload": { "audio": audio }
        })
    };
    socket
        .send(Message::Text(body.to_string().into()))
        .map_err(|_| down())
}

pub fn speak(text: &str, should_cancel: &dyn Fn() -> bool) -> Result<(), SpeechError> {
    let spoken = text.trim();
    if spoken.is_empty() {
        return Ok(());
    }
    CLOUD_BARGE.store(false, Ordering::Relaxed);
    CLOUD_SPEAKING.store(true, Ordering::Relaxed);
    let result = (|| {
        let pcm = synthesize(spoken)?;
        voice::play_pcm16_mono(&pcm, &|| {
            should_cancel() || CLOUD_BARGE.load(Ordering::Relaxed)
        })
    })();
    CLOUD_SPEAKING.store(false, Ordering::Relaxed);
    result
}

fn synthesize(text: &str) -> Result<Vec<u8>, SpeechError> {
    let ticket = load_ticket()?;
    let (mut socket, _) = connect(ticket.tts_url.as_str()).map_err(|_| down())?;
    let body = json!({
        "header": { "app_id": ticket.app_id, "status": 2 },
        "parameter": {
            "oral": { "oral_level": "mid" },
            "tts": {
                "vcn": "x5_lingxiaoyue_flow",
                "speed": 50,
                "volume": 50,
                "pitch": 50,
                "bgs": 0,
                "reg": 0,
                "rdn": 0,
                "rhy": 0,
                "audio": {
                    "encoding": "raw",
                    "sample_rate": 16000,
                    "channels": 1,
                    "bit_depth": 16,
                    "frame_size": 0
                }
            }
        },
        "payload": {
            "text": {
                "encoding": "utf8",
                "compress": "raw",
                "format": "plain",
                "status": 2,
                "seq": 0,
                "text": STANDARD.encode(text.as_bytes())
            }
        }
    });
    socket
        .send(Message::Text(body.to_string().into()))
        .map_err(|_| down())?;
    let mut pcm = Vec::new();
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        if std::time::Instant::now() > deadline {
            return Err(down());
        }
        let message = socket.read().map_err(|_| down())?;
        let Message::Text(raw) = message else {
            continue;
        };
        let value: Value = serde_json::from_str(&raw).map_err(|_| down())?;
        let code = value
            .pointer("/header/code")
            .and_then(|item| item.as_i64())
            .unwrap_or(0);
        if code != 0 {
            return Err(down());
        }
        if let Some(encoded) = value
            .pointer("/payload/audio/audio")
            .and_then(|item| item.as_str())
        {
            if let Ok(chunk) = STANDARD.decode(encoded) {
                pcm.extend_from_slice(&chunk);
            }
        }
        let header_done = value
            .pointer("/header/status")
            .and_then(|item| item.as_i64())
            == Some(2);
        let audio_done = value
            .pointer("/payload/audio/status")
            .and_then(|item| item.as_i64())
            == Some(2);
        if header_done || audio_done {
            break;
        }
    }
    if pcm.is_empty() {
        return Err(down());
    }
    Ok(pcm)
}

fn read_message(
    socket: &mut WebSocket<MaybeTlsStream<TcpStream>>,
) -> Result<Option<String>, SpeechError> {
    match socket.read() {
        Ok(Message::Text(text)) => Ok(Some(text.to_string())),
        Ok(Message::Ping(payload)) => {
            let _ = socket.send(Message::Pong(payload));
            Ok(None)
        }
        Ok(_) => Ok(None),
        Err(WsError::Io(err))
            if err.kind() == ErrorKind::WouldBlock || err.kind() == ErrorKind::TimedOut =>
        {
            Ok(None)
        }
        Err(WsError::ConnectionClosed) | Err(WsError::AlreadyClosed) => Err(down()),
        Err(_) => Err(down()),
    }
}

fn set_read_timeout(socket: &mut WebSocket<MaybeTlsStream<TcpStream>>) {
    let timeout = Some(Duration::from_millis(20));
    match socket.get_mut() {
        MaybeTlsStream::Rustls(stream) => {
            let _ = stream.sock.set_read_timeout(timeout);
        }
        MaybeTlsStream::Plain(stream) => {
            let _ = stream.set_read_timeout(timeout);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iat_joins_words() {
        let text = r#"{"ws":[{"cw":[{"w":"改"}]},{"cw":[{"w":"一下"}]}]}"#;
        assert_eq!(iat_text(text).as_deref(), Some("改一下"));
    }
}
