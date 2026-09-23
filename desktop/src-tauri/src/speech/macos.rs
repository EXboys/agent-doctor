//! macOS: SFSpeechRecognizer + AVAudioEngine (no AVAudioSession — that is iOS-only).

#![allow(unexpected_cfgs)]

use super::backend::SpeechBackend;
use super::live_ctrl::{self, LiveAction, LiveState};
use super::types::{SpeechCapability, SpeechError, SpeechErrorCode, SpeechOptions, SpeechResult};
use block2::{RcBlock, StackBlock};
use objc2::msg_send;
use objc2::runtime::{AnyClass, AnyObject};
use objc2_foundation::NSString;
use std::ffi::CStr;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

#[link(name = "Speech", kind = "framework")]
#[link(name = "AVFAudio", kind = "framework")]
extern "C" {}

pub struct MacOsSpeechBackend;

#[derive(Default)]
struct LiveSession {
    best: Option<SpeechResult>,
    final_result: Option<SpeechResult>,
    error: Option<SpeechError>,
    last_change: Option<std::time::Instant>,
}

impl LiveSession {
    fn snapshot(&self, now: std::time::Instant, started: std::time::Instant) -> LiveState {
        LiveState {
            have_text: self
                .best
                .as_ref()
                .is_some_and(|r| !r.text.trim().is_empty()),
            last_change: self
                .last_change
                .map(|t| t.saturating_duration_since(started)),
            elapsed: now.saturating_duration_since(started),
            end_audio_sent: false,
            end_audio_at: None,
        }
    }
}

fn require_class(name: &CStr) -> Result<&'static AnyClass, SpeechError> {
    AnyClass::get(name).ok_or_else(|| {
        SpeechError::new(
            SpeechErrorCode::Unavailable,
            format!("missing Objective-C class {}", name.to_string_lossy()),
        )
    })
}

/// TCC aborts the process if usage strings are missing from the *responsible*
/// app's Info.plist. Refuse before calling into Speech when NSBundle cannot see
/// the keys (typical for bare `cargo run` / Cursor-launched binaries).
unsafe fn ensure_privacy_usage_descriptions() -> Result<(), SpeechError> {
    let bundle_class = require_class(c"NSBundle")?;
    let main: *mut AnyObject = msg_send![bundle_class, mainBundle];
    if main.is_null() {
        return Err(SpeechError::new(
            SpeechErrorCode::PermissionDenied,
            "main bundle unavailable for privacy usage check",
        ));
    }
    for key in [
        "NSSpeechRecognitionUsageDescription",
        "NSMicrophoneUsageDescription",
    ] {
        let key_ns = NSString::from_str(key);
        let value: *mut AnyObject = msg_send![main, objectForInfoDictionaryKey: &*key_ns];
        if value.is_null() {
            return Err(SpeechError::new(
                SpeechErrorCode::PermissionDenied,
                format!(
                    "missing {key} in Info.plist — launch via the macOS .app runner \
                     (npm run tauri:dev:personal) so privacy prompts can appear"
                ),
            ));
        }
    }
    Ok(())
}

unsafe fn request_speech_authorization() -> Result<(), SpeechError> {
    ensure_privacy_usage_descriptions()?;

    const AUTH_TIMEOUT: Duration = Duration::from_secs(10);
    let status_data = Arc::new((Mutex::new(None::<isize>), Condvar::new()));
    let status_clone = status_data.clone();

    let block = StackBlock::new(move |status: isize| {
        let (lock, cvar) = &*status_clone;
        let mut data = lock.lock().unwrap_or_else(|e| e.into_inner());
        *data = Some(status);
        cvar.notify_one();
    });
    let block = block.copy();

    let class = require_class(c"SFSpeechRecognizer")?;
    let _: () = msg_send![class, requestAuthorization: &*block];

    let (lock, cvar) = &*status_data;
    let data = lock.lock().unwrap_or_else(|e| e.into_inner());
    let (data, timeout) = cvar
        .wait_timeout(data, AUTH_TIMEOUT)
        .unwrap_or_else(|e| e.into_inner());

    if timeout.timed_out() {
        return Err(SpeechError::new(
            SpeechErrorCode::PermissionDenied,
            "authorization prompt timed out",
        ));
    }

    if *data == Some(3) {
        Ok(())
    } else {
        Err(SpeechError::new(
            SpeechErrorCode::PermissionDenied,
            "speech recognition not authorized",
        ))
    }
}

unsafe fn create_recognizer(language: Option<&str>) -> Result<*mut AnyObject, SpeechError> {
    let locale_str = language.unwrap_or("zh-CN");
    let locale_ns = NSString::from_str(locale_str);

    let locale_class = require_class(c"NSLocale")?;
    let locale: *mut AnyObject = msg_send![locale_class, alloc];
    let locale: *mut AnyObject = msg_send![locale, initWithLocaleIdentifier: &*locale_ns];

    let recognizer_class = require_class(c"SFSpeechRecognizer")?;
    let recognizer: *mut AnyObject = msg_send![recognizer_class, alloc];
    let recognizer: *mut AnyObject = msg_send![recognizer, initWithLocale: locale];

    if recognizer.is_null() {
        return Err(SpeechError::new(
            SpeechErrorCode::Unavailable,
            "failed to create SFSpeechRecognizer",
        ));
    }

    let is_available: bool = msg_send![recognizer, isAvailable];
    if !is_available {
        return Err(SpeechError::new(
            SpeechErrorCode::Unavailable,
            "speech recognizer reports unavailable",
        ));
    }

    let auth_status: isize = msg_send![recognizer_class, authorizationStatus];
    if auth_status == 1 || auth_status == 2 {
        return Err(SpeechError::new(
            SpeechErrorCode::PermissionDenied,
            "speech recognition denied or restricted",
        ));
    }
    if auth_status == 0 {
        request_speech_authorization()?;
    }

    Ok(recognizer)
}

fn recognize_from_microphone(
    options: SpeechOptions,
    on_partial: &dyn Fn(&str),
    should_cancel: &dyn Fn() -> bool,
) -> Result<SpeechResult, SpeechError> {
    const TICK: Duration = Duration::from_millis(200);

    unsafe {
        let recognizer = create_recognizer(options.language.as_deref())?;

        let engine_class = require_class(c"AVAudioEngine")?;
        let audio_engine: *mut AnyObject = msg_send![engine_class, new];
        if audio_engine.is_null() {
            return Err(SpeechError::new(
                SpeechErrorCode::Failed,
                "failed to create AVAudioEngine",
            ));
        }
        let input_node: *mut AnyObject = msg_send![audio_engine, inputNode];

        let request_class = require_class(c"SFSpeechAudioBufferRecognitionRequest")?;
        let request: *mut AnyObject = msg_send![request_class, new];
        if request.is_null() {
            return Err(SpeechError::new(
                SpeechErrorCode::Failed,
                "failed to create recognition request",
            ));
        }
        let _: () = msg_send![request, setShouldReportPartialResults: true];

        let session_state = Arc::new((Mutex::new(LiveSession::default()), Condvar::new()));
        let state_clone = session_state.clone();
        let last_emitted = Arc::new(Mutex::new(String::new()));
        let last_emitted_clone = last_emitted.clone();

        let block = StackBlock::new(move |result: *mut AnyObject, error: *mut AnyObject| {
            let (lock, cvar) = &*state_clone;

            if !error.is_null() {
                let desc: *mut AnyObject = msg_send![error, localizedDescription];
                let detail = if desc.is_null() {
                    "recognition error".to_string()
                } else {
                    let ns_str = &*(desc as *const NSString);
                    ns_str.to_string()
                };
                let mut data = lock.lock().unwrap_or_else(|e| e.into_inner());
                data.error = Some(SpeechError::new(SpeechErrorCode::Failed, detail));
                cvar.notify_one();
                return;
            }

            if result.is_null() {
                return;
            }

            let best_transcription: *mut AnyObject = msg_send![result, bestTranscription];
            if best_transcription.is_null() {
                return;
            }
            let formatted_string: *mut AnyObject = msg_send![best_transcription, formattedString];
            if formatted_string.is_null() {
                return;
            }
            let ns_str = &*(formatted_string as *const NSString);
            let text = ns_str.to_string();
            let is_final: bool = msg_send![result, isFinal];
            let now = std::time::Instant::now();

            {
                let mut emitted = last_emitted_clone.lock().unwrap_or_else(|e| e.into_inner());
                if !is_final && *emitted != text {
                    *emitted = text.clone();
                }
            }

            let mut data = lock.lock().unwrap_or_else(|e| e.into_inner());
            let recognized = SpeechResult {
                text,
                confidence: 0.0,
                is_final,
            };
            if is_final {
                data.final_result = Some(recognized);
            } else {
                data.best = Some(recognized);
                data.last_change = Some(now);
            }
            cvar.notify_one();
        });
        let block: RcBlock<dyn Fn(*mut AnyObject, *mut AnyObject)> = block.copy();

        let task: *mut AnyObject =
            msg_send![recognizer, recognitionTaskWithRequest: request, resultHandler: &*block];
        if task.is_null() {
            return Err(SpeechError::new(
                SpeechErrorCode::Failed,
                "failed to start recognition task",
            ));
        }

        let recording_format: *mut AnyObject = msg_send![input_node, outputFormatForBus: 0usize];
        let tap_block = StackBlock::new(move |buffer: *mut AnyObject, _when: *mut AnyObject| {
            let _: () = msg_send![request, appendAudioPCMBuffer: buffer];
        });
        let tap_block = tap_block.copy();
        let _: () = msg_send![
            input_node,
            installTapOnBus: 0usize,
            bufferSize: 1024u32,
            format: recording_format,
            block: &*tap_block
        ];

        let mut engine_error: *mut AnyObject = std::ptr::null_mut();
        let started: bool = msg_send![audio_engine, startAndReturnError: &mut engine_error];
        if !started {
            return Err(SpeechError::new(
                SpeechErrorCode::PermissionDenied,
                "microphone unavailable or permission denied",
            ));
        }

        let started_at = std::time::Instant::now();
        let mut end_audio_sent = false;
        let mut end_audio_at: Option<std::time::Instant> = None;
        let mut last_ui_partial = String::new();

        let outcome = 'session: loop {
            if should_cancel() {
                break 'session Err(SpeechError::new(
                    SpeechErrorCode::Cancelled,
                    "cancelled by user",
                ));
            }

            let (lock, cvar) = &*session_state;
            let mut guard = lock.lock().unwrap_or_else(|e| e.into_inner());

            loop {
                let now = std::time::Instant::now();

                if let Some(err) = guard.error.take() {
                    break 'session Err(err);
                }
                if let Some(final_result) = guard.final_result.clone() {
                    break 'session Ok(final_result);
                }

                if let Some(best) = guard.best.as_ref() {
                    if best.text != last_ui_partial {
                        last_ui_partial = best.text.clone();
                        on_partial(&last_ui_partial);
                    }
                }

                let mut snapshot = guard.snapshot(now, started_at);
                snapshot.end_audio_sent = end_audio_sent;
                snapshot.end_audio_at =
                    end_audio_at.map(|t| t.saturating_duration_since(started_at));

                match live_ctrl::next_action(&snapshot) {
                    LiveAction::Continue => break,
                    LiveAction::EndAudio => {
                        drop(guard);
                        let _: () = msg_send![request, endAudio];
                        end_audio_sent = true;
                        end_audio_at = Some(std::time::Instant::now());
                        guard = lock.lock().unwrap_or_else(|e| e.into_inner());
                    }
                    LiveAction::CancelNoSpeech => {
                        break 'session Err(SpeechError::new(
                            SpeechErrorCode::NoSpeech,
                            "no speech detected",
                        ));
                    }
                    LiveAction::CancelTimeout => match guard.best.clone() {
                        Some(best) if !best.text.trim().is_empty() => {
                            break 'session Ok(best);
                        }
                        _ => {
                            break 'session Err(SpeechError::new(
                                SpeechErrorCode::Failed,
                                "speech recognition timed out",
                            ));
                        }
                    },
                }
            }

            let (guard2, _timeout) = cvar
                .wait_timeout(guard, TICK)
                .unwrap_or_else(|e| e.into_inner());
            drop(guard2);
        };

        let _: () = msg_send![audio_engine, stop];
        let _: () = msg_send![input_node, removeTapOnBus: 0usize];
        if !end_audio_sent {
            let _: () = msg_send![request, endAudio];
        }
        let _: () = msg_send![task, cancel];

        outcome
    }
}

impl SpeechBackend for MacOsSpeechBackend {
    fn id(&self) -> &'static str {
        "macos-sfspeech"
    }

    fn capability(&self) -> SpeechCapability {
        let available = AnyClass::get(c"SFSpeechRecognizer").is_some()
            && AnyClass::get(c"AVAudioEngine").is_some();
        SpeechCapability {
            available,
            backend: self.id().into(),
            reason: if available {
                None
            } else {
                Some("framework_missing".into())
            },
        }
    }

    fn recognize_once(
        &self,
        options: SpeechOptions,
        on_partial: &dyn Fn(&str),
        should_cancel: &dyn Fn() -> bool,
    ) -> Result<SpeechResult, SpeechError> {
        if !self.capability().available {
            return Err(SpeechError::new(
                SpeechErrorCode::Unavailable,
                "macOS speech frameworks unavailable",
            ));
        }
        recognize_from_microphone(options, on_partial, should_cancel)
    }
}
