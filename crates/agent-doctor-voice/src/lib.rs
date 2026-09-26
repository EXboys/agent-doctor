//! On-device speech for Ask.
//!
//! Backends:
//! - macOS: `SFSpeechRecognizer` + `AVSpeechSynthesizer`
//! - Windows: `Windows.Media.SpeechRecognition` + `SpeechSynthesis`
//! - other: unavailable
//!
//! Hosted-mode wording and the listen/speak state machine live in [`policy`]
//! so the desktop app can drive them without a microphone.

mod backend;
mod live_ctrl;
#[cfg(target_os = "macos")]
mod macos;
mod policy;
mod turn_end;
#[cfg(not(any(target_os = "macos", windows)))]
mod unsupported;
#[cfg(windows)]
mod windows;

use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use backend::active_backend;

pub use policy::{
    ack_sentence, asks_about_permission, classify_decision, classify_utterance, failure_sentence,
    permission_brief, permission_sentence, reduce, spoken_reply, status_sentence, HostedEffect,
    HostedInput, HostedState, HostedStep, UtteranceKind,
};
pub use turn_end::{
    assess_turn_end, set_turn_end_decider, RuleTurnEnd, TurnEnd, TurnEndDecider, COMPLETE_HOLD_MS,
    THINKING_HOLD_MS,
};
pub use types::{
    SpeechCapability, SpeechError, SpeechErrorCode, SpeechEvent, SpeechOptions, SpeechResult,
};

mod types;

static RECOGNIZER_BUSY: AtomicBool = AtomicBool::new(false);

pub fn capability() -> SpeechCapability {
    active_backend().capability()
}

fn with_recognizer<T>(work: impl FnOnce() -> Result<T, SpeechError>) -> Result<T, SpeechError> {
    if RECOGNIZER_BUSY
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Err(SpeechError::new(
            SpeechErrorCode::Busy,
            "speech recognition already running",
        ));
    }
    let result = work();
    RECOGNIZER_BUSY.store(false, Ordering::SeqCst);
    result
}

/// One utterance for the composer microphone button.
pub fn dictate(
    options: SpeechOptions,
    on_partial: &dyn Fn(&str),
    should_cancel: &dyn Fn() -> bool,
) -> Result<SpeechResult, SpeechError> {
    with_recognizer(|| {
        let backend = active_backend();
        if !backend.capability().available {
            return Err(SpeechError::new(
                SpeechErrorCode::Unavailable,
                "backend unavailable",
            ));
        }
        backend.recognize_once(options, on_partial, should_cancel)
    })
}

/// Keep listening until `should_cancel` is true.
/// Silence with no words starts another utterance instead of ending the session.
pub fn listen_session(
    options: SpeechOptions,
    on_partial: &dyn Fn(&str),
    on_utterance: &dyn Fn(&str),
    should_cancel: &dyn Fn() -> bool,
) -> Result<(), SpeechError> {
    with_recognizer(|| {
        let backend = active_backend();
        if !backend.capability().available {
            return Err(SpeechError::new(
                SpeechErrorCode::Unavailable,
                "backend unavailable",
            ));
        }
        let mut failures = 0;
        loop {
            if should_cancel() {
                return Ok(());
            }
            match backend.recognize_once(options.clone(), on_partial, should_cancel) {
                Ok(result) => {
                    failures = 0;
                    let text = result.text.trim();
                    if !text.is_empty() {
                        on_utterance(text);
                    }
                }
                Err(err) if err.code == SpeechErrorCode::NoSpeech => {}
                Err(err) if err.code == SpeechErrorCode::Cancelled => return Ok(()),
                Err(err) => {
                    eprintln!("[voice] recognition failed: {err}");
                    failures += 1;
                    if failures >= 3 {
                        return Err(err);
                    }
                    thread::sleep(Duration::from_millis(400));
                    continue;
                }
            }
            if should_cancel() {
                return Ok(());
            }
            thread::sleep(Duration::from_millis(200));
        }
    })
}

pub fn speak(
    text: &str,
    language: Option<&str>,
    should_cancel: &dyn Fn() -> bool,
) -> Result<(), SpeechError> {
    if text.trim().is_empty() {
        return Ok(());
    }
    active_backend().speak(text, language, should_cancel)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_id_is_stable() {
        let id = active_backend().id();
        assert!(!id.is_empty());
    }
}
