//! Is the cleaned microphone hearing a person?
//!
//! Playback turns this on only after echo cancellation. A later on-device
//! model (such as Silero) can replace [`EchoCleanedActivity`] without
//! changing when playback stops.

/// Loudness left on the mic after the computer's own voice is removed.
pub const AFTER_ECHO_RMS: f32 = 0.02;

pub trait SpeechActivity: Send {
    fn hearing_speech(&mut self, rms: f32) -> bool;
}

#[derive(Debug, Clone, Copy)]
pub struct EchoCleanedActivity {
    threshold: f32,
}

impl Default for EchoCleanedActivity {
    fn default() -> Self {
        Self {
            threshold: AFTER_ECHO_RMS,
        }
    }
}

impl SpeechActivity for EchoCleanedActivity {
    fn hearing_speech(&mut self, rms: f32) -> bool {
        rms >= self.threshold
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quiet_room_is_not_speech() {
        let mut ear = EchoCleanedActivity::default();
        assert!(!ear.hearing_speech(0.01));
        assert!(ear.hearing_speech(0.08));
    }
}
