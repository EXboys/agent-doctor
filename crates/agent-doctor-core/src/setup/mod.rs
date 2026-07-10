mod merge;

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::profile::agent_profile_path;
use crate::runtime::all_adapters;

pub const COMPANY_API_KEY_ENV: &str = "AGENT_DOCTOR_COMPANY_API_KEY";
pub const GATEWAY_URL_ENV: &str = "AGENT_DOCTOR_GATEWAY_URL";
pub const EVOTOWN_URL_ENV: &str = "EVOTOWN_URL";
pub const EVOTOWN_API_KEY_ENV: &str = "EVOTOWN_API_KEY";
pub const EVOTOWN_RUNTIME_ENV: &str = "EVOTOWN_RUNTIME";
pub const EVOTOWN_BUNDLE_ID_ENV: &str = "EVOTOWN_BUNDLE_ID";
pub const EVOTOWN_SKILLS_DIR_ENV: &str = "EVOTOWN_SKILLS_DIR";
pub const DEFAULT_EVOTOWN_RUNTIME: &str = "openclaw";
pub const DEFAULT_EVOTOWN_BUNDLE_ID: &str = "default-agent-skills";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetupOptions {
    pub gateway_url: String,
    pub api_key: String,
    /// Hermes provider when creating or updating config (default: openai).
    pub hermes_provider: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeSetupResult {
    pub runtime_id: String,
    pub display_name: String,
    pub applied: bool,
    pub config_path: Option<String>,
    pub backup_path: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetupReport {
    pub profile_env_path: String,
    pub gateway_url: String,
    pub evotown_base_url: String,
    pub evotown_agent_env_path: Option<String>,
    pub runtimes: Vec<RuntimeSetupResult>,
}

pub fn execute_setup(options: &SetupOptions) -> Result<SetupReport> {
    let gateway_url = normalize_gateway_url(&gateway_url_from_evotown_base(&options.gateway_url))?;
    let evotown_base = evotown_base_from_gateway(&gateway_url);
    let api_key = options.api_key.trim();
    if api_key.is_empty() {
        bail!("--key must not be empty");
    }

    let profile_path = agent_profile_path().context("could not resolve config directory")?;
    write_company_profile_with_gateway(&profile_path, &gateway_url, api_key, &evotown_base)?;
    let evotown_agent_env_path =
        write_evotown_agent_env(&evotown_base, api_key, DEFAULT_EVOTOWN_RUNTIME)
            .ok()
            .map(|path| path.display().to_string());

    let mut runtimes = Vec::new();
    for adapter in all_adapters() {
        let result = match adapter.id() {
            "openclaw" => merge::apply_openclaw(&gateway_url, api_key),
            "hermes" => merge::apply_hermes(&gateway_url, api_key, &options.hermes_provider),
            "claude-code" => merge::apply_claude_code(&gateway_url, api_key),
            "codex" => merge::apply_codex(&gateway_url, api_key),
            other => Ok(RuntimeSetupResult {
                runtime_id: other.to_string(),
                display_name: adapter.display_name().to_string(),
                applied: false,
                config_path: None,
                backup_path: None,
                message: "no company setup merge for this runtime yet".to_string(),
            }),
        }?;
        runtimes.push(result);
    }

    Ok(SetupReport {
        profile_env_path: profile_path.display().to_string(),
        gateway_url,
        evotown_base_url: evotown_base,
        evotown_agent_env_path,
        runtimes,
    })
}

pub fn normalize_gateway_url(url: &str) -> Result<String> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        bail!("--url must not be empty");
    }
    if !trimmed.starts_with("http://") && !trimmed.starts_with("https://") {
        bail!("--url must start with http:// or https://");
    }
    Ok(trimmed.trim_end_matches('/').to_string())
}

/// Strip `/api/gateway/v1` to recover the Evotown base URL.
pub fn evotown_base_from_gateway(gateway_url: &str) -> String {
    let trimmed = gateway_url.trim().trim_end_matches('/');
    const SUFFIX: &str = "/api/gateway/v1";
    if let Some(base) = trimmed.strip_suffix(SUFFIX) {
        base.trim_end_matches('/').to_string()
    } else {
        trimmed.to_string()
    }
}

/// Append the LiteLLM gateway path when the user supplied an Evotown base URL.
pub fn gateway_url_from_evotown_base(base_url: &str) -> String {
    let base = base_url.trim().trim_end_matches('/');
    if base.ends_with("/api/gateway/v1") {
        base.to_string()
    } else {
        format!("{base}/api/gateway/v1")
    }
}

pub fn evotown_agent_env_path() -> Option<PathBuf> {
    dirs::config_dir().map(|base| base.join("evotown").join("evotown.agent.env"))
}

pub fn default_evotown_skills_dir() -> PathBuf {
    dirs::home_dir()
        .map(|home| home.join(".evotown").join("skills"))
        .unwrap_or_else(|| PathBuf::from(".evotown/skills"))
}

pub fn write_evotown_agent_env(
    base_url: &str,
    api_key: &str,
    runtime_target: &str,
) -> Result<PathBuf> {
    let path = evotown_agent_env_path().context("could not resolve config directory")?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let base = base_url.trim().trim_end_matches('/');
    let lines = [
        "# Evotown employee agent config — written by Agent Doctor setup".to_string(),
        format!("{EVOTOWN_URL_ENV}={base}"),
        format!("{EVOTOWN_API_KEY_ENV}={api_key}"),
        format!("{EVOTOWN_RUNTIME_ENV}={runtime_target}"),
        format!("{EVOTOWN_BUNDLE_ID_ENV}={DEFAULT_EVOTOWN_BUNDLE_ID}"),
        format!(
            "{EVOTOWN_SKILLS_DIR_ENV}={}",
            default_evotown_skills_dir().display()
        ),
        format!("# Gateway for OpenAI-compatible clients: {base}/api/gateway/v1"),
    ];

    fs::write(&path, lines.join("\n") + "\n")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(path)
}

pub fn write_company_profile_with_gateway(
    path: &Path,
    gateway_url: &str,
    api_key: &str,
    evotown_base: &str,
) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut file = fs::File::create(path).context("failed to create profile.env")?;
    writeln!(
        file,
        "# Agent Doctor company profile — written by agent-doctor setup (Evotown)"
    )?;
    writeln!(
        file,
        "# Source before running agents: set -a && source \"{}\" && set +a",
        path.display()
    )?;
    writeln!(file, "{GATEWAY_URL_ENV}={gateway_url}")?;
    writeln!(file, "AGENT_DOCTOR_EVOTOWN_URL={evotown_base}")?;
    writeln!(file, "{COMPANY_API_KEY_ENV}={api_key}")?;
    writeln!(file, "COMPANY_API_KEY={api_key}")?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }

    Ok(())
}

pub(crate) fn backup_file(path: &Path) -> Result<Option<PathBuf>> {
    if !path.exists() {
        return Ok(None);
    }
    let original = fs::read_to_string(path)?;
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let backup_path = path.with_extension(format!(
        "{}.bak.{ts}",
        path.extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("bak")
    ));
    std::fs::write(&backup_path, original)?;
    Ok(Some(backup_path))
}

pub(crate) fn ensure_parent(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_gateway_url() {
        assert_eq!(
            normalize_gateway_url("https://gw.example/v1/").unwrap(),
            "https://gw.example/v1"
        );
    }

    #[test]
    fn rejects_invalid_gateway_url() {
        assert!(normalize_gateway_url("").is_err());
        assert!(normalize_gateway_url("ftp://x").is_err());
    }
}
