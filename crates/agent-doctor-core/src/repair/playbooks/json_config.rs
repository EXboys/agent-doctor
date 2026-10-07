//! JSON config helpers shared by every runtime playbook, plus the shared
//! "fix JSON format" rule wired in from the runtime registry.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde_json::{json, Value};

use crate::probe::{ProbeCheck, ProbeStatus, RuntimeProbeReport};
use crate::repair::{SkippedRepairAction, SuggestedRepair};

use super::should_run;
use super::PlaybookApplyResult;

pub(crate) const CONFIG_SYNTAX_FIX_ID: &str = "fix-config-syntax";

/// Missing file reads as `{}` so callers can create it.
pub(crate) fn load_json_config(path: &Path) -> Result<Value> {
    if path.exists() {
        let raw = fs::read_to_string(path)?;
        serde_json::from_str(&raw).with_context(|| format!("failed to parse {}", path.display()))
    } else {
        Ok(json!({}))
    }
}

/// Writes pretty JSON, keeping a timestamped `.bak` copy of any existing file.
pub(crate) fn write_json_config(path: &Path, root: &Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    if path.exists() {
        backup_config(path)?;
    }
    fs::write(path, serde_json::to_string_pretty(root)?)
        .with_context(|| format!("failed to write {}", path.display()))
}

pub(crate) fn backup_config(path: &Path) -> Result<()> {
    let original = fs::read_to_string(path)?;
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let backup_path = path.with_extension(format!("json.bak.{ts}"));
    fs::write(&backup_path, original)?;
    Ok(())
}

/// JSON config files whose parse check failed in this probe.
fn broken_json_configs(probe: &RuntimeProbeReport) -> Vec<PathBuf> {
    probe
        .checks
        .iter()
        .filter_map(broken_json_config_path)
        .collect()
}

fn broken_json_config_path(check: &ProbeCheck) -> Option<PathBuf> {
    if check.status != ProbeStatus::Fail {
        return None;
    }
    let path = check.id.strip_prefix("config.parse:")?;
    path.ends_with(".json").then(|| PathBuf::from(path))
}

pub(crate) fn suggest_config_syntax_repairs(probe: &RuntimeProbeReport) -> Vec<SuggestedRepair> {
    if broken_json_configs(probe).is_empty() {
        return Vec::new();
    }
    vec![SuggestedRepair {
        id: CONFIG_SYNTAX_FIX_ID.to_string(),
        title: "Fix config file format".to_string(),
        description: "Remove trailing commas and comments from JSON config files \
            when that makes them valid. Nothing is written otherwise."
            .to_string(),
        auto_fixable: true,
    }]
}

/// Runs before runtime playbooks: their fixes all need the config to parse.
pub(crate) fn apply_config_syntax_repair(
    probe: &RuntimeProbeReport,
    only_ids: Option<&[String]>,
) -> PlaybookApplyResult {
    let mut result = PlaybookApplyResult::default();
    if !should_run(CONFIG_SYNTAX_FIX_ID, only_ids) {
        return result;
    }
    let mut changed = false;
    for path in broken_json_configs(probe) {
        match fix_json_syntax(&path) {
            Ok(fixed) => changed |= fixed,
            Err(error) => result.skipped.push(SkippedRepairAction {
                id: CONFIG_SYNTAX_FIX_ID.to_string(),
                reason: error.to_string(),
            }),
        }
    }
    if changed {
        result.executed.push(CONFIG_SYNTAX_FIX_ID.to_string());
    }
    result
}

/// Returns `false` when the file already parses.
pub(crate) fn fix_json_syntax(path: &Path) -> Result<bool> {
    let raw =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    if serde_json::from_str::<Value>(&raw).is_ok() {
        return Ok(false);
    }
    let relaxed = strip_trailing_commas(&strip_json_comments(&raw));
    let root: Value = serde_json::from_str(&relaxed).with_context(|| {
        format!(
            "{} has a format error that cannot be fixed automatically",
            path.display()
        )
    })?;
    write_json_config(path, &root)?;
    Ok(true)
}

/// Drops `//` and `/* */` comments outside of strings.
fn strip_json_comments(raw: &str) -> String {
    let chars: Vec<char> = raw.chars().collect();
    let mut out = String::with_capacity(raw.len());
    let mut i = 0;
    let mut in_string = false;
    while i < chars.len() {
        let c = chars[i];
        if in_string {
            out.push(c);
            if c == '\\' && i + 1 < chars.len() {
                out.push(chars[i + 1]);
                i += 2;
                continue;
            }
            if c == '"' {
                in_string = false;
            }
            i += 1;
            continue;
        }
        if c == '"' {
            in_string = true;
            out.push(c);
            i += 1;
        } else if c == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
        } else if c == '/' && chars.get(i + 1) == Some(&'*') {
            i += 2;
            while i < chars.len() && !(chars[i] == '*' && chars.get(i + 1) == Some(&'/')) {
                i += 1;
            }
            i = (i + 2).min(chars.len());
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}

/// Drops commas that directly precede `}` or `]` outside of strings.
fn strip_trailing_commas(raw: &str) -> String {
    let chars: Vec<char> = raw.chars().collect();
    let mut out = String::with_capacity(raw.len());
    let mut in_string = false;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if in_string {
            out.push(c);
            if c == '\\' && i + 1 < chars.len() {
                out.push(chars[i + 1]);
                i += 2;
                continue;
            }
            if c == '"' {
                in_string = false;
            }
            i += 1;
            continue;
        }
        if c == '"' {
            in_string = true;
        } else if c == ',' {
            let next = chars[i + 1..].iter().find(|ch| !ch.is_whitespace());
            if matches!(next, Some('}') | Some(']')) {
                i += 1;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe::ProbeSeverity;
    use crate::repair::SensitivityLevel;

    fn probe_with(checks: Vec<ProbeCheck>) -> RuntimeProbeReport {
        RuntimeProbeReport {
            runtime_id: "claude-code".to_string(),
            display_name: "Claude Code".to_string(),
            binary_name: "claude".to_string(),
            checks,
            facts: vec![],
        }
    }

    fn parse_check(path: &Path, status: ProbeStatus) -> ProbeCheck {
        ProbeCheck::new(
            format!("config.parse:{}", path.display()),
            "Config parse",
            status,
            ProbeSeverity::Error,
            "invalid JSON",
            SensitivityLevel::SensitiveLog,
        )
    }

    #[test]
    fn suggests_only_for_failed_json_configs() {
        let json = probe_with(vec![parse_check(
            Path::new("/x/settings.json"),
            ProbeStatus::Fail,
        )]);
        assert_eq!(suggest_config_syntax_repairs(&json).len(), 1);

        let yaml = probe_with(vec![parse_check(
            Path::new("/x/config.yaml"),
            ProbeStatus::Fail,
        )]);
        assert!(suggest_config_syntax_repairs(&yaml).is_empty());

        let ok = probe_with(vec![parse_check(
            Path::new("/x/settings.json"),
            ProbeStatus::Pass,
        )]);
        assert!(suggest_config_syntax_repairs(&ok).is_empty());
    }

    #[test]
    fn apply_fixes_every_broken_json_config() {
        let temp = tempfile::tempdir().expect("tempdir");
        let a = temp.path().join("settings.json");
        let b = temp.path().join("other.json");
        fs::write(&a, "{\"a\": 1,}").unwrap();
        fs::write(&b, "{\"b\": [1,],}").unwrap();
        let probe = probe_with(vec![
            parse_check(&a, ProbeStatus::Fail),
            parse_check(&b, ProbeStatus::Fail),
        ]);
        let result = apply_config_syntax_repair(&probe, None);
        assert_eq!(result.executed, vec![CONFIG_SYNTAX_FIX_ID.to_string()]);
        assert_eq!(load_json_config(&a).unwrap(), json!({"a": 1}));
        assert_eq!(load_json_config(&b).unwrap(), json!({"b": [1]}));
    }

    #[test]
    fn apply_respects_only_ids_filter() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("settings.json");
        fs::write(&path, "{\"a\": 1,}").unwrap();
        let probe = probe_with(vec![parse_check(&path, ProbeStatus::Fail)]);
        let only = vec!["fix-something-else".to_string()];
        let result = apply_config_syntax_repair(&probe, Some(&only));
        assert!(result.executed.is_empty());
        assert_eq!(fs::read_to_string(&path).unwrap(), "{\"a\": 1,}");
    }

    #[test]
    fn fix_removes_trailing_commas_and_comments_but_keeps_strings() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("openclaw.json");
        fs::write(
            &path,
            "{\n  // note\n  \"url\": \"https://a.b/c,}\",\n  \"list\": [1, 2,],\n  /* x */ \"n\": {\"k\": 1,},\n}\n",
        )
        .unwrap();
        assert!(fix_json_syntax(&path).unwrap());
        let updated = load_json_config(&path).unwrap();
        assert_eq!(updated.pointer("/url"), Some(&json!("https://a.b/c,}")));
        assert_eq!(updated.pointer("/list"), Some(&json!([1, 2])));
        assert_eq!(updated.pointer("/n/k"), Some(&json!(1)));
    }

    #[test]
    fn fix_leaves_unfixable_file_alone() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("openclaw.json");
        let broken = "{\"a\": 1 \"b\": 2}";
        fs::write(&path, broken).unwrap();
        assert!(fix_json_syntax(&path).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), broken);
    }

    #[test]
    fn fix_is_noop_for_valid_file() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("settings.json");
        fs::write(&path, "{\"a\": 1}").unwrap();
        assert!(!fix_json_syntax(&path).unwrap());
    }
}
