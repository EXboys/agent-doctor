use std::path::Path;

use crate::repair_types::DiagnosticFact;

use super::super::config::ParsedConfig;
use super::super::schema::schema_error;
use super::super::ProbeCheck;

pub(crate) fn probe_json_object_schema(
    path: &Path,
    parsed: &ParsedConfig,
    checks: &mut Vec<ProbeCheck>,
    _facts: &mut Vec<DiagnosticFact>,
    label: &str,
) {
    let ParsedConfig::Json(value) = parsed else {
        return;
    };
    if !value.is_object() {
        checks.push(schema_error(
            path,
            format!("{label} settings root must be a JSON object"),
        ));
    }
}

pub(crate) fn probe_schema_qoder(
    path: &Path,
    parsed: &ParsedConfig,
    checks: &mut Vec<ProbeCheck>,
    facts: &mut Vec<DiagnosticFact>,
) {
    probe_json_object_schema(path, parsed, checks, facts, "Qoder");
}

pub(crate) fn probe_schema_cursor(
    path: &Path,
    parsed: &ParsedConfig,
    checks: &mut Vec<ProbeCheck>,
    facts: &mut Vec<DiagnosticFact>,
) {
    probe_json_object_schema(path, parsed, checks, facts, "Cursor");
}

pub(crate) fn probe_schema_zcode(
    path: &Path,
    parsed: &ParsedConfig,
    checks: &mut Vec<ProbeCheck>,
    facts: &mut Vec<DiagnosticFact>,
) {
    probe_json_object_schema(path, parsed, checks, facts, "ZCode");
}

pub(crate) fn probe_schema_workbuddy(
    path: &Path,
    parsed: &ParsedConfig,
    checks: &mut Vec<ProbeCheck>,
    facts: &mut Vec<DiagnosticFact>,
) {
    probe_json_object_schema(path, parsed, checks, facts, "WorkBuddy");
}

pub(crate) fn probe_schema_pi(
    path: &Path,
    parsed: &ParsedConfig,
    checks: &mut Vec<ProbeCheck>,
    facts: &mut Vec<DiagnosticFact>,
) {
    probe_json_object_schema(path, parsed, checks, facts, "Pi");
    let ParsedConfig::Json(value) = parsed else {
        return;
    };
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return;
    };
    if name != "models.json" {
        return;
    }
    let Some(url) = value
        .pointer("/providers/agent-doctor/baseUrl")
        .and_then(serde_json::Value::as_str)
    else {
        return;
    };
    let url = url.trim();
    if url.is_empty() {
        return;
    }
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        checks.push(schema_error(
            path,
            "Pi provider address must start with http:// or https://".to_string(),
        ));
    }
}

pub(crate) fn probe_deep_noop(_checks: &mut Vec<ProbeCheck>, _facts: &mut Vec<DiagnosticFact>) {}
