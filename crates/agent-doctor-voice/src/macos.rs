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
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// `AVAudioEngine` only delivers microphone buffers when it is started on the
/// main thread. The wait loop stays off the main thread so the window can
/// still paint and deliver events.
fn on_main<R: Send + 'static, F: FnOnce() -> R + Send + 'static>(work: F) -> R {
    let already_main = unsafe {
        AnyClass::get(c"NSThread")
            .map(|class| {
                let value: bool = msg_send![class, isMainThread];
                value
            })
            .unwrap_or(false)
    };
    if already_main {
        return work();
    }
    let slot = Arc::new((Mutex::new(None::<R>), Condvar::new()));
    let slot_for_block = slot.clone();
    let work = Arc::new(Mutex::new(Some(work)));
    let block = StackBlock::new(move || {
        let Some(work) = work.lock().unwrap_or_else(|e| e.into_inner()).take() else {
            return;
        };
        let result = work();
        let (lock, cvar) = &*slot_for_block;
        *lock.lock().unwrap_or_else(|e| e.into_inner()) = Some(result);
        cvar.notify_one();
    });
    let block = block.copy();
    unsafe {
        let queue_class = AnyClass::get(c"NSOperationQueue").expect("NSOperationQueue");
        let queue: *mut AnyObject = msg_send![queue_class, mainQueue];
        let _: () = msg_send![queue, addOperationWithBlock: &*block];
    }
    let (lock, cvar) = &*slot;
    let mut guard = lock.lock().unwrap_or_else(|e| e.into_inner());
    while guard.is_none() {
        guard = cvar.wait(guard).unwrap_or_else(|e| e.into_inner());
    }
    guard.take().expect("main-thread speech job did not finish")
}

#[link(name = "Speech", kind = "framework")]
extern "C" {}

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

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Loudness of one microphone buffer. Used to notice when the person stops talking.
fn buffer_rms(buffer: *mut AnyObject) -> f32 {
    unsafe {
        let frames: usize = msg_send![buffer, frameLength];
        if frames == 0 {
            return 0.0;
        }
        let step = (frames / 400).max(1);
        let float_channels: *mut *mut f32 = msg_send![buffer, floatChannelData];
        if !float_channels.is_null() && !(*float_channels).is_null() {
            let samples = std::slice::from_raw_parts(*float_channels, frames);
            return rms_samples(samples, step, |sample| *sample);
        }
        let int_channels: *mut *mut i16 = msg_send![buffer, int16ChannelData];
        if !int_channels.is_null() && !(*int_channels).is_null() {
            let samples = std::slice::from_raw_parts(*int_channels, frames);
            return rms_samples(samples, step, |sample| *sample as f32 / 32768.0);
        }
        0.0
    }
}

fn rms_samples<T>(samples: &[T], step: usize, sample: impl Fn(&T) -> f32) -> f32 {
    let mut sum = 0f32;
    let mut n = 0usize;
    let mut i = 0usize;
    while i < samples.len() {
        let value = sample(&samples[i]);
        sum += value * value;
        n += 1;
        i += step;
    }
    if n == 0 {
        0.0
    } else {
        (sum / n as f32).sqrt()
    }
}

fn classify_recognition_nserror(detail: &str) -> SpeechErrorCode {
    let lower = detail.to_lowercase();
    if lower.contains("cancel") {
        return SpeechErrorCode::Cancelled;
    }
    // Apple reports silence as an NSError (often kAFAssistantErrorDomain 1110 / 203),
    // not as an empty result. Hosted mode must keep listening through that.
    if lower.contains("no speech")
        || lower.contains("no match")
        || lower.contains("1110")
        || lower.contains("203")
        || lower.contains("216")
        || lower.contains("retry")
    {
        return SpeechErrorCode::NoSpeech;
    }
    SpeechErrorCode::Failed
}

fn end_request_audio(request_bits: usize) {
    on_main(move || unsafe {
        let request = request_bits as *mut AnyObject;
        let _: () = msg_send![request, endAudio];
    });
}

fn recognize_from_microphone(
    options: SpeechOptions,
    on_partial: &dyn Fn(&str),
    should_cancel: &dyn Fn() -> bool,
) -> Result<SpeechResult, SpeechError> {
    const TICK: Duration = Duration::from_millis(200);

    let recognizer = unsafe { create_recognizer(options.language.as_deref())? };
    let recognizer_bits = recognizer as usize;
    let session_state = Arc::new((Mutex::new(LiveSession::default()), Condvar::new()));
    let state_for_handler = session_state.clone();
    let last_voice_ms = Arc::new(AtomicU64::new(0));
    let last_voice_for_tap = last_voice_ms.clone();
    let saw_audio = Arc::new(AtomicBool::new(false));
    let saw_audio_for_tap = saw_audio.clone();

    let setup: Result<(usize, usize, usize, usize), SpeechError> = on_main(move || unsafe {
        let recognizer = recognizer_bits as *mut AnyObject;
        let engine_class = require_class(c"AVAudioEngine")?;
        let audio_engine: *mut AnyObject = msg_send![engine_class, new];
        if audio_engine.is_null() {
            return Err(SpeechError::new(
                SpeechErrorCode::Failed,
                "failed to create AVAudioEngine",
            ));
        }
        // Reading the input node attaches it. Preparing before that throws
        // and quits the whole app.
        let input_node: *mut AnyObject = msg_send![audio_engine, inputNode];
        if input_node.is_null() {
            return Err(SpeechError::new(
                SpeechErrorCode::Failed,
                "microphone input is unavailable",
            ));
        }

        let request_class = require_class(c"SFSpeechAudioBufferRecognitionRequest")?;
        let request: *mut AnyObject = msg_send![request_class, new];
        if request.is_null() {
            return Err(SpeechError::new(
                SpeechErrorCode::Failed,
                "failed to create recognition request",
            ));
        }
        let _: () = msg_send![request, setShouldReportPartialResults: true];

        let block = StackBlock::new(move |result: *mut AnyObject, error: *mut AnyObject| {
            let (lock, cvar) = &*state_for_handler;

            if !error.is_null() {
                let desc: *mut AnyObject = msg_send![error, localizedDescription];
                let detail = if desc.is_null() {
                    "recognition error".to_string()
                } else {
                    let ns_str = &*(desc as *const NSString);
                    ns_str.to_string()
                };
                let code = classify_recognition_nserror(&detail);
                let mut data = lock.lock().unwrap_or_else(|e| e.into_inner());
                // A trailing "no speech" / cancel error must not wipe text we already have.
                if code == SpeechErrorCode::Failed || data.best.is_none() {
                    data.error = Some(SpeechError::new(code, detail));
                    cvar.notify_one();
                }
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
            let mut data = lock.lock().unwrap_or_else(|e| e.into_inner());
            let recognized = SpeechResult {
                text,
                confidence: 0.0,
                is_final,
            };
            if is_final {
                data.final_result = Some(recognized);
            } else if data.best.as_ref().map(|r| r.text.as_str()) != Some(recognized.text.as_str())
            {
                data.last_change = Some(now);
                data.best = Some(recognized);
            } else {
                data.best = Some(recognized);
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
        let _: *mut AnyObject = msg_send![task, retain];

        let recording_format: *mut AnyObject = msg_send![input_node, outputFormatForBus: 0usize];
        if recording_format.is_null() {
            return Err(SpeechError::new(
                SpeechErrorCode::Failed,
                "microphone format unavailable",
            ));
        }
        let sample_rate: f64 = msg_send![recording_format, sampleRate];
        let channels: u32 = msg_send![recording_format, channelCount];
        if sample_rate < 1.0 || channels == 0 {
            return Err(SpeechError::new(
                SpeechErrorCode::Failed,
                format!("microphone format is empty ({sample_rate} Hz, {channels} ch)"),
            ));
        }
        eprintln!("[voice] listening {sample_rate} Hz {channels} ch");

        let tap_block = StackBlock::new(move |buffer: *mut AnyObject, _when: *mut AnyObject| {
            let frames: usize = msg_send![buffer, frameLength];
            if frames > 0 && !saw_audio_for_tap.swap(true, Ordering::Relaxed) {
                eprintln!("[voice] microphone is delivering audio");
            }
            if buffer_rms(buffer) >= 0.02 {
                last_voice_for_tap.store(now_ms(), Ordering::Relaxed);
            }
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

        Ok((
            audio_engine as usize,
            request as usize,
            task as usize,
            input_node as usize,
        ))
    });
    let (engine_bits, request_bits, task_bits, input_bits) = setup?;

    let started_at = std::time::Instant::now();
    let mut end_audio_sent = false;
    let mut end_audio_at: Option<std::time::Instant> = None;
    let mut last_ui_partial = String::new();
    let mut unwritten = crate::turn_end::UnwrittenVoice::default();
    let mut tracked_text = String::new();

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
            // The system often marks a sentence final the moment the voice dips.
            // That is not the send. Keep the text and let the quiet timer decide.
            if let Some(final_result) = guard.final_result.take() {
                let changed = guard.best.as_ref().map(|result| result.text.as_str())
                    != Some(final_result.text.as_str());
                if changed {
                    guard.last_change = Some(now);
                }
                guard.best = Some(final_result);
            }

            if let Some(best) = guard.best.clone() {
                if best.text != last_ui_partial {
                    last_ui_partial = best.text.clone();
                    drop(guard);
                    on_partial(&last_ui_partial);
                    guard = lock.lock().unwrap_or_else(|e| e.into_inner());
                }
            }

            // Closing the buffer is what makes the recognizer finish a sentence.
            // Text often arrives only after that, so quiet audio ends the sentence
            // even when no words have been reported yet.
            let heard = guard
                .best
                .as_ref()
                .map(|result| result.text.clone())
                .unwrap_or_default();
            let text_changed = heard != tracked_text;
            let text_stable_ms = guard
                .last_change
                .map(|changed| now.saturating_duration_since(changed).as_millis() as u64)
                .unwrap_or(0);
            let voice_at = last_voice_ms.load(Ordering::Relaxed);
            let quiet_for = if voice_at == 0 {
                0
            } else {
                now_ms().saturating_sub(voice_at)
            };
            unwritten = crate::turn_end::note_unwritten_voice(
                unwritten,
                !heard.trim().is_empty(),
                text_changed,
                text_stable_ms,
                quiet_for,
            );
            tracked_text = heard.clone();
            let required_quiet = crate::turn_end::quiet_ms(&heard, unwritten.sound_after_words);
            if !end_audio_sent && voice_at > 0 && quiet_for >= required_quiet {
                drop(guard);
                end_request_audio(request_bits);
                end_audio_sent = true;
                end_audio_at = Some(std::time::Instant::now());
                guard = lock.lock().unwrap_or_else(|e| e.into_inner());
            }

            let mut snapshot = guard.snapshot(now, started_at);
            snapshot.end_audio_sent = end_audio_sent;
            snapshot.end_audio_at = end_audio_at.map(|t| t.saturating_duration_since(started_at));

            let voice_recent = voice_at > 0 && quiet_for < required_quiet;
            match live_ctrl::next_action_with_voice(&snapshot, voice_recent) {
                LiveAction::Continue => break,
                LiveAction::EndAudio => {
                    drop(guard);
                    end_request_audio(request_bits);
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
                            SpeechErrorCode::NoSpeech,
                            "no speech detected",
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

    on_main(move || unsafe {
        let audio_engine = engine_bits as *mut AnyObject;
        let input_node = input_bits as *mut AnyObject;
        let request = request_bits as *mut AnyObject;
        let task = task_bits as *mut AnyObject;
        let _: () = msg_send![audio_engine, stop];
        let _: () = msg_send![input_node, removeTapOnBus: 0usize];
        if !end_audio_sent {
            let _: () = msg_send![request, endAudio];
        }
        let _: () = msg_send![task, cancel];
        let _: () = msg_send![task, release];
        let _: () = msg_send![request, release];
        let _: () = msg_send![audio_engine, release];
    });

    outcome
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

    fn speak(
        &self,
        text: &str,
        language: Option<&str>,
        should_cancel: &dyn Fn() -> bool,
    ) -> Result<(), SpeechError> {
        speak_text(text, language, should_cancel)
    }
}

fn speak_text(
    text: &str,
    language: Option<&str>,
    should_cancel: &dyn Fn() -> bool,
) -> Result<(), SpeechError> {
    if text.trim().is_empty() {
        return Ok(());
    }
    unsafe {
        let synth_class = require_class(c"AVSpeechSynthesizer")?;
        let synth: *mut AnyObject = msg_send![synth_class, new];
        if synth.is_null() {
            return Err(SpeechError::new(
                SpeechErrorCode::Unavailable,
                "speech synthesizer unavailable",
            ));
        }
        let utterance_class = require_class(c"AVSpeechUtterance")?;
        let spoken = NSString::from_str(text);
        let utterance: *mut AnyObject =
            msg_send![utterance_class, speechUtteranceWithString: &*spoken];
        if utterance.is_null() {
            return Err(SpeechError::new(
                SpeechErrorCode::Failed,
                "failed to create speech utterance",
            ));
        }
        if let Some(lang) = language {
            let voice_class = require_class(c"AVSpeechSynthesisVoice")?;
            let lang_ns = NSString::from_str(lang);
            let voice: *mut AnyObject = msg_send![voice_class, voiceWithLanguage: &*lang_ns];
            if !voice.is_null() {
                let _: () = msg_send![utterance, setVoice: voice];
            }
        }
        let _: () = msg_send![synth, speakUtterance: utterance];

        let started = std::time::Instant::now();
        let mut heard = false;
        loop {
            if should_cancel() {
                let _: () = msg_send![synth, stopSpeakingAtBoundary: 0isize];
                return Err(SpeechError::new(
                    SpeechErrorCode::Cancelled,
                    "speech cancelled",
                ));
            }
            if started.elapsed() > Duration::from_secs(180) {
                let _: () = msg_send![synth, stopSpeakingAtBoundary: 0isize];
                return Ok(());
            }
            let speaking: bool = msg_send![synth, isSpeaking];
            if speaking {
                heard = true;
            } else if heard || started.elapsed() > Duration::from_millis(700) {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(80));
        }
    }
}
