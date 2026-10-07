use anyhow::Result;

use crate::adapters::DEEPSEEK_HARNESS_VERSION;
use crate::lifecycle::{run_deepseek_harness_lifecycle, DeepSeekHarnessLifecycleAction};
use crate::probe::{ProbeStatus, RuntimeProbeReport};
use crate::repair::SuggestedRepair;

use super::rule::{apply_rules, suggest_rules, CheckMatch, Fix, Rule, Versions};
use super::PlaybookApplyResult;

/// DeepSeek Harness repair rules. `{pinned}` in text becomes the pinned package version.
/// The version check only exists when the binary is installed, so install and pin never both run.
pub(crate) const DEEPSEEK_HARNESS_RULES: &[Rule] = &[
    Rule {
        id: "fix-deepseek-harness-install",
        title: "Install DeepSeek Harness",
        description: "Install the official npm package pinned to {pinned}.",
        check: CheckMatch::Is("binary.exists", ProbeStatus::Fail),
        versions: Versions::ANY,
        fix: Fix::Auto(|_| {
            run_deepseek_harness_lifecycle(DeepSeekHarnessLifecycleAction::Install).map(|_| None)
        }),
    },
    Rule {
        id: "fix-deepseek-harness-version",
        title: "Pin DeepSeek Harness to {pinned}",
        description: "Install the required official npm package version exactly.",
        check: CheckMatch::Is("deepseek-harness.version.pinned", ProbeStatus::Warn),
        versions: Versions::ANY,
        fix: Fix::Auto(|_| {
            run_deepseek_harness_lifecycle(DeepSeekHarnessLifecycleAction::Update).map(|_| None)
        }),
    },
    Rule {
        id: "configure-deepseek-harness-credentials",
        title: "Configure DeepSeek credentials",
        description: "Add the API key in Agent Doctor wiring or the dsh web Models page; \
            secrets are never auto-filled.",
        check: CheckMatch::Is("deepseek-harness.api_key.configured", ProbeStatus::Warn),
        versions: Versions::ANY,
        fix: Fix::Manual,
    },
];

pub fn suggest_deepseek_harness_repairs(probe: &RuntimeProbeReport) -> Vec<SuggestedRepair> {
    let mut items: Vec<_> = suggest_rules(DEEPSEEK_HARNESS_RULES, probe)
        .into_iter()
        .map(|mut item| {
            item.title = item.title.replace("{pinned}", DEEPSEEK_HARNESS_VERSION);
            item.description = item
                .description
                .replace("{pinned}", DEEPSEEK_HARNESS_VERSION);
            item
        })
        .collect();
    items.extend(super::npm_cli::suggest_browser_mcp_repairs(
        "deepseek-harness",
        "DeepSeek Harness",
        probe,
    ));
    items
}

pub fn apply_deepseek_harness_playbook(probe: &RuntimeProbeReport) -> Result<PlaybookApplyResult> {
    apply_deepseek_harness_playbook_filtered(probe, None)
}

pub fn apply_deepseek_harness_playbook_filtered(
    probe: &RuntimeProbeReport,
    only_ids: Option<&[String]>,
) -> Result<PlaybookApplyResult> {
    let mut result = apply_rules(DEEPSEEK_HARNESS_RULES, probe, only_ids);
    let browser = super::npm_cli::apply_browser_mcp_repair("deepseek-harness", probe, only_ids)?;
    result.executed.extend(browser.executed);
    result.skipped.extend(browser.skipped);
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
            .any(|item| item.id == "fix-deepseek-harness-version"
                && item.title.ends_with(DEEPSEEK_HARNESS_VERSION)));
    }

    #[test]
    fn rules_have_unique_ids_and_desktop_labels() {
        super::super::rule::assert_unique_ids(DEEPSEEK_HARNESS_RULES);
        super::super::rule::assert_desktop_labels(&super::super::rule::fix_ids(
            DEEPSEEK_HARNESS_RULES,
        ));
    }
}
