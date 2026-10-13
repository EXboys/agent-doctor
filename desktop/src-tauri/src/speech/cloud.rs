//! iFlytek cloud ear and voice.
//!
//! Listening opens one dictation socket per sentence, when the microphone hears
//! someone start talking. The sentence is sent only when iFlytek says the speaker
//! stopped. A second socket reads the assistant's own words.

use std::collections::{BTreeMap, VecDeque};
use std::io::ErrorKind;
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use agent_doctor_core::teamups_voice_ticket;
use agent_doctor_voice::{self as voice, SpeechError, SpeechErrorCode, SpeechEvent, SpeechResult};
use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{connect, Error as WsError, Message, WebSocket};

type Socket = WebSocket<MaybeTlsStream<TcpStream>>;

const FRAME_BYTES: usize = 1280;
const FRAME_MS: u64 = 40;
const BYTES_PER_MS: usize = 32;
/// Audio kept from before the voice gate opened, so the first word is not cut.
const PREROLL_BYTES: usize = 600 * BYTES_PER_MS;
/// Laptop microphones at arm's length often sit near 0.01; a higher gate never opens.
const START_RMS: f32 = 0.008;
const START_HOLD_MS: u64 = 120;
/// iFlytek ends the sentence after this much trailing silence.
const EOS_MS: u64 = 1500;
/// Audio after this much quiet belongs to the next sentence, so a slow reply cannot eat it.
const HANDOFF_MS: u64 = 1000;
/// A socket that has heard no words is asked to finish after this much local quiet.
const QUIET_GIVEUP_MS: u64 = 2500;
/// The transcript often arrives only with the final status, after the end frame.
const QUIET_GIVEUP_WAIT_MS: u64 = 3000;
/// Fallback when iFlytek never says the sentence ended.
const QUIET_FORCE_END_MS: u64 = EOS_MS + 3000;
/// iFlytek stops at 60 s of audio. Roll to a new socket before that and keep the words.
const MAX_SESSION_MS: u64 = 55_000;
/// A connection that lasted this long was working; a later drop is not a refusal.
const HEALTHY_SESSION: Duration = Duration::from_secs(5);
/// Signed links are checked against a 300 s clock window.
const TICKET_TTL: Duration = Duration::from_secs(240);
const LOUD_RMS: f32 = 0.08;
const LOUD_HOLD_MS: u64 = 300;

static CLOUD_SPEAKING: AtomicBool = AtomicBool::new(false);
static CLOUD_BARGE: AtomicBool = AtomicBool::new(false);
static LOUD_MS: AtomicU64 = AtomicU64::new(0);
/// Dictation services this iFlytek app refused. Tried again after `REFUSED_TTL_SECS`.
static REFUSED: AtomicU8 = AtomicU8::new(0);
static REFUSED_SINCE: AtomicU64 = AtomicU64::new(0);
const REFUSED_TTL_SECS: u64 = 30 * 60;
/// A spare socket opened while waiting for a voice. The network here needs 1.5–3 s to connect.
const WARM_TTL: Duration = Duration::from_secs(7);
static TICKET_CACHE: Mutex<Option<(Instant, Ticket)>> = Mutex::new(None);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Dialect {
    SparkV2,
    SparkV1,
    Classic,
}

impl Dialect {
    fn from_kind(kind: &str) -> Option<Self> {
        match kind {
            "spark_v2" => Some(Self::SparkV2),
            "spark_v1" => Some(Self::SparkV1),
            "classic" => Some(Self::Classic),
            _ => None,
        }
    }

    fn bit(self) -> u8 {
        match self {
            Self::SparkV2 => 1,
            Self::SparkV1 => 2,
            Self::Classic => 4,
        }
    }
}

#[derive(Clone)]
struct IatEngine {
    dialect: Dialect,
    url: String,
}

#[derive(Clone)]
struct Ticket {
    app_id: String,
    iat: Vec<IatEngine>,
    tts_url: String,
}

pub fn credentials_ready() -> bool {
    load_ticket().is_ok()
}

fn load_ticket() -> Result<Ticket, SpeechError> {
    if let Ok(slot) = TICKET_CACHE.lock() {
        if let Some((at, ticket)) = slot.as_ref() {
            if at.elapsed() < TICKET_TTL {
                return Ok(ticket.clone());
            }
        }
    }
    let ticket = match teamups_voice_ticket() {
        Ok(ticket) => Ticket {
            app_id: ticket.app_id,
            iat: ticket
                .iat
                .into_iter()
                .filter_map(|item| {
                    Some(IatEngine {
                        dialect: Dialect::from_kind(&item.kind)?,
                        url: item.url,
                    })
                })
                .collect(),
            tts_url: ticket.tts_url,
        },
        Err(err) if err.to_string().contains("cloud_unready") => return Err(missing()),
        Err(_) => return Err(down()),
    };
    if ticket.iat.is_empty() {
        return Err(missing());
    }
    if let Ok(mut slot) = TICKET_CACHE.lock() {
        *slot = Some((Instant::now(), ticket.clone()));
    }
    Ok(ticket)
}

fn missing() -> SpeechError {
    SpeechError::new(SpeechErrorCode::Unavailable, "cloud_unready")
}

fn down() -> SpeechError {
    SpeechError::new(SpeechErrorCode::Failed, "cloud_down")
}

/// 11200 and 10005: this iFlytek app has no licence for the service.
/// 11201: today's quota is used up.
fn refused(code: i64) -> bool {
    matches!(code, 11200 | 11201 | 10005)
}

type Events<'a> = &'a dyn Fn(SpeechEvent);

/// States and iFlytek codes only. The words people say are not written.
fn trace(line: &str) {
    use std::io::Write;
    let path = std::env::temp_dir().join("agent-doctor-cloud-ear.log");
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let _ = writeln!(file, "{stamp} {line}");
    }
}

fn emit(app: Events, event: SpeechEvent) {
    app(event);
}

pub fn listen_once(
    app: &AppHandle,
    should_cancel: &dyn Fn() -> bool,
) -> Result<SpeechResult, SpeechError> {
    let mut heard = None;
    let events = |event| {
        let _ = app.emit("speech-event", event);
    };
    run_ear(&events, should_cancel, true, &mut |text| {
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
    let events = |event| {
        let _ = app.emit("speech-event", event);
    };
    run_ear(&events, should_cancel, false, &mut |_| {})
}

struct Mic {
    rx: Receiver<(Vec<u8>, f32)>,
    error: Arc<Mutex<Option<SpeechError>>>,
}

impl Mic {
    fn next(&self, app: Events) -> Result<Option<(Vec<u8>, f32)>, SpeechError> {
        match self.rx.recv_timeout(Duration::from_millis(FRAME_MS)) {
            Ok((pcm, rms)) => {
                note_loudness(rms, pcm_ms(&pcm), app);
                Ok(Some((pcm, rms)))
            }
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => {
                let stored = self.error.lock().ok().and_then(|slot| slot.clone());
                Err(stored.unwrap_or_else(down))
            }
        }
    }
}

fn unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// A socket being opened in the background so the first word does not wait for the network.
struct Warm {
    dialect: Dialect,
    started: Instant,
    rx: Receiver<Option<Socket>>,
}

impl Warm {
    fn start(engine: &IatEngine) -> Self {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let url = engine.url.clone();
        std::thread::spawn(move || {
            let socket = match connect(url.as_str()) {
                Ok((socket, _)) => Some(socket),
                Err(err) => {
                    trace(&format!("warm connect failed: {err}"));
                    None
                }
            };
            let _ = tx.send(socket);
        });
        Self {
            dialect: engine.dialect,
            started: Instant::now(),
            rx,
        }
    }

    fn take(self, wait: Duration) -> Option<Socket> {
        self.rx.recv_timeout(wait).ok().flatten()
    }
}

fn pcm_ms(pcm: &[u8]) -> u64 {
    ((pcm.len() / BYTES_PER_MS) as u64).max(1)
}

fn run_ear(
    app: Events,
    should_cancel: &dyn Fn() -> bool,
    once: bool,
    on_final: &mut dyn FnMut(&str),
) -> Result<(), SpeechError> {
    if unix_secs().saturating_sub(REFUSED_SINCE.load(Ordering::Relaxed)) >= REFUSED_TTL_SECS {
        REFUSED.store(0, Ordering::Relaxed);
    }
    load_ticket()?;

    let (audio_tx, audio_rx) = std::sync::mpsc::sync_channel::<(Vec<u8>, f32)>(256);
    let capture_cancel = Arc::new(AtomicBool::new(false));
    let capture_flag = capture_cancel.clone();
    let capture_error: Arc<Mutex<Option<SpeechError>>> = Arc::new(Mutex::new(None));
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
    let mic = Mic {
        rx: audio_rx,
        error: capture_error,
    };
    trace("listen started");
    let result = ear_loop(app, &mic, should_cancel, once, on_final);
    if let Err(err) = &result {
        trace(&format!("listen stopped: {}", err.detail));
    }
    capture_cancel.store(true, Ordering::Relaxed);
    let _ = capture.join();
    result
}

fn ear_loop(
    app: Events,
    mic: &Mic,
    should_cancel: &dyn Fn() -> bool,
    once: bool,
    on_final: &mut dyn FnMut(&str),
) -> Result<(), SpeechError> {
    let mut preroll: VecDeque<u8> = VecDeque::new();
    let mut replay: Option<Vec<u8>> = None;
    let mut sentence = Sentence::default();
    let mut failures = 0u8;
    // One spare per service not yet refused, so a refusal hands over without a new connect.
    let mut warm: Vec<Warm> = Vec::new();
    loop {
        if should_cancel() {
            return Ok(());
        }
        let opening = match replay.take() {
            Some(audio) => audio,
            None => {
                // Words kept from a dropped socket reconnect at once; otherwise wait for a voice.
                if sentence.is_empty() {
                    let mut keep_warm = || {
                        let Ok(ticket) = load_ticket() else {
                            return;
                        };
                        let refused = REFUSED.load(Ordering::Relaxed);
                        warm.retain(|spare| {
                            spare.started.elapsed() < WARM_TTL && refused & spare.dialect.bit() == 0
                        });
                        for engine in ticket.iat.iter().filter(|e| refused & e.dialect.bit() == 0) {
                            if !warm.iter().any(|spare| spare.dialect == engine.dialect) {
                                warm.push(Warm::start(engine));
                            }
                        }
                    };
                    if !wait_for_voice(mic, app, &mut preroll, should_cancel, &mut keep_warm)? {
                        return Ok(());
                    }
                }
                preroll.drain(..).collect()
            }
        };
        let ticket = load_ticket()?;
        let engine = pick_engine(&ticket)?;
        let spare = warm
            .iter()
            .position(|spare| spare.dialect == engine.dialect)
            .map(|index| warm.remove(index))
            .filter(|spare| spare.started.elapsed() < WARM_TTL + Duration::from_secs(3))
            .and_then(|spare| spare.take(Duration::from_secs(4)));
        let opened = Instant::now();
        let mut session_audio = Vec::new();
        let mut handoff = Vec::new();
        let outcome = run_session(
            app,
            mic,
            &ticket.app_id,
            &engine,
            spare,
            opening,
            &mut sentence,
            &mut session_audio,
            &mut handoff,
            on_final,
            should_cancel,
        )?;
        trace(&format!("{:?}: {outcome:?}", engine.dialect));
        if replay.is_none() && loud_pcm(&handoff) {
            replay = Some(handoff);
        }
        match outcome {
            Outcome::Cancelled => return Ok(()),
            Outcome::Paused => {
                failures = 0;
            }
            Outcome::Finished { delivered } => {
                failures = 0;
                if once && delivered {
                    return Ok(());
                }
            }
            Outcome::Rolled => {
                failures = 0;
                sentence.keep();
            }
            Outcome::Refused => refuse(&engine, &mut sentence, &mut replay, session_audio),
            Outcome::Dropped {
                heard: false,
                silent: true,
            } if has_fallback(&ticket, &engine) => {
                trace(&format!(
                    "{:?} closed without a reply; trying the next service",
                    engine.dialect
                ));
                refuse(&engine, &mut sentence, &mut replay, session_audio);
            }
            Outcome::Dropped { heard, .. } => {
                // Keep what was heard. A dropped socket never sends the sentence.
                if heard {
                    sentence.keep();
                } else {
                    replay = Some(session_audio);
                }
                if opened.elapsed() >= HEALTHY_SESSION {
                    failures = 0;
                } else {
                    failures = failures.saturating_add(1);
                }
                if failures >= 3 {
                    return Err(down());
                }
                std::thread::sleep(Duration::from_millis(200));
            }
        }
    }
}

fn refuse(
    engine: &IatEngine,
    sentence: &mut Sentence,
    replay: &mut Option<Vec<u8>>,
    session_audio: Vec<u8>,
) {
    if REFUSED.fetch_or(engine.dialect.bit(), Ordering::Relaxed) == 0 {
        REFUSED_SINCE.store(unix_secs(), Ordering::Relaxed);
    }
    sentence.forget_session();
    *replay = Some(session_audio);
}

fn has_fallback(ticket: &Ticket, engine: &IatEngine) -> bool {
    let refused = REFUSED.load(Ordering::Relaxed) | engine.dialect.bit();
    ticket
        .iat
        .iter()
        .any(|other| refused & other.dialect.bit() == 0)
}

fn pick_engine(ticket: &Ticket) -> Result<IatEngine, SpeechError> {
    let refused = REFUSED.load(Ordering::Relaxed);
    ticket
        .iat
        .iter()
        .find(|engine| refused & engine.dialect.bit() == 0)
        .cloned()
        .ok_or_else(missing)
}

/// Returns false when cancelled. Nothing is sent to iFlytek while waiting.
fn wait_for_voice(
    mic: &Mic,
    app: Events,
    preroll: &mut VecDeque<u8>,
    should_cancel: &dyn Fn() -> bool,
    keep_warm: &mut dyn FnMut(),
) -> Result<bool, SpeechError> {
    let mut loud = 0u64;
    let mut peak = 0f32;
    let mut frames = 0u64;
    let mut reported = Instant::now();
    loop {
        if should_cancel() {
            return Ok(false);
        }
        keep_warm();
        if reported.elapsed() >= Duration::from_secs(5) {
            trace(&format!(
                "waiting: {frames} mic frames, peak level {peak:.4}"
            ));
            peak = 0.0;
            frames = 0;
            reported = Instant::now();
        }
        let Some((pcm, rms)) = mic.next(app)? else {
            continue;
        };
        frames += 1;
        peak = peak.max(rms);
        let ms = pcm_ms(&pcm);
        preroll.extend(pcm);
        if preroll.len() > PREROLL_BYTES {
            let extra = preroll.len() - PREROLL_BYTES;
            preroll.drain(..extra);
        }
        if CLOUD_SPEAKING.load(Ordering::Relaxed) {
            loud = 0;
            continue;
        }
        loud = if rms >= START_RMS { loud + ms } else { 0 };
        if loud >= START_HOLD_MS {
            trace("voice heard, opening dictation");
            return Ok(true);
        }
    }
}

#[derive(Debug)]
enum Outcome {
    Finished {
        delivered: bool,
    },
    /// The assistant is speaking, so this listen is not the user's sentence.
    Paused,
    /// This service is not licensed for the app. Try the next one with the same audio.
    Refused,
    /// `silent`: the socket opened but closed before any reply, the way a refusal sometimes looks.
    Dropped {
        heard: bool,
        silent: bool,
    },
    /// Near the 60 s limit; the words carry over to a new socket.
    Rolled,
    Cancelled,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Closing {
    Roll,
    End,
}

#[allow(clippy::too_many_arguments)]
fn run_session(
    app: Events,
    mic: &Mic,
    app_id: &str,
    engine: &IatEngine,
    spare: Option<Socket>,
    opening: Vec<u8>,
    sentence: &mut Sentence,
    session_audio: &mut Vec<u8>,
    handoff: &mut Vec<u8>,
    on_final: &mut dyn FnMut(&str),
    should_cancel: &dyn Fn() -> bool,
) -> Result<Outcome, SpeechError> {
    session_audio.extend_from_slice(&opening);
    let reused = spare.is_some();
    let connecting = Instant::now();
    let mut socket = match spare {
        Some(socket) => socket,
        None => match connect(engine.url.as_str()) {
            Ok((socket, _)) => socket,
            Err(WsError::Http(response)) => {
                trace(&format!("handshake refused {}", response.status()));
                return Ok(Outcome::Refused);
            }
            Err(err) => {
                trace(&format!("connect failed: {err}"));
                return Ok(Outcome::Dropped {
                    heard: false,
                    silent: false,
                });
            }
        },
    };
    trace(&format!(
        "{:?} socket ready in {} ms (spare: {reused}), opening {} ms of audio",
        engine.dialect,
        connecting.elapsed().as_millis(),
        opening.len() / BYTES_PER_MS
    ));
    set_nonblocking(&mut socket);
    let mut sender = Sender {
        dialect: engine.dialect,
        app_id,
        seq: 0,
        started: false,
        replies: 0,
    };
    let outcome = session_loop(
        app,
        mic,
        &mut socket,
        &mut sender,
        opening,
        sentence,
        session_audio,
        handoff,
        on_final,
        should_cancel,
    );
    let _ = socket.close(None);
    trace(&format!(
        "session: {} ms of audio, {} frames sent, {} replies, peak level {:.4}",
        session_audio.len() / BYTES_PER_MS,
        sender.seq,
        sender.replies,
        peak_level(session_audio)
    ));
    let _ = std::fs::write(
        std::env::temp_dir().join("agent-doctor-last-ear.pcm"),
        &session_audio,
    );
    outcome
}

fn peak_level(pcm: &[u8]) -> f32 {
    pcm.chunks(FRAME_BYTES)
        .map(|chunk| {
            let mut sum = 0f32;
            let mut n = 0usize;
            for b in chunk.chunks_exact(2) {
                let s = i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0;
                sum += s * s;
                n += 1;
            }
            if n == 0 {
                0.0
            } else {
                (sum / n as f32).sqrt()
            }
        })
        .fold(0.0, f32::max)
}

#[allow(clippy::too_many_arguments)]
fn session_loop(
    app: Events,
    mic: &Mic,
    socket: &mut Socket,
    sender: &mut Sender,
    opening: Vec<u8>,
    sentence: &mut Sentence,
    session_audio: &mut Vec<u8>,
    handoff: &mut Vec<u8>,
    on_final: &mut dyn FnMut(&str),
    should_cancel: &dyn Fn() -> bool,
) -> Result<Outcome, SpeechError> {
    let mut pending = opening;
    let mut sent_ms = 0u64;
    let mut quiet_ms = 0u64;
    let mut heard = false;
    let mut answered = false;
    let mut closing: Option<Closing> = None;
    // When the quiet give-up sent its end frame. The final words are still read until this expires.
    let mut quiet_end_at: Option<Instant> = None;
    let begun = Instant::now();
    loop {
        if should_cancel() {
            let _ = sender.finish(socket);
            return Ok(Outcome::Cancelled);
        }
        if CLOUD_SPEAKING.load(Ordering::Relaxed) {
            if !heard {
                return Ok(Outcome::Paused);
            }
            if closing.is_none() {
                let _ = sender.finish(socket);
                closing = Some(Closing::End);
            }
        }
        if closing.is_none() {
            while pending.len() >= FRAME_BYTES {
                let frame: Vec<u8> = pending.drain(..FRAME_BYTES).collect();
                if sender.audio(socket, &frame).is_err() {
                    return Ok(Outcome::Dropped {
                        heard,
                        silent: !answered,
                    });
                }
                sent_ms += FRAME_MS;
            }
            if sent_ms >= MAX_SESSION_MS {
                if sender.finish(socket).is_err() {
                    return Ok(Outcome::Dropped {
                        heard,
                        silent: !answered,
                    });
                }
                closing = Some(Closing::Roll);
            }
        }
        loop {
            let raw = match read_message(socket) {
                Ok(Some(raw)) => raw,
                Ok(None) => break,
                Err(_) => {
                    trace("socket closed while reading");
                    return Ok(Outcome::Dropped {
                        heard,
                        silent: !answered,
                    });
                }
            };
            answered = true;
            sender.replies += 1;
            let parsed = parse_reply(sender.dialect, &raw);
            if let Err(code) = parsed {
                trace(&format!("iFlytek code {code}"));
            }
            let reply = match parsed {
                Ok(reply) => reply,
                Err(code) if refused(code) && !heard => return Ok(Outcome::Refused),
                Err(_) => {
                    return Ok(Outcome::Dropped {
                        heard,
                        silent: !answered,
                    })
                }
            };
            if let Some(piece) = reply.piece {
                let before = sentence.text();
                sentence.apply(piece);
                if !heard {
                    trace(&format!(
                        "first words after {} ms",
                        begun.elapsed().as_millis()
                    ));
                }
                heard = true;
                let now = sentence.text();
                if now != before && !now.trim().is_empty() {
                    emit(app, SpeechEvent::Partial { text: now });
                }
            }
            if reply.done {
                if closing == Some(Closing::Roll) {
                    return Ok(Outcome::Rolled);
                }
                handoff.extend_from_slice(&pending);
                let delivered = deliver(app, sentence, on_final);
                return Ok(Outcome::Finished { delivered });
            }
        }
        // Quiet counts only once earlier audio has left; buffered silence is not yet heard.
        let caught_up = pending.len() < FRAME_BYTES;
        if let Some((pcm, rms)) = mic.next(app)? {
            let ms = pcm_ms(&pcm);
            if rms >= START_RMS {
                quiet_ms = 0;
            } else if caught_up {
                quiet_ms += ms;
            }
            session_audio.extend_from_slice(&pcm);
            let parking = !handoff.is_empty() || quiet_ms >= HANDOFF_MS;
            if closing.is_some() || parking {
                handoff.extend_from_slice(&pcm);
                if closing.is_none() {
                    pending.extend(std::iter::repeat(0).take(pcm.len()));
                }
            } else {
                pending.extend_from_slice(&pcm);
            }
        }
        if !caught_up {
            continue;
        }
        if quiet_end_at.is_none()
            && !heard
            && closing.is_none()
            && quiet_ms >= QUIET_GIVEUP_MS
            && begun.elapsed() >= Duration::from_millis(QUIET_GIVEUP_MS)
        {
            if sender.finish(socket).is_err() {
                return Ok(Outcome::Dropped {
                    heard,
                    silent: !answered,
                });
            }
            closing = Some(Closing::End);
            quiet_end_at = Some(Instant::now());
        }
        if quiet_end_at
            .is_some_and(|at| at.elapsed() >= Duration::from_millis(QUIET_GIVEUP_WAIT_MS))
        {
            let delivered = deliver(app, sentence, on_final);
            return Ok(Outcome::Finished { delivered });
        }
        if closing.is_none() && quiet_ms >= QUIET_FORCE_END_MS {
            if sender.finish(socket).is_err() {
                return Ok(Outcome::Dropped {
                    heard,
                    silent: !answered,
                });
            }
            closing = Some(Closing::End);
        }
    }
}

fn loud_pcm(pcm: &[u8]) -> bool {
    let mut sum = 0f32;
    let mut n = 0usize;
    for chunk in pcm.chunks_exact(2) {
        let sample = i16::from_le_bytes([chunk[0], chunk[1]]) as f32 / 32768.0;
        sum += sample * sample;
        n += 1;
    }
    n > 0 && (sum / n as f32).sqrt() >= START_RMS
}

fn deliver(app: Events, sentence: &mut Sentence, on_final: &mut dyn FnMut(&str)) -> bool {
    let text = sentence.take();
    if text.is_empty() {
        return false;
    }
    emit(
        app,
        SpeechEvent::Final {
            text: text.clone(),
            confidence: 1.0,
        },
    );
    on_final(&text);
    true
}

fn note_loudness(rms: f32, frame_ms: u64, app: Events) {
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

/// Words of one sentence. `kept` survives a dropped or rolled socket.
#[derive(Default)]
struct Sentence {
    kept: String,
    pieces: BTreeMap<i64, String>,
}

impl Sentence {
    fn apply(&mut self, piece: Piece) {
        if piece.pgs.as_deref() == Some("rpl") {
            if let Some((from, to)) = piece.rg {
                self.pieces.retain(|sn, _| *sn < from || *sn > to);
            }
        }
        self.pieces.insert(piece.sn, piece.text);
    }

    fn text(&self) -> String {
        let mut text = self.kept.clone();
        for piece in self.pieces.values() {
            text.push_str(piece);
        }
        text
    }

    fn keep(&mut self) {
        self.kept = self.text();
        self.pieces.clear();
    }

    fn forget_session(&mut self) {
        self.pieces.clear();
    }

    fn take(&mut self) -> String {
        let text = self.text().trim().to_string();
        self.kept.clear();
        self.pieces.clear();
        text
    }

    fn is_empty(&self) -> bool {
        self.text().trim().is_empty()
    }
}

#[derive(Debug)]
struct Piece {
    sn: i64,
    pgs: Option<String>,
    rg: Option<(i64, i64)>,
    text: String,
}

#[derive(Debug, Default)]
struct Reply {
    piece: Option<Piece>,
    done: bool,
}

/// One server message. `Err` carries a non-zero iFlytek code.
fn parse_reply(dialect: Dialect, raw: &str) -> Result<Reply, i64> {
    let Ok(value) = serde_json::from_str::<Value>(raw) else {
        return Ok(Reply::default());
    };
    match dialect {
        Dialect::Classic => {
            let code = value.get("code").and_then(Value::as_i64).unwrap_or(0);
            if code != 0 {
                return Err(code);
            }
            Ok(Reply {
                piece: value.pointer("/data/result").and_then(piece_from),
                done: value.pointer("/data/status").and_then(Value::as_i64) == Some(2),
            })
        }
        Dialect::SparkV1 | Dialect::SparkV2 => {
            let code = value
                .pointer("/header/code")
                .and_then(Value::as_i64)
                .unwrap_or(0);
            if code != 0 {
                return Err(code);
            }
            let done = value.pointer("/header/status").and_then(Value::as_i64) == Some(2)
                || value
                    .pointer("/payload/result/status")
                    .and_then(Value::as_i64)
                    == Some(2);
            let piece = value
                .pointer("/payload/result/text")
                .and_then(Value::as_str)
                .and_then(|encoded| STANDARD.decode(encoded).ok())
                .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                .and_then(|inner| {
                    let mut piece = piece_from(&inner)?;
                    if piece.pgs.is_none() {
                        piece.pgs = value
                            .pointer("/payload/result/pgs")
                            .and_then(Value::as_str)
                            .map(str::to_string);
                    }
                    if piece.rg.is_none() {
                        piece.rg = range(value.pointer("/payload/result/rg"));
                    }
                    Some(piece)
                });
            Ok(Reply { piece, done })
        }
    }
}

fn piece_from(value: &Value) -> Option<Piece> {
    let words = value.get("ws")?.as_array()?;
    let text: String = words
        .iter()
        .filter_map(|word| word.get("cw")?.as_array()?.first()?.get("w")?.as_str())
        .collect();
    Some(Piece {
        sn: value.get("sn").and_then(Value::as_i64).unwrap_or(0),
        pgs: value.get("pgs").and_then(Value::as_str).map(str::to_string),
        rg: range(value.get("rg")),
        text,
    })
}

fn range(value: Option<&Value>) -> Option<(i64, i64)> {
    let items = value?.as_array()?;
    Some((items.first()?.as_i64()?, items.get(1)?.as_i64()?))
}

struct Sender<'a> {
    dialect: Dialect,
    app_id: &'a str,
    seq: u32,
    started: bool,
    replies: u32,
}

impl Sender<'_> {
    fn audio(&mut self, socket: &mut Socket, pcm: &[u8]) -> Result<(), SpeechError> {
        let status = if self.started { 1 } else { 0 };
        self.seq += 1;
        self.started = true;
        send_text(
            socket,
            frame(self.dialect, self.app_id, pcm, status, self.seq),
        )
    }

    fn finish(&mut self, socket: &mut Socket) -> Result<(), SpeechError> {
        if !self.started {
            return Ok(());
        }
        self.seq += 1;
        send_text(socket, frame(self.dialect, self.app_id, &[], 2, self.seq))
    }
}

fn frame(dialect: Dialect, app_id: &str, pcm: &[u8], status: i32, seq: u32) -> String {
    let audio = STANDARD.encode(pcm);
    let body = match dialect {
        Dialect::Classic => {
            let data = json!({
                "status": status,
                "format": "audio/L16;rate=16000",
                "encoding": "raw",
                "audio": audio
            });
            if status == 0 {
                json!({
                    "common": { "app_id": app_id },
                    "business": {
                        "language": "zh_cn",
                        "domain": "iat",
                        "accent": "mandarin",
                        "vad_eos": EOS_MS,
                        "dwa": "wpgs"
                    },
                    "data": data
                })
            } else {
                json!({ "data": data })
            }
        }
        Dialect::SparkV1 | Dialect::SparkV2 => {
            let header = json!({ "app_id": app_id, "status": status });
            let payload = json!({
                "audio": {
                    "encoding": "raw",
                    "sample_rate": 16000,
                    "channels": 1,
                    "bit_depth": 16,
                    "seq": seq,
                    "status": status,
                    "audio": audio
                }
            });
            if status == 0 {
                let domain = if dialect == Dialect::SparkV2 {
                    "spark_asr_v2"
                } else {
                    "slm"
                };
                json!({
                    "header": header,
                    "parameter": {
                        "iat": {
                            "domain": domain,
                            "language": "zh_cn",
                            "accent": "mandarin",
                            "eos": EOS_MS,
                            "dwa": "wpgs",
                            "result": { "encoding": "utf8", "compress": "raw", "format": "json" }
                        }
                    },
                    "payload": payload
                })
            } else {
                json!({ "header": header, "payload": payload })
            }
        }
    };
    body.to_string()
}

/// Queues the frame once, then waits for it to leave. Retrying `send` would queue it twice.
fn send_text(socket: &mut Socket, body: String) -> Result<(), SpeechError> {
    match socket.write(Message::Text(body.into())) {
        Ok(()) => {}
        Err(WsError::Io(err)) if err.kind() == ErrorKind::WouldBlock => {}
        Err(err) => {
            trace(&format!("send failed: {err}"));
            return Err(down());
        }
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match socket.flush() {
            Ok(()) => return Ok(()),
            Err(WsError::Io(err)) if err.kind() == ErrorKind::WouldBlock => {
                if Instant::now() > deadline {
                    trace("send stalled");
                    return Err(down());
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(err) => {
                trace(&format!("send failed: {err}"));
                return Err(down());
            }
        }
    }
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
                "vcn": "x6_lingxiaoyue_pro",
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
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if Instant::now() > deadline {
            return Err(down());
        }
        let raw = match socket.read().map_err(|_| down())? {
            Message::Text(text) => text.to_string(),
            Message::Binary(bytes) => match String::from_utf8(bytes.to_vec()) {
                Ok(text) => text,
                Err(_) => continue,
            },
            _ => continue,
        };
        let value: Value = serde_json::from_str(&raw).map_err(|_| down())?;
        let code = value
            .pointer("/header/code")
            .and_then(|item| item.as_i64())
            .unwrap_or(0);
        if code != 0 {
            let message = value
                .pointer("/header/message")
                .and_then(|item| item.as_str())
                .unwrap_or("");
            trace(&format!("speak code {code}: {message}"));
            return Err(if refused(code) { missing() } else { down() });
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

fn read_message(socket: &mut Socket) -> Result<Option<String>, SpeechError> {
    match socket.read() {
        Ok(Message::Text(text)) => Ok(Some(text.to_string())),
        Ok(Message::Binary(bytes)) => Ok(String::from_utf8(bytes.to_vec()).ok()),
        Ok(Message::Ping(payload)) => {
            let _ = socket.send(Message::Pong(payload));
            Ok(None)
        }
        Ok(Message::Close(frame)) => {
            trace(&format!("server closed: {frame:?}"));
            Ok(None)
        }
        Ok(_) => Ok(None),
        Err(WsError::Io(err))
            if err.kind() == ErrorKind::WouldBlock || err.kind() == ErrorKind::TimedOut =>
        {
            Ok(None)
        }
        Err(err) => {
            trace(&format!("read failed: {err}"));
            Err(down())
        }
    }
}

fn set_nonblocking(socket: &mut Socket) {
    // A short read timeout can cut a frame in half and the next read looks like
    // the connection died. Non-blocking reads leave the partial frame in place.
    match socket.get_mut() {
        MaybeTlsStream::Rustls(stream) => {
            let _ = stream.sock.set_nonblocking(true);
        }
        MaybeTlsStream::Plain(stream) => {
            let _ = stream.set_nonblocking(true);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spark_message(inner: &str, status: i64) -> String {
        json!({
            "header": { "code": 0, "status": status },
            "payload": { "result": { "status": status, "text": STANDARD.encode(inner.as_bytes()) } }
        })
        .to_string()
    }

    #[test]
    fn spark_reply_reads_words_and_end() {
        let raw = spark_message(
            r#"{"sn":1,"ls":true,"ws":[{"cw":[{"w":"改"}]},{"cw":[{"w":"一下"}]}]}"#,
            2,
        );
        let reply = parse_reply(Dialect::SparkV2, &raw).unwrap();
        assert_eq!(reply.piece.unwrap().text, "改一下");
        assert!(reply.done);
    }

    #[test]
    fn classic_reply_reads_words() {
        let raw = json!({
            "code": 0,
            "data": { "status": 1, "result": { "sn": 1, "pgs": "apd", "ws": [{ "cw": [{ "w": "你好" }] }] } }
        })
        .to_string();
        let reply = parse_reply(Dialect::Classic, &raw).unwrap();
        assert_eq!(reply.piece.unwrap().text, "你好");
        assert!(!reply.done);
    }

    #[test]
    fn unlicensed_service_is_refused() {
        let spark = json!({ "header": { "code": 11200, "status": 2 } }).to_string();
        let classic = json!({ "code": 10005, "message": "licc fail" }).to_string();
        assert!(refused(parse_reply(Dialect::SparkV2, &spark).unwrap_err()));
        assert!(refused(
            parse_reply(Dialect::Classic, &classic).unwrap_err()
        ));
    }

    fn piece(sn: i64, pgs: &str, rg: Option<(i64, i64)>, text: &str) -> Piece {
        Piece {
            sn,
            pgs: Some(pgs.into()),
            rg,
            text: text.into(),
        }
    }

    #[test]
    fn correction_replaces_the_given_range() {
        let mut sentence = Sentence::default();
        sentence.apply(piece(1, "apd", None, "帮我"));
        sentence.apply(piece(2, "apd", None, "看衣"));
        sentence.apply(piece(3, "rpl", Some((2, 2)), "看一下"));
        assert_eq!(sentence.text(), "帮我看一下");
    }

    #[test]
    fn dropped_socket_keeps_words_and_new_socket_appends() {
        let mut sentence = Sentence::default();
        sentence.apply(piece(1, "apd", None, "帮我看一下"));
        sentence.keep();
        sentence.apply(piece(1, "rpl", Some((1, 1)), "这个"));
        assert_eq!(sentence.text(), "帮我看一下这个");
        assert_eq!(sentence.take(), "帮我看一下这个");
        assert!(sentence.is_empty());
    }

    #[test]
    fn refused_service_does_not_keep_its_words() {
        let mut sentence = Sentence {
            kept: "前面".into(),
            ..Default::default()
        };
        sentence.apply(piece(1, "apd", None, "半句"));
        sentence.forget_session();
        assert_eq!(sentence.text(), "前面");
    }

    #[test]
    fn first_spark_frame_names_the_model() {
        let first: Value =
            serde_json::from_str(&frame(Dialect::SparkV2, "app", &[0; 4], 0, 1)).unwrap();
        assert_eq!(
            first.pointer("/parameter/iat/domain").unwrap(),
            "spark_asr_v2"
        );
        let next: Value =
            serde_json::from_str(&frame(Dialect::SparkV2, "app", &[0; 4], 1, 2)).unwrap();
        assert!(next.get("parameter").is_none());
        let classic: Value =
            serde_json::from_str(&frame(Dialect::Classic, "app", &[0; 4], 0, 1)).unwrap();
        assert_eq!(classic.pointer("/business/domain").unwrap(), "iat");
    }

    fn rms16(pcm: &[u8]) -> f32 {
        let samples: Vec<f32> = pcm
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0)
            .collect();
        if samples.is_empty() {
            return 0.0;
        }
        (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
    }

    /// Feeds a spoken sentence through the real ear loop as if it came from the microphone.
    /// Needs `/tmp/agent-doctor-ear.pcm` (16 kHz mono s16le), e.g. from `say` + `afconvert`.
    #[test]
    #[ignore]
    fn live_ear_with_recorded_speech() {
        let speech = std::fs::read("/tmp/agent-doctor-ear.pcm").expect("pcm");
        let (tx, rx) = std::sync::mpsc::sync_channel::<(Vec<u8>, f32)>(256);
        let feeder = std::thread::spawn(move || {
            let silence = vec![0u8; 3200];
            for _ in 0..30 {
                let _ = tx.send((silence.clone(), 0.0));
                std::thread::sleep(Duration::from_millis(100));
            }
            for _round in 0..2 {
                for chunk in speech.chunks(3200) {
                    let _ = tx.send((chunk.to_vec(), rms16(chunk)));
                    std::thread::sleep(Duration::from_millis(100));
                }
                for _ in 0..25 {
                    let _ = tx.send((silence.clone(), 0.0));
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
            for _ in 0..20 {
                let _ = tx.send((silence.clone(), 0.0));
                std::thread::sleep(Duration::from_millis(100));
            }
        });
        let mic = Mic {
            rx,
            error: Arc::new(Mutex::new(None)),
        };
        let events = |event: SpeechEvent| match event {
            SpeechEvent::Partial { text } => println!("partial: {text}"),
            SpeechEvent::Final { text, .. } => println!("final: {text}"),
            _ => {}
        };
        if let Ok(skip) = std::env::var("EAR_SKIP") {
            REFUSED.store(skip.parse().unwrap_or(0), Ordering::Relaxed);
        }
        let started = Instant::now();
        let mut finals = Vec::new();
        let result = ear_loop(
            &events,
            &mic,
            &|| started.elapsed() > Duration::from_secs(40),
            false,
            &mut |text| finals.push(text.to_string()),
        );
        println!(
            "result: {:?}, finals: {finals:?}",
            result.err().map(|e| e.detail)
        );
        drop(mic);
        let _ = feeder.join();
    }

    /// Real iFlytek check with the saved TeamUps login. Prints only service kinds and codes.
    /// `cargo test -p agent-doctor-desktop --no-default-features --lib live_cloud_probe -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_cloud_probe() {
        let ticket = load_ticket().expect("ticket");
        for engine in &ticket.iat {
            let label = format!("{:?}", engine.dialect);
            let mut socket = match connect(engine.url.as_str()) {
                Ok((socket, _)) => socket,
                Err(WsError::Http(response)) => {
                    println!("{label}: handshake refused {}", response.status());
                    continue;
                }
                Err(_) => {
                    println!("{label}: no connection");
                    continue;
                }
            };
            set_nonblocking(&mut socket);
            let mut sender = Sender {
                dialect: engine.dialect,
                app_id: &ticket.app_id,
                seq: 0,
                started: false,
                replies: 0,
            };
            for _ in 0..25 {
                sender
                    .audio(&mut socket, &[0u8; FRAME_BYTES])
                    .expect("send");
            }
            sender.finish(&mut socket).expect("finish");
            let opened = Instant::now();
            let mut verdict = "no reply".to_string();
            while opened.elapsed() < Duration::from_secs(4) {
                match read_message(&mut socket) {
                    Ok(Some(raw)) => match parse_reply(engine.dialect, &raw) {
                        Ok(reply) => {
                            verdict = "ok".into();
                            if reply.done {
                                break;
                            }
                        }
                        Err(code) => {
                            verdict = format!("code {code}");
                            break;
                        }
                    },
                    Ok(None) => std::thread::sleep(Duration::from_millis(20)),
                    Err(_) => {
                        verdict = format!("{verdict}, then closed");
                        break;
                    }
                }
            }
            println!("{label}: {verdict}");
        }
        match synthesize("你好") {
            Ok(pcm) => println!("speak ok: {} bytes", pcm.len()),
            Err(err) => println!("speak failed: {}", err.detail),
        }
    }
}
