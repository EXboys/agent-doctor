use std::fs;
use std::path::Path;

use crate::adapters::{DeepSeekHarnessAdapter, DEEPSEEK_API_KEY_ENV, DEEPSEEK_HARNESS_VERSION};
use crate::prompt_session::env::{collect_overlay_env, resolve_deepseek_harness_overlay};
use crate::repair::{DiagnosticFact, SensitivityLevel};

use super::super::config::{parse_env_file, ParsedConfig};
use super::super::schema::{schema_error, schema_warn};
use super::super::{ProbeCheck, ProbeSeverity, ProbeStatus};

pub(crate) fn probe_schema(
    path: &Path,
    parsed: &ParsedConfig,
    checks: &mut Vec<ProbeCheck>,
    _facts: &mut Vec<DiagnosticFact>,
) {
    let ParsedConfig::Yaml(value) = parsed else {
        return;
    };
    if !value.is_mapping() {
        checks.push(schema_error(
            path,
            "DeepSeek Harness YAML root must be a mapping",
        ));
        return;
    }
    if value.as_mapping().is_some_and(|mapping| mapping.is_empty()) {
        checks.push(schema_warn(
            path,
            "DeepSeek Harness YAML mapping is empty".to_string(),
        ));
    }
}

pub(crate) fn probe_deep(checks: &mut Vec<ProbeCheck>, facts: &mut Vec<DiagnosticFact>) {
    probe_api_key(checks, facts);
    probe_pinned_version(checks, facts);
}

fn probe_api_key(checks: &mut Vec<ProbeCheck>, facts: &mut Vec<DiagnosticFact>) {
    let process_configured = std::env::var(DEEPSEEK_API_KEY_ENV)
        .ok()
        .is_some_and(|value| !value.trim().is_empty());
    let env_path = DeepSeekHarnessAdapter::env_path();
    let file_configured = fs::read_to_string(&env_path).ok().is_some_and(|raw| {
        parse_env_file(&raw)
            .entries
            .iter()
            .any(|entry| entry.key == DEEPSEEK_API_KEY_ENV && !entry.value_empty)
    });
    let wiring_configured = resolve_deepseek_harness_overlay(&collect_overlay_env())
        .1
        .is_some();
    let configured = process_configured || file_configured || wiring_configured;
    let source = if process_configured {
        "process environment".to_string()
    } else if file_configured {
        env_path.display().to_string()
    } else if wiring_configured {
        "Agent Doctor wiring".to_string()
    } else {
        "none".to_string()
    };

    checks.push(ProbeCheck::new(
        "deepseek-harness.api_key.configured",
        "DeepSeek API key configured",
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
            format!("{DEEPSEEK_API_KEY_ENV} is configured via {source}")
        } else {
            format!(
                "{DEEPSEEK_API_KEY_ENV} is missing from Agent Doctor wiring, the process environment, and {}",
                env_path.display()
            )
        },
        if file_configured {
            SensitivityLevel::LocalPath
        } else {
            SensitivityLevel::ConfigShape
        },
    ));
    facts.push(DiagnosticFact::new(
        "deepseek-harness.api_key.configured",
        configured.to_string(),
        SensitivityLevel::Public,
    ));
    if configured {
        facts.push(DiagnosticFact::new(
            "deepseek-harness.api_key.source",
            source,
            if file_configured {
                SensitivityLevel::LocalPath
            } else {
                SensitivityLevel::ConfigShape
            },
        ));
    }
}

fn probe_pinned_version(checks: &mut Vec<ProbeCheck>, facts: &mut Vec<DiagnosticFact>) {
    let Some(raw_version) = facts
        .iter()
        .find(|fact| fact.key == "binary.version")
        .map(|fact| fact.value.as_str())
    else {
        return;
    };
    let detected = extract_version(raw_version);
    let exact = detected == Some(DEEPSEEK_HARNESS_VERSION);
    let (status, message) = match detected {
        Some(_) if exact => (
            ProbeStatus::Pass,
            format!("installed version exactly matches {DEEPSEEK_HARNESS_VERSION}"),
        ),
        Some(version) => (
            ProbeStatus::Warn,
            format!(
                "installed version {version} does not match required {DEEPSEEK_HARNESS_VERSION}"
            ),
        ),
        None => (
            ProbeStatus::Warn,
            format!("could not parse installed version; expected {DEEPSEEK_HARNESS_VERSION}"),
        ),
    };
    checks.push(ProbeCheck::new(
        "deepseek-harness.version.pinned",
        "Pinned DeepSeek Harness version",
        status,
        if exact {
            ProbeSeverity::Info
        } else {
            ProbeSeverity::Warning
        },
        message,
        SensitivityLevel::Public,
    ));
    facts.push(DiagnosticFact::new(
        "deepseek-harness.version.matches",
        exact.to_string(),
        SensitivityLevel::Public,
    ));
}

fn extract_version(raw: &str) -> Option<&str> {
    raw.split(|character: char| {
        character.is_whitespace() || matches!(character, ',' | ';' | '(' | ')' | '[' | ']')
    })
    .map(|part| part.trim_start_matches('v'))
    .find(|part| {
        let first = part.chars().next();
        first.is_some_and(|character| character.is_ascii_digit())
            && part.chars().any(|character| character == '.')
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_release_candidate_version_safely() {
        assert_eq!(
            extract_version("dsh version 0.1.0-rc.6"),
            Some("0.1.0-rc.6")
        );
        assert_eq!(extract_version("unknown build"), None);
    }

    #[test]
    fn version_mismatch_is_warning_only() {
        let mut checks = Vec::new();
        let mut facts = vec![DiagnosticFact::new(
            "binary.version",
            "dsh 0.1.0-rc.5",
            SensitivityLevel::Public,
        )];
        probe_pinned_version(&mut checks, &mut facts);
        assert_eq!(checks[0].status, ProbeStatus::Warn);
        assert_eq!(checks[0].severity, ProbeSeverity::Warning);
    }
}
