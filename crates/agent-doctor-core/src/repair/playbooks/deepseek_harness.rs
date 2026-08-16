use anyhow::Result;

use crate::adapters::DEEPSEEK_HARNESS_VERSION;
use crate::lifecycle::{run_deepseek_harness_lifecycle, DeepSeekHarnessLifecycleAction};
use crate::probe::{ProbeStatus, RuntimeProbeReport};
use crate::repair::{SkippedRepairAction, SuggestedRepair};

use super::{should_run, PlaybookApplyResult};

pub fn suggest_deepseek_harness_repairs(probe: &RuntimeProbeReport) -> Vec<SuggestedRepair> {
    let mut items = Vec::new();
    for check in &probe.checks {
        if check.id == "binary.exists" && check.status == ProbeStatus::Fail {
            items.push(SuggestedRepair {
                id: "fix-deepseek-harness-install".to_string(),
                title: "Install DeepSeek Harness".to_string(),
                description: format!(
                    "Install the official npm package pinned to {DEEPSEEK_HARNESS_VERSION}."
                ),
                auto_fixable: true,
            });
        }
        if check.id == "deepseek-harness.version.pinned" && check.status == ProbeStatus::Warn {
            items.push(SuggestedRepair {
                id: "fix-deepseek-harness-version".to_string(),
                title: format!("Pin DeepSeek Harness to {DEEPSEEK_HARNESS_VERSION}"),
                description: "Install the required official npm package version exactly."
                    .to_string(),
                auto_fixable: true,
            });
        }
        if check.id == "deepseek-harness.api_key.configured" && check.status == ProbeStatus::Warn {
            items.push(SuggestedRepair {
                id: "configure-deepseek-harness-credentials".to_string(),
                title: "Configure DeepSeek credentials".to_string(),
                description:
                    "Add the API key in Agent Doctor wiring or the dsh web Models page; secrets are never auto-filled."
                        .to_string(),
                auto_fixable: false,
            });
        }
    }
    items
}

pub fn apply_deepseek_harness_playbook(probe: &RuntimeProbeReport) -> Result<PlaybookApplyResult> {
    apply_deepseek_harness_playbook_filtered(probe, None)
}

pub fn apply_deepseek_harness_playbook_filtered(
    probe: &RuntimeProbeReport,
    only_ids: Option<&[String]>,
) -> Result<PlaybookApplyResult> {
    let mut result = PlaybookApplyResult::default();
    let missing = probe
        .checks
        .iter()
        .any(|check| check.id == "binary.exists" && check.status == ProbeStatus::Fail);
    let mismatch = probe.checks.iter().any(|check| {
        check.id == "deepseek-harness.version.pinned" && check.status == ProbeStatus::Warn
    });

    let action = if missing && should_run("fix-deepseek-harness-install", only_ids) {
        Some((
            "fix-deepseek-harness-install",
            DeepSeekHarnessLifecycleAction::Install,
        ))
    } else if mismatch && should_run("fix-deepseek-harness-version", only_ids) {
        Some((
            "fix-deepseek-harness-version",
            DeepSeekHarnessLifecycleAction::Update,
        ))
    } else {
        None
    };

    if let Some((id, lifecycle_action)) = action {
        match run_deepseek_harness_lifecycle(lifecycle_action) {
            Ok(()) => result.executed.push(id.to_string()),
            Err(error) => result.skipped.push(SkippedRepairAction {
                id: id.to_string(),
                reason: error.to_string(),
            }),
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe::{ProbeCheck, ProbeSeverity};
    use crate::repair::SensitivityLevel;

    #[test]
    fn suggests_pin_for_version_warning() {
        let probe = RuntimeProbeReport {
            runtime_id: "deepseek-harness".into(),
            display_name: "DeepSeek Harness".into(),
            binary_name: "dsh".into(),
            checks: vec![ProbeCheck::new(
                "deepseek-harness.version.pinned",
                "version",
                ProbeStatus::Warn,
                ProbeSeverity::Warning,
                "mismatch",
                SensitivityLevel::Public,
            )],
            facts: Vec::new(),
        };
        assert!(suggest_deepseek_harness_repairs(&probe)
            .iter()
            .any(|item| item.id == "fix-deepseek-harness-version"));
    }
}
