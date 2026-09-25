//! Live speech-recognition control state machine (pure logic, testable).

use std::time::Duration;

/// End-of-utterance silence → send `endAudio`.
pub const SILENCE_LIMIT: Duration = Duration::from_millis(1200);
/// No text at all within this budget → cancel with no-speech.
pub const NO_SPEECH_LIMIT: Duration = Duration::from_secs(8);
/// Hard cap for continuous speech → finalize.
pub const HARD_CAP: Duration = Duration::from_secs(30);
/// Grace after `endAudio` before falling back to the text we already have.
pub const UTTERANCE_GRACE: Duration = Duration::from_millis(700);
/// Grace when `endAudio` produced no text yet. The recognizer often
/// returns the sentence only after the buffer is closed.
pub const FINAL_GRACE: Duration = Duration::from_millis(2000);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveAction {
    Continue,
    EndAudio,
    CancelNoSpeech,
    CancelTimeout,
}

#[derive(Debug, Clone, Copy)]
pub struct LiveState {
    pub have_text: bool,
    pub last_change: Option<Duration>,
    pub elapsed: Duration,
    pub end_audio_sent: bool,
    pub end_audio_at: Option<Duration>,
}

impl LiveState {
    fn silence_since(self) -> Option<Duration> {
        self.last_change.map(|c| self.elapsed.saturating_sub(c))
    }
}

pub fn next_action(state: &LiveState) -> LiveAction {
    if state.end_audio_sent {
        let sent_at = state.end_audio_at.unwrap_or(Duration::ZERO);
        let wait = if state.have_text {
            UTTERANCE_GRACE
        } else {
            FINAL_GRACE
        };
        if state.elapsed.saturating_sub(sent_at) >= wait {
            return LiveAction::CancelTimeout;
        }
        return LiveAction::Continue;
    }

    if state.have_text {
        if let Some(silence) = state.silence_since() {
            if silence >= SILENCE_LIMIT {
                return LiveAction::EndAudio;
            }
        }
    }

    if !state.have_text && state.elapsed >= NO_SPEECH_LIMIT {
        return LiveAction::CancelNoSpeech;
    }

    if state.elapsed >= HARD_CAP {
        return LiveAction::EndAudio;
    }

    LiveAction::Continue
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(have_text: bool, last_change: Option<Duration>, elapsed: Duration) -> LiveState {
        LiveState {
            have_text,
            last_change,
            elapsed,
            end_audio_sent: false,
            end_audio_at: None,
        }
    }

    #[test]
    fn silence_after_speech_ends_audio() {
        let s = state(
            true,
            Some(Duration::from_millis(1000)),
            Duration::from_millis(2300),
        );
        assert_eq!(next_action(&s), LiveAction::EndAudio);
    }

    #[test]
    fn stable_text_after_end_audio_finishes_quickly() {
        let s = LiveState {
            have_text: true,
            last_change: Some(Duration::from_millis(1000)),
            elapsed: Duration::from_millis(2000),
            end_audio_sent: true,
            end_audio_at: Some(Duration::from_millis(1200)),
        };
        assert_eq!(next_action(&s), LiveAction::CancelTimeout);
    }

    #[test]
    fn no_speech_within_budget_cancels() {
        let s = state(false, None, Duration::from_secs(8));
        assert_eq!(next_action(&s), LiveAction::CancelNoSpeech);
    }
}
