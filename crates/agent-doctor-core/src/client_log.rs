//! One manual diagnosis report: the user pressed "上报日志" on a failed
//! confirmation card, so the local prompt-session log travels to the server.
//!
//! Consent is the click itself — nothing is sent automatically. The log holds
//! ids, tool names, error classes, and timings only; prompts, commands, and
//! file paths never enter it (see the desktop `prompt_log` module). The
//! payload is anonymous and correlates with the install ping through the same
//! `install_id`.
//!
//! Server contract (agentdoctor.cn): `POST /api/v1/doctor/client-log`,
//! JSON body, any 2xx accepted. Override the URL with
//! `AGENT_DOCTOR_CLIENT_LOG_URL` for local testing.

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

/// What went wrong, as shown on the card. Today only failed allows offer the
/// report button; more reasons can join without a shape change.
pub const REASON_ALLOW_MISSED: &str = "allow-missed";

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ClientLogReport {
    pub install_id: String,
    pub version: &'static str,
    /// std::env::consts::OS.
    pub os: &'static str,
    /// std::env::consts::ARCH.
    pub arch: &'static str,
    pub client: &'static str,
    /// Why the button showed ("allow-missed").
    pub reason: String,
    /// The rolling prompt-session log. Ids and error classes, no chat content.
    pub log: String,
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
}
