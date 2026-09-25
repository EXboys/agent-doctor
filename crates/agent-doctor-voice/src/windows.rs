//! Windows: Windows.Media.SpeechRecognition (one utterance via RecognizeAsync).

use super::backend::SpeechBackend;
use super::types::{SpeechCapability, SpeechError, SpeechErrorCode, SpeechOptions, SpeechResult};
use windows::{
    core::HSTRING,
    Foundation::IAsyncOperation,
    Globalization::Language,
    Media::SpeechRecognition::{SpeechRecognitionResult, SpeechRecognizer},
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

fn recognize_once_inner(
    options: SpeechOptions,
    should_cancel: &dyn Fn() -> bool,
) -> Result<SpeechResult, SpeechError> {
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
        let _ = timeouts.SetInitialSilenceTimeout(std::time::Duration::from_secs(5).into());
        let _ = timeouts.SetEndSilenceTimeout(std::time::Duration::from_secs(2).into());
    }

    let compile_op = recognizer.CompileConstraintsAsync().map_err(|e| {
        SpeechError::new(SpeechErrorCode::Failed, format!("compile constraints: {e}"))
    })?;
    let _ = block_on(compile_op, should_cancel)?;

    let recognize_op = recognizer
        .RecognizeAsync()
        .map_err(|e| SpeechError::new(SpeechErrorCode::Failed, format!("start recognize: {e}")))?;
    let result: SpeechRecognitionResult = block_on(recognize_op, should_cancel)?;

    let status = result
        .Status()
        .map_err(|e| SpeechError::new(SpeechErrorCode::Failed, format!("result status: {e}")))?;

    use windows::Media::SpeechRecognition::SpeechRecognitionResultStatus;
    match status {
        SpeechRecognitionResultStatus::Success => {}
        SpeechRecognitionResultStatus::NoMatch | SpeechRecognitionResultStatus::TimeoutExceeded => {
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
    let confidence = result.RawConfidence().unwrap_or(0.0) as f32;

    if text.trim().is_empty() {
        return Err(SpeechError::new(
            SpeechErrorCode::NoSpeech,
            "empty recognition result",
        ));
    }

    Ok(SpeechResult {
        text,
        confidence,
        is_final: true,
    })
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
        _on_partial: &dyn Fn(&str),
        should_cancel: &dyn Fn() -> bool,
    ) -> Result<SpeechResult, SpeechError> {
        recognize_once_inner(options, should_cancel)
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
