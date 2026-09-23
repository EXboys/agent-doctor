//! Pluggable on-device speech recognition for Ask composer.
//!
//! Backends (compile-time selected):
//! - macOS: `SFSpeechRecognizer` + `AVAudioEngine`
//! - Windows: `Windows.Media.SpeechRecognition`
//! - other: unavailable stub
//!
//! Session API is shared so the UI stays backend-agnostic.

mod backend;
mod live_ctrl;
mod types;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(any(target_os = "macos", windows)))]
mod unsupported;
#[cfg(windows)]
mod windows;

use backend::active_backend;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter};
use types::{
    SpeechCapability, SpeechError, SpeechErrorCode, SpeechEvent, SpeechOptions, SpeechResult,
};

pub use types::{SpeechCapability as SpeechCapabilityDto, SpeechResult as SpeechResultDto};

static DICTATION_BUSY: AtomicBool = AtomicBool::new(false);
static DICTATION_CANCEL: AtomicBool = AtomicBool::new(false);
static LAST_PARTIAL: Mutex<String> = Mutex::new(String::new());

pub fn capability() -> SpeechCapability {
    active_backend().capability()
}

pub fn cancel_dictation() {
    DICTATION_CANCEL.store(true, Ordering::SeqCst);
}

fn emit_event(app: &AppHandle, event: SpeechEvent) {
    let _ = app.emit("speech-event", event);
}

pub fn dictate(app: AppHandle, language: Option<String>) -> Result<SpeechResult, SpeechError> {
    if DICTATION_BUSY
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Err(SpeechError::new(
            SpeechErrorCode::Busy,
            "dictation already running",
        ));
    }
    DICTATION_CANCEL.store(false, Ordering::SeqCst);
    if let Ok(mut guard) = LAST_PARTIAL.lock() {
        guard.clear();
    }

    let backend = active_backend();
    let options = SpeechOptions { language };

    let result = (|| {
        if !backend.capability().available {
            return Err(SpeechError::new(
                SpeechErrorCode::Unavailable,
                "backend unavailable",
            ));
        }

        let app_partial = app.clone();
        backend.recognize_once(
            options,
            &|text| {
                if let Ok(mut guard) = LAST_PARTIAL.lock() {
                    if *guard == text {
                        return;
                    }
                    *guard = text.to_string();
                }
                emit_event(
                    &app_partial,
                    SpeechEvent::Partial {
                        text: text.to_string(),
                    },
                );
            },
            &|| DICTATION_CANCEL.load(Ordering::SeqCst),
        )
    })();

    DICTATION_BUSY.store(false, Ordering::SeqCst);

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
        Err(err) if err.code == SpeechErrorCode::Cancelled => {
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

/// Test helper: ensure the factory returns a named backend.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_id_is_stable() {
        let id = active_backend().id();
        assert!(!id.is_empty());
    }
}
