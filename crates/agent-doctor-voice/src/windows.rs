//! Windows: system speech recognition with live text, then a final utterance
//! after about 2.5s of quiet. Speech synthesis uses the system voice.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::backend::SpeechBackend;
use super::types::{SpeechCapability, SpeechError, SpeechErrorCode, SpeechOptions, SpeechResult};
use windows::{
    core::{Interface, HSTRING},
    Foundation::{IAsyncAction, IAsyncOperation, TypedEventHandler},
    Globalization::Language,
    Media::SpeechRecognition::{
        ISpeechRecognitionConstraint, SpeechContinuousRecognitionCompletedEventArgs,
        SpeechContinuousRecognitionResultGeneratedEventArgs, SpeechContinuousRecognitionSession,
        SpeechRecognitionHypothesisGeneratedEventArgs, SpeechRecognitionResult,
        SpeechRecognitionResultStatus, SpeechRecognitionScenario, SpeechRecognitionTopicConstraint,
        SpeechRecognizer,
    },
};

pub struct WindowsSpeechBackend;

fn block_on<T: windows::core::RuntimeType>(
    op: IAsyncOperation<T>,
    should_cancel: &dyn Fn() -> bool,
) -> Result<T, SpeechError> {
    loop {
        if should_cancel() {
            let _ = op.Cancel();
            return Err(SpeechError::new(
                SpeechErrorCode::Cancelled,
                "cancelled by user",
            ));
        }
        match op
            .Status()
            .map_err(|e| SpeechError::new(SpeechErrorCode::Failed, format!("async status: {e}")))?
        {
            windows::Foundation::AsyncStatus::Completed => {
                return op.GetResults().map_err(|e| {
                    SpeechError::new(SpeechErrorCode::Failed, format!("get results: {e}"))
                });
            }
            windows::Foundation::AsyncStatus::Error => {
                let code = op.ErrorCode().unwrap_or(windows::core::HRESULT(-1));
                return Err(SpeechError::new(
                    SpeechErrorCode::Failed,
                    format!("async error: {code:?}"),
                ));
            }
            windows::Foundation::AsyncStatus::Canceled => {
                return Err(SpeechError::new(
                    SpeechErrorCode::Cancelled,
                    "recognition cancelled",
                ));
            }
            _ => std::thread::sleep(std::time::Duration::from_millis(20)),
        }
    }
}

fn block_on_action(
    action: IAsyncAction,
    should_cancel: &dyn Fn() -> bool,
) -> Result<(), SpeechError> {
    loop {
        if should_cancel() {
            let _ = action.Cancel();
            return Err(SpeechError::new(
                SpeechErrorCode::Cancelled,
                "cancelled by user",
            ));
        }
        match action
            .Status()
            .map_err(|e| SpeechError::new(SpeechErrorCode::Failed, format!("async status: {e}")))?
        {
            windows::Foundation::AsyncStatus::Completed => return Ok(()),
            windows::Foundation::AsyncStatus::Error => {
                let code = action.ErrorCode().unwrap_or(windows::core::HRESULT(-1));
                return Err(SpeechError::new(
                    SpeechErrorCode::Failed,
                    format!("async error: {code:?}"),
                ));
            }
            windows::Foundation::AsyncStatus::Canceled => {
                return Err(SpeechError::new(
                    SpeechErrorCode::Cancelled,
                    "recognition cancelled",
                ));
            }
            _ => std::thread::sleep(Duration::from_millis(20)),
        }
    }
}

fn create_recognizer(options: &SpeechOptions) -> Result<SpeechRecognizer, SpeechError> {
    let recognizer = if let Some(lang) = options.language.as_deref() {
        let language = Language::CreateLanguage(&HSTRING::from(lang)).map_err(|e| {
            SpeechError::new(
                SpeechErrorCode::Failed,
                format!("invalid language {lang}: {e}"),
            )
        })?;
        SpeechRecognizer::Create(&language)
    } else {
        SpeechRecognizer::new()
    }
    .map_err(|e| {
        SpeechError::new(
            SpeechErrorCode::Unavailable,
            format!("create SpeechRecognizer: {e}"),
        )
    })?;

    if let Ok(timeouts) = recognizer.Timeouts() {
        let _ = timeouts.SetInitialSilenceTimeout(Duration::from_secs(8).into());
        let _ = timeouts.SetEndSilenceTimeout(Duration::from_millis(2500).into());
        let _ = timeouts.SetBabbleTimeout(Duration::from_secs(30).into());
    }
    Ok(recognizer)
}

fn result_text(result: &SpeechRecognitionResult) -> Result<SpeechResult, SpeechError> {
    let status = result
        .Status()
        .map_err(|e| SpeechError::new(SpeechErrorCode::Failed, format!("result status: {e}")))?;
    match status {
        SpeechRecognitionResultStatus::Success => {}
        SpeechRecognitionResultStatus::UserCanceled => {
            return Err(SpeechError::new(
                SpeechErrorCode::Cancelled,
                "recognition cancelled",
            ));
        }
        SpeechRecognitionResultStatus::TimeoutExceeded
        | SpeechRecognitionResultStatus::PauseLimitExceeded
        | SpeechRecognitionResultStatus::Unknown
        | SpeechRecognitionResultStatus::MicrophoneUnavailable => {
            return Err(SpeechError::new(
                SpeechErrorCode::NoSpeech,
                format!("status={status:?}"),
            ));
        }
        other => {
            return Err(SpeechError::new(
                SpeechErrorCode::Failed,
                format!("status={other:?}"),
            ));
        }
    }

    let text = result
        .Text()
        .map_err(|e| SpeechError::new(SpeechErrorCode::Failed, format!("text: {e}")))?
        .to_string();
    if text.trim().is_empty() {
        return Err(SpeechError::new(
            SpeechErrorCode::NoSpeech,
            "empty recognition result",
        ));
    }
    Ok(SpeechResult {
        text,
        confidence: result.RawConfidence().unwrap_or(0.0) as f32,
        is_final: true,
    })
}

fn recognize_once_inner(
    options: SpeechOptions,
    on_partial: &dyn Fn(&str),
    should_cancel: &dyn Fn() -> bool,
) -> Result<SpeechResult, SpeechError> {
    let recognizer = create_recognizer(&options)?;
    let constraint = SpeechRecognitionTopicConstraint::Create(
        SpeechRecognitionScenario::Dictation,
        &HSTRING::from("dictation"),
    )
    .map_err(|e| {
        SpeechError::new(
            SpeechErrorCode::Failed,
            format!("dictation constraint: {e}"),
        )
    })?;
    let constraint = constraint
        .cast::<ISpeechRecognitionConstraint>()
        .map_err(|e| {
            SpeechError::new(
                SpeechErrorCode::Failed,
                format!("dictation constraint cast: {e}"),
            )
        })?;
    recognizer
        .Constraints()
        .map_err(|e| SpeechError::new(SpeechErrorCode::Failed, format!("constraints: {e}")))?
        .Append(&constraint)
        .map_err(|e| SpeechError::new(SpeechErrorCode::Failed, format!("add constraint: {e}")))?;

    let compile_op = recognizer.CompileConstraintsAsync().map_err(|e| {
        SpeechError::new(SpeechErrorCode::Failed, format!("compile constraints: {e}"))
    })?;
    let _ = block_on(compile_op, should_cancel)?;

    match recognize_live(&recognizer, on_partial, should_cancel) {
        Ok(result) => Ok(result),
        Err(err) if err.code == SpeechErrorCode::Failed => {
            recognize_single(&recognizer, should_cancel)
        }
        Err(err) => Err(err),
    }
}

/// One utterance with hypothesis text while the person is still talking.
fn recognize_live(
    recognizer: &SpeechRecognizer,
    on_partial: &dyn Fn(&str),
    should_cancel: &dyn Fn() -> bool,
) -> Result<SpeechResult, SpeechError> {
    let session = recognizer.ContinuousRecognitionSession().map_err(|e| {
        SpeechError::new(SpeechErrorCode::Failed, format!("continuous session: {e}"))
    })?;

    let hypothesis = Arc::new(Mutex::new(String::new()));
    let finished = Arc::new(Mutex::new(None::<Result<SpeechResult, SpeechError>>));
    let completed = Arc::new(AtomicBool::new(false));

    let hypothesis_slot = hypothesis.clone();
    let hypothesis_token = recognizer
        .HypothesisGenerated(&TypedEventHandler::<
            SpeechRecognizer,
            SpeechRecognitionHypothesisGeneratedEventArgs,
        >::new(move |_sender, args| {
            let Some(args) = args else {
                return Ok(());
            };
            if let Ok(hypothesis) = args.Hypothesis() {
                if let Ok(text) = hypothesis.Text() {
                    let value = text.to_string();
                    if !value.trim().is_empty() {
                        *hypothesis_slot.lock().unwrap_or_else(|e| e.into_inner()) = value;
                    }
                }
            }
            Ok(())
        }))
        .map_err(|e| SpeechError::new(SpeechErrorCode::Failed, format!("hypothesis: {e}")))?;

    let finished_slot = finished.clone();
    let result_token = session
        .ResultGenerated(&TypedEventHandler::<
            SpeechContinuousRecognitionSession,
            SpeechContinuousRecognitionResultGeneratedEventArgs,
        >::new(move |_sender, args| {
            let Some(args) = args else {
                return Ok(());
            };
            if let Ok(result) = args.Result() {
                *finished_slot.lock().unwrap_or_else(|e| e.into_inner()) =
                    Some(result_text(&result));
            }
            Ok(())
        }))
        .map_err(|e| {
            let _ = recognizer.RemoveHypothesisGenerated(hypothesis_token);
            SpeechError::new(SpeechErrorCode::Failed, format!("result event: {e}"))
        })?;

    let completed_flag = completed.clone();
    let completed_token = session
        .Completed(&TypedEventHandler::<
            SpeechContinuousRecognitionSession,
            SpeechContinuousRecognitionCompletedEventArgs,
        >::new(move |_sender, _args| {
            completed_flag.store(true, Ordering::SeqCst);
            Ok(())
        }))
        .map_err(|e| {
            let _ = recognizer.RemoveHypothesisGenerated(hypothesis_token);
            let _ = session.RemoveResultGenerated(result_token);
            SpeechError::new(SpeechErrorCode::Failed, format!("completed event: {e}"))
        })?;

    let outcome = listen_until_utterance(
        &session,
        &hypothesis,
        &finished,
        &completed,
        on_partial,
        should_cancel,
    );

    let _ = recognizer.RemoveHypothesisGenerated(hypothesis_token);
    let _ = session.RemoveResultGenerated(result_token);
    let _ = session.RemoveCompleted(completed_token);
    if let Ok(stop) = session.StopAsync() {
        let _ = block_on_action(stop, &|| false);
    }
    outcome
}

fn listen_until_utterance(
    session: &SpeechContinuousRecognitionSession,
    hypothesis: &Mutex<String>,
    finished: &Mutex<Option<Result<SpeechResult, SpeechError>>>,
    completed: &AtomicBool,
    on_partial: &dyn Fn(&str),
    should_cancel: &dyn Fn() -> bool,
) -> Result<SpeechResult, SpeechError> {
    block_on_action(
        session.StartAsync().map_err(|e| {
            SpeechError::new(SpeechErrorCode::Failed, format!("start session: {e}"))
        })?,
        should_cancel,
    )?;

    let started = std::time::Instant::now();
    let mut shown = String::new();
    loop {
        if should_cancel() {
            if let Ok(cancel) = session.CancelAsync() {
                let _ = block_on_action(cancel, &|| false);
            }
            return Err(SpeechError::new(
                SpeechErrorCode::Cancelled,
                "cancelled by user",
            ));
        }
        let current = hypothesis.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if !current.is_empty() && current != shown {
            shown = current;
            on_partial(&shown);
        }
        if let Some(result) = finished.lock().unwrap_or_else(|e| e.into_inner()).take() {
            return result;
        }
        if completed.load(Ordering::SeqCst) {
            if shown.is_empty() {
                return Err(SpeechError::new(
                    SpeechErrorCode::NoSpeech,
                    "continuous recognition ended",
                ));
            }
            return Ok(SpeechResult {
                text: shown,
                confidence: 0.0,
                is_final: true,
            });
        }
        if started.elapsed() > Duration::from_secs(30) && !shown.is_empty() {
            return Ok(SpeechResult {
                text: shown,
                confidence: 0.0,
                is_final: true,
            });
        }
        std::thread::sleep(Duration::from_millis(40));
    }
}

fn recognize_single(
    recognizer: &SpeechRecognizer,
    should_cancel: &dyn Fn() -> bool,
) -> Result<SpeechResult, SpeechError> {
    let recognize_op = recognizer
        .RecognizeAsync()
        .map_err(|e| SpeechError::new(SpeechErrorCode::Failed, format!("start recognize: {e}")))?;
    let result: SpeechRecognitionResult = block_on(recognize_op, should_cancel)?;
    result_text(&result)
}

impl SpeechBackend for WindowsSpeechBackend {
    fn id(&self) -> &'static str {
        "windows-media-speech"
    }

    fn capability(&self) -> SpeechCapability {
        let available = SpeechRecognizer::new().is_ok();
        SpeechCapability {
            available,
            backend: self.id().into(),
            reason: if available {
                None
            } else {
                Some("recognizer_init_failed".into())
            },
        }
    }

    fn recognize_once(
        &self,
        options: SpeechOptions,
        on_partial: &dyn Fn(&str),
        should_cancel: &dyn Fn() -> bool,
    ) -> Result<SpeechResult, SpeechError> {
        recognize_once_inner(options, on_partial, should_cancel)
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
    use windows::Media::Core::MediaSource;
    use windows::Media::Playback::{MediaPlaybackState, MediaPlayer};
    use windows::Media::SpeechSynthesis::SpeechSynthesizer;

    let synth = SpeechSynthesizer::new().map_err(|e| {
        SpeechError::new(
            SpeechErrorCode::Unavailable,
            format!("create SpeechSynthesizer: {e}"),
        )
    })?;
    if let Some(lang) = language {
        if let Ok(voices) = SpeechSynthesizer::AllVoices() {
            let wanted = lang.to_ascii_lowercase();
            let count = voices.Size().unwrap_or(0);
            for index in 0..count {
                let Ok(voice) = voices.GetAt(index) else {
                    continue;
                };
                let voice_lang = voice
                    .Language()
                    .map(|value| value.to_string())
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                if voice_lang.starts_with(&wanted) || wanted.starts_with(&voice_lang) {
                    let _ = synth.SetVoice(&voice);
                    break;
                }
            }
        }
    }

    let stream = block_on(
        synth
            .SynthesizeTextToStreamAsync(&HSTRING::from(text))
            .map_err(|e| SpeechError::new(SpeechErrorCode::Failed, format!("synthesize: {e}")))?,
        should_cancel,
    )?;
    let source = MediaSource::CreateFromStream(&stream, &HSTRING::from("audio/wav"))
        .map_err(|e| SpeechError::new(SpeechErrorCode::Failed, format!("media source: {e}")))?;
    let player = MediaPlayer::new()
        .map_err(|e| SpeechError::new(SpeechErrorCode::Failed, format!("media player: {e}")))?;
    player
        .SetSource(&source)
        .map_err(|e| SpeechError::new(SpeechErrorCode::Failed, format!("set source: {e}")))?;
    player
        .Play()
        .map_err(|e| SpeechError::new(SpeechErrorCode::Failed, format!("play: {e}")))?;

    let started = std::time::Instant::now();
    let mut heard = false;
    loop {
        if should_cancel() {
            let _ = player.Pause();
            return Err(SpeechError::new(
                SpeechErrorCode::Cancelled,
                "speech cancelled",
            ));
        }
        if started.elapsed() > std::time::Duration::from_secs(180) {
            let _ = player.Pause();
            return Ok(());
        }
        let playing = player
            .PlaybackSession()
            .and_then(|session| session.PlaybackState())
            .unwrap_or(MediaPlaybackState::None);
        let active = matches!(
            playing,
            MediaPlaybackState::Playing
                | MediaPlaybackState::Buffering
                | MediaPlaybackState::Opening
        );
        if active {
            heard = true;
        } else if heard || started.elapsed() > std::time::Duration::from_millis(800) {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(80));
    }
}
