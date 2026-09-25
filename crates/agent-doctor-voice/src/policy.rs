//! Hosted-voice decisions that do not touch the microphone or speaker.

use serde::{Deserialize, Serialize};

const MAX_BODY_CHARS: usize = 180;
const MAX_STATUS_CHARS: usize = 80;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UtteranceKind {
    Exit,
    Status,
    Message,
}

pub fn classify_utterance(text: &str) -> UtteranceKind {
    let normalized = normalize(text);
    if normalized.is_empty() {
        return UtteranceKind::Message;
    }
    if is_exit(&normalized) {
        return UtteranceKind::Exit;
    }
    if is_status(&normalized) {
        return UtteranceKind::Status;
    }
    UtteranceKind::Message
}

fn normalize(text: &str) -> String {
    text.trim()
        .trim_matches(|c: char| {
            c.is_whitespace() || matches!(c, '。' | '！' | '？' | '!' | '.' | ',' | '，' | '、')
        })
        .to_lowercase()
}

fn is_exit(normalized: &str) -> bool {
    matches!(
        normalized,
        "退出"
            | "退出语音"
            | "退出语音对话"
            | "停止语音"
            | "结束语音"
            | "stop"
            | "stop listening"
            | "exit"
            | "exit voice"
            | "quit"
    )
}

fn is_status(normalized: &str) -> bool {
    if normalized.chars().count() > 32 {
        return false;
    }
    let loose = normalized.trim_end_matches(['呢', '啊', '呀', '吧', '?', '？']);
    const PHRASES: &[&str] = &[
        "到哪了",
        "到哪儿了",
        "还在吗",
        "什么进度",
        "进度",
        "进度怎么样",
        "什么状态",
        "现在什么情况",
        "好了没",
        "完成了吗",
        "怎么样了",
        "status",
        "how's it going",
        "still there",
        "what is the status",
        "what's the status",
        "how far",
        "are you done",
    ];
    PHRASES
        .iter()
        .any(|phrase| loose == *phrase || normalized == *phrase)
}

pub fn status_sentence(activity: &str, zh: bool) -> String {
    let activity = clip_chars(activity.trim(), MAX_STATUS_CHARS);
    if activity.is_empty() {
        return if zh {
            "还在处理。".into()
        } else {
            "Still working.".into()
        };
    }
    if zh {
        format!("还在处理。{activity}")
    } else {
        format!("Still working. {activity}")
    }
}

pub fn ack_sentence(zh: bool) -> String {
    if zh {
        "先记下了，这轮结束后再问。".into()
    } else {
        "I'll ask that after this reply.".into()
    }
}

pub fn permission_sentence(zh: bool) -> String {
    if zh {
        "屏幕上有一项需要你点允许或拒绝。".into()
    } else {
        "Something on the screen needs you to choose allow or deny.".into()
    }
}

/// Lead-in plus a short spoken body. Code blocks and tables stay on screen.
pub fn spoken_reply(markdown: &str, zh: bool) -> String {
    let (prose, had_code, had_table) = strip_markdown(markdown);
    let mut body = prose;
    if had_code {
        let note = if zh {
            "这里有一段代码，完整内容在屏幕上。"
        } else {
            "There is some code. The full text is on the screen."
        };
        body = if body.is_empty() {
            note.to_string()
        } else {
            format!("{body} {note}")
        };
    } else if had_table && body.is_empty() {
        body = if zh {
            "有一张表，完整内容在屏幕上。".into()
        } else {
            "There is a table. The full text is on the screen.".into()
        };
    }
    if body.is_empty() {
        body = if zh {
            "完整内容在屏幕上。".into()
        } else {
            "The full text is on the screen.".into()
        };
    }
    let truncated = body.chars().count() > MAX_BODY_CHARS;
    if truncated {
        let mut clipped: String = body.chars().take(MAX_BODY_CHARS).collect();
        clipped.push_str(if zh {
            "后面还有，完整内容在屏幕上。"
        } else {
            " The rest is on the screen."
        });
        body = clipped;
    }
    let lead = if zh {
        "AI 已经有回复，内容如下。"
    } else {
        "The reply is ready. Here it is."
    };
    format!("{lead}{body}")
}

fn clip_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    text.chars().take(max).collect()
}

fn strip_markdown(input: &str) -> (String, bool, bool) {
    let mut prose = String::new();
    let mut in_code = false;
    let mut had_code = false;
    let mut had_table = false;
    for line in input.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            in_code = !in_code;
            had_code = true;
            continue;
        }
        if in_code {
            had_code = true;
            continue;
        }
        if is_table_line(trimmed) {
            had_table = true;
            continue;
        }
        let inline = strip_inline(trimmed);
        if inline.is_empty() {
            continue;
        }
        if !prose.is_empty() {
            prose.push(' ');
        }
        prose.push_str(&inline);
    }
    (prose, had_code, had_table)
}

fn is_table_line(trimmed: &str) -> bool {
    if trimmed.starts_with('|') {
        return true;
    }
    let dashes = trimmed.chars().filter(|c| *c == '-').count();
    trimmed.contains('|') && dashes >= 3
}

fn strip_inline(line: &str) -> String {
    let without_heading = line.trim_start_matches('#').trim();
    let mut out = String::new();
    let chars: Vec<char> = without_heading.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '`' {
            let mut j = i + 1;
            while j < chars.len() && chars[j] != '`' {
                j += 1;
            }
            if j < chars.len() {
                let inner: String = chars[i + 1..j].iter().collect();
                if inner.chars().count() <= 40 {
                    out.push_str(inner.trim());
                }
                i = j + 1;
                continue;
            }
        }
        if chars[i] == '!' && chars.get(i + 1) == Some(&'[') {
            if let Some(end) = take_link(&chars, i + 1) {
                out.push_str(&end.label);
                i = end.next;
                continue;
            }
        }
        if chars[i] == '[' {
            if let Some(end) = take_link(&chars, i) {
                if !out.is_empty() && !out.ends_with(' ') {
                    out.push(' ');
                }
                out.push_str(&end.label);
                i = end.next;
                continue;
            }
        }
        if matches!(chars[i], '*' | '_' | '~') {
            i += 1;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

struct LinkEnd {
    label: String,
    next: usize,
}

fn take_link(chars: &[char], start: usize) -> Option<LinkEnd> {
    if chars.get(start) != Some(&'[') {
        return None;
    }
    let mut j = start + 1;
    let mut label = String::new();
    while j < chars.len() && chars[j] != ']' {
        label.push(chars[j]);
        j += 1;
    }
    if j >= chars.len() || chars.get(j + 1) != Some(&'(') {
        return None;
    }
    j += 2;
    while j < chars.len() && chars[j] != ')' {
        j += 1;
    }
    if j >= chars.len() {
        return None;
    }
    Some(LinkEnd {
        label: label.trim().to_string(),
        next: j + 1,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct HostedState {
    pub active: bool,
    pub model_busy: bool,
    pub speaking: bool,
    pub permission_spoken: bool,
    pub held: Option<String>,
    pub queued_reply: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum HostedInput {
    Enter,
    Leave,
    Heard {
        text: String,
        status_line: String,
        zh: bool,
    },
    ModelFinished {
        text: String,
        announce: bool,
        zh: bool,
    },
    SpeakFinished,
    PermissionNeeded {
        zh: bool,
    },
    ListenFailed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum HostedEffect {
    StartListen,
    StopListen,
    StopSpeak,
    Speak { text: String },
    Send { text: String },
    Leave,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostedStep {
    pub state: HostedState,
    pub effects: Vec<HostedEffect>,
}

pub fn reduce(mut state: HostedState, input: HostedInput) -> HostedStep {
    let mut effects = Vec::new();
    match input {
        HostedInput::Enter => {
            if !state.active {
                state = HostedState {
                    active: true,
                    ..HostedState::default()
                };
                effects.push(HostedEffect::StartListen);
            }
        }
        HostedInput::Leave | HostedInput::ListenFailed => {
            if state.active || state.speaking {
                state = HostedState::default();
                effects.push(HostedEffect::StopListen);
                effects.push(HostedEffect::StopSpeak);
                effects.push(HostedEffect::Leave);
            }
        }
        HostedInput::Heard {
            text,
            status_line,
            zh,
        } => {
            if state.active && !state.speaking {
                match classify_utterance(&text) {
                    UtteranceKind::Exit => {
                        state = HostedState::default();
                        effects.push(HostedEffect::StopListen);
                        effects.push(HostedEffect::StopSpeak);
                        effects.push(HostedEffect::Leave);
                    }
                    UtteranceKind::Status => {
                        state.speaking = true;
                        effects.push(HostedEffect::StopListen);
                        effects.push(HostedEffect::Speak {
                            text: status_sentence(&status_line, zh),
                        });
                    }
                    UtteranceKind::Message => {
                        let text = text.trim().to_string();
                        if text.is_empty() {
                            // Keep listening.
                        } else if state.model_busy {
                            state.held = Some(text);
                            state.speaking = true;
                            effects.push(HostedEffect::StopListen);
                            effects.push(HostedEffect::Speak {
                                text: ack_sentence(zh),
                            });
                        } else {
                            state.model_busy = true;
                            state.permission_spoken = false;
                            effects.push(HostedEffect::StopListen);
                            effects.push(HostedEffect::Send { text });
                            effects.push(HostedEffect::StartListen);
                        }
                    }
                }
            }
        }
        HostedInput::ModelFinished { text, announce, zh } => {
            if state.active {
                state.model_busy = false;
                state.permission_spoken = false;
                let script = if announce && !text.trim().is_empty() {
                    Some(spoken_reply(&text, zh))
                } else {
                    None
                };
                if state.speaking {
                    if script.is_some() {
                        state.queued_reply = script;
                    }
                } else if let Some(script) = script {
                    state.speaking = true;
                    effects.push(HostedEffect::StopListen);
                    effects.push(HostedEffect::Speak { text: script });
                } else if let Some(held) = state.held.take() {
                    state.model_busy = true;
                    effects.push(HostedEffect::Send { text: held });
                }
            }
        }
        HostedInput::SpeakFinished => {
            if state.active && state.speaking {
                state.speaking = false;
                if let Some(script) = state.queued_reply.take() {
                    state.speaking = true;
                    effects.push(HostedEffect::Speak { text: script });
                } else if !state.model_busy {
                    if let Some(held) = state.held.take() {
                        state.model_busy = true;
                        state.permission_spoken = false;
                        effects.push(HostedEffect::Send { text: held });
                        effects.push(HostedEffect::StartListen);
                    } else {
                        effects.push(HostedEffect::StartListen);
                    }
                } else {
                    effects.push(HostedEffect::StartListen);
                }
            }
        }
        HostedInput::PermissionNeeded { zh } => {
            if state.active && !state.speaking && !state.permission_spoken {
                state.permission_spoken = true;
                state.speaking = true;
                effects.push(HostedEffect::StopListen);
                effects.push(HostedEffect::Speak {
                    text: permission_sentence(zh),
                });
            }
        }
    }
    HostedStep { state, effects }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heard_from_the_window_reaches_the_model() {
        let input: HostedInput = serde_json::from_str(
            r#"{"type":"heard","text":"帮我看一下","statusLine":"","zh":true}"#,
        )
        .expect("window sends camelCase fields");
        let state = HostedState {
            active: true,
            ..HostedState::default()
        };
        let step = reduce(state, input);
        assert!(step.effects.contains(&HostedEffect::Send {
            text: "帮我看一下".into()
        }));
    }

    #[test]
    fn exit_and_status_are_not_sent_to_the_model() {
        assert_eq!(classify_utterance("退出。"), UtteranceKind::Exit);
        assert_eq!(classify_utterance("Stop listening"), UtteranceKind::Exit);
        assert_eq!(classify_utterance("还在吗"), UtteranceKind::Status);
        assert_eq!(
            classify_utterance("what's the status?"),
            UtteranceKind::Status
        );
        assert_eq!(
            classify_utterance("帮我看看这个项目"),
            UtteranceKind::Message
        );
    }

    #[test]
    fn spoken_reply_skips_code_and_caps_length() {
        let spoken = spoken_reply("先说结论。\n```rust\nfn main() {}\n```\n", true);
        assert!(spoken.starts_with("AI 已经有回复，内容如下。"));
        assert!(spoken.contains("先说结论"));
        assert!(spoken.contains("这里有一段代码"));
        assert!(!spoken.contains("fn main"));

        let long = "字".repeat(400);
        let clipped = spoken_reply(&long, true);
        assert!(clipped.contains("后面还有，完整内容在屏幕上。"));
        assert!(clipped.chars().count() < 250);
    }

    #[test]
    fn hosted_holds_a_question_until_the_reply_is_spoken() {
        let entered = reduce(HostedState::default(), HostedInput::Enter);
        assert!(entered.state.active);
        assert_eq!(entered.effects, vec![HostedEffect::StartListen]);

        let asking = reduce(
            entered.state,
            HostedInput::Heard {
                text: "帮我看一下".into(),
                status_line: String::new(),
                zh: true,
            },
        );
        assert!(asking.state.model_busy);
        assert!(asking.effects.iter().any(|effect| matches!(
            effect,
            HostedEffect::Send { text } if text == "帮我看一下"
        )));

        let held = reduce(
            asking.state,
            HostedInput::Heard {
                text: "顺便改个标题".into(),
                status_line: String::new(),
                zh: true,
            },
        );
        assert_eq!(held.state.held.as_deref(), Some("顺便改个标题"));
        assert!(held.effects.iter().any(|effect| matches!(
            effect,
            HostedEffect::Speak { text } if text.contains("先记下了")
        )));

        let done = reduce(
            held.state,
            HostedInput::ModelFinished {
                text: "看完了".into(),
                announce: true,
                zh: true,
            },
        );
        assert!(done
            .state
            .queued_reply
            .as_ref()
            .is_some_and(|text| text.contains("看完了")));
        assert!(!done.state.model_busy);

        let after_ack = reduce(done.state, HostedInput::SpeakFinished);
        assert!(after_ack.state.speaking);
        assert!(after_ack.effects.iter().any(|effect| matches!(
            effect,
            HostedEffect::Speak { text } if text.contains("看完了")
        )));

        let after_reply = reduce(after_ack.state, HostedInput::SpeakFinished);
        assert!(after_reply.state.model_busy);
        assert_eq!(
            after_reply.effects.iter().find_map(|effect| match effect {
                HostedEffect::Send { text } => Some(text.as_str()),
                _ => None,
            }),
            Some("顺便改个标题")
        );
    }

    #[test]
    fn status_while_waiting_does_not_send() {
        let step = reduce(
            HostedState {
                active: true,
                model_busy: true,
                ..HostedState::default()
            },
            HostedInput::Heard {
                text: "到哪了".into(),
                status_line: "正在使用工具".into(),
                zh: true,
            },
        );
        assert!(step.state.held.is_none());
        assert!(step.effects.iter().any(|effect| matches!(
            effect,
            HostedEffect::Speak { text } if text.contains("正在使用工具")
        )));
        assert!(!step
            .effects
            .iter()
            .any(|effect| matches!(effect, HostedEffect::Send { .. })));
    }
}
