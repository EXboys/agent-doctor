use std::fs;

use anyhow::{Context, Result};
use serde_json::{json, Value};

use super::client::EvotownClient;
use super::config::EvotownConfig;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PolicyPullReport {
    pub base_url: String,
    pub cache_path: String,
    pub policy_count: usize,
    pub fetched_at: String,
}

pub fn execute_policy_pull(config: &EvotownConfig) -> Result<PolicyPullReport> {
    let client = EvotownClient::new(&config.base_url, &config.api_key)?;
    let payload = client.get_json("/api/v1/policies?enabled_only=true")?;
    let policies = payload
        .get("policies")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let fetched_at = utc_now_rfc3339();
    let cached = json!({
        "fetched_at": fetched_at,
        "policies": policies,
    });

    if let Some(parent) = config.policy_cache_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(
        &config.policy_cache_path,
        serde_json::to_string_pretty(&cached)? + "\n",
    )
    .with_context(|| format!("failed to write {}", config.policy_cache_path.display()))?;

    Ok(PolicyPullReport {
        base_url: config.base_url.clone(),
        cache_path: config.policy_cache_path.display().to_string(),
        policy_count: policies.len(),
        fetched_at,
    })
}

fn utc_now_rfc3339() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // Simple UTC timestamp compatible with Evotown Python cache format.
    let days = seconds / 86_400;
    let rem = seconds % 86_400;
    let hour = rem / 3600;
    let minute = (rem % 3600) / 60;
    let second = rem % 60;
    let (year, month, day) = civil_from_days(days as i64);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    let year = y + if m <= 2 { 1 } else { 0 };
    (year, m, d)
}
