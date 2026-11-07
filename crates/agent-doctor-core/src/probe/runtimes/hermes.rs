use std::fs;
use std::path::{Path, PathBuf};

use crate::adapters::HermesAdapter;
use crate::repair_types::{DiagnosticFact, SensitivityLevel};

use super::super::config::{parse_env_file, EnvFile, ParsedConfig};
use super::super::schema::{schema_error, schema_warn};
#[cfg(unix)]
use super::super::ProbeCategory;
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
    probe_dotted_model_rewrite(model, checks);
}

/// Hermes turns `glm-5.3` into `glm-5-3` when the provider or URL is Anthropic.
fn probe_dotted_model_rewrite(model: &serde_yaml::Value, checks: &mut Vec<ProbeCheck>) {
    use crate::setup::merge::hermes_model_will_be_rewritten;

    let provider = model
        .get("provider")
        .and_then(serde_yaml::Value::as_str)
        .unwrap_or("");
    let api_mode = model
        .get("api_mode")
        .and_then(serde_yaml::Value::as_str)
        .unwrap_or("");
    let base_url = model
        .get("base_url")
        .and_then(serde_yaml::Value::as_str)
        .unwrap_or("");
    let name = model
        .get("default")
        .and_then(serde_yaml::Value::as_str)
        .unwrap_or("");
    if !hermes_model_will_be_rewritten(provider, api_mode, base_url, name) {
        return;
    }
    checks.push(ProbeCheck::new(
        "hermes.model.dot_rewrite",
        "Hermes model id",
        ProbeStatus::Fail,
        ProbeSeverity::Error,
        format!("Hermes would rewrite model `{name}` (provider={provider}, api_mode={api_mode})"),
        SensitivityLevel::ConfigShape,
    ));
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
        checks.push(
            ProbeCheck::new(
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
            )
            .with_category(ProbeCategory::Hardening),
        );
    }
}

#[cfg(not(unix))]
fn probe_env_permissions(_path: &Path, _checks: &mut Vec<ProbeCheck>) {}

pub(crate) fn probe_deep(checks: &mut Vec<ProbeCheck>, facts: &mut Vec<DiagnosticFact>) {
    probe_gateway_serving(checks);
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

/// A resident gateway that booted before Agent Doctor's workspace profiles
/// existed records "only one profile exists" and refuses those profiles
/// ("profile parked") on every Ask until it restarts. The verdict survives in
/// the default home's gateway_state.json, so the file alone names the trap —
/// a not-running gateway leaves no file and gets no check.
fn probe_gateway_serving(checks: &mut Vec<ProbeCheck>) {
    probe_gateway_serving_at(
        &crate::adapters::util::home_join(".hermes/gateway_state.json"),
        checks,
    );
}

fn probe_gateway_serving_at(path: &Path, checks: &mut Vec<ProbeCheck>) {
    let Ok(raw) = fs::read_to_string(path) else {
        return;
    };
    let Ok(state) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return;
    };
    let Some(reason) = gateway_standalone_reason(&state) else {
        return;
    };
    checks.push(ProbeCheck::new(
        "hermes.gateway.serving",
        "Hermes gateway serves Ask profiles",
        ProbeStatus::Warn,
        ProbeSeverity::Warning,
        format!(
            "the resident gateway went standalone ({reason}); Ask profiles are refused \
             (profile parked) until it restarts — run `hermes gateway restart`"
        ),
        SensitivityLevel::ConfigShape,
    ));
}

/// The recorded standalone verdict. Hermes clears the field whenever a boot
/// ends up serving profiles, so an empty or absent field means healthy.
fn gateway_standalone_reason(state: &serde_json::Value) -> Option<String> {
    let reason = state.get("multiplex_standalone_reason")?.as_str()?.trim();
    (!reason.is_empty()).then(|| reason.to_string())
}

/// What the `~/.hermes/.env` scan concluded about the required key.
#[derive(Debug, PartialEq)]
enum DotenvKeyVerdict {
    NoFile,
    Unreadable(String),
    Absent,
    Empty,
    Configured,
}

/// One source among several: the app's overlay env, a launcher, or the parent
/// shell can hand Hermes the key through the process environment. A key
/// missing from the .env file is therefore "unverified", never "broken" —
/// Hermes actually working is the deciding evidence.
fn key_in_process_env(env_key: &str) -> bool {
    let overlay = crate::session_env::collect_overlay_env();
    overlay
        .get(env_key)
        .is_some_and(|value| !value.trim().is_empty())
        || std::env::var(env_key)
            .ok()
            .is_some_and(|value| !value.trim().is_empty())
}

/// The verdict plus how many times the key appears in the .env (dupes warn on
/// top of whatever the verdict is).
fn scan_dotenv_key(env_path: &Path, env_key: &str) -> (DotenvKeyVerdict, Option<usize>) {
    if !env_path.exists() {
        return (DotenvKeyVerdict::NoFile, None);
    }
    let raw = match fs::read_to_string(env_path) {
        Ok(raw) => raw,
        Err(error) => return (DotenvKeyVerdict::Unreadable(error.to_string()), None),
    };
    let env = parse_env_file(&raw);
    let matches: Vec<_> = env
        .entries
        .iter()
        .filter(|entry| entry.key == env_key)
        .collect();
    if matches.is_empty() {
        return (DotenvKeyVerdict::Absent, None);
    }
    let duplicates = (matches.len() > 1).then_some(matches.len());
    let configured = matches
        .iter()
        .any(|entry| entry.value_present && !entry.value_empty);
    (
        if configured {
            DotenvKeyVerdict::Configured
        } else {
            DotenvKeyVerdict::Empty
        },
        duplicates,
    )
}

/// The single check that says whether Hermes can authenticate. `pass` means a
/// source was found; the wording never upgrades a static gap to a fault.
fn push_key_verdict(
    env_key: &str,
    verdict: &DotenvKeyVerdict,
    from_process_env: bool,
    checks: &mut Vec<ProbeCheck>,
) {
    use DotenvKeyVerdict::*;
    let pass = from_process_env || *verdict == Configured;
    let (status, severity) = if pass {
        (ProbeStatus::Pass, ProbeSeverity::Info)
    } else {
        (ProbeStatus::Warn, ProbeSeverity::Warning)
    };
    let sensitivity = if matches!(verdict, NoFile | Unreadable(_)) {
        SensitivityLevel::LocalPath
    } else {
        SensitivityLevel::ConfigShape
    };
    let detail = match (verdict, from_process_env) {
        (Configured, _) => format!("{env_key} is configured in ~/.hermes/.env"),
        (Absent, true) => {
            format!("{env_key} is set in the process environment; it is not in ~/.hermes/.env")
        }
        (Empty, true) => {
            format!("{env_key} in ~/.hermes/.env is empty, but the process environment sets it")
        }
        (NoFile, true) => {
            format!("{env_key} is set in the process environment; ~/.hermes/.env does not exist")
        }
        (Unreadable(_), true) => {
            format!("{env_key} is set in the process environment; ~/.hermes/.env could not be read")
        }
        (Absent, false) => format!(
            "{env_key} was not found in ~/.hermes/.env or the process environment; \
             if Hermes still works, its credential source is elsewhere (unverified)"
        ),
        (NoFile, false) => format!(
            "{env_key} is required but was not found in ~/.hermes/.env or the process environment; \
             if Hermes still works, its credential source is elsewhere (unverified)"
        ),
        (Empty, false) => format!("{env_key} exists in ~/.hermes/.env but is empty"),
        (Unreadable(error), false) => format!("failed to read ~/.hermes/.env: {error}"),
    };
    checks.push(ProbeCheck::new(
        "hermes.api_key.configured",
        "Hermes API key configured",
        status,
        severity,
        detail,
        sensitivity,
    ));
}

fn probe_required_key(
    env_key: &str,
    checks: &mut Vec<ProbeCheck>,
    facts: &mut Vec<DiagnosticFact>,
) {
    let from_process_env = key_in_process_env(env_key);
    let env_path = dirs::home_dir()
        .map(|home| home.join(".hermes/.env"))
        .unwrap_or_else(|| PathBuf::from(".hermes/.env"));
    let (verdict, duplicates) = scan_dotenv_key(&env_path, env_key);

    let configured = verdict == DotenvKeyVerdict::Configured;
    let usable = configured || from_process_env;
    if !matches!(verdict, DotenvKeyVerdict::Unreadable(_)) {
        facts.push(DiagnosticFact::new(
            "hermes.api_key.configured",
            usable.to_string(),
            SensitivityLevel::Public,
        ));
        let source = if configured {
            "dotenv"
        } else if from_process_env {
            "process_env"
        } else {
            ""
        };
        if !source.is_empty() {
            facts.push(DiagnosticFact::new(
                "hermes.api_key.source",
                source,
                SensitivityLevel::Public,
            ));
        }
    }

    if let Some(count) = duplicates {
        checks.push(ProbeCheck::new(
            "hermes.api_key.duplicates",
            "Hermes API key duplicates",
            ProbeStatus::Warn,
            ProbeSeverity::Warning,
            format!("{env_key} appears {count} times in ~/.hermes/.env"),
            SensitivityLevel::ConfigShape,
        ));
    }

    push_key_verdict(env_key, &verdict, from_process_env, checks);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A static gap alone never grades as a fault: without the process env the
    /// wording stays "unverified", with it the check passes outright.
    #[test]
    fn key_verdicts_stay_unverified_without_runtime_evidence() {
        let cases: Vec<(DotenvKeyVerdict, ProbeStatus)> = vec![
            (DotenvKeyVerdict::Configured, ProbeStatus::Pass),
            (DotenvKeyVerdict::Absent, ProbeStatus::Warn),
            (DotenvKeyVerdict::Empty, ProbeStatus::Warn),
            (DotenvKeyVerdict::NoFile, ProbeStatus::Warn),
            (
                DotenvKeyVerdict::Unreadable("denied".into()),
                ProbeStatus::Warn,
            ),
        ];
        for (verdict, status) in cases {
            let mut checks = Vec::new();
            push_key_verdict("CUSTOM_API_KEY", &verdict, false, &mut checks);
            assert_eq!(checks[0].status, status, "verdict {verdict:?}");
            assert!(
                !checks[0].message.contains("无法调用"),
                "never claims a call failure"
            );
            assert!(
                !checks[0].message.contains("unverified") || status == ProbeStatus::Warn,
                "pass wording never says unverified"
            );
        }
    }

    #[test]
    fn process_env_evidence_upgrades_every_static_gap_to_pass() {
        for verdict in [
            DotenvKeyVerdict::Absent,
            DotenvKeyVerdict::Empty,
            DotenvKeyVerdict::NoFile,
            DotenvKeyVerdict::Unreadable("denied".into()),
        ] {
            let mut checks = Vec::new();
            push_key_verdict("CUSTOM_API_KEY", &verdict, true, &mut checks);
            assert_eq!(checks[0].status, ProbeStatus::Pass, "verdict {verdict:?}");
            assert!(checks[0].message.contains("process environment"));
        }
    }

    #[test]
    fn gateway_standalone_verdict_is_the_only_warn_trigger() {
        let parked: serde_json::Value = serde_json::from_str(
            r#"{"multiplex_standalone_reason": "only one profile exists (nothing to multiplex)"}"#,
        )
        .unwrap();
        assert_eq!(
            gateway_standalone_reason(&parked).as_deref(),
            Some("only one profile exists (nothing to multiplex)")
        );
        // A gateway that serves profiles clears the field; absence is healthy too.
        let serving: serde_json::Value =
            serde_json::from_str(r#"{"multiplex_standalone_reason": null}"#).unwrap();
        assert_eq!(gateway_standalone_reason(&serving), None);
        let no_field: serde_json::Value = serde_json::from_str(r#"{"pid": 12}"#).unwrap();
        assert_eq!(gateway_standalone_reason(&no_field), None);
        let empty: serde_json::Value =
            serde_json::from_str(r#"{"multiplex_standalone_reason": "  "}"#).unwrap();
        assert_eq!(gateway_standalone_reason(&empty), None);
    }

    #[test]
    fn a_recorded_standalone_verdict_becomes_a_warn_check() {
        let mut checks = Vec::new();
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("gateway_state.json");
        std::fs::write(
            &state,
            r#"{"multiplex_standalone_reason": "only one profile exists (nothing to multiplex)"}"#,
        )
        .unwrap();
        probe_gateway_serving_at(&state, &mut checks);
        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0].status, ProbeStatus::Warn);
        assert!(checks[0].message.contains("hermes gateway restart"));

        // No file (gateway not resident) and a healthy verdict stay silent.
        let mut checks = Vec::new();
        probe_gateway_serving_at(&dir.path().join("missing.json"), &mut checks);
        assert!(checks.is_empty());
        let mut checks = Vec::new();
        std::fs::write(&state, r#"{"multiplex_standalone_reason": null}"#).unwrap();
        probe_gateway_serving_at(&state, &mut checks);
        assert!(checks.is_empty());
    }
}
