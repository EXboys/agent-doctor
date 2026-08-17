use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde_json::{json, Value};

use crate::adapters::util::home_join;
use crate::lifecycle::{run_openclaw_lifecycle, OpenClawLifecycleAction};
use crate::probe::{ProbeStatus, RuntimeProbeReport};
use crate::profile::read_company_profile;
use crate::repair::playbooks::hermes::dedupe_env_key_lines;
use crate::repair::{SkippedRepairAction, SuggestedRepair};

use super::should_run;
use super::PlaybookApplyResult;

const DEFAULT_TOOL_PROFILE: &str = "coding";
const OPENCLAW_API_KEY_VARS: &[&str] = &["OPENAI_API_KEY", "ANTHROPIC_API_KEY"];

pub fn suggest_openclaw_repairs(probe: &RuntimeProbeReport) -> Vec<SuggestedRepair> {
    let mut items = Vec::new();
    let company_gateway = read_company_profile()
        .ok()
        .flatten()
        .and_then(|profile| profile.gateway_url)
        .is_some();

    for check in &probe.checks {
        if check.id == "binary.exists" && check.status == ProbeStatus::Fail {
            items.push(SuggestedRepair {
                id: "fix-openclaw-install".to_string(),
                title: "Install OpenClaw".to_string(),
                description: "Run the official OpenClaw installer (openclaw.ai/install.sh) \
                    with --no-onboard. Requires network access."
                    .to_string(),
                auto_fixable: true,
            });
        }

        if check.id.starts_with("config.exists:") && check.status == ProbeStatus::Warn {
            items.push(SuggestedRepair {
                id: "fix-openclaw-create-config".to_string(),
                title: "Create OpenClaw config".to_string(),
                description: "Create ~/.openclaw/openclaw.json with models.providers from \
                    company profile when available."
                    .to_string(),
                auto_fixable: true,
            });
        }

        if check.id == "gateway.configured" && check.status == ProbeStatus::Warn {
            items.push(SuggestedRepair {
                id: "fix-openclaw-gateway-from-profile".to_string(),
                title: "Apply company gateway to OpenClaw".to_string(),
                description: if company_gateway {
                    "Set models.providers.evotown|personal.baseUrl from profile.env.".to_string()
                } else {
                    "Run `agent-doctor setup --url ... --key ...` first.".to_string()
                },
                auto_fixable: company_gateway,
            });
        }

        if check.id == "openclaw.schema.legacy_gateway_url" && check.status == ProbeStatus::Warn {
            items.push(SuggestedRepair {
                id: "fix-openclaw-legacy-gateway-url".to_string(),
                title: "Migrate OpenClaw LLM URL to models.providers".to_string(),
                description: "Remove invalid gateway.url / evotown.url and write \
                    models.providers.evotown|personal.baseUrl."
                    .to_string(),
                auto_fixable: true,
            });
        }

        if check.id == "openclaw.schema.legacy_timeout" && check.status == ProbeStatus::Warn {
            items.push(SuggestedRepair {
                id: "fix-openclaw-legacy-timeout".to_string(),
                title: "Migrate agent timeout field".to_string(),
                description: "Rename agents.defaults.timeout to timeoutSeconds.".to_string(),
                auto_fixable: true,
            });
        }

        if check.id.starts_with("openclaw.schema.env_string:") && check.status == ProbeStatus::Warn
        {
            items.push(SuggestedRepair {
                id: "fix-openclaw-env-object".to_string(),
                title: "Fix env.vars / env.shellEnv shape".to_string(),
                description: "Parse string env sections into JSON objects.".to_string(),
                auto_fixable: true,
            });
        }

        if check.id == "openclaw.schema.tools_profile" && check.status == ProbeStatus::Warn {
            items.push(SuggestedRepair {
                id: "fix-openclaw-tools-profile".to_string(),
                title: "Reset tools.profile".to_string(),
                description: format!("Set tools.profile to '{DEFAULT_TOOL_PROFILE}'."),
                auto_fixable: true,
            });
        }

        if check.id.starts_with("openclaw.env.permissions:") && check.status == ProbeStatus::Warn {
            items.push(SuggestedRepair {
                id: "fix-openclaw-env-permissions".to_string(),
                title: "Tighten ~/.openclaw/.env permissions".to_string(),
                description: "Set .env to mode 600.".to_string(),
                auto_fixable: cfg!(unix),
            });
        }

        if check.id == "openclaw.api_key.duplicates" && check.status == ProbeStatus::Warn {
            items.push(SuggestedRepair {
                id: "fix-openclaw-api-key-duplicates".to_string(),
                title: "Deduplicate OpenClaw .env API keys".to_string(),
                description: "Keep the last non-empty API key assignment.".to_string(),
                auto_fixable: true,
            });
        }

        if check.id == "openclaw.api_key.configured" && check.status == ProbeStatus::Warn {
            items.push(SuggestedRepair {
                id: "configure-openclaw-api-key".to_string(),
                title: "Configure an OpenClaw API key".to_string(),
                description:
                    "Add the key in Agent Doctor wiring or OpenClaw configuration; secrets are never auto-filled."
                        .to_string(),
                auto_fixable: false,
            });
        }
    }

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
    let mut result = PlaybookApplyResult::default();

    if should_run("fix-openclaw-install", only_ids) && openclaw_needs_install(probe) {
        match run_openclaw_lifecycle(OpenClawLifecycleAction::Install) {
            Ok(()) => result.executed.push("fix-openclaw-install".to_string()),
            Err(error) => result.skipped.push(SkippedRepairAction {
                id: "fix-openclaw-install".to_string(),
                reason: error.to_string(),
            }),
        }
    }

    if should_run("fix-openclaw-create-config", only_ids) && openclaw_config_missing(probe) {
        match create_openclaw_config() {
            Ok(()) => result
                .executed
                .push("fix-openclaw-create-config".to_string()),
            Err(error) => result.skipped.push(SkippedRepairAction {
                id: "fix-openclaw-create-config".to_string(),
                reason: error.to_string(),
            }),
        }
    }

    if should_run("fix-openclaw-gateway-from-profile", only_ids) && openclaw_gateway_missing(probe)
    {
        match apply_gateway_from_company_profile() {
            Ok(()) => result
                .executed
                .push("fix-openclaw-gateway-from-profile".to_string()),
            Err(error) => result.skipped.push(SkippedRepairAction {
                id: "fix-openclaw-gateway-from-profile".to_string(),
                reason: error.to_string(),
            }),
        }
    }

    if should_run("fix-openclaw-legacy-gateway-url", only_ids)
        && probe_has_check(
            probe,
            "openclaw.schema.legacy_gateway_url",
            ProbeStatus::Warn,
        )
    {
        match migrate_legacy_gateway_url() {
            Ok(()) => result
                .executed
                .push("fix-openclaw-legacy-gateway-url".to_string()),
            Err(error) => result.skipped.push(SkippedRepairAction {
                id: "fix-openclaw-legacy-gateway-url".to_string(),
                reason: error.to_string(),
            }),
        }
    }

    for check in &probe.checks {
        if should_run("fix-openclaw-env-permissions", only_ids)
            && check.id.starts_with("openclaw.env.permissions:")
            && check.status == ProbeStatus::Warn
        {
            match tighten_env_permissions() {
                Ok(()) => result
                    .executed
                    .push("fix-openclaw-env-permissions".to_string()),
                Err(error) => result.skipped.push(SkippedRepairAction {
                    id: "fix-openclaw-env-permissions".to_string(),
                    reason: error.to_string(),
                }),
            }
        }

        if should_run("fix-openclaw-api-key-duplicates", only_ids)
            && check.id == "openclaw.api_key.duplicates"
            && check.status == ProbeStatus::Warn
        {
            match dedupe_openclaw_dotenv(probe) {
                Ok(()) => result
                    .executed
                    .push("fix-openclaw-api-key-duplicates".to_string()),
                Err(error) => result.skipped.push(SkippedRepairAction {
                    id: "fix-openclaw-api-key-duplicates".to_string(),
                    reason: error.to_string(),
                }),
            }
        }
    }

    if should_run("fix-openclaw-legacy-timeout", only_ids)
        && probe_has_check(probe, "openclaw.schema.legacy_timeout", ProbeStatus::Warn)
    {
        match fix_legacy_timeout_field() {
            Ok(()) => result
                .executed
                .push("fix-openclaw-legacy-timeout".to_string()),
            Err(error) => result.skipped.push(SkippedRepairAction {
                id: "fix-openclaw-legacy-timeout".to_string(),
                reason: error.to_string(),
            }),
        }
    }

    if should_run("fix-openclaw-env-object", only_ids)
        && probe
            .checks
            .iter()
            .any(|check| check.id.starts_with("openclaw.schema.env_string:"))
    {
        match fix_env_string_sections() {
            Ok(()) => result.executed.push("fix-openclaw-env-object".to_string()),
            Err(error) => result.skipped.push(SkippedRepairAction {
                id: "fix-openclaw-env-object".to_string(),
                reason: error.to_string(),
            }),
        }
    }

    if should_run("fix-openclaw-tools-profile", only_ids)
        && probe_has_check(probe, "openclaw.schema.tools_profile", ProbeStatus::Warn)
    {
        match fix_tools_profile() {
            Ok(()) => result
                .executed
                .push("fix-openclaw-tools-profile".to_string()),
            Err(error) => result.skipped.push(SkippedRepairAction {
                id: "fix-openclaw-tools-profile".to_string(),
                reason: error.to_string(),
            }),
        }
    }

    if should_run("fix-openclaw-api-key-scaffold", only_ids) && needs_api_key_scaffold(probe) {
        match prepare_api_key_scaffold() {
            Ok(guide_path) => {
                result
                    .executed
                    .push("fix-openclaw-api-key-scaffold".to_string());
                result.guide_path = Some(guide_path);
            }
            Err(error) => result.skipped.push(SkippedRepairAction {
                id: "fix-openclaw-api-key-scaffold".to_string(),
                reason: error.to_string(),
            }),
        }
    }

    let browser = super::npm_cli::apply_browser_mcp_repair("openclaw", probe, only_ids)?;
    result.executed.extend(browser.executed);
    result.skipped.extend(browser.skipped);

    Ok(result)
}

fn openclaw_needs_install(probe: &RuntimeProbeReport) -> bool {
    probe
        .checks
        .iter()
        .any(|check| check.id == "binary.exists" && check.status == ProbeStatus::Fail)
}

fn openclaw_config_missing(probe: &RuntimeProbeReport) -> bool {
    probe
        .checks
        .iter()
        .any(|check| check.id.starts_with("config.exists:") && check.status == ProbeStatus::Warn)
}

fn openclaw_gateway_missing(probe: &RuntimeProbeReport) -> bool {
    probe
        .checks
        .iter()
        .any(|check| check.id == "gateway.configured" && check.status == ProbeStatus::Warn)
}

fn needs_api_key_scaffold(probe: &RuntimeProbeReport) -> bool {
    probe
        .checks
        .iter()
        .any(|check| check.id == "openclaw.api_key.configured" && check.status == ProbeStatus::Warn)
}

fn probe_has_check(probe: &RuntimeProbeReport, id: &str, status: ProbeStatus) -> bool {
    probe
        .checks
        .iter()
        .any(|check| check.id == id && check.status == status)
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

fn load_json_config(path: &Path) -> Result<Value> {
    if path.exists() {
        let raw = fs::read_to_string(path)?;
        serde_json::from_str(&raw).with_context(|| format!("failed to parse {}", path.display()))
    } else {
        Ok(json!({}))
    }
}

fn write_json_config(path: &Path, root: &Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    if path.exists() {
        backup_config(path)?;
    }
    fs::write(path, serde_json::to_string_pretty(root)?)
        .with_context(|| format!("failed to write {}", path.display()))
}

fn backup_config(path: &Path) -> Result<()> {
    let original = fs::read_to_string(path)?;
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let backup_path = path.with_extension(format!("json.bak.{ts}"));
    fs::write(&backup_path, original)?;
    Ok(())
}

#[cfg(test)]
mod tests {
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
        ]);
        let items = suggest_openclaw_repairs(&probe);
        assert!(items
            .iter()
            .any(|item| item.id == "fix-openclaw-legacy-timeout"));
        assert!(items
            .iter()
            .any(|item| item.id == "fix-openclaw-tools-profile"));
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
