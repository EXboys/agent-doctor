use std::fs;
use std::path::{Path, PathBuf};

use crate::adapters::util::home_join;
use crate::repair::{DiagnosticFact, SensitivityLevel};

use super::super::config::{parse_env_file, ParsedConfig};
use super::super::schema::schema_error;
use super::super::{ProbeCheck, ProbeSeverity, ProbeStatus};

const ALLOWED_TOOL_PROFILES: &[&str] = &["minimal", "coding", "messaging", "full"];
const OPENCLAW_API_KEY_VARS: &[&str] = &["OPENAI_API_KEY", "ANTHROPIC_API_KEY", "OPENCLAW_API_KEY"];

pub(crate) fn probe_schema(
    path: &Path,
    parsed: &ParsedConfig,
    checks: &mut Vec<ProbeCheck>,
    facts: &mut Vec<DiagnosticFact>,
) {
    let ParsedConfig::Json(value) = parsed else {
        return;
    };

    if !value.is_object() {
        checks.push(schema_error(
            path,
            "OpenClaw config root must be a JSON object",
        ));
        return;
    }

    if let Some(url) = crate::adapters::configured_base_url(value) {
        facts.push(DiagnosticFact::new(
            "gateway.url",
            url,
            SensitivityLevel::ConfigShape,
        ));
    }

    if value.pointer("/gateway/url").is_some() || value.get("evotown").is_some() {
        checks.push(ProbeCheck::new(
            "openclaw.schema.legacy_gateway_url",
            "OpenClaw legacy LLM URL keys",
            ProbeStatus::Warn,
            ProbeSeverity::Warning,
            "gateway.url / evotown.url are invalid; use models.providers.*.baseUrl".to_string(),
            SensitivityLevel::ConfigShape,
        ));
    }

    if let Some(profile) = value
        .pointer("/tools/profile")
        .and_then(serde_json::Value::as_str)
    {
        if !ALLOWED_TOOL_PROFILES.contains(&profile) {
            checks.push(ProbeCheck::new(
                "openclaw.schema.tools_profile",
                "OpenClaw tools.profile",
                ProbeStatus::Warn,
                ProbeSeverity::Warning,
                format!("tools.profile has unsupported value '{profile}'"),
                SensitivityLevel::ConfigShape,
            ));
        }
    }

    if value.pointer("/agents/defaults/timeout").is_some() {
        checks.push(ProbeCheck::new(
            "openclaw.schema.legacy_timeout",
            "OpenClaw agent timeout",
            ProbeStatus::Warn,
            ProbeSeverity::Warning,
            "agents.defaults.timeout is legacy; expected timeoutSeconds".to_string(),
            SensitivityLevel::ConfigShape,
        ));
    }

    for pointer in ["/env/vars", "/env/shellEnv"] {
        if value
            .pointer(pointer)
            .is_some_and(serde_json::Value::is_string)
        {
            checks.push(ProbeCheck::new(
                format!("openclaw.schema.env_string:{pointer}"),
                "OpenClaw env section",
                ProbeStatus::Warn,
                ProbeSeverity::Warning,
                format!("{pointer} is a string; expected object"),
                SensitivityLevel::ConfigShape,
            ));
        }
    }
}

pub(crate) fn probe_deep(checks: &mut Vec<ProbeCheck>, facts: &mut Vec<DiagnosticFact>) {
    probe_dotenv_file(&openclaw_env_path(), checks, facts);
    probe_json_api_keys(&openclaw_config_path(), checks, facts);
}

fn openclaw_config_path() -> PathBuf {
    home_join(".openclaw/openclaw.json")
}

fn openclaw_env_path() -> PathBuf {
    home_join(".openclaw/.env")
}

fn probe_dotenv_file(path: &Path, checks: &mut Vec<ProbeCheck>, facts: &mut Vec<DiagnosticFact>) {
    if !path.exists() {
        return;
    }

    facts.push(DiagnosticFact::new(
        "openclaw.env.path",
        path.display().to_string(),
        SensitivityLevel::LocalPath,
    ));

    match fs::read_to_string(path) {
        Ok(raw) => {
            let env = parse_env_file(&raw);
            if env.malformed_lines.is_empty() {
                checks.push(ProbeCheck::new(
                    format!("openclaw.env.parse:{}", path.display()),
                    "OpenClaw .env parse",
                    ProbeStatus::Pass,
                    ProbeSeverity::Info,
                    "OpenClaw .env contains valid KEY=value entries",
                    SensitivityLevel::ConfigShape,
                ));
            } else {
                checks.push(
                    ProbeCheck::new(
                        format!("openclaw.env.parse:{}", path.display()),
                        "OpenClaw .env parse",
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
            probe_dotenv_api_keys(&env, path, checks, facts);
        }
        Err(error) => checks.push(ProbeCheck::new(
            format!("openclaw.env.read:{}", path.display()),
            "OpenClaw .env read",
            ProbeStatus::Warn,
            ProbeSeverity::Warning,
            format!("failed to read {}: {error}", path.display()),
            SensitivityLevel::LocalPath,
        )),
    }
}

#[cfg(unix)]
fn probe_env_permissions(path: &Path, checks: &mut Vec<ProbeCheck>) {
    use std::os::unix::fs::PermissionsExt;

    if let Ok(metadata) = fs::metadata(path) {
        let mode = metadata.permissions().mode() & 0o777;
        let too_open = mode & 0o077 != 0;
        checks.push(ProbeCheck::new(
            format!("openclaw.env.permissions:{}", path.display()),
            "OpenClaw .env permissions",
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

fn probe_dotenv_api_keys(
    env: &super::super::config::EnvFile,
    path: &Path,
    checks: &mut Vec<ProbeCheck>,
    facts: &mut Vec<DiagnosticFact>,
) {
    for key in OPENCLAW_API_KEY_VARS {
        let matches: Vec<_> = env
            .entries
            .iter()
            .filter(|entry| entry.key == *key)
            .collect();
        if matches.is_empty() {
            continue;
        }
        let configured = matches
            .iter()
            .any(|entry| entry.value_present && !entry.value_empty);
        facts.push(DiagnosticFact::new(
            format!("openclaw.api_key.{key}"),
            configured.to_string(),
            SensitivityLevel::Public,
        ));
        if matches.len() > 1 {
            checks.push(ProbeCheck::new(
                "openclaw.api_key.duplicates",
                "OpenClaw API key duplicates",
                ProbeStatus::Warn,
                ProbeSeverity::Warning,
                format!(
                    "{key} appears {} times in {}",
                    matches.len(),
                    path.display()
                ),
                SensitivityLevel::ConfigShape,
            ));
        }
        if !configured {
            checks.push(ProbeCheck::new(
                "openclaw.api_key.configured",
                "OpenClaw API key configured",
                ProbeStatus::Warn,
                ProbeSeverity::Warning,
                format!("{key} exists in {} but is empty", path.display()),
                SensitivityLevel::ConfigShape,
            ));
        }
    }
}

fn probe_json_api_keys(
    config_path: &Path,
    checks: &mut Vec<ProbeCheck>,
    facts: &mut Vec<DiagnosticFact>,
) {
    if !config_path.exists() {
        return;
    }
    let Ok(raw) = fs::read_to_string(config_path) else {
        return;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return;
    };
    let Some(vars) = value
        .pointer("/env/vars")
        .and_then(serde_json::Value::as_object)
    else {
        checks.push(ProbeCheck::new(
            "openclaw.api_key.configured",
            "OpenClaw API key configured",
            ProbeStatus::Warn,
            ProbeSeverity::Warning,
            "env.vars is missing; API keys cannot be verified".to_string(),
            SensitivityLevel::ConfigShape,
        ));
        return;
    };

    let mut any_required = false;
    for key in OPENCLAW_API_KEY_VARS {
        if let Some(entry) = vars.get(*key) {
            any_required = true;
            let configured = entry
                .as_str()
                .map(|value| !value.trim().is_empty())
                .unwrap_or(false);
            facts.push(DiagnosticFact::new(
                format!("openclaw.api_key.{key}"),
                configured.to_string(),
                SensitivityLevel::Public,
            ));
            if !configured {
                checks.push(ProbeCheck::new(
                    "openclaw.api_key.configured",
                    "OpenClaw API key configured",
                    ProbeStatus::Warn,
                    ProbeSeverity::Warning,
                    format!("env.vars.{key} is empty in openclaw.json"),
                    SensitivityLevel::ConfigShape,
                ));
            }
        }
    }
    if !any_required {
        checks.push(ProbeCheck::new(
            "openclaw.api_key.configured",
            "OpenClaw API key configured",
            ProbeStatus::Warn,
            ProbeSeverity::Warning,
            "no OPENAI/ANTHROPIC API keys found in env.vars".to_string(),
            SensitivityLevel::ConfigShape,
        ));
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn detects_openclaw_schema_warnings() {
        let value = serde_json::json!({
            "tools": { "profile": "bad" },
            "agents": { "defaults": { "timeout": 10 } },
            "env": { "vars": "{\"OPENAI_API_KEY\":\"x\"}" }
        });
        let mut checks = Vec::new();
        let mut facts = Vec::new();
        probe_schema(
            Path::new("/tmp/openclaw.json"),
            &ParsedConfig::Json(value),
            &mut checks,
            &mut facts,
        );
        assert!(checks
            .iter()
            .any(|c| c.id == "openclaw.schema.tools_profile"));
        assert!(checks
            .iter()
            .any(|c| c.id == "openclaw.schema.legacy_timeout"));
        assert!(checks
            .iter()
            .any(|c| c.id.starts_with("openclaw.schema.env_string:")));
    }
}
