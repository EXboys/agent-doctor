use std::fs;
use std::path::{Path, PathBuf};

use crate::adapters::HermesAdapter;
use crate::repair::{DiagnosticFact, SensitivityLevel};

use super::super::config::{parse_env_file, EnvFile, ParsedConfig};
use super::super::schema::{schema_error, schema_warn};
use super::super::{ProbeCheck, ProbeSeverity, ProbeStatus};

pub(crate) fn probe_schema(
    path: &Path,
    parsed: &ParsedConfig,
    checks: &mut Vec<ProbeCheck>,
    facts: &mut Vec<DiagnosticFact>,
) {
    match parsed {
        ParsedConfig::Yaml(value) => probe_yaml_schema(path, value, checks, facts),
        ParsedConfig::Env(env) => probe_env_schema(path, env, checks),
        _ => {}
    }
}

fn probe_yaml_schema(
    path: &Path,
    value: &serde_yaml::Value,
    checks: &mut Vec<ProbeCheck>,
    facts: &mut Vec<DiagnosticFact>,
) {
    let Some(model) = value.get("model") else {
        checks.push(schema_warn(path, "model section is missing".to_string()));
        return;
    };
    if !model.is_mapping() {
        checks.push(schema_error(path, "model section must be a mapping"));
        return;
    }

    for key in ["provider", "default", "base_url"] {
        match model.get(key).and_then(serde_yaml::Value::as_str) {
            Some(value) if !value.trim().is_empty() => {
                facts.push(DiagnosticFact::new(
                    format!(
                        "{}.{}",
                        if key == "default" { "model" } else { "hermes" },
                        if key == "default" { "name" } else { key }
                    ),
                    value,
                    SensitivityLevel::ConfigShape,
                ));
            }
            Some(_) => checks.push(schema_warn(path, format!("model.{key} is empty"))),
            None => checks.push(schema_warn(path, format!("model.{key} is missing"))),
        }
    }
    if let Some(base_url) = model.get("base_url").and_then(serde_yaml::Value::as_str) {
        if !base_url.starts_with("http://") && !base_url.starts_with("https://") {
            checks.push(schema_warn(
                path,
                "model.base_url should start with http:// or https://".to_string(),
            ));
        }
        facts.push(DiagnosticFact::new(
            "gateway.url",
            base_url,
            SensitivityLevel::ConfigShape,
        ));
    }
}

fn probe_env_schema(path: &Path, env: &EnvFile, checks: &mut Vec<ProbeCheck>) {
    if env.malformed_lines.is_empty() {
        checks.push(ProbeCheck::new(
            format!("hermes.env.parse:{}", path.display()),
            "Hermes .env parse",
            ProbeStatus::Pass,
            ProbeSeverity::Info,
            "Hermes .env contains valid KEY=value entries",
            SensitivityLevel::ConfigShape,
        ));
    } else {
        checks.push(
            ProbeCheck::new(
                format!("hermes.env.parse:{}", path.display()),
                "Hermes .env parse",
                ProbeStatus::Warn,
                ProbeSeverity::Warning,
                format!(
                    "{} .env lines are not KEY=value assignments",
                    env.malformed_lines.len()
                ),
                SensitivityLevel::ConfigShape,
            )
            .with_details(
                env.malformed_lines
                    .iter()
                    .map(|line| format!("line {line}"))
                    .collect(),
            ),
        );
    }
    probe_env_permissions(path, checks);
}

#[cfg(unix)]
fn probe_env_permissions(path: &Path, checks: &mut Vec<ProbeCheck>) {
    use std::os::unix::fs::PermissionsExt;

    if let Ok(metadata) = fs::metadata(path) {
        let mode = metadata.permissions().mode() & 0o777;
        let too_open = mode & 0o077 != 0;
        checks.push(ProbeCheck::new(
            format!("hermes.env.permissions:{}", path.display()),
            "Hermes .env permissions",
            if too_open {
                ProbeStatus::Warn
            } else {
                ProbeStatus::Pass
            },
            if too_open {
                ProbeSeverity::Warning
            } else {
                ProbeSeverity::Info
            },
            if too_open {
                format!(".env permissions are {mode:o}; recommended 600")
            } else {
                format!(".env permissions are {mode:o}")
            },
            SensitivityLevel::LocalPath,
        ));
    }
}

#[cfg(not(unix))]
fn probe_env_permissions(_path: &Path, _checks: &mut Vec<ProbeCheck>) {}

pub(crate) fn probe_deep(checks: &mut Vec<ProbeCheck>, facts: &mut Vec<DiagnosticFact>) {
    let provider_index = facts.iter().position(|fact| fact.key == "hermes.provider");
    let configured_provider = provider_index
        .and_then(|index| facts.get(index))
        .map(|fact| fact.value.trim().to_string())
        .unwrap_or_default();
    let base_url = facts
        .iter()
        .find(|fact| fact.key == "gateway.url")
        .map(|fact| fact.value.as_str())
        .unwrap_or_default();
    let provider = HermesAdapter::effective_provider(&configured_provider, base_url);

    if provider.is_empty() {
        checks.push(ProbeCheck::new(
            "hermes.provider",
            "Hermes provider",
            ProbeStatus::Warn,
            ProbeSeverity::Warning,
            "Hermes model.provider is missing; API key requirement cannot be determined",
            SensitivityLevel::ConfigShape,
        ));
        return;
    }

    if provider != configured_provider {
        if let Some(index) = provider_index {
            facts[index].value = provider.clone();
        }
        facts.push(DiagnosticFact::new(
            "hermes.provider.configured",
            configured_provider,
            SensitivityLevel::ConfigShape,
        ));
    }

    let api_key_env = HermesAdapter::provider_api_key_env(&provider);
    match api_key_env {
        None => {
            checks.push(ProbeCheck::new(
                "hermes.api_key.required",
                "Hermes API key requirement",
                ProbeStatus::NotApplicable,
                ProbeSeverity::Info,
                format!("provider '{provider}' does not require an API key"),
                SensitivityLevel::ConfigShape,
            ));
            facts.push(DiagnosticFact::new(
                "hermes.api_key.required",
                "false",
                SensitivityLevel::Public,
            ));
        }
        Some(env_key) => {
            facts.push(DiagnosticFact::new(
                "hermes.api_key.env",
                env_key.clone(),
                SensitivityLevel::ConfigShape,
            ));
            facts.push(DiagnosticFact::new(
                "hermes.api_key.required",
                "true",
                SensitivityLevel::Public,
            ));
            probe_required_key(&env_key, checks, facts);
        }
    }
}

fn probe_required_key(
    env_key: &str,
    checks: &mut Vec<ProbeCheck>,
    facts: &mut Vec<DiagnosticFact>,
) {
    let env_path = dirs::home_dir()
        .map(|home| home.join(".hermes/.env"))
        .unwrap_or_else(|| PathBuf::from(".hermes/.env"));

    if !env_path.exists() {
        checks.push(ProbeCheck::new(
            "hermes.api_key.configured",
            "Hermes API key configured",
            ProbeStatus::Warn,
            ProbeSeverity::Warning,
            format!("{env_key} is required but ~/.hermes/.env does not exist"),
            SensitivityLevel::LocalPath,
        ));
        facts.push(DiagnosticFact::new(
            "hermes.api_key.configured",
            "false",
            SensitivityLevel::Public,
        ));
        return;
    }

    let raw = match fs::read_to_string(&env_path) {
        Ok(raw) => raw,
        Err(error) => {
            checks.push(ProbeCheck::new(
                "hermes.api_key.configured",
                "Hermes API key configured",
                ProbeStatus::Warn,
                ProbeSeverity::Warning,
                format!("failed to read ~/.hermes/.env: {error}"),
                SensitivityLevel::LocalPath,
            ));
            return;
        }
    };

    let env = parse_env_file(&raw);
    let matches: Vec<_> = env
        .entries
        .iter()
        .filter(|entry| entry.key == env_key)
        .collect();
    let configured = matches
        .iter()
        .any(|entry| entry.value_present && !entry.value_empty);
    facts.push(DiagnosticFact::new(
        "hermes.api_key.configured",
        configured.to_string(),
        SensitivityLevel::Public,
    ));

    if matches.is_empty() {
        checks.push(ProbeCheck::new(
            "hermes.api_key.configured",
            "Hermes API key configured",
            ProbeStatus::Warn,
            ProbeSeverity::Warning,
            format!("{env_key} is missing from ~/.hermes/.env"),
            SensitivityLevel::ConfigShape,
        ));
        return;
    }

    if matches.len() > 1 {
        checks.push(ProbeCheck::new(
            "hermes.api_key.duplicates",
            "Hermes API key duplicates",
            ProbeStatus::Warn,
            ProbeSeverity::Warning,
            format!(
                "{env_key} appears {} times in ~/.hermes/.env",
                matches.len()
            ),
            SensitivityLevel::ConfigShape,
        ));
    }

    checks.push(ProbeCheck::new(
        "hermes.api_key.configured",
        "Hermes API key configured",
        if configured {
            ProbeStatus::Pass
        } else {
            ProbeStatus::Warn
        },
        if configured {
            ProbeSeverity::Info
        } else {
            ProbeSeverity::Warning
        },
        if configured {
            format!("{env_key} is configured in ~/.hermes/.env")
        } else {
            format!("{env_key} exists in ~/.hermes/.env but is empty")
        },
        SensitivityLevel::ConfigShape,
    ));
}
