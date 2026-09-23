use super::types::{SpeechCapability, SpeechError, SpeechOptions, SpeechResult};

/// Pluggable native speech backend. Swap via `active_backend()`.
pub trait SpeechBackend: Send + Sync {
    fn id(&self) -> &'static str;
    fn capability(&self) -> SpeechCapability;
    /// One utterance from the microphone. May emit partials via `on_partial`.
    fn recognize_once(
        &self,
        options: SpeechOptions,
        on_partial: &dyn Fn(&str),
        should_cancel: &dyn Fn() -> bool,
    ) -> Result<SpeechResult, SpeechError>;
}

pub fn active_backend() -> Box<dyn SpeechBackend> {
    #[cfg(target_os = "macos")]
    {
        Box::new(super::macos::MacOsSpeechBackend)
    }
    #[cfg(windows)]
    {
        Box::new(super::windows::WindowsSpeechBackend)
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        Box::new(super::unsupported::UnsupportedSpeechBackend)
    }
}
