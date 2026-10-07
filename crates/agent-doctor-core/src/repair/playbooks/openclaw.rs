use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde_json::{json, Value};

use crate::adapters::util::home_join;
use crate::lifecycle::{run_openclaw_doctor_fix, run_openclaw_lifecycle, OpenClawLifecycleAction};
use crate::probe::{ProbeStatus, RuntimeProbeReport};
use crate::profile::read_company_profile;
use crate::repair::playbooks::hermes::dedupe_env_key_lines;
use crate::repair::SuggestedRepair;

use super::json_config::{load_json_config, write_json_config};
use super::rule::{apply_rules, suggest_rules, CheckMatch, Fix, Rule, Versions};
use super::PlaybookApplyResult;

const DEFAULT_TOOL_PROFILE: &str = "coding";
const OPENCLAW_API_KEY_VARS: &[&str] = &["OPENAI_API_KEY", "ANTHROPIC_API_KEY"];

/// OpenClaw repair rules, in the order they run.
/// When OpenClaw drops or renames a setting, give the old-format rule a `before` version
/// instead of deleting it, so people on older installs keep getting the fix.
pub(crate) const OPENCLAW_RULES: &[Rule] = &[
    Rule {
        id: "fix-openclaw-install",
        title: "Install OpenClaw",
        description: "Run the official OpenClaw installer (openclaw.ai/install.sh) \
            with --no-onboard. Requires network access.",
        check: CheckMatch::Is("binary.exists", ProbeStatus::Fail),
        versions: Versions::ANY,
        fix: Fix::Auto(|_| run_openclaw_lifecycle(OpenClawLifecycleAction::Install).map(|_| None)),
    },
    Rule {
        id: "fix-openclaw-create-config",
        title: "Create OpenClaw config",
        description: "Create ~/.openclaw/openclaw.json with models.providers from \
            company profile when available.",
        check: CheckMatch::Custom(|check| {
            check.id.starts_with("config.exists:")
                && check.status == ProbeStatus::Warn
                && check.id.contains("openclaw.json")
        }),
        versions: Versions::ANY,
        fix: Fix::Auto(|_| create_openclaw_config().map(|_| None)),
    },
    Rule {
        id: "fix-openclaw-gateway-from-profile",
        title: "Apply company gateway to OpenClaw",
        description: "Set models.providers.evotown|personal.baseUrl from profile.env.",
        check: CheckMatch::Is("gateway.configured", ProbeStatus::Warn),
        versions: Versions::ANY,
        fix: Fix::AutoWhen {
            ready: |_| company_gateway_url().is_ok(),
            blocked: "Run `agent-doctor setup --url ... --key ...` first.",
            run: |_| apply_gateway_from_company_profile().map(|_| None),
        },
    },
    Rule {
        id: "fix-openclaw-legacy-gateway-url",
        title: "Migrate OpenClaw LLM URL to models.providers",
        description: "Remove invalid gateway.url / evotown.url and write \
            models.providers.evotown|personal.baseUrl.",
        check: CheckMatch::Is("openclaw.schema.legacy_gateway_url", ProbeStatus::Warn),
        versions: Versions::ANY,
        fix: Fix::Auto(|_| migrate_legacy_gateway_url().map(|_| None)),
    },
    Rule {
        id: "fix-openclaw-env-permissions",
        title: "Tighten ~/.openclaw/.env permissions",
        description: "Set .env to mode 600.",
        check: CheckMatch::StartsWith("openclaw.env.permissions:", ProbeStatus::Warn),
        versions: Versions::ANY,
        fix: Fix::AutoWhen {
            ready: |_| cfg!(unix),
            blocked: "Permission tightening is only supported on macOS and Linux.",
            run: |_| tighten_env_permissions().map(|_| None),
        },
    },
    Rule {
        id: "fix-openclaw-api-key-duplicates",
        title: "Deduplicate OpenClaw .env API keys",
        description: "Keep the last non-empty API key assignment.",
        check: CheckMatch::Is("openclaw.api_key.duplicates", ProbeStatus::Warn),
        versions: Versions::ANY,
        fix: Fix::Auto(|probe| dedupe_openclaw_dotenv(probe).map(|_| None)),
    },
    Rule {
        id: "fix-openclaw-legacy-agents-list",
        title: "Update OpenClaw config format",
        description: "agents.list is no longer accepted. Run openclaw doctor --fix.",
        check: CheckMatch::Is("openclaw.schema.legacy_agents_list", ProbeStatus::Warn),
        versions: Versions::ANY,
        fix: Fix::Auto(|_| run_openclaw_doctor_fix().map(|_| None)),
    },
    Rule {
        id: "fix-openclaw-legacy-timeout",
        title: "Migrate agent timeout field",
        description: "Rename agents.defaults.timeout to timeoutSeconds.",
        check: CheckMatch::Is("openclaw.schema.legacy_timeout", ProbeStatus::Warn),
        versions: Versions::ANY,
        fix: Fix::Auto(|_| fix_legacy_timeout_field().map(|_| None)),
    },
    Rule {
        id: "fix-openclaw-env-object",
        title: "Fix env.vars / env.shellEnv shape",
        description: "Parse string env sections into JSON objects.",
        check: CheckMatch::StartsWith("openclaw.schema.env_string:", ProbeStatus::Warn),
        versions: Versions::ANY,
        fix: Fix::Auto(|_| fix_env_string_sections().map(|_| None)),
    },
    Rule {
        id: "fix-openclaw-tools-profile",
        title: "Reset tools.profile",
        description: "Set tools.profile to 'coding'.",
        check: CheckMatch::Is("openclaw.schema.tools_profile", ProbeStatus::Warn),
        versions: Versions::ANY,
        fix: Fix::Auto(|_| fix_tools_profile().map(|_| None)),
    },
    Rule {
        id: "fix-openclaw-api-key-scaffold",
        title: "Prepare OpenClaw API key placeholders",
        description: "Add empty key slots and a short guide.",
        check: CheckMatch::Is("openclaw.api_key.configured", ProbeStatus::Warn),
        versions: Versions::ANY,
        fix: Fix::Prep(|_| prepare_api_key_scaffold().map(Some)),
    },
    Rule {
        id: "configure-openclaw-api-key",
        title: "Configure an OpenClaw API key",
        description: "Add the key in Agent Doctor wiring or OpenClaw configuration; \
            secrets are never auto-filled.",
        check: CheckMatch::Is("openclaw.api_key.configured", ProbeStatus::Warn),
        versions: Versions::ANY,
        fix: Fix::Manual,
    },
];

pub fn suggest_openclaw_repairs(probe: &RuntimeProbeReport) -> Vec<SuggestedRepair> {
    let mut items = suggest_rules(OPENCLAW_RULES, probe);
    items.extend(super::npm_cli::suggest_browser_mcp_repairs(
        "openclaw", "OpenClaw", probe,
    ));
    items
}

pub fn apply_openclaw_playbook(probe: &RuntimeProbeReport) -> Result<PlaybookApplyResult> {
    apply_openclaw_playbook_filtered(probe, None)
}

pub fn apply_openclaw_playbook_filtered(
    probe: &RuntimeProbeReport,
    only_ids: Option<&[String]>,
) -> Result<PlaybookApplyResult> {
    let mut result = apply_rules(OPENCLAW_RULES, probe, only_ids);
    let browser = super::npm_cli::apply_browser_mcp_repair("openclaw", probe, only_ids)?;
    result.executed.extend(browser.executed);
    result.skipped.extend(browser.skipped);
    Ok(result)
}

fn openclaw_config_path() -> PathBuf {
    home_join(".openclaw/openclaw.json")
}

fn openclaw_env_path() -> PathBuf {
    home_join(".openclaw/.env")
}

fn company_gateway_url() -> Result<String> {
    read_company_profile()?
        .and_then(|profile| profile.gateway_url)
        .context("no company gateway in profile.env — run agent-doctor setup first")
}

fn create_openclaw_config() -> Result<()> {
    let path = openclaw_config_path();
    if path.exists() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let gateway_url = read_company_profile()
        .ok()
        .flatten()
        .and_then(|profile| profile.gateway_url);
    if let Some(url) = gateway_url {
        crate::setup::merge::apply_openclaw(&url, "", None)?;
        let mut root = load_json_config(&path)?;
        if let Some(obj) = root.as_object_mut() {
            obj.entry("env").or_insert_with(|| json!({ "vars": {} }));
        }
        write_json_config(&path, &root)
    } else {
        write_json_config(
            &path,
            &json!({
                "tools": { "profile": DEFAULT_TOOL_PROFILE },
                "env": { "vars": {} }
            }),
        )
    }
}

fn apply_gateway_from_company_profile() -> Result<()> {
    let url = company_gateway_url()?;
    crate::setup::merge::apply_openclaw(&url, "", None)?;
    Ok(())
}

fn migrate_legacy_gateway_url() -> Result<()> {
    let path = openclaw_config_path();
    let root = load_json_config(&path)?;
    let url =
        crate::adapters::configured_base_url(&root).context("no LLM base URL found to migrate")?;
    crate::setup::merge::apply_openclaw(&url, "", None)?;
    Ok(())
}

fn fix_legacy_timeout_field() -> Result<()> {
    let path = openclaw_config_path();
    let mut root = load_json_config(&path)?;
    let defaults = root
        .pointer_mut("/agents/defaults")
        .and_then(Value::as_object_mut)
        .context("agents.defaults must be an object")?;
    let timeout = defaults.remove("timeout");
    if let Some(value) = timeout {
        defaults
            .entry("timeoutSeconds".to_string())
            .or_insert(value);
    }
    write_json_config(&path, &root)
}

fn fix_env_string_sections() -> Result<()> {
    let path = openclaw_config_path();
    let mut root = load_json_config(&path)?;
    let env = root
        .get_mut("env")
        .and_then(Value::as_object_mut)
        .context("env section must be an object")?;
    let mut changed = false;
    for key in ["vars", "shellEnv"] {
        if let Some(string_value) = env.get(key).and_then(Value::as_str) {
            let parsed: Value = serde_json::from_str(string_value)
                .with_context(|| format!("failed to parse env.{key} JSON string"))?;
            env.insert(key.to_string(), parsed);
            changed = true;
        }
    }
    if !changed {
        anyhow::bail!("no string env sections to fix");
    }
    write_json_config(&path, &root)
}

fn fix_tools_profile() -> Result<()> {
    let path = openclaw_config_path();
    let mut root = load_json_config(&path)?;
    let tools = root
        .pointer_mut("/tools")
        .and_then(Value::as_object_mut)
        .context("tools must be an object")?;
    tools.insert("profile".to_string(), json!(DEFAULT_TOOL_PROFILE));
    write_json_config(&path, &root)
}

fn tighten_env_permissions() -> Result<()> {
    let path = openclaw_env_path();
    if !path.exists() {
        anyhow::bail!(".env does not exist");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let metadata = fs::metadata(&path)?;
        let mode = metadata.permissions().mode() & 0o777;
        if mode & 0o077 == 0 {
            return Ok(());
        }
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
            .with_context(|| format!("failed to chmod 600 {}", path.display()))?;
        Ok(())
    }
    #[cfg(not(unix))]
    anyhow::bail!("permission tightening is only supported on Unix")
}

fn dedupe_openclaw_dotenv(probe: &RuntimeProbeReport) -> Result<()> {
    let env_key = probe
        .checks
        .iter()
        .find(|check| check.id == "openclaw.api_key.duplicates")
        .and_then(|check| {
            OPENCLAW_API_KEY_VARS
                .iter()
                .find_map(|key| check.message.contains(key).then(|| key.to_string()))
        })
        .context("could not determine duplicate API key env var")?;
    let path = openclaw_env_path();
    let raw = fs::read_to_string(&path)?;
    let updated = dedupe_env_key_lines(&raw, &env_key)?;
    if updated == raw {
        return Ok(());
    }
    fs::write(&path, updated)?;
    Ok(())
}

fn prepare_api_key_scaffold() -> Result<PathBuf> {
    let config_path = openclaw_config_path();
    let guide_path = api_key_guide_path()?;

    if config_path.exists() {
        let mut root = load_json_config(&config_path)?;
        if !root.get("env").map(Value::is_object).unwrap_or(false) {
            root.as_object_mut()
                .context("config root must be object")?
                .insert("env".to_string(), json!({}));
        }
        let env = root
            .get_mut("env")
            .and_then(Value::as_object_mut)
            .context("env section must be an object")?;
        let vars = env
            .entry("vars")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .context("env.vars must be an object")?;
        for key in OPENCLAW_API_KEY_VARS {
            vars.entry(key.to_string()).or_insert(json!(""));
        }
        write_json_config(&config_path, &root)?;
    }

    let env_path = openclaw_env_path();
    if let Some(parent) = env_path.parent() {
        fs::create_dir_all(parent)?;
    }
    if !env_path.exists() {
        let scaffold = "# Agent Doctor scaffold — paste API keys after the equals sign.\n\
             # Secrets are never auto-filled.\n\
             OPENAI_API_KEY=\n\
             ANTHROPIC_API_KEY=\n"
            .to_string();
        fs::write(&env_path, scaffold)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&env_path, fs::Permissions::from_mode(0o600))?;
        }
    }

    let guide = "# OpenClaw API key setup\n\n\
        Agent Doctor added placeholders to `~/.openclaw/openclaw.json` (`env.vars`) \
        and/or `~/.openclaw/.env`.\n\n\
        ## Steps\n\n\
        1. Paste keys into `env.vars` in openclaw.json **or** `~/.openclaw/.env`.\n\
        2. Run `agent-doctor repair openclaw` again to verify.\n\n\
        If you use a company gateway, prefer `agent-doctor setup --url ... --key ...`.\n\
        Secrets stay on this machine.\n";
    if let Some(parent) = guide_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&guide_path, guide)?;
    Ok(guide_path)
}

fn api_key_guide_path() -> Result<PathBuf> {
    let root = dirs::config_dir()
        .map(|dir| dir.join("agent-doctor").join("guides"))
        .context("could not resolve config directory")?;
    Ok(root.join("openclaw-api-key.md"))
}

#[cfg(test)]
mod tests {
    use super::super::rule;
    use super::*;
    use crate::probe::{ProbeCheck, ProbeSeverity, ProbeStatus};
    use crate::repair::SensitivityLevel;

    fn sample_probe(checks: Vec<ProbeCheck>) -> RuntimeProbeReport {
        RuntimeProbeReport {
            runtime_id: "openclaw".to_string(),
            display_name: "OpenClaw".to_string(),
            binary_name: "openclaw".to_string(),
            checks,
            facts: vec![],
        }
    }

    #[test]
    fn suggest_install_when_binary_missing() {
        let probe = sample_probe(vec![ProbeCheck::new(
            "binary.exists",
            "Binary on PATH",
            ProbeStatus::Fail,
            ProbeSeverity::Error,
            "missing",
            SensitivityLevel::Public,
        )]);
        let items = suggest_openclaw_repairs(&probe);
        assert!(items.iter().any(|item| item.id == "fix-openclaw-install"));
    }

    #[test]
    fn suggest_schema_fixes() {
        let probe = sample_probe(vec![
            ProbeCheck::new(
                "openclaw.schema.legacy_timeout",
                "timeout",
                ProbeStatus::Warn,
                ProbeSeverity::Warning,
                "legacy",
                SensitivityLevel::Public,
            ),
            ProbeCheck::new(
                "openclaw.schema.tools_profile",
                "profile",
                ProbeStatus::Warn,
                ProbeSeverity::Warning,
                "bad profile",
                SensitivityLevel::Public,
            ),
            ProbeCheck::new(
                "openclaw.schema.legacy_agents_list",
                "agents.list",
                ProbeStatus::Warn,
                ProbeSeverity::Warning,
                "legacy",
                SensitivityLevel::Public,
            ),
        ]);
        let items = suggest_openclaw_repairs(&probe);
        assert!(items
            .iter()
            .any(|item| item.id == "fix-openclaw-legacy-timeout"));
        assert!(items
            .iter()
            .any(|item| item.id == "fix-openclaw-tools-profile"));
        assert!(items
            .iter()
            .any(|item| item.id == "fix-openclaw-legacy-agents-list" && item.auto_fixable));
    }

    #[test]
    fn rules_have_unique_ids_and_desktop_labels() {
        rule::assert_unique_ids(OPENCLAW_RULES);
        rule::assert_desktop_labels(&rule::fix_ids(OPENCLAW_RULES));
    }

    #[test]
    fn tools_profile_rule_names_the_default() {
        let rule = OPENCLAW_RULES
            .iter()
            .find(|rule| rule.id == "fix-openclaw-tools-profile")
            .unwrap();
        assert!(rule.description.contains(DEFAULT_TOOL_PROFILE));
    }

    #[test]
    fn api_key_warning_suggests_manual_step_only() {
        let probe = sample_probe(vec![ProbeCheck::new(
            "openclaw.api_key.configured",
            "OpenClaw API key configured",
            ProbeStatus::Warn,
            ProbeSeverity::Warning,
            "missing",
            SensitivityLevel::Public,
        )]);
        let ids: Vec<_> = suggest_openclaw_repairs(&probe)
            .into_iter()
            .map(|item| (item.id, item.auto_fixable))
            .collect();
        assert_eq!(ids, vec![("configure-openclaw-api-key".to_string(), false)]);
    }

    #[test]
    fn fix_legacy_timeout_migrates_field() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("openclaw.json");
        fs::write(&path, r#"{"agents":{"defaults":{"timeout":120}}}"#).unwrap();
        let mut root = load_json_config(&path).unwrap();
        let defaults = root
            .pointer_mut("/agents/defaults")
            .unwrap()
            .as_object_mut()
            .unwrap();
        let timeout = defaults.remove("timeout");
        defaults.insert("timeoutSeconds".to_string(), timeout.unwrap());
        write_json_config(&path, &root).unwrap();
        let updated = load_json_config(&path).unwrap();
        assert_eq!(
            updated.pointer("/agents/defaults/timeoutSeconds"),
            Some(&json!(120))
        );
        assert!(updated.pointer("/agents/defaults/timeout").is_none());
    }

    #[test]
    fn fix_env_string_parses_json() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("openclaw.json");
        fs::write(&path, r#"{"env":{"vars":"{\"OPENAI_API_KEY\":\"x\"}"}}"#).unwrap();
        let mut root = load_json_config(&path).unwrap();
        let parsed: Value =
            serde_json::from_str(root.pointer("/env/vars").unwrap().as_str().unwrap()).unwrap();
        root.as_object_mut()
            .unwrap()
            .entry("env")
            .or_insert(json!({}))
            .as_object_mut()
            .unwrap()
            .insert("vars".to_string(), parsed);
        write_json_config(&path, &root).unwrap();
        let updated = load_json_config(&path).unwrap();
        assert_eq!(
            updated.pointer("/env/vars/OPENAI_API_KEY"),
            Some(&json!("x"))
        );
    }
}
