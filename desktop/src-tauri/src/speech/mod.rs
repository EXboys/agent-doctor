//! Thin Tauri bridge over `agent-doctor-voice`.
//! Recognition and synthesis run on blocking threads; this module only emits events.

use std::sync::atomic::{AtomicU64, Ordering};

use agent_doctor_voice::{self as voice, SpeechError, SpeechEvent, SpeechOptions};
use tauri::{AppHandle, Emitter};

pub use voice::{SpeechCapability as SpeechCapabilityDto, SpeechResult as SpeechResultDto};

static LISTEN_EPOCH: AtomicU64 = AtomicU64::new(0);
static LISTEN_CANCEL_EPOCH: AtomicU64 = AtomicU64::new(0);
static DICTATE_CANCEL: AtomicU64 = AtomicU64::new(0);
static DICTATE_EPOCH: AtomicU64 = AtomicU64::new(0);
static SPEAK_EPOCH: AtomicU64 = AtomicU64::new(0);
static SPEAK_CANCEL_EPOCH: AtomicU64 = AtomicU64::new(0);

fn emit_event(app: &AppHandle, event: SpeechEvent) {
    let _ = app.emit("speech-event", event);
}

pub fn capability() -> voice::SpeechCapability {
    voice::capability()
}

pub fn cancel_dictation() {
    let epoch = DICTATE_EPOCH.load(Ordering::SeqCst);
    DICTATE_CANCEL.store(epoch, Ordering::SeqCst);
}

pub fn dictate(
    app: AppHandle,
    language: Option<String>,
) -> Result<voice::SpeechResult, SpeechError> {
    let epoch = DICTATE_EPOCH.fetch_add(1, Ordering::SeqCst) + 1;
    let options = SpeechOptions { language };
    let app_partial = app.clone();
    let result = voice::dictate(
        options,
        &|text| {
            emit_event(
                &app_partial,
                SpeechEvent::Partial {
                    text: text.to_string(),
                },
            );
        },
        &|| DICTATE_CANCEL.load(Ordering::SeqCst) >= epoch,
    );

    match &result {
        Ok(ok) => {
            emit_event(
                &app,
                SpeechEvent::Final {
                    text: ok.text.clone(),
                    confidence: ok.confidence,
                },
            );
        }
        Err(err) if err.code == voice::SpeechErrorCode::Cancelled => {
            emit_event(&app, SpeechEvent::Cancelled);
        }
        Err(err) => {
            emit_event(
                &app,
                SpeechEvent::Error {
                    code: err.code.as_str().into(),
                    detail: err.detail.clone(),
                },
            );
        }
    }
    result
}

pub fn cancel_listen() {
    let epoch = LISTEN_EPOCH.load(Ordering::SeqCst);
    LISTEN_CANCEL_EPOCH.store(epoch, Ordering::SeqCst);
}

pub fn listen(app: AppHandle, language: Option<String>) -> Result<(), SpeechError> {
    let epoch = LISTEN_EPOCH.fetch_add(1, Ordering::SeqCst) + 1;
    let options = SpeechOptions { language };
    let app_partial = app.clone();
    voice::listen_session(
        options,
        &|text| {
            emit_event(
                &app_partial,
                SpeechEvent::Partial {
                    text: text.to_string(),
                },
            );
        },
        &|text| {
            emit_event(
                &app_partial,
                SpeechEvent::Final {
                    text: text.to_string(),
                    confidence: 1.0,
                },
            );
        },
        &|| LISTEN_CANCEL_EPOCH.load(Ordering::SeqCst) >= epoch,
    )
}

pub fn cancel_speak() {
    let epoch = SPEAK_EPOCH.load(Ordering::SeqCst);
    SPEAK_CANCEL_EPOCH.store(epoch, Ordering::SeqCst);
}

pub fn speak(text: String, language: Option<String>) -> Result<(), SpeechError> {
    let epoch = SPEAK_EPOCH.fetch_add(1, Ordering::SeqCst) + 1;
    voice::speak(&text, language.as_deref(), &|| {
        SPEAK_CANCEL_EPOCH.load(Ordering::SeqCst) >= epoch
    })
}
