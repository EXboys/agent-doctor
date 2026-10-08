use std::fs;

use anyhow::{Context, Result as AnyhowResult};

use crate::adapters::util::home_join;
use crate::setup::{backup_file, ensure_parent, RuntimeSetupResult};

use super::*;

pub fn apply_codex(
    gateway_url: &str,
    _api_key: &str,
    model: Option<&str>,
) -> AnyhowResult<RuntimeSetupResult> {
    apply_codex_slot(gateway_url, _api_key, model, None)
}

/// Additive Codex wiring: upsert `model_providers.{company|personal}`, point
/// `model_provider` at the active slot, leave the other slot intact.
///
/// Prefer the active workspace isolated `CODEX_HOME` when present. Writing
/// provider keys into `~/.codex/config.toml` while Ask cwd is `$HOME` (default
/// workspace) makes newer Codex treat that file as project-local and warn on
/// every turn — keep those keys only in the isolated home in that case.
pub fn apply_codex_slot(
    gateway_url: &str,
    _api_key: &str,
    model: Option<&str>,
    provider_slot: Option<&str>,
) -> AnyhowResult<RuntimeSetupResult> {
    let model_id = model
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .unwrap_or(COMPANY_DEFAULT_MODEL);
    let slot = provider_slot
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| infer_codex_hermes_slot(gateway_url).to_string());

    let global_path = home_join(".codex/config.toml");
    let global_codex_home = home_join(".codex");
    let isolated_config = crate::workspace::load_workspaces().ok().and_then(|doc| {
        let active = doc.active.as_deref()?;
        let entry = doc.workspaces.get(active)?;
        if !entry.codex_home.exists() {
            return None;
        }
        if crate::workspace::path::paths_equal(&entry.codex_home, &global_codex_home) {
            return None;
        }
        Some(entry.codex_home.join("config.toml"))
    });

    let path = isolated_config
        .clone()
        .unwrap_or_else(|| global_path.clone());
    let backup_path = backup_file(&path)?;
    ensure_parent(&path)?;
    write_codex_provider_config(&path, gateway_url, model_id, &slot)?;

    if isolated_config.is_some() {
        // Drop denylist keys from ~/.codex so home-as-cwd Ask sessions do not
        // rediscover them as unsupported project-local config.
        let _ = strip_codex_project_denied_provider_keys(&global_path);
    }

    clear_codex_placeholder_auth()?;

    let env_key = codex_slot_env_key(&slot);
    Ok(RuntimeSetupResult {
        runtime_id: "codex".to_string(),
        display_name: "Codex".to_string(),
        applied: true,
        config_path: Some(path.display().to_string()),
        backup_path: backup_path.map(|p| p.display().to_string()),
        message: format!(
            "set model_provider={slot} + openai_base_url (wire_api=responses, env_key={env_key}, model={model_id})"
        ),
        ..Default::default()
    })
}

/// Keys Codex refuses in project-local `.codex/config.toml` (credential routing).
const CODEX_PROJECT_DENIED_PROVIDER_KEYS: &[&str] = &[
    "openai_base_url",
    "chatgpt_base_url",
    "model_provider",
    "model_providers",
];

/// Remove provider/auth routing keys from a Codex `config.toml`.
///
/// Used when `~/.codex` would be loaded as project-local (cwd under `$HOME`)
/// while the real user layer is an isolated `CODEX_HOME`.
pub fn strip_codex_project_denied_provider_keys(path: &std::path::Path) -> AnyhowResult<bool> {
    if !path.exists() {
        return Ok(false);
    }
    let raw = fs::read_to_string(path)?;
    let mut doc = raw
        .parse::<toml_edit::DocumentMut>()
        .unwrap_or_else(|_| toml_edit::DocumentMut::new());
    let mut changed = false;
    for key in CODEX_PROJECT_DENIED_PROVIDER_KEYS {
        if doc.remove(key).is_some() {
            changed = true;
        }
    }
    if changed {
        fs::write(path, doc.to_string())?;
    }
    Ok(changed)
}

pub(crate) fn codex_slot_display_name(slot: &str) -> &'static str {
    if slot == CODEX_TEAM_SLOT {
        "Company Gateway"
    } else {
        "Personal Provider"
    }
}

pub(crate) fn codex_slot_env_key(slot: &str) -> &'static str {
    if slot == CODEX_TEAM_SLOT {
        // Prefer Evotown key so personal DeepSeek OPENAI_API_KEY does not shadow team.
        "EVOTOWN_API_KEY"
    } else {
        "OPENAI_API_KEY"
    }
}

/// Codex ≥0.84 uses OpenAI Responses (`wire_api=responses`). Zhipu GLM exposes that
/// on `/api/v1`, not on `/api/paas/v4` (chat completions only).
pub fn codex_responses_gateway_url(gateway_url: &str) -> String {
    let trimmed = gateway_url.trim().trim_end_matches('/');
    let lower = trimmed.to_ascii_lowercase();
    if lower.contains("open.bigmodel.cn")
        && (lower.contains("paas/v4") || lower.contains("/api/paas/v4"))
    {
        return "https://open.bigmodel.cn/api/v1".into();
    }
    if lower.contains("api.z.ai") && lower.contains("paas/v4") {
        return "https://api.z.ai/api/v1".into();
    }
    trimmed.to_string()
}

pub(crate) fn write_codex_provider_config(
    path: &std::path::Path,
    gateway_url: &str,
    model_id: &str,
    slot: &str,
) -> AnyhowResult<()> {
    ensure_parent(path)?;
    let gateway_url = codex_responses_gateway_url(gateway_url);

    let mut doc = if path.exists() {
        let raw = fs::read_to_string(path)?;
        raw.parse::<toml_edit::DocumentMut>()
            .unwrap_or_else(|_| toml_edit::DocumentMut::new())
    } else {
        toml_edit::DocumentMut::new()
    };

    doc["model"] = toml_edit::value(model_id);
    doc["model_provider"] = toml_edit::value(slot);
    // Codex 0.14x still falls back to the built-in `openai` provider (api.openai.com)
    // unless this top-level override is set — custom model_providers alone is not enough.
    doc["openai_base_url"] = toml_edit::value(gateway_url.as_str());

    let display = codex_slot_display_name(slot);

    let providers =
        doc["model_providers"].or_insert(toml_edit::Item::Table(toml_edit::Table::new()));
    let providers_table = providers
        .as_table_mut()
        .context("Codex model_providers must be a table")?;
    providers_table.set_implicit(true);

    let entry = providers_table
        .entry(slot)
        .or_insert(toml_edit::Item::Table(toml_edit::Table::new()));
    let entry_table = entry
        .as_table_mut()
        .with_context(|| format!("model_providers.{slot} must be a table"))?;

    entry_table["name"] = toml_edit::value(display);
    entry_table["base_url"] = toml_edit::value(gateway_url);
    entry_table["env_key"] = toml_edit::value(codex_slot_env_key(slot));
    entry_table["requires_openai_auth"] = toml_edit::value(false);
    // OpenAI Codex CLI (≥0.84) only accepts Responses wire API.
    entry_table["wire_api"] = toml_edit::value("responses");
    entry_table["supports_websockets"] = toml_edit::value(false);

    fs::write(path, doc.to_string())?;
    Ok(())
}

pub(crate) fn infer_codex_hermes_slot(gateway_url: &str) -> &'static str {
    // Same heuristics as OpenClaw team detection.
    if infer_openclaw_slot(gateway_url) == OPENCLAW_TEAM_SLOT {
        CODEX_TEAM_SLOT
    } else {
        CODEX_PERSONAL_SLOT
    }
}

/// Remove Agent Doctor placeholder / empty apikey auth.json so Codex uses env_key auth.
pub fn clear_codex_placeholder_auth() -> AnyhowResult<()> {
    let path = home_join(".codex/auth.json");
    if !path.exists() {
        return Ok(());
    }
    let raw = fs::read_to_string(&path)?;
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return Ok(());
    };
    let is_placeholder = value
        .get("placeholder")
        .and_then(serde_json::Value::as_bool)
        == Some(true);
    let is_empty_apikey = value.get("auth_mode").and_then(serde_json::Value::as_str)
        == Some("apikey")
        && value.get("OPENAI_API_KEY").is_none()
        && value.get("api_key").is_none()
        && value.get("tokens").is_none();
    if is_placeholder || is_empty_apikey {
        fs::remove_file(&path)?;
    }
    Ok(())
}

/// Drop ChatGPT-login auth.json before gateway launches.
///
/// Stored ChatGPT tokens make Codex talk to api.openai.com even when
/// OPENAI_BASE_URL / openai_base_url point at the company gateway.
pub fn clear_codex_chatgpt_auth_for_gateway() -> AnyhowResult<()> {
    let path = home_join(".codex/auth.json");
    if !path.exists() {
        return Ok(());
    }
    let raw = fs::read_to_string(&path)?;
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return Ok(());
    };
    let mode = value
        .get("auth_mode")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let has_chatgpt_tokens = value.get("tokens").is_some()
        || mode.eq_ignore_ascii_case("chatgpt")
        || value.get("refresh_token").is_some()
        || value.get("access_token").is_some();
    if !has_chatgpt_tokens {
        return Ok(());
    }
    let _ = crate::setup::backup_file(&path);
    fs::remove_file(&path)?;
    Ok(())
}

#[cfg(test)]
mod codex_responses_tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn strips_project_denied_provider_keys() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(
            &path,
            r#"
model = "keep-me"
model_provider = "company"
openai_base_url = "https://gateway.example/v1"
approval_policy = "on-request"

[model_providers.company]
base_url = "https://gateway.example/v1"
"#,
        )
        .unwrap();

        assert!(strip_codex_project_denied_provider_keys(&path).unwrap());
        let rendered = fs::read_to_string(&path).unwrap();
        assert!(rendered.contains("model = \"keep-me\""));
        assert!(rendered.contains("approval_policy"));
        assert!(!rendered.contains("model_provider"));
        assert!(!rendered.contains("openai_base_url"));
        assert!(!rendered.contains("model_providers"));
    }

    #[test]
    fn glm_chat_base_url_maps_to_responses_api_v1() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        write_codex_provider_config(
            &path,
            "https://open.bigmodel.cn/api/paas/v4",
            "glm-5.3",
            CODEX_PERSONAL_SLOT,
        )
        .unwrap();
        let rendered = fs::read_to_string(&path).unwrap();
        assert!(rendered.contains("https://open.bigmodel.cn/api/v1"));
        assert!(!rendered.contains("api/paas/v4"));
    }

    #[test]
    fn personal_slot_is_wired_for_any_openai_compatible_host() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        write_codex_provider_config(
            &path,
            "https://api.deepseek.com/v1",
            "deepseek-v4-flash",
            CODEX_PERSONAL_SLOT,
        )
        .unwrap();

        let rendered = fs::read_to_string(&path).unwrap();
        assert!(rendered.contains("model_provider = \"personal\""));
        assert!(rendered.contains("openai_base_url = \"https://api.deepseek.com/v1\""));
        assert!(rendered.contains("base_url = \"https://api.deepseek.com/v1\""));
        assert!(rendered.contains("model = \"deepseek-v4-flash\""));
    }

    #[test]
    fn write_codex_provider_preserves_comments() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(
            &path,
            r#"# top comment
model = "keep-me-later"

# unrelated section
[features]
# feature comment
rich_ui = true
"#,
        )
        .unwrap();

        write_codex_provider_config(
            &path,
            "https://example.com/v1",
            "gpt-test",
            CODEX_PERSONAL_SLOT,
        )
        .unwrap();

        let rendered = fs::read_to_string(&path).unwrap();
        assert!(rendered.contains("# top comment"));
        assert!(rendered.contains("# feature comment"));
        assert!(rendered.contains("rich_ui = true"));
        assert!(rendered.contains("model = \"gpt-test\""));
        assert!(rendered.contains("model_provider = \"personal\""));
        assert!(rendered.contains("openai_base_url = \"https://example.com/v1\""));
        assert!(rendered.contains("[model_providers.personal]"));
        assert!(rendered.contains("base_url = \"https://example.com/v1\""));
    }

    #[test]
    fn drops_every_tool_whose_program_file_is_missing() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(
            &path,
            r#"
model = "keep-me"

[mcp_servers.node_repl]
command = "/Applications/ChatGPT.app/Contents/Resources/cua_node/bin/node_repl"
args = []

[mcp_servers.old_browser]
command = "/no/such/agent-doctor"

[mcp_servers.browser]
command = "agent-doctor"
args = ["mcp", "browser"]

[mcp_servers.remote]
url = "https://example.com/mcp"
"#,
        )
        .unwrap();

        assert!(drop_unreachable_codex_mcp_servers(&path).unwrap());
        let rendered = fs::read_to_string(&path).unwrap();
        assert!(!rendered.contains("node_repl"));
        assert!(!rendered.contains("old_browser"));
        assert!(rendered.contains("[mcp_servers.browser]"));
        assert!(rendered.contains("[mcp_servers.remote]"));
        assert!(rendered.contains("model = \"keep-me\""));
        assert!(!drop_unreachable_codex_mcp_servers(&path).unwrap());
    }

    #[test]
    fn keeps_node_repl_when_its_program_exists() {
        let dir = tempdir().unwrap();
        let program = dir.path().join("node_repl");
        fs::write(&program, "").unwrap();
        let path = dir.path().join("config.toml");
        fs::write(
            &path,
            format!(
                "[mcp_servers.node_repl]\ncommand = \"{}\"\n",
                program.display()
            ),
        )
        .unwrap();

        assert!(!drop_unreachable_codex_mcp_servers(&path).unwrap());
        let rendered = fs::read_to_string(&path).unwrap();
        assert!(rendered.contains("node_repl"));
    }

    #[test]
    fn drops_missing_tools_from_claude_openclaw_hermes_and_dsh() {
        let dir = tempdir().unwrap();

        let claude = dir.path().join(".mcp.json");
        fs::write(
            &claude,
            r#"{"mcpServers":{"gone":{"command":"/no/such/claude-tool"},"browser":{"command":"agent-doctor","args":["mcp","browser"]}}}"#,
        )
        .unwrap();
        assert!(drop_unreachable_json_mcp_servers(&claude).unwrap());
        let claude_text = fs::read_to_string(&claude).unwrap();
        assert!(!claude_text.contains("gone"));
        assert!(claude_text.contains("browser"));

        let openclaw = dir.path().join("openclaw.json");
        fs::write(
            &openclaw,
            r#"{"mcp":{"servers":{"gone":{"command":"/no/such/openclaw-tool"},"browser":{"command":"agent-doctor"}}}}"#,
        )
        .unwrap();
        assert!(drop_unreachable_json_mcp_servers(&openclaw).unwrap());
        let openclaw_text = fs::read_to_string(&openclaw).unwrap();
        assert!(!openclaw_text.contains("gone"));
        assert!(openclaw_text.contains("browser"));

        let hermes = dir.path().join("config.yaml");
        fs::write(
            &hermes,
            "mcp_servers:\n  gone:\n    command: /no/such/hermes-tool\n  browser:\n    command: agent-doctor\n",
        )
        .unwrap();
        assert!(drop_unreachable_yaml_mcp_servers(&hermes).unwrap());
        let hermes_text = fs::read_to_string(&hermes).unwrap();
        assert!(!hermes_text.contains("gone"));
        assert!(hermes_text.contains("browser"));

        let dsh = dir.path().join("cordis.patch.yml");
        fs::write(
            &dsh,
            "plugins:\n  - id: mcp-browser\n    name: '@deepseek-ai/dsh-mcp-client'\n    config:\n      command: /no/such/dsh-tool\n  - id: keep-me\n    name: other-plugin\n    config:\n      command: /no/such/not-a-tool\n",
        )
        .unwrap();
        assert!(drop_unreachable_dsh_mcp_plugins(&dsh).unwrap());
        let dsh_text = fs::read_to_string(&dsh).unwrap();
        assert!(!dsh_text.contains("dsh-mcp-client"));
        assert!(dsh_text.contains("keep-me"));
    }

    #[test]
    fn clears_every_assistant_config_before_a_message() {
        let dir = tempdir().unwrap();
        let home = dir.path().join("home");
        let cwd = dir.path().join("project");
        let codex_home = dir.path().join("codex-home");
        fs::create_dir_all(home.join(".codex")).unwrap();
        fs::create_dir_all(home.join(".claude")).unwrap();
        fs::create_dir_all(home.join(".hermes")).unwrap();
        fs::create_dir_all(home.join(".openclaw")).unwrap();
        fs::create_dir_all(home.join(".dsh")).unwrap();
        fs::create_dir_all(cwd.join(".codex")).unwrap();
        fs::create_dir_all(&codex_home).unwrap();

        let missing = "command: /no/such/tool\n";
        fs::write(
            home.join(".codex/config.toml"),
            "[mcp_servers.gone]\ncommand = \"/no/such/codex-tool\"\n[mcp_servers.keep]\ncommand = \"agent-doctor\"\n",
        )
        .unwrap();
        fs::write(
            cwd.join(".codex/config.toml"),
            "[mcp_servers.local_gone]\ncommand = \"bin/missing\"\n",
        )
        .unwrap();
        fs::write(
            cwd.join(".mcp.json"),
            r#"{"mcpServers":{"gone":{"command":"/no/such/project-tool"},"keep":{"command":"agent-doctor"}}}"#,
        )
        .unwrap();
        fs::write(
            home.join(".claude.json"),
            r#"{"mcpServers":{"gone":{"command":"/no/such/claude-tool"},"keep":{"command":"claude"}}}"#,
        )
        .unwrap();
        fs::write(
            home.join(".claude/settings.json"),
            r#"{"mcpServers":{"gone":{"command":"/no/such/settings-tool"}}}"#,
        )
        .unwrap();
        fs::write(
            home.join(".hermes/config.yaml"),
            format!("mcp_servers:\n  gone:\n    {missing}  keep:\n    command: agent-doctor\n"),
        )
        .unwrap();
        fs::write(
            home.join(".openclaw/openclaw.json"),
            r#"{"mcp":{"servers":{"gone":{"command":"/no/such/openclaw-tool"},"keep":{"command":"agent-doctor"}}},"mcpServers":{"legacy":{"command":"/no/such/legacy-tool"}}}"#,
        )
        .unwrap();
        fs::write(
            home.join(".dsh/cordis.patch.yml"),
            "- id: mcp-browser\n  name: '@deepseek-ai/dsh-mcp-client'\n  config:\n    command: /no/such/dsh-tool\n- id: keep-me\n  name: other-plugin\n",
        )
        .unwrap();
        fs::write(
            codex_home.join("config.toml"),
            "[mcp_servers.gone]\ncommand = \"/no/such/isolated-tool\"\n[mcp_servers.keep]\ncommand = \"agent-doctor\"\n",
        )
        .unwrap();
        let profile = home.join(".hermes/profiles/work/config.yaml");
        fs::create_dir_all(profile.parent().unwrap()).unwrap();
        fs::write(
            &profile,
            "mcp_servers:\n  gone:\n    command: /no/such/profile-tool\n",
        )
        .unwrap();

        drop_unreachable_ask_tools_in(&AskToolRoots {
            home: home.clone(),
            cwd: cwd.clone(),
            codex_homes: vec![codex_home.clone()],
            hermes_configs: vec![profile.clone()],
            dsh_patches: Vec::new(),
            extra_mcp_json: Vec::new(),
        });

        let codex = fs::read_to_string(home.join(".codex/config.toml")).unwrap();
        assert!(!codex.contains("gone"));
        assert!(codex.contains("keep"));
        let project_codex = fs::read_to_string(cwd.join(".codex/config.toml")).unwrap();
        assert!(!project_codex.contains("local_gone"));
        assert!(!project_codex.contains("bin/missing"));
        let project_mcp = fs::read_to_string(cwd.join(".mcp.json")).unwrap();
        assert!(!project_mcp.contains("gone"));
        assert!(project_mcp.contains("keep"));
        let claude = fs::read_to_string(home.join(".claude.json")).unwrap();
        assert!(!claude.contains("/no/such/claude-tool"));
        assert!(claude.contains("keep"));
        let settings = fs::read_to_string(home.join(".claude/settings.json")).unwrap();
        assert!(!settings.contains("gone"));
        let hermes = fs::read_to_string(home.join(".hermes/config.yaml")).unwrap();
        assert!(!hermes.contains("/no/such/tool"));
        assert!(hermes.contains("keep"));
        let openclaw = fs::read_to_string(home.join(".openclaw/openclaw.json")).unwrap();
        assert!(!openclaw.contains("/no/such/openclaw-tool"));
        assert!(!openclaw.contains("legacy"));
        assert!(openclaw.contains("keep"));
        let dsh = fs::read_to_string(home.join(".dsh/cordis.patch.yml")).unwrap();
        assert!(!dsh.contains("dsh-mcp-client"));
        assert!(dsh.contains("keep-me"));
        let isolated = fs::read_to_string(codex_home.join("config.toml")).unwrap();
        assert!(!isolated.contains("gone"));
        assert!(isolated.contains("keep"));
        let profile_text = fs::read_to_string(&profile).unwrap();
        assert!(!profile_text.contains("gone"));
    }
}
