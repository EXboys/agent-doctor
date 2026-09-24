use std::path::Path;

use crate::repair::DiagnosticFact;

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

pub(crate) fn probe_schema_workbuddy(
    path: &Path,
    parsed: &ParsedConfig,
    checks: &mut Vec<ProbeCheck>,
    facts: &mut Vec<DiagnosticFact>,
) {
    probe_json_object_schema(path, parsed, checks, facts, "WorkBuddy");
}

pub(crate) fn probe_deep_noop(_checks: &mut Vec<ProbeCheck>, _facts: &mut Vec<DiagnosticFact>) {}
