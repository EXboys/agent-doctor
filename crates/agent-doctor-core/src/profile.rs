use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

pub const GATEWAY_URL_ENV: &str = "AGENT_DOCTOR_GATEWAY_URL";
pub const COMPANY_API_KEY_ENV: &str = "AGENT_DOCTOR_COMPANY_API_KEY";
pub const PROVIDER_KIND_ENV: &str = "AGENT_DOCTOR_PROVIDER_KIND";
pub const PROVIDER_KIND_PERSONAL: &str = "personal";
pub const PROVIDER_KIND_COMPANY: &str = "company";

/// Active runtime overlay written by company `setup` or personal provider activate.
pub fn agent_profile_path() -> Option<PathBuf> {
    dirs::config_dir().map(|base| base.join("agent-doctor").join("profile.env"))
}

/// Durable team baseline — never overwritten by personal provider activate.
pub fn company_baseline_path() -> Option<PathBuf> {
    dirs::config_dir().map(|base| base.join("agent-doctor").join("company-profile.env"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProviderKind {
    Company,
    Personal,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Default)]
pub struct CompanyProfile {
    pub gateway_url: Option<String>,
    pub api_key: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct AgentProfile {
    pub kind: ProviderKind,
    pub gateway_url: Option<String>,
    pub api_key: Option<String>,
}

pub fn write_company_profile(path: &Path, gateway_url: &str, api_key: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut file = fs::File::create(path).context("failed to create profile.env")?;
    writeln!(
        file,
        "# Agent Doctor company profile — written by agent-doctor setup"
    )?;
    writeln!(
        file,
        "# Source before running agents: set -a && source \"{}\" && set +a",
        path.display()
    )?;
    writeln!(file, "{GATEWAY_URL_ENV}={gateway_url}")?;
    writeln!(file, "{COMPANY_API_KEY_ENV}={api_key}")?;
    writeln!(file, "COMPANY_API_KEY={api_key}")?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }

    Ok(())
}

/// Persist the durable team baseline used by workspace company-drift checks.
pub fn write_company_baseline(
    gateway_url: &str,
    api_key: &str,
    evotown_base: Option<&str>,
) -> Result<()> {
    let Some(path) = company_baseline_path() else {
        anyhow::bail!("could not resolve config directory");
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut file = fs::File::create(&path).context("failed to create company-profile.env")?;
    writeln!(
        file,
        "# Agent Doctor company baseline — durable team gateway (not overwritten by personal providers)"
    )?;
    writeln!(file, "{GATEWAY_URL_ENV}={gateway_url}")?;
    if let Some(base) = evotown_base.filter(|b| !b.trim().is_empty()) {
        writeln!(file, "AGENT_DOCTOR_EVOTOWN_URL={base}")?;
    }
    writeln!(file, "{COMPANY_API_KEY_ENV}={api_key}")?;
    writeln!(file, "COMPANY_API_KEY={api_key}")?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    }

    Ok(())
}

pub fn read_env_map(path: &Path) -> Result<HashMap<String, String>> {
    let raw = fs::read_to_string(path)?;
    let mut env = HashMap::new();
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        env.insert(key.trim().to_string(), value.trim().to_string());
    }
    Ok(env)
}

fn profile_from_env(env: &HashMap<String, String>) -> CompanyProfile {
    CompanyProfile {
        gateway_url: env.get(GATEWAY_URL_ENV).cloned(),
        api_key: env.get(COMPANY_API_KEY_ENV).cloned(),
    }
}

fn kind_from_env(env: &HashMap<String, String>) -> ProviderKind {
    match env.get(PROVIDER_KIND_ENV).map(String::as_str) {
        Some(PROVIDER_KIND_PERSONAL) => ProviderKind::Personal,
        Some(PROVIDER_KIND_COMPANY) => ProviderKind::Company,
        // Legacy company setup wrote profile.env without PROVIDER_KIND.
        None | Some("") => {
            if env.contains_key(GATEWAY_URL_ENV) || env.contains_key(COMPANY_API_KEY_ENV) {
                ProviderKind::Company
            } else {
                ProviderKind::Unknown
            }
        }
        Some(_) => ProviderKind::Unknown,
    }
}

/// Active overlay in `profile.env` (personal or company). Used by repair / doctor wiring.
pub fn read_agent_profile() -> Result<Option<AgentProfile>> {
    let Some(path) = agent_profile_path() else {
        return Ok(None);
    };
    if !path.exists() {
        return Ok(None);
    }
    let env = read_env_map(&path)?;
    let profile = profile_from_env(&env);
    if profile.gateway_url.is_none() && profile.api_key.is_none() {
        return Ok(None);
    }
    Ok(Some(AgentProfile {
        kind: kind_from_env(&env),
        gateway_url: profile.gateway_url,
        api_key: profile.api_key,
    }))
}

/// Durable team baseline for compliance / workspace drift.
///
/// Prefers `company-profile.env`. Falls back to `profile.env` only when it is
/// a company (non-personal) overlay — never when personal is active.
pub fn read_company_baseline() -> Result<Option<CompanyProfile>> {
    if let Some(path) = company_baseline_path() {
        if path.exists() {
            let env = read_env_map(&path)?;
            let profile = profile_from_env(&env);
            if profile.gateway_url.is_some() || profile.api_key.is_some() {
                return Ok(Some(profile));
            }
        }
    }

    let Some(active) = read_agent_profile()? else {
        return Ok(None);
    };
    if active.kind == ProviderKind::Personal {
        return Ok(None);
    }
    Ok(Some(CompanyProfile {
        gateway_url: active.gateway_url,
        api_key: active.api_key,
    }))
}

/// Active gateway/key from `profile.env` (personal or company).
///
/// Prefer [`read_company_baseline`] for team compliance checks, and
/// [`read_agent_profile`] when you need provider kind.
pub fn read_company_profile() -> Result<Option<CompanyProfile>> {
    Ok(read_agent_profile()?.map(|p| CompanyProfile {
        gateway_url: p.gateway_url,
        api_key: p.api_key,
    }))
}

/// Before personal activate overwrites `profile.env`, snapshot a company overlay
/// into the durable baseline if one is not stored yet.
pub fn ensure_company_baseline_snapshot_from_active() -> Result<()> {
    if company_baseline_path()
        .as_ref()
        .is_some_and(|path| path.exists())
    {
        return Ok(());
    }
    let Some(active) = read_agent_profile()? else {
        return Ok(());
    };
    if active.kind == ProviderKind::Personal {
        return Ok(());
    }
    let Some(gateway) = active
        .gateway_url
        .as_deref()
        .filter(|u| !u.trim().is_empty())
    else {
        return Ok(());
    };
    let api_key = active.api_key.unwrap_or_default();
    let evotown_base = agent_profile_path()
        .filter(|path| path.exists())
        .and_then(|path| read_env_map(&path).ok())
        .and_then(|env| env.get("AGENT_DOCTOR_EVOTOWN_URL").cloned());
    write_company_baseline(gateway, &api_key, evotown_base.as_deref())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn writes_and_reads_company_profile() {
        let temp = TempDir::new().expect("tempdir");
        let path = temp.path().join("profile.env");
        write_company_profile(&path, "https://gateway.example/v1", "sk-test").unwrap();
        let env = read_env_map(&path).unwrap();
        assert_eq!(
            env.get(GATEWAY_URL_ENV).map(String::as_str),
            Some("https://gateway.example/v1")
        );
        assert_eq!(
            env.get(COMPANY_API_KEY_ENV).map(String::as_str),
            Some("sk-test")
        );
    }

    #[test]
    fn personal_active_does_not_serve_as_company_baseline() {
        let temp = TempDir::new().expect("tempdir");
        let profile = temp.path().join("profile.env");
        let baseline = temp.path().join("company-profile.env");

        let mut file = fs::File::create(&profile).unwrap();
        writeln!(file, "{PROVIDER_KIND_ENV}={PROVIDER_KIND_PERSONAL}").unwrap();
        writeln!(file, "{GATEWAY_URL_ENV}=https://personal.example/v1").unwrap();
        writeln!(file, "{COMPANY_API_KEY_ENV}=sk-personal").unwrap();

        // Simulate path helpers via direct reads used by baseline logic pieces.
        let env = read_env_map(&profile).unwrap();
        assert_eq!(kind_from_env(&env), ProviderKind::Personal);
        assert!(!baseline.exists());
        // Without a durable baseline file, personal overlay must not be treated as team.
        let kind = kind_from_env(&env);
        assert_ne!(kind, ProviderKind::Company);
    }

    #[test]
    fn company_baseline_survives_personal_overlay_semantics() {
        let temp = TempDir::new().expect("tempdir");
        let baseline = temp.path().join("company-profile.env");
        let mut file = fs::File::create(&baseline).unwrap();
        writeln!(
            file,
            "{GATEWAY_URL_ENV}=https://company.example/api/gateway/v1"
        )
        .unwrap();
        writeln!(file, "{COMPANY_API_KEY_ENV}=evk_team").unwrap();

        let env = read_env_map(&baseline).unwrap();
        let profile = profile_from_env(&env);
        assert_eq!(
            profile.gateway_url.as_deref(),
            Some("https://company.example/api/gateway/v1")
        );
    }
}
