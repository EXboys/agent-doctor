//! One manual diagnosis report: the user pressed "上报日志" on a failed
//! confirmation card, rated a reply, or filed feedback from the tray/settings.
//!
//! Consent is the click itself — a rating click is a consent click like any
//! other. The log holds ids, tool names, error classes, and timings only;
//! prompts, commands, and file paths never enter it (see the desktop
//! `prompt_log` module). The payload is anonymous and correlates with the
//! install ping through the same `install_id`.
//!
//! Server contract (agentdoctor.cn): `POST /api/v1/doctor/client-log`,
//! JSON body, any 2xx accepted. Optional fields (`note`, `runtime`,
//! `turn_status`) are omitted when absent, so older receivers keep working.
//! Override the URL with `AGENT_DOCTOR_CLIENT_LOG_URL` for local testing.

use std::time::Duration;

use anyhow::{Context, Result};
use serde::Serialize;

pub const CLIENT_LOG_URL: &str = "https://agentdoctor.cn/api/v1/doctor/client-log";

pub fn client_log_url() -> String {
    std::env::var("AGENT_DOCTOR_CLIENT_LOG_URL")
        .ok()
        .map(|v| v.trim().trim_end_matches('/').to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| CLIENT_LOG_URL.to_string())
}

/// Why the report exists. The set only grows; receivers treat it as open.
pub const REASON_ALLOW_MISSED: &str = "allow-missed";
pub const REASON_REPLY_UP: &str = "reply-up";
pub const REASON_REPLY_DOWN: &str = "reply-down";
pub const REASON_REPLY_NOTE: &str = "reply-note";
pub const REASON_MANUAL: &str = "manual";
pub const REASON_TURN_FAILURE: &str = "turn-failure";

/// Longest user description accepted, in characters. Descriptions are
/// user-authored free text, unlike the generated log, so they get a cap.
pub const NOTE_MAX_CHARS: usize = 2000;

/// Trim a user-written description; `None` when nothing remains. Long text is
/// cut on a char boundary so the JSON payload stays valid UTF-8.
pub fn clean_note(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.chars().count() <= NOTE_MAX_CHARS {
        return Some(trimmed.to_string());
    }
    Some(trimmed.chars().take(NOTE_MAX_CHARS).collect())
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ClientLogReport {
    pub install_id: String,
    pub version: &'static str,
    /// std::env::consts::OS.
    pub os: &'static str,
    /// std::env::consts::ARCH.
    pub arch: &'static str,
    pub client: &'static str,
    /// Why the report exists ("allow-missed", "reply-down", …).
    pub reason: String,
    /// The rolling prompt-session log. Ids and error classes, no chat content.
    pub log: String,
    /// Optional user-written description (feedback form, rating supplement).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Ask runtime of the rated turn, when the report is about one ("claude").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime: Option<String>,
    /// Terminal status of the rated turn, when known ("succeeded", "failed").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_status: Option<String>,
}

pub fn post_client_log(url: &str, report: &ClientLogReport) -> Result<()> {
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(20))
        .build()
        .context("failed to build client log HTTP client")?;
    let response = client
        .post(url)
        .json(report)
        .send()
        .context("failed to send client log")?;
    let status = response.status();
    if !status.is_success() {
        anyhow::bail!("client log endpoint replied {status}");
    }
    Ok(())
}

/// Send through the configured URL (default agentdoctor.cn).
pub fn send_client_log(report: &ClientLogReport) -> Result<()> {
    post_client_log(&client_log_url(), report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_payload_carries_only_anonymous_fields() {
        let report = ClientLogReport {
            install_id: "00000000-0000-0000-0000-000000000000".into(),
            version: "0.0.0",
            os: "windows",
            arch: "x86_64",
            client: "desktop",
            reason: REASON_ALLOW_MISSED.into(),
            log: "2026-11-09T23:28:01.000Z resolve_err class=dead-pipe ms=4012\n".into(),
            note: None,
            runtime: None,
            turn_status: None,
        };
        let json = serde_json::to_value(&report).unwrap();
        let mut names: Vec<_> = json
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        names.sort_unstable();
        assert_eq!(
            names,
            [
                "arch",
                "client",
                "install_id",
                "log",
                "os",
                "reason",
                "version"
            ]
        );
    }

    #[test]
    fn optional_context_fields_serialize_when_present() {
        let report = ClientLogReport {
            install_id: "00000000-0000-0000-0000-000000000000".into(),
            version: "0.0.0",
            os: "macos",
            arch: "aarch64",
            client: "desktop",
            reason: REASON_REPLY_DOWN.into(),
            log: String::new(),
            note: Some("工具一直转圈".into()),
            runtime: Some("claude".into()),
            turn_status: Some("succeeded".into()),
        };
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["note"], "工具一直转圈");
        assert_eq!(json["runtime"], "claude");
        assert_eq!(json["turn_status"], "succeeded");
        assert_eq!(json["reason"], REASON_REPLY_DOWN);
    }

    #[test]
    fn clean_note_trims_caps_and_drops_blanks() {
        assert_eq!(clean_note("  \n "), None);
        assert_eq!(clean_note("  转圈不停 \n"), Some("转圈不停".into()));
        let long = "ab".repeat(NOTE_MAX_CHARS + 50);
        let capped = clean_note(&long).unwrap();
        assert_eq!(capped.chars().count(), NOTE_MAX_CHARS);
    }
}
