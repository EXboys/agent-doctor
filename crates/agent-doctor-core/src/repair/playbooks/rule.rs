use std::cmp::Ordering;
use std::path::PathBuf;

use anyhow::Result;

use crate::probe::{ProbeCheck, ProbeStatus, RuntimeProbeReport};
use crate::repair::{SkippedRepairAction, SuggestedRepair};

use super::should_run;
use super::PlaybookApplyResult;

/// Returns a guide file to open when the fix leaves the user one manual step.
pub(crate) type FixFn = fn(&RuntimeProbeReport) -> Result<Option<PathBuf>>;

/// One repair rule: which check triggers it, which tool versions it covers, and the fix.
/// Every rule id that runs a fix needs a label in `desktop/src/repair-ui.ts`.
pub(crate) struct Rule {
    pub id: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub check: CheckMatch,
    pub versions: Versions,
    pub fix: Fix,
}

pub(crate) enum CheckMatch {
    Is(&'static str, ProbeStatus),
    StartsWith(&'static str, ProbeStatus),
    Custom(fn(&ProbeCheck) -> bool),
}

impl CheckMatch {
    fn matches(&self, check: &ProbeCheck) -> bool {
        match self {
            CheckMatch::Is(id, status) => check.id == *id && check.status == *status,
            CheckMatch::StartsWith(prefix, status) => {
                check.id.starts_with(prefix) && check.status == *status
            }
            CheckMatch::Custom(matches) => matches(check),
        }
    }
}

pub(crate) enum Fix {
    /// Agent Doctor makes the change.
    Auto(FixFn),
    /// Agent Doctor makes the change only when `ready` passes; otherwise the user sees `blocked`.
    AutoWhen {
        ready: fn(&RuntimeProbeReport) -> bool,
        blocked: &'static str,
        run: FixFn,
    },
    /// Runs during apply without its own suggestion, as prep for a `Manual` rule on the same check.
    Prep(FixFn),
    /// Only the user can do this; listed as a suggestion.
    Manual,
}

/// Installed tool versions a rule applies to: `from` inclusive, `before` exclusive.
/// When the installed version is unknown, the rule applies.
#[derive(Clone, Copy)]
pub(crate) struct Versions {
    pub from: Option<&'static str>,
    pub before: Option<&'static str>,
}

impl Versions {
    pub const ANY: Versions = Versions {
        from: None,
        before: None,
    };

    pub(crate) fn contains(&self, installed: Option<&str>) -> bool {
        let Some(installed) = installed else {
            return true;
        };
        let after_from = self.from.map_or(true, |from| {
            compare_versions(installed, from) != Ordering::Less
        });
        let below_before = self.before.map_or(true, |before| {
            compare_versions(installed, before) == Ordering::Less
        });
        after_from && below_before
    }
}

fn version_parts(version: &str) -> Vec<u64> {
    version
        .trim()
        .trim_start_matches(['v', 'V'])
        .split(['.', '-', '+'])
        .map_while(|part| part.parse::<u64>().ok())
        .collect()
}

pub(crate) fn compare_versions(a: &str, b: &str) -> Ordering {
    let (a, b) = (version_parts(a), version_parts(b));
    for index in 0..a.len().max(b.len()) {
        let left = a.get(index).copied().unwrap_or(0);
        let right = b.get(index).copied().unwrap_or(0);
        match left.cmp(&right) {
            Ordering::Equal => continue,
            other => return other,
        }
    }
    Ordering::Equal
}

pub(crate) fn installed_version(probe: &RuntimeProbeReport) -> Option<String> {
    probe
        .checks
        .iter()
        .find(|check| check.id == "binary.version" && check.status == ProbeStatus::Pass)
        .and_then(|check| crate::version_check::extract_version(&check.message))
}

fn triggered<'a>(
    rules: &'a [Rule],
    probe: &'a RuntimeProbeReport,
) -> impl Iterator<Item = &'a Rule> {
    let version = installed_version(probe);
    rules.iter().filter(move |rule| {
        rule.versions.contains(version.as_deref())
            && probe.checks.iter().any(|check| rule.check.matches(check))
    })
}

pub(crate) fn suggest_rules(rules: &[Rule], probe: &RuntimeProbeReport) -> Vec<SuggestedRepair> {
    triggered(rules, probe)
        .filter_map(|rule| {
            let (auto_fixable, description) = match &rule.fix {
                Fix::Auto(_) => (true, rule.description),
                Fix::AutoWhen { ready, blocked, .. } => {
                    if ready(probe) {
                        (true, rule.description)
                    } else {
                        (false, *blocked)
                    }
                }
                Fix::Prep(_) => return None,
                Fix::Manual => (false, rule.description),
            };
            Some(SuggestedRepair {
                id: rule.id.to_string(),
                title: rule.title.to_string(),
                description: description.to_string(),
                auto_fixable,
            })
        })
        .collect()
}

pub(crate) fn apply_rules(
    rules: &[Rule],
    probe: &RuntimeProbeReport,
    only_ids: Option<&[String]>,
) -> PlaybookApplyResult {
    let mut result = PlaybookApplyResult::default();
    for rule in triggered(rules, probe) {
        let run = match &rule.fix {
            Fix::Auto(run) | Fix::Prep(run) | Fix::AutoWhen { run, .. } => run,
            Fix::Manual => continue,
        };
        if !should_run(rule.id, only_ids) {
            continue;
        }
        match run(probe) {
            Ok(guide_path) => {
                result.executed.push(rule.id.to_string());
                if guide_path.is_some() {
                    result.guide_path = guide_path;
                }
            }
            Err(error) => result.skipped.push(SkippedRepairAction {
                id: rule.id.to_string(),
                reason: error.to_string(),
            }),
        }
    }
    result
}

/// Rule ids whose fix can run.
#[cfg(test)]
pub(crate) fn fix_ids(rules: &[Rule]) -> Vec<&'static str> {
    rules
        .iter()
        .filter(|rule| !matches!(rule.fix, Fix::Manual))
        .map(|rule| rule.id)
        .collect()
}

/// Fails when a fix id has no plain-language label in the desktop app.
#[cfg(test)]
pub(crate) fn assert_desktop_labels(ids: &[&str]) {
    let labels = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../desktop/src/repair-ui.ts"
    ));
    let missing: Vec<_> = ids
        .iter()
        .filter(|id| !labels.contains(&format!("\"{id}\":")))
        .collect();
    assert!(
        missing.is_empty(),
        "add these ids to REPAIR_FIX_LABEL_KEYS in desktop/src/repair-ui.ts: {missing:?}"
    );
}

#[cfg(test)]
pub(crate) fn assert_unique_ids(rules: &[Rule]) {
    let mut ids: Vec<_> = rules.iter().map(|rule| rule.id).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), rules.len(), "duplicate rule id");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe::ProbeSeverity;
    use crate::repair::SensitivityLevel;

    fn check(id: &str, status: ProbeStatus, message: &str) -> ProbeCheck {
        ProbeCheck::new(
            id,
            id,
            status,
            ProbeSeverity::Warning,
            message,
            SensitivityLevel::Public,
        )
    }

    fn probe(checks: Vec<ProbeCheck>) -> RuntimeProbeReport {
        RuntimeProbeReport {
            runtime_id: "tool".into(),
            display_name: "Tool".into(),
            binary_name: "tool".into(),
            checks,
            facts: vec![],
        }
    }

    fn ok(_: &RuntimeProbeReport) -> Result<Option<PathBuf>> {
        Ok(None)
    }

    fn rule(id: &'static str, versions: Versions, fix: Fix) -> Rule {
        Rule {
            id,
            title: id,
            description: id,
            check: CheckMatch::Is("schema.old_field", ProbeStatus::Warn),
            versions,
            fix,
        }
    }

    #[test]
    fn compares_dotted_versions() {
        assert_eq!(compare_versions("2026.9.5", "2026.10.0"), Ordering::Less);
        assert_eq!(compare_versions("v1.2", "1.2.0"), Ordering::Equal);
        assert_eq!(compare_versions("2.0.0-beta.1", "1.9"), Ordering::Greater);
    }

    #[test]
    fn version_range_is_from_inclusive_before_exclusive() {
        let range = Versions {
            from: Some("2026.3.0"),
            before: Some("2026.9.0"),
        };
        assert!(range.contains(Some("2026.3.0")));
        assert!(range.contains(Some("2026.8.9")));
        assert!(!range.contains(Some("2026.9.0")));
        assert!(!range.contains(Some("2025.12.1")));
        assert!(range.contains(None));
    }

    #[test]
    fn rules_outside_installed_version_are_skipped() {
        let rules = [
            rule(
                "fix-old",
                Versions {
                    from: None,
                    before: Some("2026.9.0"),
                },
                Fix::Auto(ok),
            ),
            rule("fix-any", Versions::ANY, Fix::Auto(ok)),
        ];
        let report = probe(vec![
            check("binary.version", ProbeStatus::Pass, "Tool 2026.9.5 (abc)"),
            check("schema.old_field", ProbeStatus::Warn, "old"),
        ]);
        let ids: Vec<_> = suggest_rules(&rules, &report)
            .into_iter()
            .map(|item| item.id)
            .collect();
        assert_eq!(ids, vec!["fix-any"]);
        assert_eq!(apply_rules(&rules, &report, None).executed, vec!["fix-any"]);
    }

    #[test]
    fn one_suggestion_per_rule_and_prep_stays_hidden() {
        let rules = [
            rule("fix-auto", Versions::ANY, Fix::Auto(ok)),
            rule("prep", Versions::ANY, Fix::Prep(ok)),
            rule("do-it-yourself", Versions::ANY, Fix::Manual),
        ];
        let report = probe(vec![
            check("schema.old_field", ProbeStatus::Warn, "a"),
            check("schema.old_field", ProbeStatus::Warn, "b"),
        ]);
        let suggested = suggest_rules(&rules, &report);
        let ids: Vec<_> = suggested.iter().map(|item| item.id.as_str()).collect();
        assert_eq!(ids, vec!["fix-auto", "do-it-yourself"]);
        assert!(!suggested[1].auto_fixable);
        let applied = apply_rules(&rules, &report, None);
        assert_eq!(applied.executed, vec!["fix-auto", "prep"]);
    }

    #[test]
    fn blocked_rule_is_manual_and_says_why() {
        let rules = [rule(
            "fix-gateway",
            Versions::ANY,
            Fix::AutoWhen {
                ready: |_| false,
                blocked: "set up first",
                run: ok,
            },
        )];
        let report = probe(vec![check("schema.old_field", ProbeStatus::Warn, "x")]);
        let suggested = suggest_rules(&rules, &report);
        assert!(!suggested[0].auto_fixable);
        assert_eq!(suggested[0].description, "set up first");
    }
}
