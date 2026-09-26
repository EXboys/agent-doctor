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
        .any(|phrase| loose == *phrase || normalized == *phrase || loose.contains(phrase))
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

pub fn failure_sentence(zh: bool) -> String {
    if zh {
        "这次 AI 没有答成。详细说明在屏幕上。你可以再说一遍，或点对话打字。".into()
    } else {
        "The AI didn't finish this time. Details are on the screen. Say another question, or tap Chat to type.".into()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionRisk {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceDecision {
    Allow,
    Deny,
    Neither,
}

pub fn classify_decision(text: &str) -> VoiceDecision {
    let normalized = normalize(text);
    if normalized.is_empty() {
        return VoiceDecision::Neither;
    }
    const ALLOW: &[&str] = &["允许", "同意", "allow", "yes"];
    const DENY: &[&str] = &["拒绝", "不行", "不可以", "deny", "no"];
    if ALLOW.iter().any(|phrase| normalized == *phrase) {
        return VoiceDecision::Allow;
    }
    if DENY.iter().any(|phrase| normalized == *phrase) {
        return VoiceDecision::Deny;
    }
    VoiceDecision::Neither
}

fn permission_risk(command: &str) -> PermissionRisk {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return PermissionRisk::Low;
    }
    let lower = trimmed.to_lowercase();
    const HIGH: &[&str] = &[
        "api_key",
        "api-key",
        "secret",
        "password",
        "credential",
        "token",
        "key=",
        "_key",
        "sudo",
        " rm ",
        "rm -",
        "curl",
        "wget",
        "chmod",
        "chown",
        "printenv",
        ".env",
        "settings.json",
        " -delete",
        " -exec",
    ];
    if lower.split_whitespace().next() == Some("env")
        || lower.contains(" env ")
        || lower.contains(";env")
        || lower.contains("|env")
        || HIGH.iter().any(|needle| lower.contains(needle))
    {
        return PermissionRisk::High;
    }
    if command_is_basic(&lower) {
        return PermissionRisk::Low;
    }
    PermissionRisk::Medium
}

/// Look, list, and version checks. Writing, installing, and running scripts stay medium.
fn command_is_basic(lower: &str) -> bool {
    if lower.contains('>') || lower.contains('<') {
        return false;
    }
    let segments = lower
        .split([';', '\n', '|', '&'])
        .map(str::trim)
        .filter(|part| !part.is_empty());
    let mut any = false;
    for part in segments {
        any = true;
        if !segment_is_basic(part) {
            return false;
        }
    }
    any
}

fn segment_is_basic(part: &str) -> bool {
    let mut words = part.split_whitespace();
    let Some(bin) = words.next() else {
        return false;
    };
    let bin = bin.rsplit('/').next().unwrap_or(bin);
    const LOOK: &[&str] = &[
        "ls", "pwd", "whoami", "date", "uname", "hostname", "echo", "printf", "cat", "head",
        "tail", "wc", "grep", "rg", "find", "which", "type", "file", "stat", "du", "df", "tree",
        "basename", "dirname", "realpath", "readlink", "sw_vers", "uptime",
    ];
    if bin == "pmset" {
        return part.contains("-g") || part.contains("batt");
    }
    if LOOK.contains(&bin) {
        return true;
    }
    if bin == "git" {
        const READ: &[&str] = &[
            "status",
            "diff",
            "log",
            "show",
            "branch",
            "remote",
            "rev-parse",
            "ls-files",
            "blame",
        ];
        return words.next().is_some_and(|sub| READ.contains(&sub));
    }
    const VERSIONED: &[&str] = &[
        "node", "npm", "pnpm", "yarn", "bun", "python", "python3", "cargo", "rustc", "go",
    ];
    if VERSIONED.contains(&bin) {
        let rest: Vec<&str> = words.collect();
        return rest.is_empty()
            || rest
                .iter()
                .all(|arg| matches!(*arg, "-v" | "--version" | "-V"));
    }
    false
}

/// Spoken ask: what it wants, how risky it is, and whether to allow it.
/// The raw command stays on screen.
pub fn permission_brief(summary: &str, command: &str, zh: bool) -> String {
    let risk = permission_risk(command);
    let summary = summary.trim();
    let human = !summary.is_empty()
        && !summary.contains('{')
        && !summary.contains('|')
        && summary.chars().count() < 48
        && !(zh && summary.is_ascii());
    if zh {
        let what = match risk {
            PermissionRisk::High => "它想查看设置或密钥。".to_string(),
            PermissionRisk::Medium | PermissionRisk::Low if human => {
                format!("它想做这件事：{summary}。")
            }
            PermissionRisk::Low => "它想做几项只读检查，看看这台电脑的情况。".to_string(),
            PermissionRisk::Medium => "它想在这台电脑上执行一条命令。".to_string(),
        };
        let (level, advice) = match risk {
            PermissionRisk::High => ("高", "不建议允许。"),
            PermissionRisk::Medium => ("中等", "请你看完屏幕上的内容再决定。"),
            PermissionRisk::Low => ("低", "可以允许。"),
        };
        format!("{what}危险程度{level}，{advice}说「允许」或「拒绝」。没听清就问「确认的是什么」。")
    } else {
        let what = match risk {
            PermissionRisk::High => "It wants to look at settings or secrets.".to_string(),
            PermissionRisk::Medium | PermissionRisk::Low if human => {
                format!("It wants to do this: {summary}.")
            }
            PermissionRisk::Low => "This is a basic command that only looks.".to_string(),
            PermissionRisk::Medium => "It wants to run a command on this computer.".to_string(),
        };
        let (level, advice) = match risk {
            PermissionRisk::High => ("high", "Allowing is not recommended."),
            PermissionRisk::Medium => ("medium", "Read what is on the screen, then decide."),
            PermissionRisk::Low => ("low", "Allowing is fine."),
        };
        format!(
            "{what} Risk is {level}. {advice} Say allow or deny. The full text is on the screen."
        )
    }
}

pub fn asks_about_permission(text: &str) -> bool {
    let normalized = normalize(text);
    [
        "确认",
        "什么内容",
        "危险",
        "允不允许",
        "要不要允许",
        "能不能允许",
    ]
    .iter()
    .any(|phrase| normalized.contains(phrase))
}

pub fn permission_repeat(zh: bool) -> String {
    if zh {
        "请说「允许」或「拒绝」。".into()
    } else {
        "Say allow or deny.".into()
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
    #[serde(default)]
    pub awaiting_permission: bool,
    #[serde(default)]
    pub permission_script: Option<String>,
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
        #[serde(default)]
        failed: bool,
    },
    SpeakFinished,
    PermissionNeeded {
        summary: String,
        command: String,
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
    Decide { allow: bool },
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
        HostedInput::Leave => {
            if state.active || state.speaking {
                state = HostedState::default();
                effects.push(HostedEffect::StopListen);
                effects.push(HostedEffect::StopSpeak);
                effects.push(HostedEffect::Leave);
            }
        }
        HostedInput::ListenFailed => {
            if state.active && !state.speaking && (!state.model_busy || state.awaiting_permission) {
                effects.push(HostedEffect::StopListen);
                effects.push(HostedEffect::StartListen);
            }
        }
        HostedInput::Heard {
            text,
            status_line,
            zh,
        } => {
            if state.active && !state.speaking {
                if state.awaiting_permission {
                    match classify_decision(&text) {
                        VoiceDecision::Allow | VoiceDecision::Deny => {
                            let allow = matches!(classify_decision(&text), VoiceDecision::Allow);
                            state.awaiting_permission = false;
                            state.permission_script = None;
                            effects.push(HostedEffect::StopListen);
                            effects.push(HostedEffect::Decide { allow });
                        }
                        VoiceDecision::Neither => match classify_utterance(&text) {
                            _ if asks_about_permission(&text) => {
                                state.speaking = true;
                                effects.push(HostedEffect::StopListen);
                                effects.push(HostedEffect::Speak {
                                    text: state
                                        .permission_script
                                        .clone()
                                        .unwrap_or_else(|| permission_repeat(zh)),
                                });
                            }
                            UtteranceKind::Exit => {
                                state = HostedState::default();
                                effects.push(HostedEffect::StopListen);
                                effects.push(HostedEffect::StopSpeak);
                                effects.push(HostedEffect::Leave);
                            }
                            _ => {
                                state.speaking = true;
                                effects.push(HostedEffect::StopListen);
                                effects.push(HostedEffect::Speak {
                                    text: permission_repeat(zh),
                                });
                            }
                        },
                    }
                } else {
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
                            let text = if state.permission_script.is_some() {
                                state
                                    .permission_script
                                    .clone()
                                    .unwrap_or_else(|| permission_repeat(zh))
                            } else {
                                status_sentence(&status_line, zh)
                            };
                            effects.push(HostedEffect::Speak { text });
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
                            }
                        }
                    }
                }
            }
        }
        HostedInput::ModelFinished {
            text,
            announce,
            zh,
            failed,
        } => {
            if state.active {
                state.model_busy = false;
                state.permission_spoken = false;
                state.awaiting_permission = false;
                state.permission_script = None;
                let script = if failed {
                    Some(failure_sentence(zh))
                } else if announce && !text.trim().is_empty() {
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
                } else if state.awaiting_permission {
                    effects.push(HostedEffect::StartListen);
                } else if !state.model_busy {
                    if let Some(held) = state.held.take() {
                        state.model_busy = true;
                        state.permission_spoken = false;
                        effects.push(HostedEffect::Send { text: held });
                    } else {
                        effects.push(HostedEffect::StartListen);
                    }
                }
            }
        }
        HostedInput::PermissionNeeded {
            summary,
            command,
            zh,
        } => {
            if state.active {
                state.awaiting_permission = true;
                state.permission_spoken = true;
                let script = permission_brief(&summary, &command, zh);
                state.permission_script = Some(script.clone());
                state.queued_reply = None;
                state.speaking = true;
                effects.push(HostedEffect::StopSpeak);
                effects.push(HostedEffect::StopListen);
                effects.push(HostedEffect::Speak { text: script });
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
        assert!(!asking
            .effects
            .iter()
            .any(|effect| matches!(effect, HostedEffect::StartListen)));

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
                failed: false,
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
    fn failed_run_is_spoken_without_success_leadin() {
        let state = HostedState {
            active: true,
            model_busy: true,
            ..HostedState::default()
        };
        let step = reduce(
            state,
            HostedInput::ModelFinished {
                text: String::new(),
                announce: true,
                zh: true,
                failed: true,
            },
        );
        assert!(!step.state.model_busy);
        assert!(step.effects.iter().any(|effect| matches!(
            effect,
            HostedEffect::Speak { text } if text.contains("没有答成") || text.contains("didn't finish")
        )));
    }

    #[test]
    fn basic_lookup_commands_can_be_allowed() {
        for command in ["ls", "pwd", "git status", "cat README.md", "node --version"] {
            let spoken = permission_brief("看看项目", command, true);
            assert!(spoken.contains("危险程度低"), "{command}: {spoken}");
            assert!(spoken.contains("可以允许"), "{command}: {spoken}");
        }
        let install = permission_brief("安装依赖", "npm install", true);
        assert!(install.contains("危险程度中等"), "{install}");
    }

    #[test]
    fn secret_command_is_high_risk_and_not_recommended() {
        let spoken = permission_brief(
            "Check Claude Code endpoint settings",
            "cat ~/.claude/settings.json; env | grep KEY",
            true,
        );
        assert!(spoken.contains("密钥"));
        assert!(spoken.contains("危险程度高"));
        assert!(spoken.contains("不建议允许"));
        assert!(spoken.contains("允许"));
        assert!(!spoken.contains("settings.json"));
    }

    #[test]
    fn voice_allow_and_deny_resolve_the_card() {
        let waiting = HostedState {
            active: true,
            model_busy: true,
            awaiting_permission: true,
            ..HostedState::default()
        };
        let allowed = reduce(
            waiting.clone(),
            HostedInput::Heard {
                text: "允许".into(),
                status_line: String::new(),
                zh: true,
            },
        );
        assert!(!allowed.state.awaiting_permission);
        assert!(allowed
            .effects
            .contains(&HostedEffect::Decide { allow: true }));
        let denied = reduce(
            waiting,
            HostedInput::Heard {
                text: "拒绝".into(),
                status_line: String::new(),
                zh: true,
            },
        );
        assert!(denied
            .effects
            .contains(&HostedEffect::Decide { allow: false }));
    }

    #[test]
    fn listen_failure_retries_without_leaving_voice() {
        let state = HostedState {
            active: true,
            ..HostedState::default()
        };
        let step = reduce(state, HostedInput::ListenFailed);
        assert!(step.state.active);
        assert!(step.effects.contains(&HostedEffect::StopListen));
        assert!(step.effects.contains(&HostedEffect::StartListen));
        assert!(!step
            .effects
            .iter()
            .any(|effect| matches!(effect, HostedEffect::Leave)));
    }

    #[test]
    fn listen_failure_while_busy_does_not_open_mic() {
        let state = HostedState {
            active: true,
            model_busy: true,
            ..HostedState::default()
        };
        let step = reduce(state, HostedInput::ListenFailed);
        assert!(step.state.active);
        assert!(step.effects.is_empty());
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
