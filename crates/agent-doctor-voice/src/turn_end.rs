//! When to send a spoken sentence.
//!
//! [`assess_turn_end`] decides how long the microphone must stay quiet before a
//! sentence is sent. Hearing a voice only means "don't send yet". Quiet time is
//! a separate wait: about 2.5s when the sentence looks finished, about 5s when
//! it still looks like thinking. A later on-device model replaces the rules by
//! calling [`set_turn_end_decider`].

use std::sync::{Arc, OnceLock, RwLock};

use serde::{Deserialize, Serialize};

/// Microphone must stay quiet this long before a finished sentence is sent.
pub const COMPLETE_HOLD_MS: u64 = 2500;
/// Longer quiet time when the words still look unfinished, such as a trailing「额」.
pub const THINKING_HOLD_MS: u64 = 5000;

/// Hesitation and "still going" endings. Longest first so "那个" wins over "个".
const FILLERS: &[&str] = &[
    "那个", "就是", "然后", "呃", "额", "嗯", "啊", "呀", "哦", "唔",
];
const UNFINISHED: &[&str] = &[
    "可不可以",
    "能不能",
    "是不是",
    "一下",
    "的话",
    "然后",
    "就是",
    "那个",
    "如果",
    "但是",
    "所以",
    "因为",
    "还有",
    "以及",
    "或者",
    "帮我",
    "给我",
    "让我",
    "怎么",
    "什么",
    "哪个",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnEnd {
    pub thinking: bool,
    pub hold_ms: u64,
    pub text_to_send: String,
}

pub trait TurnEndDecider: Send + Sync {
    fn assess(&self, text: &str) -> TurnEnd;
}

pub struct RuleTurnEnd;

static DECIDER: OnceLock<RwLock<Arc<dyn TurnEndDecider>>> = OnceLock::new();

/// Swap the pause decision. The next assess uses `decider`.
pub fn set_turn_end_decider(decider: Arc<dyn TurnEndDecider>) {
    if let Some(slot) = DECIDER.get() {
        *slot.write().unwrap_or_else(|err| err.into_inner()) = decider;
    } else {
        let _ = DECIDER.set(RwLock::new(decider));
    }
}

/// How long the words must sit still before a later sound counts as thinking.
pub const TEXT_SETTLED_MS: u64 = 300;
/// A real gap, so ongoing speech is not treated as a new sound.
pub const QUIET_GAP_MS: u64 = 250;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UnwrittenVoice {
    pub saw_quiet: bool,
    pub sound_after_words: bool,
}

/// Words already stopped, the mic went quiet, then a sound came back and no new
/// word was written. That sound is the thinking noise the recognizer dropped.
pub fn note_unwritten_voice(
    state: UnwrittenVoice,
    has_text: bool,
    text_changed: bool,
    text_stable_ms: u64,
    quiet_for_ms: u64,
) -> UnwrittenVoice {
    if !has_text || text_changed {
        return UnwrittenVoice::default();
    }
    let saw_quiet =
        state.saw_quiet || (text_stable_ms >= TEXT_SETTLED_MS && quiet_for_ms >= QUIET_GAP_MS);
    let sound_after_words = state.sound_after_words || (saw_quiet && quiet_for_ms < QUIET_GAP_MS);
    UnwrittenVoice {
        saw_quiet,
        sound_after_words,
    }
}

pub fn quiet_ms(text: &str, sound_after_words: bool) -> u64 {
    if text.trim().is_empty() {
        return COMPLETE_HOLD_MS;
    }
    if sound_after_words || assess_turn_end(text).thinking {
        THINKING_HOLD_MS
    } else {
        COMPLETE_HOLD_MS
    }
}

pub fn assess_turn_end(text: &str) -> TurnEnd {
    let slot = DECIDER.get_or_init(|| RwLock::new(Arc::new(RuleTurnEnd)));
    slot.read()
        .unwrap_or_else(|err| err.into_inner())
        .assess(text)
}

impl TurnEndDecider for RuleTurnEnd {
    fn assess(&self, text: &str) -> TurnEnd {
        let trimmed = text.trim();
        let send = strip_fillers(trimmed);
        let thinking =
            send.is_empty() || ends_with_any(&send, UNFINISHED) || ends_with_any(trimmed, FILLERS);
        TurnEnd {
            thinking,
            hold_ms: if thinking {
                THINKING_HOLD_MS
            } else {
                COMPLETE_HOLD_MS
            },
            text_to_send: send,
        }
    }
}

fn ends_with_any(text: &str, parts: &[&str]) -> bool {
    parts.iter().any(|part| text.ends_with(part))
}

fn strip_fillers(text: &str) -> String {
    let mut rest = trim_punct(text).to_string();
    loop {
        let before = rest.clone();
        for filler in FILLERS {
            if let Some(next) = rest.strip_suffix(filler) {
                rest = trim_punct(next).to_string();
                break;
            }
        }
        if rest == before {
            break;
        }
    }
    rest
}

fn trim_punct(text: &str) -> &str {
    text.trim().trim_matches(|c: char| {
        c.is_whitespace()
            || matches!(
                c,
                '。' | '，' | '、' | '！' | '？' | '!' | '.' | ',' | '?' | '…'
            )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finished_sentence_waits_the_short_pause() {
        let end = RuleTurnEnd.assess("今天天气怎么样");
        assert!(!end.thinking);
        assert_eq!(end.hold_ms, COMPLETE_HOLD_MS);
        assert_eq!(end.text_to_send, "今天天气怎么样");
    }

    #[test]
    fn hesitation_waits_longer_and_is_not_sent() {
        let end = RuleTurnEnd.assess("帮我看一下桌面上的文件额");
        assert!(end.thinking);
        assert_eq!(end.hold_ms, THINKING_HOLD_MS);
        assert_eq!(end.text_to_send, "帮我看一下桌面上的文件");
    }

    #[test]
    fn unfinished_phrase_waits_longer() {
        let end = RuleTurnEnd.assess("帮我看一下");
        assert!(end.thinking);
        assert_eq!(end.text_to_send, "帮我看一下");
    }

    #[test]
    fn sound_after_a_pause_extends_the_wait_without_a_new_word() {
        let paused = note_unwritten_voice(UnwrittenVoice::default(), true, false, 400, 300);
        assert!(paused.saw_quiet);
        assert!(!paused.sound_after_words);
        let hummed = note_unwritten_voice(paused, true, false, 500, 0);
        assert!(hummed.sound_after_words);
        assert_eq!(quiet_ms("今天天气怎么样", true), THINKING_HOLD_MS);
        assert_eq!(quiet_ms("今天天气怎么样", false), COMPLETE_HOLD_MS);
    }

    #[test]
    fn ongoing_speech_does_not_count_as_a_later_sound() {
        let speaking = note_unwritten_voice(UnwrittenVoice::default(), true, false, 100, 0);
        assert!(!speaking.sound_after_words);
    }

    #[test]
    fn filler_only_is_not_a_message() {
        let end = RuleTurnEnd.assess("嗯啊");
        assert!(end.thinking);
        assert!(end.text_to_send.is_empty());
    }
}
