use crate::adapters::util::home_join;
use crate::presets::load_profiles;
use crate::profile::read_company_baseline;

use super::backends::{openclaw_default_agent_id, openclaw_defaults_workspace};
use super::{WorkspaceCheck, WorkspaceCheckStatus, WorkspaceEntry};

pub fn baseline_drift_checks(entry: &WorkspaceEntry) -> Vec<WorkspaceCheck> {
    let mut checks = Vec::new();
    // Team baseline only — never the personal provider overlay in profile.env.
    let company = match read_company_baseline() {
        Ok(Some(profile)) => profile,
        _ => return checks,
    };
    let Some(expected_gateway) = company.gateway_url.filter(|url| !url.is_empty()) else {
        return checks;
    };
    let expected_gateway = normalize_url(&expected_gateway);
    let expected_label = expected_gateway.clone();

    if let Some(actual) = read_openclaw_gateway_url() {
        let actual = normalize_url(&actual);
        if actual != expected_gateway {
            checks.push(WorkspaceCheck {
                id: "workspace.baseline.gateway.openclaw".into(),
                title: "OpenClaw gateway drifts from company profile".into(),
                status: WorkspaceCheckStatus::Warn,
                detail: format!("profile={expected_label} openclaw={actual}"),
            });
        } else {
            checks.push(WorkspaceCheck {
                id: "workspace.baseline.gateway.openclaw".into(),
                title: "OpenClaw gateway matches company profile".into(),
                status: WorkspaceCheckStatus::Pass,
                detail: expected_label.clone(),
            });
        }
    }

    if let Some(actual) = read_hermes_profile_gateway(&entry.hermes_profile) {
        let actual = normalize_url(&actual);
        if actual != expected_gateway {
            checks.push(WorkspaceCheck {
                id: "workspace.baseline.gateway.hermes".into(),
                title: "Hermes workspace profile gateway drifts from company profile".into(),
                status: WorkspaceCheckStatus::Warn,
                detail: format!(
                    "profile={expected_label} hermes_profile={} url={actual}",
                    entry.hermes_profile
                ),
            });
        } else {
            checks.push(WorkspaceCheck {
                id: "workspace.baseline.gateway.hermes".into(),
                title: "Hermes workspace profile gateway matches company profile".into(),
                status: WorkspaceCheckStatus::Pass,
                detail: expected_label.clone(),
            });
        }
    }

    if let Ok(profiles) = load_profiles() {
        if let Some(active) = profiles.active.as_ref().and_then(|name| {
            profiles
                .profiles
                .get(name)
                .and_then(|entry| entry.hermes.as_ref())
        }) {
            let preset = normalize_url(&active.base_url);
            if preset != expected_gateway {
                checks.push(WorkspaceCheck {
                    id: "workspace.baseline.preset.gateway".into(),
                    title: "Active Agent Doctor preset gateway differs from company profile".into(),
                    status: WorkspaceCheckStatus::Warn,
                    detail: format!("company={expected_label} preset={preset}"),
                });
            }
        }
    }

    checks
}

pub fn openclaw_routing_checks(entry: &WorkspaceEntry) -> Vec<WorkspaceCheck> {
    let mut checks = Vec::new();
    let Some(default_agent) = openclaw_default_agent_id() else {
        return checks;
    };

    if default_agent != entry.openclaw_agent_id {
        checks.push(WorkspaceCheck {
            id: "workspace.openclaw.routing.default".into(),
            title: "OpenClaw default agent does not match workspace".into(),
            status: WorkspaceCheckStatus::Warn,
            detail: format!(
                "expected='{}' actual default='{default_agent}' — run workspace use/fix",
                entry.openclaw_agent_id
            ),
        });
    } else {
        checks.push(WorkspaceCheck {
            id: "workspace.openclaw.routing.default".into(),
            title: "OpenClaw default agent matches workspace".into(),
            status: WorkspaceCheckStatus::Pass,
            detail: entry.openclaw_agent_id.clone(),
        });
    }

    if let Some(defaults_workspace) = openclaw_defaults_workspace() {
        if !super::backends::workspace_paths_match(&entry.openclaw_workspace, &defaults_workspace) {
            checks.push(WorkspaceCheck {
                id: "workspace.openclaw.routing.defaults_workspace".into(),
                title: "agents.defaults.workspace differs from workspace".into(),
                status: WorkspaceCheckStatus::Warn,
                detail: format!(
                    "expected={} actual={}",
                    entry.openclaw_workspace.display(),
                    defaults_workspace.display()
                ),
            });
        } else {
            checks.push(WorkspaceCheck {
                id: "workspace.openclaw.routing.defaults_workspace".into(),
                title: "agents.defaults.workspace matches workspace".into(),
                status: WorkspaceCheckStatus::Pass,
                detail: entry.openclaw_workspace.display().to_string(),
            });
        }
    }

    checks
}

pub fn codex_isolation_checks(entry: &WorkspaceEntry) -> Vec<WorkspaceCheck> {
    let global_home = home_join(".codex");
    if super::backends::workspace_paths_match(&entry.codex_home, &global_home) {
        return vec![WorkspaceCheck {
            id: "workspace.codex.shared_global_home".into(),
            title: "Workspace CODEX_HOME points at global ~/.codex".into(),
            status: WorkspaceCheckStatus::Fail,
            detail: "Re-init workspace or run workspace fix to restore isolated CODEX_HOME".into(),
        }];
    }

    let marker = entry.codex_home.join(".agent-doctor-codex-home");
    if marker.exists() {
        vec![WorkspaceCheck {
            id: "workspace.codex.isolation_marker".into(),
            title: "Codex isolation marker present".into(),
            status: WorkspaceCheckStatus::Pass,
            detail: entry.codex_home.display().to_string(),
        }]
    } else {
        vec![WorkspaceCheck {
            id: "workspace.codex.isolation_marker".into(),
            title: "Codex isolation marker missing".into(),
            status: WorkspaceCheckStatus::Warn,
            detail: "Run workspace use/fix to refresh isolated CODEX_HOME scaffold".into(),
        }]
    }
}

fn read_openclaw_gateway_url() -> Option<String> {
    let path = home_join(".openclaw/openclaw.json");
    let raw = std::fs::read_to_string(path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&raw).ok()?;
    crate::adapters::configured_base_url(&value)
}

fn read_hermes_profile_gateway(profile: &str) -> Option<String> {
    let path = home_join(".hermes/profiles")
        .join(profile)
        .join("config.yaml");
    let raw = std::fs::read_to_string(path).ok()?;
    let value: serde_yaml::Value = serde_yaml::from_str(&raw).ok()?;
    value
        .get("model")
        .and_then(|model| model.get("base_url"))
        .and_then(serde_yaml::Value::as_str)
        .map(str::to_string)
}

fn normalize_url(url: &str) -> String {
    url.trim().trim_end_matches('/').to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_url_strips_trailing_slash() {
        assert_eq!(
            normalize_url("https://Example.com/v1/"),
            "https://example.com/v1"
        );
    }
}
