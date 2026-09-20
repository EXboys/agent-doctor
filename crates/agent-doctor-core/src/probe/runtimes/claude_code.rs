use std::fs;
use std::path::{Path, PathBuf};

use crate::adapters::util::home_join;
use crate::prompt_session::env::collect_overlay_env;
use crate::repair::{DiagnosticFact, SensitivityLevel};

use super::super::config::ParsedConfig;
use super::super::schema::{schema_error, schema_warn};
use super::super::{ProbeCheck, ProbeSeverity, ProbeStatus};

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
            "Claude settings root must be a JSON object",
        ));
        return;
    }

    if value.get("env").is_some_and(|env| !env.is_object()) {
        checks.push(schema_warn(path, "env should be an object".to_string()));
        return;
    }

    let env = value.get("env").and_then(|v| v.as_object());
    let env_base = env
        .and_then(|e| e.get("ANTHROPIC_BASE_URL"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let top_base = value
        .get("anthropicBaseUrl")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    let effective_url = env_base.clone().or_else(|| top_base.clone());
    if let Some(url) = effective_url {
        facts.push(DiagnosticFact::new(
            "gateway.url",
            &url,
            SensitivityLevel::ConfigShape,
        ));
        if !url.starts_with("http://") && !url.starts_with("https://") {
            checks.push(schema_warn(
                path,
                "ANTHROPIC_BASE_URL / anthropicBaseUrl should start with http:// or https://"
                    .to_string(),
            ));
        }
        if let (Some(a), Some(b)) = (&env_base, &top_base) {
            if normalize_url(a) != normalize_url(b) {
                checks.push(ProbeCheck::new(
                    "claude.schema.base_url_mismatch",
                    "Claude base URL consistency",
                    ProbeStatus::Warn,
                    ProbeSeverity::Warning,
                    "env.ANTHROPIC_BASE_URL and anthropicBaseUrl disagree".to_string(),
                    SensitivityLevel::ConfigShape,
                ));
            }
        }
    } else if path
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n == "settings.json")
    {
        checks.push(schema_warn(
            path,
            "ANTHROPIC_BASE_URL / anthropicBaseUrl is missing".to_string(),
        ));
    }

    let api_key_present = env
        .and_then(|e| e.get("ANTHROPIC_API_KEY"))
        .and_then(|v| v.as_str())
        .is_some_and(|s| !s.trim().is_empty());
    facts.push(DiagnosticFact::new(
        "claude.api_key.in_settings",
        if api_key_present { "true" } else { "false" },
        SensitivityLevel::Public,
    ));
}

pub(crate) fn probe_deep(checks: &mut Vec<ProbeCheck>, facts: &mut Vec<DiagnosticFact>) {
    let path = settings_path();
    probe_api_key_configured(&path, checks, facts);
    probe_settings_permissions(&path, checks);
}

fn probe_api_key_configured(
    path: &Path,
    checks: &mut Vec<ProbeCheck>,
    facts: &mut Vec<DiagnosticFact>,
) {
    let in_settings = facts
        .iter()
        .find(|f| f.key == "claude.api_key.in_settings")
        .is_some_and(|f| f.value.eq_ignore_ascii_case("true"))
        || settings_has_api_key(path);

    let overlay = collect_overlay_env();
    let from_wiring = overlay
        .get("ANTHROPIC_API_KEY")
        .is_some_and(|v| !v.trim().is_empty());

    let configured = in_settings || from_wiring;
    checks.push(ProbeCheck::new(
        "claude.api_key.configured",
        "Claude API key configured",
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
        if in_settings {
            "ANTHROPIC_API_KEY is set in ~/.claude/settings.json"
        } else if from_wiring {
            "ANTHROPIC_API_KEY is available from Agent Doctor wiring"
        } else {
            "ANTHROPIC_API_KEY is missing in settings and Agent Doctor wiring"
        },
        SensitivityLevel::ConfigShape,
    ));
    facts.push(DiagnosticFact::new(
        "claude.api_key.configured",
        if configured { "true" } else { "false" },
        SensitivityLevel::Public,
    ));
    facts.push(DiagnosticFact::new(
        "claude.api_key.env",
        "ANTHROPIC_API_KEY",
        SensitivityLevel::ConfigShape,
    ));
}

fn settings_has_api_key(path: &Path) -> bool {
    let Ok(raw) = fs::read_to_string(path) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return false;
    };
    value
        .pointer("/env/ANTHROPIC_API_KEY")
        .and_then(|v| v.as_str())
        .is_some_and(|s| !s.trim().is_empty())
}

#[cfg(unix)]
fn probe_settings_permissions(path: &Path, checks: &mut Vec<ProbeCheck>) {
    use std::os::unix::fs::PermissionsExt;

    if !path.exists() || !settings_has_api_key(path) {
        return;
    }
    if let Ok(metadata) = fs::metadata(path) {
        let mode = metadata.permissions().mode() & 0o777;
        let too_open = mode & 0o077 != 0;
        checks.push(ProbeCheck::new(
            format!("claude.settings.permissions:{}", path.display()),
            "Claude settings permissions",
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
                format!(
                    "settings.json permissions are {mode:o}; recommended 600 (contains API key)"
                )
            } else {
                format!("settings.json permissions are {mode:o}")
            },
            SensitivityLevel::LocalPath,
        ));
    }
}

#[cfg(not(unix))]
fn probe_settings_permissions(_path: &Path, _checks: &mut Vec<ProbeCheck>) {}

fn settings_path() -> PathBuf {
    home_join(".claude/settings.json")
}

fn normalize_url(url: &str) -> String {
    url.trim().trim_end_matches('/').to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn schema_warns_on_missing_base_url() {
        let mut checks = Vec::new();
        let mut facts = Vec::new();
        let path = Path::new("/tmp/settings.json");
        probe_schema(
            path,
            &ParsedConfig::Json(json!({"env": {}})),
            &mut checks,
            &mut facts,
        );
        assert!(checks
            .iter()
            .any(|c| c.message.contains("ANTHROPIC_BASE_URL")));
        assert!(facts
            .iter()
            .any(|f| f.key == "claude.api_key.in_settings" && f.value == "false"));
    }

    #[test]
    fn schema_records_gateway_and_key_shape() {
        let mut checks = Vec::new();
        let mut facts = Vec::new();
        let path = Path::new("/tmp/settings.json");
        probe_schema(
            path,
            &ParsedConfig::Json(json!({
                "env": {
                    "ANTHROPIC_BASE_URL": "https://gateway.example/v1",
                    "ANTHROPIC_API_KEY": "sk-test"
                },
                "anthropicBaseUrl": "https://gateway.example/v1"
            })),
            &mut checks,
            &mut facts,
        );
        assert!(facts
            .iter()
            .any(|f| f.key == "gateway.url" && f.value.contains("gateway.example")));
        assert!(facts
            .iter()
            .any(|f| f.key == "claude.api_key.in_settings" && f.value == "true"));
        assert!(!checks.iter().any(|c| c.status == ProbeStatus::Warn));
    }

    #[test]
    fn schema_warns_on_base_url_mismatch() {
        let mut checks = Vec::new();
        let mut facts = Vec::new();
        probe_schema(
            Path::new("/tmp/settings.json"),
            &ParsedConfig::Json(json!({
                "env": { "ANTHROPIC_BASE_URL": "https://a.example/v1" },
                "anthropicBaseUrl": "https://b.example/v1"
            })),
            &mut checks,
            &mut facts,
        );
        assert!(checks
            .iter()
            .any(|c| c.id == "claude.schema.base_url_mismatch"));
    }
}
