use super::backend::SpeechBackend;
use super::types::{SpeechCapability, SpeechError, SpeechErrorCode, SpeechOptions, SpeechResult};

pub struct UnsupportedSpeechBackend;

impl SpeechBackend for UnsupportedSpeechBackend {
    fn id(&self) -> &'static str {
        "unsupported"
    }

    fn capability(&self) -> SpeechCapability {
        SpeechCapability {
            available: false,
            backend: self.id().into(),
            reason: Some("this_os".into()),
        }
    }

    fn recognize_once(
        &self,
        _options: SpeechOptions,
        _on_partial: &dyn Fn(&str),
        _should_cancel: &dyn Fn() -> bool,
    ) -> Result<SpeechResult, SpeechError> {
        Err(SpeechError::new(
            SpeechErrorCode::Unavailable,
            "speech recognition is not available on this system",
        ))
    }

    fn speak(
        &self,
        _text: &str,
        _language: Option<&str>,
        _should_cancel: &dyn Fn() -> bool,
    ) -> Result<(), SpeechError> {
        Err(SpeechError::new(
            SpeechErrorCode::Unavailable,
            "speech synthesis is not available on this system",
        ))
    }
}
