//! Browser login for TeamUps (device code → open site → poll → save license).

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

use crate::store::{default_teamups_base_url, open_settings_store, SkillsSourceKind};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamupsLoginStart {
    pub device_code: String,
    pub user_code: String,
    pub verification_url: String,
    pub interval_sec: u64,
    pub expires_in_sec: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TeamupsLoginPollStatus {
    Pending,
    Approved,
    Expired,
    Denied,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamupsLoginPoll {
    pub status: TeamupsLoginPollStatus,
    #[serde(default)]
    pub packs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamupsAccountStatus {
    pub base_url: String,
    pub signed_in: bool,
    #[serde(default)]
    pub pack_count: u32,
    #[serde(default)]
    pub packs: Vec<String>,
}

fn mall_base_url() -> Result<String> {
    let store = open_settings_store()?;
    let configured = store.get_skills_source_settings()?;
    let kind = configured.source.unwrap_or(SkillsSourceKind::Teamups);
    if matches!(kind, SkillsSourceKind::Teamups | SkillsSourceKind::Custom) {
        if let Some(url) = configured
            .base_url
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Ok(url.trim_end_matches('/').to_string());
        }
    }
    Ok(default_teamups_base_url())
}

/// Error bodies from the site's HTML 404 page are useless to show; keep JSON/text short.
fn error_body(body: &str) -> String {
    let trimmed = body.trim();
    if trimmed.starts_with('<') {
        return "(html page)".to_string();
    }
    trimmed.chars().take(300).collect()
}

fn http_client() -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .context("failed to build TeamUps auth HTTP client")
}

/// Start device login; caller opens `verification_url` in the system browser.
pub fn start_teamups_login() -> Result<TeamupsLoginStart> {
    let base = mall_base_url()?;
    let url = format!("{base}/api/v1/doctor/device/start");
    let resp = http_client()?
        .post(&url)
        .header("Accept", "application/json")
        .send()
        .with_context(|| format!("POST {url}"))?;
    let status = resp.status();
    let body_text = resp.text().unwrap_or_default();
    if !status.is_success() {
        bail!(
            "TeamUps login start failed ({status}): {}",
            error_body(&body_text)
        );
    }
    let body: Value =
        serde_json::from_str(&body_text).context("invalid TeamUps device start JSON")?;
    let device_code = body
        .get("deviceCode")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .context("device start missing deviceCode")?
        .to_string();
    let user_code = body
        .get("userCode")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let verification_url = body
        .get("verificationUriComplete")
        .or_else(|| body.get("verificationUri"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .context("device start missing verification URL")?
        .to_string();
    let interval_sec = body
        .get("interval")
        .and_then(Value::as_u64)
        .unwrap_or(3)
        .max(1);
    let expires_in_sec = body.get("expiresIn").and_then(Value::as_u64).unwrap_or(900);
    Ok(TeamupsLoginStart {
        device_code,
        user_code,
        verification_url,
        interval_sec,
        expires_in_sec,
    })
}

/// One poll tick. On approved, persists license locally.
pub fn poll_teamups_login(device_code: &str) -> Result<TeamupsLoginPoll> {
    let device_code = device_code.trim();
    if device_code.is_empty() {
        bail!("device code must not be empty");
    }
    let base = mall_base_url()?;
    let url = format!("{base}/api/v1/doctor/device/poll");
    let resp = http_client()?
        .post(&url)
        .header("Accept", "application/json")
        .json(&serde_json::json!({ "deviceCode": device_code }))
        .send()
        .with_context(|| format!("POST {url}"))?;
    let status = resp.status();
    let body_text = resp.text().unwrap_or_default();
    if !status.is_success() {
        bail!(
            "TeamUps login poll failed ({status}): {}",
            error_body(&body_text)
        );
    }
    let body: Value =
        serde_json::from_str(&body_text).context("invalid TeamUps device poll JSON")?;
    let poll_status = body
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("expired");
    match poll_status {
        "pending" => Ok(TeamupsLoginPoll {
            status: TeamupsLoginPollStatus::Pending,
            packs: Vec::new(),
        }),
        "denied" => Ok(TeamupsLoginPoll {
            status: TeamupsLoginPollStatus::Denied,
            packs: Vec::new(),
        }),
        "expired" => Ok(TeamupsLoginPoll {
            status: TeamupsLoginPollStatus::Expired,
            packs: Vec::new(),
        }),
        "approved" => {
            let license = body
                .get("license")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .context("approved login missing license")?;
            let packs = body
                .get("packs")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            let store = open_settings_store()?;
            store.set_teamups_license(license)?;
            Ok(TeamupsLoginPoll {
                status: TeamupsLoginPollStatus::Approved,
                packs,
            })
        }
        other => bail!("unknown TeamUps login status: {other}"),
    }
}

pub fn teamups_account_status() -> Result<TeamupsAccountStatus> {
    let store = open_settings_store()?;
    let base_url = mall_base_url()?;
    let Some(license) = store
        .get_teamups_license()?
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
    else {
        return Ok(TeamupsAccountStatus {
            base_url,
            signed_in: false,
            pack_count: 0,
            packs: Vec::new(),
        });
    };

    let url = format!("{base_url}/api/v1/doctor/me");
    let resp = http_client()?
        .get(&url)
        .header("Accept", "application/json")
        .header("Authorization", format!("Bearer {license}"))
        .send()
        .with_context(|| format!("GET {url}"))?;
    let status = resp.status();
    let body_text = resp.text().unwrap_or_default();
    if status.as_u16() == 401 || status.as_u16() == 403 {
        let _ = store.clear_teamups_license();
        return Ok(TeamupsAccountStatus {
            base_url,
            signed_in: false,
            pack_count: 0,
            packs: Vec::new(),
        });
    }
    if !status.is_success() {
        bail!(
            "TeamUps account check failed ({status}): {}",
            error_body(&body_text)
        );
    }
    let body: Value = serde_json::from_str(&body_text).context("invalid doctor/me JSON")?;
    let packs = body
        .get("packs")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Ok(TeamupsAccountStatus {
        base_url,
        signed_in: true,
        pack_count: packs.len() as u32,
        packs,
    })
}

pub fn sign_out_teamups() -> Result<TeamupsAccountStatus> {
    let store = open_settings_store()?;
    store.clear_teamups_license()?;
    teamups_account_status()
}
