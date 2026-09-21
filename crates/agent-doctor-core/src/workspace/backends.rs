use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use serde_json::{json, Value as JsonValue};
use serde_yaml::{Mapping, Value as YamlValue};

use crate::adapters::util::{find_binary, home_join};
use crate::lifecycle::ShellCapture;

use super::path::paths_equal;

#[derive(Debug, Clone, serde::Serialize)]
pub struct RuntimeBindReport {
    pub runtime_id: &'static str,
    pub action: String,
    pub detail: String,
    pub isolation_tier: &'static str,
}

pub fn bind_hermes(profile: &str, project_path: &Path) -> Result<RuntimeBindReport> {
    let profile_dir = home_join(".hermes/profiles").join(profile);
    if !profile_dir.exists() {
        if find_binary("hermes").is_some() {
            let create = run_hermes(&["profile", "create", profile])?;
            if !create.success {
                fs::create_dir_all(&profile_dir)
                    .with_context(|| format!("create {}", profile_dir.display()))?;
            }
        } else {
            fs::create_dir_all(&profile_dir)
                .with_context(|| format!("create {}", profile_dir.display()))?;
        }
    }

    write_hermes_terminal_cwd(&profile_dir, project_path)?;
    seed_hermes_profile_credentials(&profile_dir)?;
    activate_hermes_profile(profile)?;

    Ok(RuntimeBindReport {
        runtime_id: "hermes",
        action: "bind profile".to_string(),
        detail: format!(
            "profile={profile} home={} cwd={}",
            profile_dir.display(),
            project_path.display()
        ),
        isolation_tier: "L3 (profile memory/sessions/skills)",
    })
}

pub fn bind_claude_code(project_path: &Path) -> Result<RuntimeBindReport> {
    let claude_dir = project_path.join(".claude");
    fs::create_dir_all(&claude_dir).with_context(|| format!("create {}", claude_dir.display()))?;

    let settings = claude_dir.join("settings.json");
    if !settings.exists() {
        fs::write(&settings, "{}\n").with_context(|| format!("create {}", settings.display()))?;
    }

    Ok(RuntimeBindReport {
        runtime_id: "claude-code",
        action: "ensure project scope".to_string(),
        detail: format!(
            "project={} claude_dir={} (memory under ~/.claude/projects/<hash>/)",
            project_path.display(),
            claude_dir.display()
        ),
        isolation_tier: "L3 (project hash memory)",
    })
}

pub fn bind_codex(codex_home: &Path) -> Result<RuntimeBindReport> {
    bind_codex_for_project(codex_home, None)
}

/// Isolate `CODEX_HOME` and mark the workspace project (and user home) as trusted so
/// Codex stops warning about disabled project-local config/hooks for 小白 installs.
pub fn bind_codex_for_project(
    codex_home: &Path,
    project_path: Option<&Path>,
) -> Result<RuntimeBindReport> {
    fs::create_dir_all(codex_home).with_context(|| format!("create {}", codex_home.display()))?;

    let default_home = home_join(".codex");
    let default_config = default_home.join("config.toml");
    let target_config = codex_home.join("config.toml");
    if default_config.exists() && !target_config.exists() {
        fs::copy(&default_config, &target_config).with_context(|| {
            format!(
                "seed {} from {}",
                target_config.display(),
                default_config.display()
            )
        })?;
    }

    let default_auth = default_home.join("auth.json");
    let target_auth = codex_home.join("auth.json");
    if default_auth.exists() && !target_auth.exists() {
        fs::copy(&default_auth, &target_auth).with_context(|| {
            format!(
                "seed {} from {}",
                target_auth.display(),
                default_auth.display()
            )
        })?;
    }

    fs::create_dir_all(codex_home.join("memories")).ok();
    fs::write(
        codex_home.join(".agent-doctor-codex-home"),
        "# Agent Doctor isolated CODEX_HOME — do not symlink to ~/.codex\n",
    )
    .ok();

    let mut trust_paths = Vec::new();
    if let Some(path) = project_path {
        trust_paths.push(path.to_path_buf());
    }
    if let Some(home) = dirs::home_dir() {
        trust_paths.push(home);
    }
    let _ = ensure_codex_projects_trusted(codex_home, &trust_paths);

    Ok(RuntimeBindReport {
        runtime_id: "codex",
        action: "isolate CODEX_HOME".to_string(),
        detail: format!(
            "CODEX_HOME={} (overlay; memories under memories/)",
            codex_home.display()
        ),
        isolation_tier: "L2 (CODEX_HOME overlay — not native per-repo memory)",
    })
}

/// Write `[projects."<path>"].trust_level = "trusted"` for each path into Codex config.
///
/// Codex disables project-local config/hooks until the cwd (often the default home
/// workspace) is trusted — Agent Doctor should do this automatically for end users.
pub fn ensure_codex_projects_trusted(codex_home: &Path, project_paths: &[PathBuf]) -> Result<()> {
    if project_paths.is_empty() {
        return Ok(());
    }
    fs::create_dir_all(codex_home).with_context(|| format!("create {}", codex_home.display()))?;
    let config_path = codex_home.join("config.toml");
    let mut doc = if config_path.exists() {
        let raw = fs::read_to_string(&config_path)
            .with_context(|| format!("read {}", config_path.display()))?;
        raw.parse::<toml_edit::DocumentMut>()
            .unwrap_or_else(|_| toml_edit::DocumentMut::new())
    } else {
        toml_edit::DocumentMut::new()
    };

    let projects = doc
        .entry("projects")
        .or_insert(toml_edit::Item::Table(toml_edit::Table::new()));
    let Some(projects_table) = projects.as_table_mut() else {
        bail!("codex config `projects` is not a table");
    };
    projects_table.set_implicit(true);

    let mut changed = false;
    for path in project_paths {
        if path.as_os_str().is_empty() {
            continue;
        }
        let key = codex_project_trust_key(path);
        if key.is_empty() {
            continue;
        }
        let entry = projects_table
            .entry(&key)
            .or_insert(toml_edit::Item::Table(toml_edit::Table::new()));
        let Some(table) = entry.as_table_mut() else {
            continue;
        };
        let already = table
            .get("trust_level")
            .and_then(|v| v.as_str())
            .map(|s| s.eq_ignore_ascii_case("trusted"))
            .unwrap_or(false);
        if already {
            continue;
        }
        table["trust_level"] = toml_edit::value("trusted");
        changed = true;
    }

    if changed || !config_path.exists() {
        fs::write(&config_path, doc.to_string())
            .with_context(|| format!("write {}", config_path.display()))?;
    }
    Ok(())
}

/// Normalize a path into the lookup key Codex uses for `[projects.<key>]`.
pub fn codex_project_trust_key(path: &Path) -> String {
    let resolved = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let mut s = resolved.to_string_lossy().replace('/', "\\");
    if let Some(stripped) = s.strip_prefix(r"\\?\") {
        s = stripped.to_string();
    }
    s.trim_end_matches(['\\', '/']).to_ascii_lowercase()
}

pub fn bind_openclaw(agent_id: &str, workspace_path: &Path) -> Result<RuntimeBindReport> {
    fs::create_dir_all(workspace_path)
        .with_context(|| format!("create {}", workspace_path.display()))?;

    seed_openclaw_workspace_files(workspace_path)?;
    upsert_openclaw_agent(agent_id, workspace_path)?;
    ensure_openclaw_agent_via_cli(agent_id, workspace_path)?;

    Ok(RuntimeBindReport {
        runtime_id: "openclaw",
        action: "bind agent workspace + default routing".to_string(),
        detail: format!(
            "agent_id={agent_id} workspace={} default=true agents.defaults.workspace set",
            workspace_path.display()
        ),
        isolation_tier: "L2 (agent workspace + default routing)",
    })
}

#[derive(Debug, Clone, Default)]
pub struct ClaudeMcpSummary {
    pub user_scope_servers: usize,
    pub claude_json_servers: usize,
    pub claude_json_project_servers: usize,
    pub project_mcp_file_servers: usize,
}

pub fn claude_mcp_summary_for_project(project_path: &Path) -> ClaudeMcpSummary {
    let mut summary = claude_global_mcp_summary();

    let project_mcp = project_path.join(".mcp.json");
    if project_mcp.exists() {
        if let Ok(raw) = fs::read_to_string(&project_mcp) {
            if let Ok(value) = serde_json::from_str::<JsonValue>(&raw) {
                summary.project_mcp_file_servers = value
                    .get("mcpServers")
                    .and_then(JsonValue::as_object)
                    .map(|map| map.len())
                    .unwrap_or(0);
            }
        }
    }

    let claude_json = home_join(".claude.json");
    if !claude_json.exists() {
        return summary;
    }
    let Ok(raw) = fs::read_to_string(&claude_json) else {
        return summary;
    };
    let Ok(value) = serde_json::from_str::<JsonValue>(&raw) else {
        return summary;
    };

    if let Some(projects) = value.get("projects").and_then(JsonValue::as_object) {
        for (key, project) in projects {
            let key_path = PathBuf::from(key);
            if paths_equal(&key_path, project_path) || project_path.starts_with(&key_path) {
                summary.claude_json_project_servers = project
                    .get("mcpServers")
                    .and_then(JsonValue::as_object)
                    .map(|map| map.len())
                    .unwrap_or(0);
                break;
            }
        }
    }

    summary
}

pub fn claude_global_mcp_summary() -> ClaudeMcpSummary {
    let mut summary = ClaudeMcpSummary::default();
    let settings = home_join(".claude/settings.json");
    if settings.exists() {
        if let Ok(raw) = fs::read_to_string(&settings) {
            if let Ok(value) = serde_json::from_str::<JsonValue>(&raw) {
                summary.user_scope_servers = value
                    .get("mcpServers")
                    .and_then(JsonValue::as_object)
                    .map(|map| map.len())
                    .unwrap_or(0);
            }
        }
    }

    let claude_json = home_join(".claude.json");
    if claude_json.exists() {
        if let Ok(raw) = fs::read_to_string(&claude_json) {
            if let Ok(value) = serde_json::from_str::<JsonValue>(&raw) {
                summary.claude_json_servers = value
                    .get("mcpServers")
                    .and_then(JsonValue::as_object)
                    .map(|map| map.len())
                    .unwrap_or(0);
            }
        }
    }

    summary
}

pub fn hermes_gateway_profiles() -> Vec<String> {
    let profiles_root = home_join(".hermes/profiles");
    let Ok(entries) = fs::read_dir(&profiles_root) else {
        return Vec::new();
    };

    let mut profiles = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if path.join("gateway.lock").exists() {
            if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
                profiles.push(name.to_string());
            }
        }
    }
    profiles.sort();
    profiles
}

pub fn hermes_active_profile() -> Option<String> {
    if let Some(profile) = read_hermes_sticky_profile_file() {
        return Some(profile);
    }
    if find_binary("hermes").is_none() {
        return infer_hermes_profile_from_home_env();
    }
    let capture = run_hermes(&["profile", "list"]).ok()?;
    if !capture.success {
        return infer_hermes_profile_from_home_env();
    }
    parse_hermes_profile_list(&capture.stdout)
        .or_else(|| parse_hermes_profile_list(&capture.stderr))
        .or_else(read_hermes_sticky_profile_file)
}

fn read_hermes_sticky_profile_file() -> Option<String> {
    for relative in [".hermes/active_profile", ".hermes/profile"] {
        let path = home_join(relative);
        if !path.exists() {
            continue;
        }
        let raw = fs::read_to_string(&path).ok()?;
        let name = raw.lines().next()?.trim();
        if !name.is_empty() {
            return Some(name.to_string());
        }
    }
    None
}

pub fn hermes_profile_cwd(profile: &str) -> Option<PathBuf> {
    let config = home_join(".hermes/profiles")
        .join(profile)
        .join("config.yaml");
    if !config.exists() {
        return None;
    }
    let raw = fs::read_to_string(&config).ok()?;
    let value: Mapping = serde_yaml::from_str(&raw).ok()?;
    value
        .get("terminal")
        .and_then(|terminal| terminal.get("cwd"))
        .and_then(|cwd| cwd.as_str())
        .map(PathBuf::from)
}

pub fn codex_home_from_env() -> PathBuf {
    std::env::var("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home_join(".codex"))
}

fn read_openclaw_config() -> Option<JsonValue> {
    let config_path = home_join(".openclaw/openclaw.json");
    if !config_path.exists() {
        return None;
    }
    let raw = fs::read_to_string(&config_path).ok()?;
    serde_json::from_str(&raw).ok()
}

fn agent_record<'a>(value: &'a JsonValue, agent_id: &str) -> Option<&'a JsonValue> {
    if let Some(entry) = value.pointer(&format!("/agents/entries/{agent_id}")) {
        return Some(entry);
    }
    value
        .pointer("/agents/list")
        .and_then(JsonValue::as_array)
        .and_then(|agents| {
            agents
                .iter()
                .find(|agent| agent.get("id").and_then(JsonValue::as_str) == Some(agent_id))
        })
}

pub fn openclaw_agent_workspace(agent_id: &str) -> Option<PathBuf> {
    let value = read_openclaw_config()?;
    agent_record(&value, agent_id)
        .and_then(|agent| agent.get("workspace"))
        .and_then(JsonValue::as_str)
        .map(PathBuf::from)
}

fn activate_hermes_profile(profile: &str) -> Result<()> {
    if find_binary("hermes").is_some() {
        let capture = run_hermes(&["profile", "use", profile])?;
        if capture.success {
            return Ok(());
        }
    }
    write_hermes_sticky_profile(profile)
}

fn write_hermes_sticky_profile(profile: &str) -> Result<()> {
    let sticky = home_join(".hermes/active_profile");
    if let Some(parent) = sticky.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&sticky, format!("{profile}\n"))
        .with_context(|| format!("write {}", sticky.display()))
}

fn write_hermes_terminal_cwd(profile_dir: &Path, project_path: &Path) -> Result<()> {
    let config_path = profile_dir.join("config.yaml");
    let mut mapping = if config_path.exists() {
        let raw = fs::read_to_string(&config_path)
            .with_context(|| format!("read {}", config_path.display()))?;
        serde_yaml::from_str::<Mapping>(&raw).unwrap_or_default()
    } else {
        Mapping::new()
    };

    let terminal = mapping
        .entry("terminal".into())
        .or_insert_with(|| YamlValue::Mapping(Mapping::new()));
    if let YamlValue::Mapping(terminal_map) = terminal {
        terminal_map.insert("backend".into(), YamlValue::String("local".into()));
        terminal_map.insert(
            "cwd".into(),
            YamlValue::String(project_path.display().to_string()),
        );
    }

    let raw = serde_yaml::to_string(&mapping)?;
    fs::write(&config_path, raw).with_context(|| format!("write {}", config_path.display()))
}

/// Seed isolated Hermes profile credentials from the default `~/.hermes` install.
/// Workspace profiles often only get `terminal.cwd`; without model/.env Hermes falls back
/// to OpenRouter and fails Ask with HTTP 401.
fn seed_hermes_profile_credentials(profile_dir: &Path) -> Result<()> {
    let default_home = home_join(".hermes");
    if paths_equal(profile_dir, &default_home) {
        return Ok(());
    }

    let src_env = default_home.join(".env");
    let dst_env = profile_dir.join(".env");
    if src_env.exists() && !dst_env.exists() {
        fs::copy(&src_env, &dst_env)
            .with_context(|| format!("seed {} from {}", dst_env.display(), src_env.display()))?;
    }

    let src_config = default_home.join("config.yaml");
    let dst_config = profile_dir.join("config.yaml");
    if !src_config.exists() {
        return Ok(());
    }
    let src_raw = fs::read_to_string(&src_config)
        .with_context(|| format!("read {}", src_config.display()))?;
    let src_root: YamlValue =
        serde_yaml::from_str(&src_raw).unwrap_or_else(|_| YamlValue::Mapping(Mapping::new()));
    let Some(src_model) = src_root
        .as_mapping()
        .and_then(|m| m.get(YamlValue::from("model")))
        .cloned()
    else {
        return Ok(());
    };

    let mut dst_root: YamlValue = if dst_config.exists() {
        let raw = fs::read_to_string(&dst_config)
            .with_context(|| format!("read {}", dst_config.display()))?;
        serde_yaml::from_str(&raw).unwrap_or_else(|_| YamlValue::Mapping(Mapping::new()))
    } else {
        YamlValue::Mapping(Mapping::new())
    };
    let dst_map = dst_root
        .as_mapping_mut()
        .context("Hermes profile config root must be a mapping")?;
    let needs_model = match dst_map.get(YamlValue::from("model")) {
        Some(YamlValue::Mapping(m)) => m
            .get(YamlValue::from("provider"))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .is_none(),
        _ => true,
    };
    if needs_model {
        dst_map.insert(YamlValue::from("model"), src_model);
        fs::write(&dst_config, serde_yaml::to_string(&dst_root)?)
            .with_context(|| format!("write {}", dst_config.display()))?;
    }
    Ok(())
}

fn seed_openclaw_workspace_files(workspace_path: &Path) -> Result<()> {
    let memory = workspace_path.join("MEMORY.md");
    if !memory.exists() {
        fs::write(
            &memory,
            "# Project memory (OpenClaw workspace)\n\nManaged by Agent Doctor workspace.\n",
        )?;
    }
    let agents = workspace_path.join("AGENTS.md");
    if !agents.exists() {
        fs::write(
            &agents,
            "# Agents (OpenClaw workspace)\n\nManaged by Agent Doctor workspace.\n",
        )?;
    }
    fs::create_dir_all(workspace_path.join("memory")).ok();
    Ok(())
}

pub fn openclaw_default_agent_id() -> Option<String> {
    let value = read_openclaw_config()?;
    if let Some(agents) = value.pointer("/agents/list").and_then(JsonValue::as_array) {
        if let Some(id) = agents.iter().find_map(|agent| {
            (agent.get("default").and_then(JsonValue::as_bool) == Some(true))
                .then(|| {
                    agent
                        .get("id")
                        .and_then(JsonValue::as_str)
                        .map(str::to_string)
                })
                .flatten()
        }) {
            return Some(id);
        }
        if let Some(id) = agents
            .first()
            .and_then(|agent| agent.get("id"))
            .and_then(JsonValue::as_str)
        {
            return Some(id.to_string());
        }
    }
    let entries = value.pointer("/agents/entries")?.as_object()?;
    entries
        .iter()
        .find(|(_, agent)| {
            agent
                .get("workspace")
                .and_then(JsonValue::as_str)
                .is_some_and(|workspace| !workspace.trim().is_empty())
        })
        .map(|(id, _)| id.clone())
        .or_else(|| entries.keys().next().cloned())
}

pub fn openclaw_defaults_workspace() -> Option<PathBuf> {
    let config_path = home_join(".openclaw/openclaw.json");
    if !config_path.exists() {
        return None;
    }
    let raw = fs::read_to_string(&config_path).ok()?;
    let value: JsonValue = serde_json::from_str(&raw).ok()?;
    value
        .pointer("/agents/defaults/workspace")
        .and_then(JsonValue::as_str)
        .map(PathBuf::from)
}

pub fn scaffold_claude_mcp_isolation(project_path: &Path) -> Result<PathBuf> {
    let hint_dir = project_path.join(".agent-doctor");
    fs::create_dir_all(&hint_dir).with_context(|| format!("create {}", hint_dir.display()))?;
    let hint = hint_dir.join("claude-mcp-isolation.md");
    if !hint.exists() {
        fs::write(
            &hint,
            r##"# Claude MCP isolation (Agent Doctor)

User-scoped MCP in `~/.claude/settings.json` or `~/.claude.json` may apply across projects.

Prefer project-scoped MCP:

1. Define servers in this project's `.mcp.json`
2. Remove or narrow user-scoped `mcpServers` when possible
3. Run `agent-doctor workspace doctor` after changes

See https://github.com/EXboys/agent-doctor/blob/main/docs/workspace.md
"##,
        )?;
    }
    Ok(hint)
}

fn upsert_openclaw_agent(agent_id: &str, workspace_path: &Path) -> Result<()> {
    let config_path = home_join(".openclaw/openclaw.json");
    if !config_path.exists() {
        return Ok(());
    }

    let raw = fs::read_to_string(&config_path)
        .with_context(|| format!("read {}", config_path.display()))?;
    let mut value: JsonValue =
        serde_json::from_str(&raw).with_context(|| format!("parse {}", config_path.display()))?;

    let workspace_str = workspace_path.display().to_string();
    {
        let agents_obj = value
            .as_object_mut()
            .context("openclaw.json root must be an object")?
            .entry("agents")
            .or_insert_with(|| json!({}));
        let agents_map = agents_obj
            .as_object_mut()
            .context("openclaw.json agents must be an object")?;
        // `agents.list` is rejected by current OpenClaw. Workspace lives on `agents.entries`.
        agents_map.remove("list");
        let entries = agents_map.entry("entries").or_insert_with(|| json!({}));
        let entries_map = entries
            .as_object_mut()
            .context("openclaw.json agents.entries must be an object")?;
        let entry = entries_map
            .entry(agent_id.to_string())
            .or_insert_with(|| json!({}));
        let entry_obj = entry
            .as_object_mut()
            .context("openclaw.json agents.entries item must be an object")?;
        entry_obj.entry("name").or_insert_with(|| json!(agent_id));
        entry_obj.insert("workspace".to_string(), json!(workspace_str.clone()));

        let defaults = agents_map.entry("defaults").or_insert_with(|| json!({}));
        if let Some(defaults_obj) = defaults.as_object_mut() {
            defaults_obj.insert("workspace".to_string(), json!(workspace_str));
        }
    }

    let updated = serde_json::to_string_pretty(&value)?;
    fs::write(&config_path, format!("{updated}\n"))
        .with_context(|| format!("write {}", config_path.display()))
}

/// Ensure OpenClaw CLI knows the agent (filesystem + routing), not only openclaw.json.
fn ensure_openclaw_agent_via_cli(agent_id: &str, workspace_path: &Path) -> Result<()> {
    if openclaw_agent_known(agent_id) {
        return Ok(());
    }
    if find_binary("openclaw").is_none() {
        return Ok(());
    }
    let capture = run_openclaw(&[
        "agents",
        "add",
        agent_id,
        "--non-interactive",
        "--workspace",
        &workspace_path.display().to_string(),
    ])?;
    if capture.success || openclaw_agent_known(agent_id) {
        return Ok(());
    }
    anyhow::bail!(
        "failed to create OpenClaw agent '{agent_id}': {}{}",
        capture.stderr.trim(),
        if capture.stdout.trim().is_empty() {
            String::new()
        } else {
            format!(" ({})", capture.stdout.trim())
        }
    )
}

fn openclaw_agent_known(agent_id: &str) -> bool {
    if home_join(".openclaw/agents").join(agent_id).is_dir() {
        return true;
    }
    openclaw_agent_workspace(agent_id).is_some()
}

pub(crate) fn run_openclaw(args: &[&str]) -> Result<ShellCapture> {
    let Some(binary) = find_binary("openclaw") else {
        anyhow::bail!("openclaw binary not found");
    };
    let output = Command::new(binary)
        .args(args)
        .output()
        .context("run openclaw")?;
    Ok(ShellCapture {
        success: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        exit_code: output.status.code(),
    })
}

pub(crate) fn run_hermes(args: &[&str]) -> Result<ShellCapture> {
    let Some(binary) = find_binary("hermes") else {
        anyhow::bail!("hermes binary not found");
    };
    let output = Command::new(binary)
        .args(args)
        .output()
        .context("run hermes")?;
    Ok(ShellCapture {
        success: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        exit_code: output.status.code(),
    })
}

fn infer_hermes_profile_from_home_env() -> Option<String> {
    std::env::var("HERMES_HOME").ok().and_then(|home| {
        let path = PathBuf::from(home);
        let profiles_root = home_join(".hermes/profiles");
        if path.starts_with(&profiles_root) {
            path.file_name()?.to_str().map(str::to_string)
        } else {
            None
        }
    })
}

fn parse_hermes_profile_list(text: &str) -> Option<String> {
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.contains('◆') || trimmed.starts_with('*') || trimmed.contains("(active)") {
            let name = trimmed
                .trim_start_matches('*')
                .trim_start_matches('◆')
                .split_whitespace()
                .next()?
                .trim_matches(|c: char| !c.is_alphanumeric() && c != '-' && c != '_');
            if !name.is_empty() {
                return Some(name.to_string());
            }
        }
    }
    None
}

pub fn workspace_paths_match(expected: &Path, actual: &Path) -> bool {
    paths_equal(expected, actual)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn parse_hermes_profile_list_finds_active_marker() {
        let text = "  work\n◆ foo-app\n  bar\n";
        assert_eq!(parse_hermes_profile_list(text).as_deref(), Some("foo-app"));
    }

    #[test]
    fn upsert_openclaw_agent_writes_entries_not_list() {
        let temp = tempdir().unwrap();
        let home = temp.path();
        crate::adapters::util::with_test_home(home, || {
            let openclaw = home.join(".openclaw");
            fs::create_dir_all(&openclaw).unwrap();
            fs::write(
                openclaw.join("openclaw.json"),
                r#"{"agents":{"defaults":{"model":{"primary":"personal/x"}}}}"#,
            )
            .unwrap();
            let ws = home.join("ws");
            fs::create_dir_all(&ws).unwrap();

            upsert_openclaw_agent("agent-doctor", &ws).unwrap();

            let raw = fs::read_to_string(openclaw.join("openclaw.json")).unwrap();
            let value: JsonValue = serde_json::from_str(&raw).unwrap();
            assert!(value.pointer("/agents/list").is_none());
            assert_eq!(
                value
                    .pointer("/agents/entries/agent-doctor/workspace")
                    .and_then(|v| v.as_str()),
                Some(ws.to_str().unwrap())
            );
            assert_eq!(
                value
                    .pointer("/agents/defaults/workspace")
                    .and_then(|v| v.as_str()),
                Some(ws.to_str().unwrap())
            );
        });
    }

    #[test]
    fn trusts_codex_project_paths_in_config() {
        let temp = tempfile::tempdir().unwrap();
        let codex_home = temp.path().join("codex-home");
        let project = temp.path().join("proj");
        fs::create_dir_all(&project).unwrap();

        ensure_codex_projects_trusted(&codex_home, std::slice::from_ref(&project)).unwrap();

        let raw = fs::read_to_string(codex_home.join("config.toml")).unwrap();
        let key = codex_project_trust_key(&project);
        assert!(raw.contains("trust_level"));
        assert!(raw.contains(&key) || raw.to_ascii_lowercase().contains(&key));

        // Idempotent
        ensure_codex_projects_trusted(&codex_home, &[project]).unwrap();
        let raw2 = fs::read_to_string(codex_home.join("config.toml")).unwrap();
        assert_eq!(
            raw.matches("trust_level").count(),
            raw2.matches("trust_level").count()
        );
    }
}
