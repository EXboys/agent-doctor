//! When playback should stop because the person started talking.
//!
//! The microphone samples are assumed to already have the computer's own
//! voice removed. A short noise does not interrupt. Speech has to last
//! [`SPEECH_HOLD_MS`] so a door or a key does not cut the reply off.

/// How long cleaned speech must continue before playback stops.
pub const SPEECH_HOLD_MS: u64 = 300;
/// Echo cancellation needs a moment after playback starts. Ignore the mic until then.
pub const ARM_AFTER_MS: u64 = 400;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BargeAction {
    Continue,
    Interrupt,
}

/// Speaker bleed learned during warmup, plus a margin, before a voice counts.
pub fn bleed_line(floor: f32) -> f32 {
    (floor * 2.5).max(floor + 0.012).max(0.018)
}

/// Time spent learning how loud the speakers sound in the mic.
pub const BLEED_LEARN_MS: u64 = 600;

/// While the computer is speaking through the speakers, the mic hears that
/// playback. The first moments measure that bleed. A voice has to stay
/// clearly louder before playback stops.
#[derive(Debug, Clone)]
pub struct BleedGate {
    floor: f32,
    elapsed_ms: u64,
    held_ms: u64,
}

impl BleedGate {
    pub fn new() -> Self {
        Self {
            floor: 0.0,
            elapsed_ms: 0,
            held_ms: 0,
        }
    }

    pub fn floor(&self) -> f32 {
        self.floor
    }

    pub fn push(&mut self, rms: f32, frame_ms: u64) -> BargeAction {
        self.elapsed_ms = self.elapsed_ms.saturating_add(frame_ms);
        if self.elapsed_ms < BLEED_LEARN_MS {
            if rms > self.floor {
                self.floor = rms;
            }
            self.held_ms = 0;
            return BargeAction::Continue;
        }
        // Gaps between syllables only drain the count halfway, so normal speech still adds up.
        if rms >= bleed_line(self.floor) {
            self.held_ms = self.held_ms.saturating_add(frame_ms);
        } else {
            self.held_ms = self.held_ms.saturating_sub(frame_ms / 2);
        }
        if self.held_ms >= SPEECH_HOLD_MS {
            BargeAction::Interrupt
        } else {
            BargeAction::Continue
        }
    }
}

impl Default for BleedGate {
    fn default() -> Self {
        Self::new()
    }
}

/// `held_ms` is how long speech has already lasted.
/// `armed` is false during the echo-cancellation warmup.
pub fn advance(
    held_ms: u64,
    armed: bool,
    hearing_speech: bool,
    frame_ms: u64,
) -> (u64, BargeAction) {
    if !armed || !hearing_speech || frame_ms == 0 {
        return (0, BargeAction::Continue);
    }
    let held = held_ms.saturating_add(frame_ms);
    if held >= SPEECH_HOLD_MS {
        (held, BargeAction::Interrupt)
    } else {
        (held, BargeAction::Continue)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warmup_and_a_short_noise_do_not_interrupt() {
        assert_eq!(advance(0, false, true, 200), (0, BargeAction::Continue));
        assert_eq!(advance(0, true, true, 80), (80, BargeAction::Continue));
        assert_eq!(advance(80, true, false, 20), (0, BargeAction::Continue));
    }

    #[test]
    fn sustained_speech_interrupts() {
        let (held, action) = advance(200, true, true, 100);
        assert_eq!(held, 300);
        assert_eq!(action, BargeAction::Interrupt);
    }

    #[test]
    fn speaker_bleed_does_not_interrupt_until_a_louder_voice_holds() {
        let mut gate = BleedGate::new();
        assert_eq!(gate.push(0.05, 300), BargeAction::Continue);
        assert_eq!(gate.push(0.05, 300), BargeAction::Continue);
        assert_eq!(gate.push(0.07, 200), BargeAction::Continue);
        assert_eq!(gate.push(0.2, 300), BargeAction::Interrupt);
    }

    #[test]
    fn a_quiet_speaker_lets_a_normal_voice_through_with_syllable_gaps() {
        let mut gate = BleedGate::new();
        assert_eq!(gate.push(0.007, 600), BargeAction::Continue);
        assert_eq!(gate.push(0.012, 400), BargeAction::Continue);
        let mut action = BargeAction::Continue;
        for _ in 0..6 {
            gate.push(0.03, 80);
            action = gate.push(0.01, 40);
        }
        assert_eq!(action, BargeAction::Interrupt);
    }
}
