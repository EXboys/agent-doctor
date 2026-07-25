//! Personal (non-Evotown) OpenAI-compatible provider setup.
//!
//! Writes a local profile and merges Codex / Hermes / Claude Code configs
//! without rewriting Evotown URLs or touching `evotown.agent.env`.

use std::fs;
use std::io::Write;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::profile::agent_profile_path;
use crate::repair::mask_secret_value;
use crate::runtime::all_adapters;
use crate::setup::merge::{self, clear_codex_placeholder_auth};
use crate::setup::{
    normalize_gateway_url, RuntimeSetupResult, COMPANY_API_KEY_ENV, GATEWAY_URL_ENV,
};

pub const PROVIDER_KIND_ENV: &str = "AGENT_DOCTOR_PROVIDER_KIND";
pub const MODEL_ENV: &str = "AGENT_DOCTOR_MODEL";
pub const OPENAI_API_KEY_ENV: &str = "OPENAI_API_KEY";
pub const PROVIDER_KIND_PERSONAL: &str = "personal";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonalProviderOptions {
    pub url: String,
    pub api_key: String,
    pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonalProviderStatus {
    pub configured: bool,
    pub gateway_url: Option<String>,
    pub model: Option<String>,
    pub api_key_hint: Option<String>,
    pub profile_env_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonalProviderVerifyReport {
    pub ok: bool,
    pub status_code: Option<u16>,
    pub checked_url: Option<String>,
    pub message: String,
    pub models_sample: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonalProviderSetupReport {
    pub profile_env_path: String,
    pub gateway_url: String,
    pub model: String,
    pub runtimes: Vec<RuntimeSetupResult>,
    pub verify: Option<PersonalProviderVerifyReport>,
}

pub fn normalize_personal_gateway_url(url: &str) -> Result<String> {
    // Keep the user's path intact (including `/v1`); do NOT append Evotown suffixes.
    normalize_gateway_url(url)
}

pub fn load_personal_provider_status() -> Result<PersonalProviderStatus> {
    let Some(path) = agent_profile_path() else {
        return Ok(PersonalProviderStatus {
            configured: false,
            gateway_url: None,
            model: None,
            api_key_hint: None,
            profile_env_path: None,
        });
    };
    if !path.exists() {
        return Ok(PersonalProviderStatus {
            configured: false,
            gateway_url: None,
            model: None,
            api_key_hint: None,
            profile_env_path: Some(path.display().to_string()),
        });
    }

    let env = read_env_file(&path)?;
    let kind = env.get(PROVIDER_KIND_ENV).map(String::as_str).unwrap_or("");
    let gateway_url = env.get(GATEWAY_URL_ENV).cloned();
    let model = env.get(MODEL_ENV).cloned();
    let api_key = env
        .get(OPENAI_API_KEY_ENV)
        .or_else(|| env.get(COMPANY_API_KEY_ENV))
        .cloned();

    let configured = kind == PROVIDER_KIND_PERSONAL
        && gateway_url.as_deref().is_some_and(|u| !u.trim().is_empty())
        && api_key.as_deref().is_some_and(|k| !k.trim().is_empty());

    Ok(PersonalProviderStatus {
        configured,
        gateway_url: if configured { gateway_url } else { None },
        model: if configured { model } else { None },
        api_key_hint: if configured {
            api_key.as_deref().map(mask_secret_value)
        } else {
            None
        },
        profile_env_path: Some(path.display().to_string()),
    })
}

pub fn verify_personal_provider(url: &str, api_key: &str) -> Result<PersonalProviderVerifyReport> {
    let gateway_url = normalize_personal_gateway_url(url)?;
    let api_key = api_key.trim();
    if api_key.is_empty() {
        bail!("API key must not be empty");
    }

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .context("failed to build HTTP client")?;

    let mut last_error = String::from("no models endpoint responded");
    for checked in models_endpoint_candidates(&gateway_url) {
        let response = match client
            .get(&checked)
            .header("Authorization", format!("Bearer {api_key}"))
            .send()
        {
            Ok(resp) => resp,
            Err(err) => {
                last_error = format!("request failed for {checked}: {err}");
                continue;
            }
        };
        let status = response.status();
        let status_code = status.as_u16();
        if !status.is_success() {
            let body = response.text().unwrap_or_default();
            last_error = format!(
                "{checked} → HTTP {status_code}: {}",
                body.chars().take(180).collect::<String>()
            );
            continue;
        }

        let body = response.text().unwrap_or_default();
        let models_sample = extract_model_ids(&body);
        return Ok(PersonalProviderVerifyReport {
            ok: true,
            status_code: Some(status_code),
            checked_url: Some(checked),
            message: if models_sample.is_empty() {
                "Gateway reachable (models list empty or non-standard).".to_string()
            } else {
                format!("Gateway reachable — saw {} model(s).", models_sample.len())
            },
            models_sample,
        });
    }

    Ok(PersonalProviderVerifyReport {
        ok: false,
        status_code: None,
        checked_url: None,
        message: last_error,
        models_sample: Vec::new(),
    })
}

pub fn execute_personal_provider_setup(
    options: &PersonalProviderOptions,
) -> Result<PersonalProviderSetupReport> {
    let gateway_url = normalize_personal_gateway_url(&options.url)?;
    let api_key = options.api_key.trim();
    if api_key.is_empty() {
        bail!("API key must not be empty");
    }
    let model = options.model.trim();
    if model.is_empty() {
        bail!("model must not be empty");
    }

    let verify = verify_personal_provider(&gateway_url, api_key)?;
    if !verify.ok {
        bail!("provider connectivity check failed: {}", verify.message);
    }

    let profile_path = agent_profile_path().context("could not resolve config directory")?;
    write_personal_profile(&profile_path, &gateway_url, api_key, model)?;
    let _ = clear_codex_placeholder_auth();

    let mut runtimes = Vec::new();
    for adapter in all_adapters() {
        let result = match adapter.id() {
            "openclaw" => merge::apply_openclaw(&gateway_url, api_key),
            "hermes" => merge::apply_hermes(&gateway_url, api_key, "custom", Some(model)),
            "claude-code" => merge::apply_claude_code(&gateway_url, api_key),
            "codex" => merge::apply_codex(&gateway_url, api_key, Some(model)),
            other => Ok(RuntimeSetupResult {
                runtime_id: other.to_string(),
                display_name: adapter.display_name().to_string(),
                applied: false,
                config_path: None,
                backup_path: None,
                message: "no personal provider merge for this runtime yet".to_string(),
            }),
        }?;
        runtimes.push(result);
    }

    Ok(PersonalProviderSetupReport {
        profile_env_path: profile_path.display().to_string(),
        gateway_url,
        model: model.to_string(),
        runtimes,
        verify: Some(verify),
    })
}

fn write_personal_profile(
    path: &std::path::Path,
    gateway_url: &str,
    api_key: &str,
    model: &str,
) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut file = fs::File::create(path).context("failed to create profile.env")?;
    writeln!(
        file,
        "# Agent Doctor personal provider — written by agent-doctor (not Evotown)"
    )?;
    writeln!(
        file,
        "# Source before running agents: set -a && source \"{}\" && set +a",
        path.display()
    )?;
    writeln!(file, "{PROVIDER_KIND_ENV}={PROVIDER_KIND_PERSONAL}")?;
    writeln!(file, "{GATEWAY_URL_ENV}={gateway_url}")?;
    writeln!(file, "{MODEL_ENV}={model}")?;
    writeln!(file, "{COMPANY_API_KEY_ENV}={api_key}")?;
    writeln!(file, "COMPANY_API_KEY={api_key}")?;
    writeln!(file, "{OPENAI_API_KEY_ENV}={api_key}")?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn models_endpoint_candidates(base: &str) -> Vec<String> {
    let base = base.trim_end_matches('/');
    if base.ends_with("/v1") {
        vec![format!("{base}/models")]
    } else {
        vec![format!("{base}/models"), format!("{base}/v1/models")]
    }
}

fn extract_model_ids(body: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(body) else {
        return Vec::new();
    };
    let Some(data) = value.get("data").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    data.iter()
        .filter_map(|item| {
            item.get("id")
                .and_then(|id| id.as_str())
                .map(str::to_string)
        })
        .take(8)
        .collect()
}

fn read_env_file(path: &std::path::Path) -> Result<std::collections::HashMap<String, String>> {
    let raw = fs::read_to_string(path)?;
    let mut map = std::collections::HashMap::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some((key, value)) = trimmed.split_once('=') else {
            continue;
        };
        map.insert(key.trim().to_string(), value.trim().to_string());
    }
    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn personal_url_keeps_v1_suffix() {
        assert_eq!(
            normalize_personal_gateway_url("https://api.example.com/v1/").unwrap(),
            "https://api.example.com/v1"
        );
    }

    #[test]
    fn models_candidates_for_bare_base() {
        assert_eq!(
            models_endpoint_candidates("https://api.example.com"),
            vec![
                "https://api.example.com/models".to_string(),
                "https://api.example.com/v1/models".to_string(),
            ]
        );
    }

    #[test]
    fn models_candidates_for_v1_base() {
        assert_eq!(
            models_endpoint_candidates("https://api.example.com/v1"),
            vec!["https://api.example.com/v1/models".to_string()]
        );
    }

    #[test]
    fn extract_model_ids_from_openai_shape() {
        let ids = extract_model_ids(
            r#"{"object":"list","data":[{"id":"gpt-4o-mini"},{"id":"deepseek-chat"}]}"#,
        );
        assert_eq!(ids, vec!["gpt-4o-mini", "deepseek-chat"]);
    }
}
