//! Compare installed runtime versions with upstream "latest" (read-only).
//!
//! No install/upgrade — personal edition only shows local vs latest and warns
//! that upgrading can break config (settings.json / gateway schema).

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::adapters::DEEPSEEK_HARNESS_VERSION;
use crate::lifecycle::{use_china_mirrors, NPM_MIRROR};
use crate::store::db::agent_doctor_config_dir;

const CACHE_TTL_SECS: u64 = 6 * 60 * 60;
const HTTP_TIMEOUT: Duration = Duration::from_secs(8);
const NPM_OFFICIAL: &str = "https://registry.npmjs.org";
const OPENCLAW_NPM: &str = "openclaw";
const CLAUDE_NPM: &str = "@anthropic-ai/claude-code";
const CODEX_NPM: &str = "@openai/codex";
const DSH_NPM: &str = "@deepseek-ai/dsh";
const HERMES_PYPROJECT_URL: &str =
    "https://raw.githubusercontent.com/NousResearch/hermes-agent/main/pyproject.toml";
const HERMES_LATEST_RELEASE_URL: &str =
    "https://github.com/NousResearch/hermes-agent/releases/latest";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VersionCompareStatus {
    /// Local matches latest (or recommended pin when that is the policy).
    UpToDate,
    /// Upstream differs — upgrade may break config. The installed copy still works here.
    UpdateAvailable,
    /// Installed copy is older than the version this app can talk to.
    BelowMinimum,
    /// Could not fetch or parse latest.
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeVersionStatus {
    pub runtime_id: String,
    /// Normalized installed version, if known.
    pub installed: Option<String>,
    /// Normalized upstream latest, if known.
    pub latest: Option<String>,
    /// Doctor-recommended pin (DeepSeek Harness), when applicable.
    pub recommended: Option<String>,
    /// Oldest version this app can talk to, when this runtime has one.
    pub minimum: Option<String>,
    pub status: VersionCompareStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct LatestCacheFile {
    fetched_at_unix: u64,
    /// runtime_id → latest version string
    latest: BTreeMap<String, String>,
}

/// Check installed runtimes against cached/fetched upstream latest versions.
/// May refresh the on-disk cache over the network (used by desktop after doctor).
pub fn check_runtime_versions(installed: &[(String, Option<String>)]) -> Vec<RuntimeVersionStatus> {
    let cache = load_or_refresh_latest_cache();
    statuses_from_cache(installed, &cache)
}

/// Same compare as [`check_runtime_versions`], but never hits the network.
/// Used by diagnose/probe so scans stay fast; desktop refresh fills the cache.
pub fn check_runtime_versions_cached(
    installed: &[(String, Option<String>)],
) -> Vec<RuntimeVersionStatus> {
    let cache = read_cache_file().unwrap_or_default();
    statuses_from_cache(installed, &cache)
}

fn statuses_from_cache(
    installed: &[(String, Option<String>)],
    cache: &LatestCacheFile,
) -> Vec<RuntimeVersionStatus> {
    installed
        .iter()
        .map(|(runtime_id, raw_installed)| {
            build_status(
                runtime_id,
                raw_installed.as_deref(),
                cache.latest.get(runtime_id).map(String::as_str),
            )
        })
        .collect()
}

fn build_status(
    runtime_id: &str,
    raw_installed: Option<&str>,
    cached_latest: Option<&str>,
) -> RuntimeVersionStatus {
    let installed = raw_installed.and_then(extract_version);
    let mut latest = cached_latest.map(str::to_string);
    let recommended = if runtime_id == "deepseek-harness" {
        Some(DEEPSEEK_HARNESS_VERSION.to_string())
    } else {
        None
    };
    if latest.is_none() && runtime_id == "deepseek-harness" {
        latest = Some(DEEPSEEK_HARNESS_VERSION.to_string());
    }
    let minimum = client_minimum_version(runtime_id).map(str::to_string);
    let below_minimum = installed
        .as_deref()
        .zip(minimum.as_deref())
        .is_some_and(|(local, floor)| version_is_older(local, floor));

    let status = if below_minimum {
        VersionCompareStatus::BelowMinimum
    } else {
        match (&installed, &latest) {
            (Some(local), Some(up)) if versions_match(local, up) => VersionCompareStatus::UpToDate,
            (Some(_), Some(_)) => VersionCompareStatus::UpdateAvailable,
            _ => VersionCompareStatus::Unknown,
        }
    };

    RuntimeVersionStatus {
        runtime_id: runtime_id.to_string(),
        installed,
        latest,
        recommended,
        minimum,
        status,
    }
}

/// Oldest release this app can talk to. An older install already on the computer
/// should be updated before chat. Runtimes without an entry keep working as-is.
pub fn client_minimum_version(runtime_id: &str) -> Option<&'static str> {
    match runtime_id {
        // `hermes chat --format stream-json` landed after the 0.21.3 tag; git installs
        // on main still report 0.21.3 and have it, so only older numbers are refused.
        "hermes" => Some("0.21.3"),
        _ => None,
    }
}

/// True when `installed` is strictly older than `minimum`.
pub fn version_is_older(installed: &str, minimum: &str) -> bool {
    compare_version_parts(installed, minimum) == std::cmp::Ordering::Less
}

fn compare_version_parts(left: &str, right: &str) -> std::cmp::Ordering {
    let left = version_numbers(left);
    let right = version_numbers(right);
    for index in 0..left.len().max(right.len()) {
        let a = left.get(index).copied().unwrap_or(0);
        let b = right.get(index).copied().unwrap_or(0);
        match a.cmp(&b) {
            std::cmp::Ordering::Equal => continue,
            other => return other,
        }
    }
    std::cmp::Ordering::Equal
}

fn version_numbers(version: &str) -> Vec<u64> {
    version
        .trim()
        .trim_start_matches(['v', 'V'])
        .split(['.', '-', '+'])
        .map_while(|part| part.parse::<u64>().ok())
        .collect()
}

fn load_or_refresh_latest_cache() -> LatestCacheFile {
    if let Some(existing) = read_cache_file() {
        let age = now_unix().saturating_sub(existing.fetched_at_unix);
        let placeholder = existing.latest.values().any(|v| is_placeholder_version(v));
        if age < CACHE_TTL_SECS && !existing.latest.is_empty() && !placeholder {
            return existing;
        }
    }

    let mut latest = BTreeMap::new();
    for (id, version) in fetch_all_latest() {
        latest.insert(id, version);
    }
    let file = LatestCacheFile {
        fetched_at_unix: now_unix(),
        latest,
    };
    if let Some(path) = cache_path() {
        let _ = write_cache(&path, &file);
    }
    file
}

fn read_cache_file() -> Option<LatestCacheFile> {
    let path = cache_path()?;
    read_cache(&path)
}

fn fetch_all_latest() -> Vec<(String, String)> {
    let mut out = Vec::new();
    if let Some(v) = fetch_npm_latest(OPENCLAW_NPM) {
        out.push(("openclaw".into(), v));
    }
    if let Some(v) = fetch_npm_latest(CLAUDE_NPM) {
        out.push(("claude-code".into(), v));
    }
    if let Some(v) = fetch_npm_latest(CODEX_NPM) {
        out.push(("codex".into(), v));
    }
    if let Some(v) = fetch_npm_latest(DSH_NPM) {
        out.push(("deepseek-harness".into(), v));
    }
    if let Some(v) = fetch_hermes_latest() {
        out.push(("hermes".into(), v));
    }
    out
}

fn npm_registry_base() -> &'static str {
    if use_china_mirrors() {
        NPM_MIRROR
    } else {
        NPM_OFFICIAL
    }
}

fn fetch_npm_latest(package: &str) -> Option<String> {
    let url = format!("{}/{}/latest", npm_registry_base(), package);
    let body = http_get_text(&url)?;
    let json: Value = serde_json::from_str(&body).ok()?;
    json.get("version")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// `main` now carries a `0.0.0` placeholder in pyproject; the release tag is the
/// version people install.
fn fetch_hermes_latest() -> Option<String> {
    fetch_hermes_release_tag().or_else(|| {
        let fetch_url = if use_china_mirrors() {
            format!("https://gh-proxy.com/{HERMES_PYPROJECT_URL}")
        } else {
            HERMES_PYPROJECT_URL.to_string()
        };
        let body = http_get_text(&fetch_url)?;
        parse_pyproject_version(&body).filter(|v| !is_placeholder_version(v))
    })
}

fn fetch_hermes_release_tag() -> Option<String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(HTTP_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("agent-doctor/", env!("CARGO_PKG_VERSION")))
        .build()
        .ok()?;
    let response = client.get(HERMES_LATEST_RELEASE_URL).send().ok()?;
    let location = response
        .headers()
        .get(reqwest::header::LOCATION)?
        .to_str()
        .ok()?;
    release_tag_version(location)
}

fn release_tag_version(location: &str) -> Option<String> {
    let tag = location.rsplit_once("/tag/")?.1.trim_end_matches('/');
    let version = tag.trim_start_matches(['v', 'V']);
    (!version_numbers(version).is_empty() && !is_placeholder_version(version))
        .then(|| version.to_string())
}

fn is_placeholder_version(version: &str) -> bool {
    version_numbers(version).iter().all(|part| *part == 0)
}

fn parse_pyproject_version(toml: &str) -> Option<String> {
    for line in toml.lines() {
        let trimmed = line.trim();
        let Some(rest) = trimmed.strip_prefix("version") else {
            continue;
        };
        let rest = rest.trim_start();
        let Some(rest) = rest.strip_prefix('=') else {
            continue;
        };
        let value = rest.trim().trim_matches('"').trim_matches('\'').trim();
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }
    None
}

fn http_get_text(url: &str) -> Option<String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(HTTP_TIMEOUT)
        .user_agent(concat!("agent-doctor/", env!("CARGO_PKG_VERSION")))
        .build()
        .ok()?;
    let response = client.get(url).send().ok()?;
    if !response.status().is_success() {
        return None;
    }
    response.text().ok()
}

fn cache_path() -> Option<PathBuf> {
    agent_doctor_config_dir().map(|dir| dir.join("runtime-latest-cache.json"))
}

fn read_cache(path: &PathBuf) -> Option<LatestCacheFile> {
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn write_cache(path: &PathBuf, file: &LatestCacheFile) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(file).unwrap_or_else(|_| "{}".into());
    fs::write(path, text)
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Pull a comparable version token from noisy CLI `--version` output.
pub fn extract_version(raw: &str) -> Option<String> {
    let text = raw.trim();
    if text.is_empty() {
        return None;
    }
    extract_version_manual(text)
}

fn extract_version_manual(raw: &str) -> Option<String> {
    let bytes = raw.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let start = i;
            let mut dots = 0;
            let mut j = i;
            while j < bytes.len() {
                let c = bytes[j];
                if c.is_ascii_digit() {
                    j += 1;
                } else if c == b'.' {
                    dots += 1;
                    j += 1;
                    if j >= bytes.len() || !bytes[j].is_ascii_digit() {
                        break;
                    }
                } else if c == b'-' && dots >= 1 {
                    j += 1;
                    while j < bytes.len()
                        && (bytes[j].is_ascii_alphanumeric()
                            || bytes[j] == b'.'
                            || bytes[j] == b'-')
                    {
                        j += 1;
                    }
                    break;
                } else {
                    break;
                }
            }
            if dots >= 1 {
                let candidate = &raw[start..j];
                if !candidate.is_empty() {
                    return Some(candidate.to_string());
                }
            }
            i = j.max(i + 1);
        } else {
            i += 1;
        }
    }
    None
}

fn versions_match(a: &str, b: &str) -> bool {
    normalize_cmp(a) == normalize_cmp(b)
}

fn normalize_cmp(v: &str) -> String {
    v.trim()
        .trim_start_matches('v')
        .trim_start_matches('V')
        .to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_common_cli_version_strings() {
        assert_eq!(
            extract_version("OpenClaw 2026.9.5 (eabc)"),
            Some("2026.9.5".into())
        );
        assert_eq!(
            extract_version("2.1.278 (Claude Code)"),
            Some("2.1.278".into())
        );
        assert_eq!(extract_version("Hermes Agent v0.5.0"), Some("0.5.0".into()));
        assert_eq!(extract_version("codex-cli 0.145.0"), Some("0.145.0".into()));
        assert_eq!(
            extract_version("dsh version 0.1.0-rc.6"),
            Some("0.1.0-rc.6".into())
        );
    }

    #[test]
    fn parse_pyproject_version_line() {
        let toml = "[project]\nname = \"hermes-agent\"\nversion = \"0.5.0\"\n";
        assert_eq!(parse_pyproject_version(toml), Some("0.5.0".into()));
    }

    #[test]
    fn status_marks_update_when_latest_differs() {
        let status = build_status("codex", Some("codex-cli 0.145.0"), Some("0.156.1"));
        assert_eq!(status.status, VersionCompareStatus::UpdateAvailable);
        assert_eq!(status.installed.as_deref(), Some("0.145.0"));
        assert_eq!(status.latest.as_deref(), Some("0.156.1"));
    }

    #[test]
    fn status_up_to_date_when_equal() {
        let status = build_status("openclaw", Some("OpenClaw 2026.9.5"), Some("2026.9.5"));
        assert_eq!(status.status, VersionCompareStatus::UpToDate);
    }

    #[test]
    fn hermes_latest_comes_from_the_release_tag_not_a_placeholder() {
        assert_eq!(
            release_tag_version(
                "https://github.com/NousResearch/hermes-agent/releases/tag/v0.21.7"
            ),
            Some("0.21.7".into())
        );
        assert_eq!(
            release_tag_version("https://github.com/x/y/releases/tag/v0.0.0"),
            None
        );
        assert_eq!(release_tag_version("https://github.com/x/y/releases"), None);
        assert!(is_placeholder_version("0.0.0"));
        assert!(!is_placeholder_version("0.21.7"));
    }

    #[test]
    fn hermes_older_than_client_minimum_asks_for_an_update() {
        let status = build_status("hermes", Some("Hermes Agent v0.20.1"), Some("0.22.0"));
        assert_eq!(status.status, VersionCompareStatus::BelowMinimum);
        assert_eq!(status.installed.as_deref(), Some("0.20.1"));
        assert_eq!(status.minimum.as_deref(), Some("0.21.3"));
    }

    #[test]
    fn hermes_at_the_client_minimum_can_stay() {
        let status = build_status(
            "hermes",
            Some("Hermes Agent v0.21.3 (2026.9.14)"),
            Some("0.22.0"),
        );
        assert_eq!(status.status, VersionCompareStatus::UpdateAvailable);
    }

    #[test]
    fn a_runtime_without_a_floor_does_not_become_below_minimum() {
        let status = build_status("codex", Some("0.1.0"), Some("0.9.0"));
        assert_eq!(status.status, VersionCompareStatus::UpdateAvailable);
        assert!(status.minimum.is_none());
    }
}
