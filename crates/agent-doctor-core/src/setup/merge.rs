use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result as AnyhowResult};
use serde_json::{json, Map, Value as JsonValue};
use serde_yaml::{Mapping, Value as YamlValue};

use crate::adapters::util::{home_dir, home_join};
use crate::adapters::HermesAdapter;
use crate::setup::{backup_file, ensure_parent, RuntimeSetupResult};

/// Legacy single-slot id (migrated away; removed when writing additive slots).
pub const OPENCLAW_PROVIDER_ID: &str = "agent-doctor";
/// Team / Evotown slot in OpenClaw `models.providers` (additive mode).
pub const OPENCLAW_TEAM_SLOT: &str = "evotown";
/// Personal slot in OpenClaw `models.providers` (additive mode).
pub const OPENCLAW_PERSONAL_SLOT: &str = "personal";

/// Codex `model_providers` team slot (pointer via `model_provider`).
pub const CODEX_TEAM_SLOT: &str = "company";
/// Codex `model_providers` personal slot.
pub const CODEX_PERSONAL_SLOT: &str = "personal";

/// Hermes additive slot ids (sidecar + active `model.*` pointer).
pub const HERMES_TEAM_SLOT: &str = "company";
pub const HERMES_PERSONAL_SLOT: &str = "personal";

/// Default model id for company/Evotown wiring when none is specified.
/// Prefer a gateway-routable id: bare `default` / `gpt-4o-*` often 502/503 upstream.
pub const COMPANY_DEFAULT_MODEL: &str = "deepseek-v4-flash";

/// OpenClaw's `gateway` key is the local control-plane listener (port/mode/bind),
/// not the LLM base URL. Custom/company endpoints belong under `models.providers`.
///
/// Additive slots: `evotown` + `personal` coexist; only `agents.defaults.model.primary`
/// flips on mode switch. Provider `apiKey` is an env ref to `OPENAI_API_KEY`. When
/// `api_key` is non-empty, it is synced to `~/.openclaw/.env` + LaunchAgent service-env,
/// then the gateway is restarted so process env picks up the new key.
pub fn apply_openclaw(
    gateway_url: &str,
    api_key: &str,
    model_id: Option<&str>,
) -> AnyhowResult<RuntimeSetupResult> {
    apply_openclaw_slot(gateway_url, api_key, model_id, None)
}

/// Like [`apply_openclaw`], but targets an explicit provider slot (`evotown` / `personal`).
/// `None` picks the slot from URL heuristics (Evotown gateway → team slot).
pub fn apply_openclaw_slot(
    gateway_url: &str,
    api_key: &str,
    model_id: Option<&str>,
    provider_slot: Option<&str>,
) -> AnyhowResult<RuntimeSetupResult> {
    let path = home_join(".openclaw/openclaw.json");
    let backup_path = backup_file(&path)?;
    ensure_parent(&path)?;

    let mut root = if path.exists() {
        let raw = fs::read_to_string(&path)?;
        serde_json::from_str(&raw).unwrap_or_else(|_| json!({}))
    } else {
        json!({})
    };

    let model = model_id
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .unwrap_or(COMPANY_DEFAULT_MODEL);

    let slot = provider_slot
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| infer_openclaw_slot(gateway_url).to_string());

    if let Some(obj) = root.as_object_mut() {
        // Strip legacy Agent Doctor keys that fail OpenClaw ≥2026.7 schema.
        obj.remove("evotown");
        ensure_openclaw_local_gateway(obj);

        let models = obj.entry("models").or_insert_with(|| json!({}));
        let models_obj = models
            .as_object_mut()
            .context("OpenClaw models section must be an object")?;
        models_obj
            .entry("mode".to_string())
            .or_insert_with(|| json!("merge"));

        let providers = models_obj
            .entry("providers".to_string())
            .or_insert_with(|| json!({}));
        let providers_obj = providers
            .as_object_mut()
            .context("OpenClaw models.providers must be an object")?;

        // Drop legacy exclusive slot so UI/status shows additive ids only.
        providers_obj.remove(OPENCLAW_PROVIDER_ID);

        providers_obj.insert(
            slot.clone(),
            json!({
                "baseUrl": gateway_url,
                "api": "openai-completions",
                "apiKey": {
                    "source": "env",
                    "provider": "default",
                    "id": "OPENAI_API_KEY"
                },
                "models": [{
                    "id": model,
                    "name": model,
                    "input": ["text"]
                }]
            }),
        );

        let agents = obj.entry("agents").or_insert_with(|| json!({}));
        let agents_obj = agents
            .as_object_mut()
            .context("OpenClaw agents section must be an object")?;
        let defaults = agents_obj
            .entry("defaults".to_string())
            .or_insert_with(|| json!({}));
        let defaults_obj = defaults
            .as_object_mut()
            .context("OpenClaw agents.defaults must be an object")?;
        defaults_obj.insert(
            "model".to_string(),
            json!({ "primary": format!("{slot}/{model}") }),
        );

        let tools = obj.entry("tools").or_insert_with(|| json!({}));
        if let Some(tools_obj) = tools.as_object_mut() {
            tools_obj
                .entry("profile".to_string())
                .or_insert_with(|| json!("coding"));
        }
    }

    fs::write(&path, serde_json::to_string_pretty(&root)?)?;

    let mut message = format!(
        "set models.providers.{slot} baseUrl={gateway_url} model={model} (primary={slot}/{model}; additive)"
    );
    if !api_key.trim().is_empty() {
        sync_openclaw_openai_api_key(api_key.trim())?;
        message.push_str("; synced OPENAI_API_KEY to ~/.openclaw/.env (+ service-env if present)");
        // Hot-reload picks up openclaw.json but NOT process env; restart so LaunchAgent
        // re-sources service-env (stale sk_/evk_ keys cause Evotown 503).
        match restart_openclaw_gateway_for_key_sync() {
            Ok(detail) => {
                message.push_str("; ");
                message.push_str(&detail);
            }
            Err(err) => {
                message.push_str(&format!(
                    "; gateway restart skipped ({err}) — run `openclaw gateway restart` if auth is stale"
                ));
            }
        }
    }

    Ok(RuntimeSetupResult {
        runtime_id: "openclaw".to_string(),
        display_name: "OpenClaw".to_string(),
        applied: true,
        config_path: Some(path.display().to_string()),
        backup_path: backup_path.map(|p| p.display().to_string()),
        message,
        ..Default::default()
    })
}

fn infer_openclaw_slot(gateway_url: &str) -> &'static str {
    let lower = gateway_url.to_ascii_lowercase();
    if lower.contains("/api/gateway/v1")
        || lower.contains("skilllite.ai")
        || lower.contains("evotown")
    {
        OPENCLAW_TEAM_SLOT
    } else {
        OPENCLAW_PERSONAL_SLOT
    }
}

#[cfg(test)]
mod openclaw_slot_tests {
    use super::*;

    #[test]
    fn infer_slot_prefers_evotown_for_company_gateway() {
        assert_eq!(
            infer_openclaw_slot("https://www.skilllite.ai/api/gateway/v1"),
            OPENCLAW_TEAM_SLOT
        );
        assert_eq!(
            infer_openclaw_slot("https://api.deepseek.com/v1"),
            OPENCLAW_PERSONAL_SLOT
        );
    }
}

/// Keep OpenClaw's env-ref `OPENAI_API_KEY` in sync with the active Agent Doctor key.
fn sync_openclaw_openai_api_key(api_key: &str) -> AnyhowResult<()> {
    let env_path = home_join(".openclaw/.env");
    ensure_parent(&env_path)?;
    let existing = if env_path.exists() {
        fs::read_to_string(&env_path)?
    } else {
        String::new()
    };
    let mut lines: Vec<String> = existing
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            !trimmed.starts_with("OPENAI_API_KEY=")
        })
        .map(str::to_string)
        .collect();
    if lines.is_empty() {
        lines.push("# Agent Doctor — OPENAI_API_KEY synced from setup / personal provider".into());
        lines.push("ANTHROPIC_API_KEY=".into());
    }
    // Keep key near the top after comments.
    let insert_at = lines
        .iter()
        .position(|line| !line.trim().is_empty() && !line.trim().starts_with('#'))
        .unwrap_or(lines.len());
    lines.insert(insert_at, format!("OPENAI_API_KEY={api_key}"));
    fs::write(&env_path, lines.join("\n") + "\n")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&env_path, fs::Permissions::from_mode(0o600))?;
    }

    // LaunchAgent gateway injects OPENAI_API_KEY from service-env; update if present.
    let service_env = home_join(".openclaw/service-env/ai.openclaw.gateway.env");
    if service_env.exists() {
        let raw = fs::read_to_string(&service_env)?;
        let escaped = api_key.replace('\'', "'\\''");
        let replacement = format!("export OPENAI_API_KEY='{escaped}'");
        let mut replaced = false;
        let mut out = Vec::new();
        for line in raw.lines() {
            if line.trim_start().starts_with("export OPENAI_API_KEY=")
                || line.trim_start().starts_with("OPENAI_API_KEY=")
            {
                out.push(replacement.clone());
                replaced = true;
            } else {
                out.push(line.to_string());
            }
        }
        if !replaced {
            out.push(replacement);
        }
        fs::write(&service_env, out.join("\n") + "\n")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&service_env, fs::Permissions::from_mode(0o600))?;
        }
    }

    Ok(())
}

/// Restart the LaunchAgent/systemd OpenClaw gateway so a freshly written
/// `OPENAI_API_KEY` in service-env is actually loaded into the process.
///
/// Prefer a fast `launchctl kickstart` on macOS. Avoid falling back to
/// `openclaw gateway restart` during mode switch — it often blocks 15–20s and
/// makes the desktop UI look frozen.
fn restart_openclaw_gateway_for_key_sync() -> AnyhowResult<String> {
    #[cfg(target_os = "macos")]
    {
        match restart_openclaw_via_launchctl() {
            Ok(detail) => Ok(detail),
            Err(err) => {
                // Do not chain a slow CLI restart here; surface a short hint instead.
                Err(anyhow::anyhow!(
                    "{err}; run `openclaw gateway restart` manually if auth is stale"
                ))
            }
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        let openclaw = which_openclaw().context("openclaw binary not found on PATH")?;
        let output = run_command_with_timeout(
            Command::new(&openclaw)
                .args(["gateway", "restart"])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped()),
            Duration::from_secs(6),
        )
        .with_context(|| format!("failed to run `{} gateway restart`", openclaw.display()))?;
        if output.status.success() {
            return Ok("restarted OpenClaw gateway (reload env key)".into());
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let detail = if stdout.trim().is_empty() {
            stderr.trim().to_string()
        } else if stderr.trim().is_empty() {
            stdout.trim().to_string()
        } else {
            format!("{} / {}", stderr.trim(), stdout.trim())
        };
        Err(anyhow::anyhow!(
            "`openclaw gateway restart` failed: {detail}"
        ))
    }
}

#[cfg(target_os = "macos")]
fn restart_openclaw_via_launchctl() -> AnyhowResult<String> {
    let uid = Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .context("could not resolve user id for launchctl")?;
    let label = format!("gui/{uid}/ai.openclaw.gateway");
    let output = run_command_with_timeout(
        Command::new("launchctl")
            .args(["kickstart", "-k", &label])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
        Duration::from_secs(4),
    )
    .with_context(|| format!("launchctl kickstart {label}"))?;
    if output.status.success() {
        // Brief settle so subsequent probe is less flaky.
        thread::sleep(Duration::from_millis(200));
        return Ok(format!(
            "restarted OpenClaw gateway via launchctl ({label})"
        ));
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(anyhow::anyhow!(
        "launchctl kickstart failed: {}",
        stderr.trim()
    ))
}

fn run_command_with_timeout(
    command: &mut Command,
    timeout: Duration,
) -> AnyhowResult<std::process::Output> {
    let child = command.spawn().context("failed to spawn process")?;
    let pid = child.id();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(child.wait_with_output());
    });
    match rx.recv_timeout(timeout) {
        Ok(Ok(output)) => Ok(output),
        Ok(Err(err)) => Err(err).context("failed waiting for process"),
        Err(_) => {
            let _ = Command::new("kill")
                .arg("-TERM")
                .arg(pid.to_string())
                .output();
            thread::sleep(Duration::from_millis(200));
            let _ = Command::new("kill")
                .arg("-KILL")
                .arg(pid.to_string())
                .output();
            anyhow::bail!("timed out after {}s", timeout.as_secs());
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn which_openclaw() -> Option<std::path::PathBuf> {
    if let Ok(path) = std::env::var("OPENCLAW_BIN") {
        let p = std::path::PathBuf::from(path);
        if p.is_file() {
            return Some(p);
        }
    }
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join("openclaw");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    #[cfg(target_os = "macos")]
    {
        let brew = std::path::PathBuf::from("/opt/homebrew/bin/openclaw");
        if brew.is_file() {
            return Some(brew);
        }
    }
    None
}

/// Ensure OpenClaw local gateway can accept `openclaw tui` clients.
///
/// OpenClaw 2026.7+ expects `gateway.mode=local` and a shared-secret token
/// (even on loopback). Preserve any existing auth token/password.
fn ensure_openclaw_local_gateway(obj: &mut Map<String, JsonValue>) {
    let gateway = obj.entry("gateway").or_insert_with(|| json!({}));
    let Some(gateway_obj) = gateway.as_object_mut() else {
        return;
    };

    // Legacy Agent Doctor wrote LLM URLs here; that is invalid now.
    gateway_obj.remove("url");
    gateway_obj
        .entry("mode".to_string())
        .or_insert_with(|| json!("local"));

    let auth = gateway_obj.entry("auth").or_insert_with(|| json!({}));
    let Some(auth_obj) = auth.as_object_mut() else {
        return;
    };

    let has_token = auth_obj
        .get("token")
        .map(|v| match v {
            JsonValue::String(s) => !s.trim().is_empty(),
            JsonValue::Object(_) => true,
            _ => false,
        })
        .unwrap_or(false);
    let has_password = auth_obj
        .get("password")
        .map(|v| match v {
            JsonValue::String(s) => !s.trim().is_empty(),
            JsonValue::Object(_) => true,
            _ => false,
        })
        .unwrap_or(false);

    if !has_token && !has_password {
        auth_obj.insert("mode".to_string(), json!("token"));
        auth_obj.insert(
            "token".to_string(),
            json!(generate_openclaw_gateway_token()),
        );
    } else {
        auth_obj
            .entry("mode".to_string())
            .or_insert_with(|| json!(if has_password { "password" } else { "token" }));
    }
}

fn generate_openclaw_gateway_token() -> String {
    // Prefer OS entropy; fall back to a time/pid mix if /dev/urandom is unavailable.
    if let Ok(mut file) = fs::File::open("/dev/urandom") {
        use std::io::Read;
        let mut bytes = [0u8; 24];
        if file.read_exact(&mut bytes).is_ok() {
            return bytes.iter().map(|b| format!("{b:02x}")).collect();
        }
    }

    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut out = String::with_capacity(48);
    for salt in [0u128, 1, 2] {
        let mut hasher = DefaultHasher::new();
        (nanos ^ (salt << 48)).hash(&mut hasher);
        std::process::id().hash(&mut hasher);
        salt.hash(&mut hasher);
        out.push_str(&format!("{:016x}", hasher.finish()));
    }
    out
}

pub fn apply_hermes(
    gateway_url: &str,
    api_key: &str,
    provider: &str,
    model_id: Option<&str>,
) -> AnyhowResult<RuntimeSetupResult> {
    apply_hermes_slot(gateway_url, api_key, provider, model_id, None)
}

/// Additive Hermes wiring: slots live in `~/.hermes/agent-doctor-slots.yaml`;
/// `config.yaml` `model.*` is the active pointer (Hermes-safe; no unknown keys).
pub fn apply_hermes_slot(
    gateway_url: &str,
    api_key: &str,
    provider: &str,
    model_id: Option<&str>,
    provider_slot: Option<&str>,
) -> AnyhowResult<RuntimeSetupResult> {
    let path = home_join(".hermes/config.yaml");
    let backup_path = backup_file(&path)?;
    ensure_parent(&path)?;

    let slot = provider_slot
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| infer_codex_hermes_slot(gateway_url).to_string());
    let model = model_id
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .unwrap_or(COMPANY_DEFAULT_MODEL);

    // Evotown gateway is OpenAI-compatible; Hermes calls that "custom".
    let effective_provider =
        if provider.trim().is_empty() || provider.trim().eq_ignore_ascii_case("openai") {
            "custom"
        } else {
            provider.trim()
        };

    upsert_hermes_slot(&slot, gateway_url, model)?;

    let mut root: YamlValue = if path.exists() {
        let raw = fs::read_to_string(&path)?;
        serde_yaml::from_str(&raw).unwrap_or_else(|_| YamlValue::Mapping(Mapping::new()))
    } else {
        YamlValue::Mapping(Mapping::new())
    };

    {
        let model_section = root
            .as_mapping_mut()
            .context("Hermes config root must be a mapping")?
            .entry(YamlValue::from("model"))
            .or_insert_with(|| YamlValue::Mapping(Mapping::new()));
        let model_map = model_section
            .as_mapping_mut()
            .context("Hermes model section must be a mapping")?;

        model_map.insert(
            YamlValue::from("provider"),
            YamlValue::from(effective_provider),
        );
        model_map.insert(YamlValue::from("default"), YamlValue::from(model));
        model_map.insert(YamlValue::from("base_url"), YamlValue::from(gateway_url));
    }

    // Keep title generation on the same gateway (avoid auto → native provider 401).
    if let Some(root_map) = root.as_mapping_mut() {
        let aux = root_map
            .entry(YamlValue::from("auxiliary"))
            .or_insert_with(|| YamlValue::Mapping(Mapping::new()));
        if let Some(aux_map) = aux.as_mapping_mut() {
            let title = aux_map
                .entry(YamlValue::from("title_generation"))
                .or_insert_with(|| YamlValue::Mapping(Mapping::new()));
            if let Some(title_map) = title.as_mapping_mut() {
                title_map.insert(YamlValue::from("provider"), YamlValue::from("custom"));
                title_map.insert(YamlValue::from("base_url"), YamlValue::from(gateway_url));
            }
        }
    }

    fs::write(&path, serde_yaml::to_string(&root)?)?;
    let env_provider = if effective_provider == "custom" {
        "openai"
    } else {
        effective_provider
    };
    HermesAdapter::apply_api_key(env_provider, api_key)?;
    // Older Hermes/.env scaffolds leave `CUSTOM_API_KEY=` empty; that made
    // probes claim credentials were missing even after OPENAI_API_KEY was set.
    if effective_provider == "custom" {
        let _ = HermesAdapter::clear_empty_env_key("CUSTOM_API_KEY");
    }

    Ok(RuntimeSetupResult {
        runtime_id: "hermes".to_string(),
        display_name: "Hermes".to_string(),
        applied: true,
        config_path: Some(path.display().to_string()),
        backup_path: backup_path.map(|p| p.display().to_string()),
        message: format!(
            "set Hermes pointer model.base_url={gateway_url} model={model} (slot={slot}; additive sidecar)"
        ),
        ..Default::default()
    })
}

fn hermes_slots_path() -> std::path::PathBuf {
    home_join(".hermes/agent-doctor-slots.yaml")
}

fn upsert_hermes_slot(slot: &str, gateway_url: &str, model: &str) -> AnyhowResult<()> {
    let path = hermes_slots_path();
    ensure_parent(&path)?;
    let mut root: YamlValue = if path.exists() {
        let raw = fs::read_to_string(&path)?;
        serde_yaml::from_str(&raw).unwrap_or_else(|_| YamlValue::Mapping(Mapping::new()))
    } else {
        YamlValue::Mapping(Mapping::new())
    };
    let map = root
        .as_mapping_mut()
        .context("Hermes slots root must be a mapping")?;
    map.insert(YamlValue::from("active"), YamlValue::from(slot));
    let slots = map
        .entry(YamlValue::from("slots"))
        .or_insert_with(|| YamlValue::Mapping(Mapping::new()));
    let slots_map = slots
        .as_mapping_mut()
        .context("Hermes slots.slots must be a mapping")?;
    let mut entry = Mapping::new();
    entry.insert(YamlValue::from("base_url"), YamlValue::from(gateway_url));
    entry.insert(YamlValue::from("default"), YamlValue::from(model));
    slots_map.insert(YamlValue::from(slot), YamlValue::Mapping(entry));
    fs::write(&path, serde_yaml::to_string(&root)?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

pub fn apply_claude_code(gateway_url: &str, api_key: &str) -> AnyhowResult<RuntimeSetupResult> {
    apply_claude_code_with_model(gateway_url, api_key, None)
}

pub fn apply_claude_code_with_model(
    gateway_url: &str,
    api_key: &str,
    model: Option<&str>,
) -> AnyhowResult<RuntimeSetupResult> {
    let path = home_join(".claude/settings.json");
    let backup_path = backup_file(&path)?;
    ensure_parent(&path)?;

    let mut root = if path.exists() {
        let raw = fs::read_to_string(&path)?;
        serde_json::from_str(&raw).unwrap_or_else(|_| json!({}))
    } else {
        json!({})
    };

    let env = root
        .as_object_mut()
        .context("Claude settings root must be an object")?
        .entry("env")
        .or_insert_with(|| json!({}));
    if let Some(env_obj) = env.as_object_mut() {
        env_obj.insert("ANTHROPIC_BASE_URL".to_string(), json!(gateway_url));
        env_obj.insert("ANTHROPIC_API_KEY".to_string(), json!(api_key));
        if let Some(model_id) = model.map(str::trim).filter(|m| !m.is_empty()) {
            env_obj.insert("ANTHROPIC_MODEL".to_string(), json!(model_id));
            env_obj.insert(
                "ANTHROPIC_DEFAULT_SONNET_MODEL".to_string(),
                json!(model_id),
            );
            env_obj.insert("ANTHROPIC_DEFAULT_OPUS_MODEL".to_string(), json!(model_id));
            env_obj.insert("ANTHROPIC_DEFAULT_HAIKU_MODEL".to_string(), json!(model_id));
            env_obj.insert("CLAUDE_CODE_SUBAGENT_MODEL".to_string(), json!(model_id));
        }
    }
    root.as_object_mut()
        .expect("object")
        .insert("anthropicBaseUrl".to_string(), json!(gateway_url));

    fs::write(&path, serde_json::to_string_pretty(&root)?)?;

    Ok(RuntimeSetupResult {
        runtime_id: "claude-code".to_string(),
        display_name: "Claude Code".to_string(),
        applied: true,
        config_path: Some(path.display().to_string()),
        backup_path: backup_path.map(|p| p.display().to_string()),
        message: format!(
            "set env.ANTHROPIC_BASE_URL to {gateway_url} (Anthropic Messages path) and API key"
        ),
        ..Default::default()
    })
}

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

/// Drop Codex tools whose program file is already gone.
///
/// Codex starts every `[mcp_servers.*]` entry when a message is sent. A path
/// left behind by ChatGPT, an old Codex install, or a removed app then shows
/// up as a failure on that message. Names like `agent-doctor` stay, because
/// those are looked up when the tool actually starts.
pub fn drop_unreachable_codex_mcp_servers(path: &Path) -> AnyhowResult<bool> {
    if !path.exists() {
        return Ok(false);
    }
    let raw = fs::read_to_string(path)?;
    let mut doc = raw.parse::<toml_edit::DocumentMut>()?;
    let Some(servers) = doc
        .get_mut("mcp_servers")
        .and_then(|item| item.as_table_mut())
    else {
        return Ok(false);
    };
    let stale: Vec<String> = servers
        .iter()
        .filter_map(|(name, entry)| {
            let command = entry
                .as_table()
                .and_then(|table| table.get("command"))
                .and_then(|value| value.as_str())
                .unwrap_or("");
            command_path_missing(command).then(|| name.to_string())
        })
        .collect();
    if stale.is_empty() {
        return Ok(false);
    }
    for name in &stale {
        servers.remove(name);
    }
    fs::write(path, doc.to_string())?;
    Ok(true)
}

/// True when `command` is a file path and that file is not there.
/// A bare program name is left alone.
pub(crate) fn command_path_missing(command: &str) -> bool {
    let command = command.trim();
    if command.is_empty() {
        return false;
    }
    let looks_like_path =
        Path::new(command).is_absolute() || command.contains('/') || command.contains('\\');
    looks_like_path && !Path::new(command).is_file()
}

struct AskToolRoots {
    home: PathBuf,
    cwd: PathBuf,
    codex_homes: Vec<PathBuf>,
    hermes_configs: Vec<PathBuf>,
    dsh_patches: Vec<PathBuf>,
    extra_mcp_json: Vec<PathBuf>,
}

/// Remove tools whose program file is gone, from every config Ask is about to start.
///
/// Claude, Codex, Hermes, OpenClaw, and DeepSeek Harness all launch their tool
/// list when a message is sent. This runs first, so a missing program never
/// becomes an error on that message.
pub fn drop_unreachable_ask_tools(cwd: &Path) {
    let mut roots = AskToolRoots {
        home: home_dir(),
        cwd: cwd.to_path_buf(),
        codex_homes: Vec::new(),
        hermes_configs: Vec::new(),
        dsh_patches: Vec::new(),
        extra_mcp_json: Vec::new(),
    };
    if let Ok(doc) = crate::workspace::load_workspaces() {
        if let Some(entry) = doc
            .active
            .as_ref()
            .and_then(|name| doc.workspaces.get(name))
        {
            roots.codex_homes.push(entry.codex_home.clone());
            roots.extra_mcp_json.push(entry.path.join(".mcp.json"));
            roots
                .extra_mcp_json
                .push(entry.openclaw_workspace.join(".mcp.json"));
            roots.hermes_configs.push(
                home_dir()
                    .join(".hermes/profiles")
                    .join(&entry.hermes_profile)
                    .join("config.yaml"),
            );
        }
    }
    let overlay = crate::prompt_session::env::collect_overlay_env();
    if let Some(home) = overlay.get("CODEX_HOME") {
        roots.codex_homes.push(PathBuf::from(home));
    }
    if let Some(home) = overlay.get("HERMES_HOME") {
        roots
            .hermes_configs
            .push(PathBuf::from(home).join("config.yaml"));
    }
    if let Some(home) = overlay.get("DSH_HOME") {
        roots
            .dsh_patches
            .push(PathBuf::from(home).join("cordis.patch.yml"));
    }
    drop_unreachable_ask_tools_in(&roots);
}

fn drop_unreachable_ask_tools_in(roots: &AskToolRoots) {
    let _ = drop_unreachable_codex_mcp_servers(&roots.home.join(".codex/config.toml"));
    let _ = drop_unreachable_codex_mcp_servers(&roots.cwd.join(".codex").join("config.toml"));
    let _ = drop_unreachable_json_mcp_servers(&roots.cwd.join(".mcp.json"));
    let _ = drop_unreachable_json_mcp_servers(&roots.home.join(".claude.json"));
    let _ = drop_unreachable_json_mcp_servers(&roots.home.join(".claude/settings.json"));
    let _ = drop_unreachable_yaml_mcp_servers(&roots.home.join(".hermes/config.yaml"));
    let _ = drop_unreachable_json_mcp_servers(&roots.home.join(".openclaw/openclaw.json"));
    let _ = drop_unreachable_dsh_mcp_plugins(&roots.home.join(".dsh/cordis.patch.yml"));
    for home in &roots.codex_homes {
        let _ = drop_unreachable_codex_mcp_servers(&home.join("config.toml"));
    }
    for path in &roots.hermes_configs {
        let _ = drop_unreachable_yaml_mcp_servers(path);
    }
    for path in &roots.dsh_patches {
        let _ = drop_unreachable_dsh_mcp_plugins(path);
    }
    for path in &roots.extra_mcp_json {
        let _ = drop_unreachable_json_mcp_servers(path);
    }
}

/// Claude, OpenClaw, and project `.mcp.json` store tools under `mcpServers`
/// and/or `mcp.servers`.
pub fn drop_unreachable_json_mcp_servers(path: &Path) -> AnyhowResult<bool> {
    if !path.exists() {
        return Ok(false);
    }
    let raw = fs::read_to_string(path)?;
    let mut doc: JsonValue = match serde_json::from_str(&raw) {
        Ok(value) => value,
        Err(_) => return Ok(false),
    };
    let mut changed = false;
    if let Some(map) = doc.get_mut("mcpServers").and_then(JsonValue::as_object_mut) {
        changed |= prune_json_server_map(map);
    }
    if let Some(map) = doc
        .pointer_mut("/mcp/servers")
        .and_then(JsonValue::as_object_mut)
    {
        changed |= prune_json_server_map(map);
    }
    if !changed {
        return Ok(false);
    }
    let rendered = serde_json::to_string_pretty(&doc)?;
    fs::write(path, format!("{rendered}\n"))?;
    Ok(true)
}

fn prune_json_server_map(map: &mut serde_json::Map<String, JsonValue>) -> bool {
    let stale: Vec<String> = map
        .iter()
        .filter(|(_, entry)| json_command_missing(entry))
        .map(|(name, _)| name.clone())
        .collect();
    if stale.is_empty() {
        return false;
    }
    for name in stale {
        map.remove(&name);
    }
    true
}

fn json_command_missing(entry: &JsonValue) -> bool {
    entry
        .get("command")
        .and_then(JsonValue::as_str)
        .is_some_and(command_path_missing)
}

/// Hermes `config.yaml` stores tools under `mcp_servers`.
pub fn drop_unreachable_yaml_mcp_servers(path: &Path) -> AnyhowResult<bool> {
    if !path.exists() {
        return Ok(false);
    }
    let raw = fs::read_to_string(path)?;
    let mut root: YamlValue = match serde_yaml::from_str(&raw) {
        Ok(value) => value,
        Err(_) => return Ok(false),
    };
    let Some(servers) = root
        .get_mut("mcp_servers")
        .and_then(YamlValue::as_mapping_mut)
    else {
        return Ok(false);
    };
    let stale: Vec<YamlValue> = servers
        .iter()
        .filter_map(|(name, entry)| {
            let command = entry.get("command").and_then(YamlValue::as_str)?;
            command_path_missing(command).then(|| name.clone())
        })
        .collect();
    if stale.is_empty() {
        return Ok(false);
    }
    for name in stale {
        servers.remove(&name);
    }
    fs::write(path, serde_yaml::to_string(&root)?)?;
    Ok(true)
}

/// DeepSeek Harness lists MCP plugins in `cordis.patch.yml`.
pub fn drop_unreachable_dsh_mcp_plugins(path: &Path) -> AnyhowResult<bool> {
    if !path.exists() {
        return Ok(false);
    }
    let raw = fs::read_to_string(path)?;
    let mut root: YamlValue = match serde_yaml::from_str(&raw) {
        Ok(value) => value,
        Err(_) => return Ok(false),
    };
    let changed = match &mut root {
        YamlValue::Sequence(seq) => prune_dsh_plugin_seq(seq),
        YamlValue::Mapping(map) => {
            let key = YamlValue::String("plugins".into());
            match map.get_mut(&key) {
                Some(YamlValue::Sequence(seq)) => prune_dsh_plugin_seq(seq),
                _ => false,
            }
        }
        _ => false,
    };
    if !changed {
        return Ok(false);
    }
    fs::write(path, serde_yaml::to_string(&root)?)?;
    Ok(true)
}

fn prune_dsh_plugin_seq(seq: &mut Vec<YamlValue>) -> bool {
    let before = seq.len();
    seq.retain(|item| !dsh_plugin_command_missing(item));
    seq.len() != before
}

fn dsh_plugin_command_missing(item: &YamlValue) -> bool {
    let Some(map) = item.as_mapping() else {
        return false;
    };
    let name = map
        .get(YamlValue::String("name".into()))
        .and_then(YamlValue::as_str)
        .unwrap_or("");
    let id = map
        .get(YamlValue::String("id".into()))
        .and_then(YamlValue::as_str)
        .unwrap_or("");
    if !name.contains("dsh-mcp-client") && id != "mcp-browser" {
        return false;
    }
    let Some(config) = map.get(YamlValue::String("config".into())) else {
        return false;
    };
    config
        .get("command")
        .and_then(YamlValue::as_str)
        .is_some_and(command_path_missing)
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

fn write_codex_provider_config(
    path: &std::path::Path,
    gateway_url: &str,
    model_id: &str,
    slot: &str,
) -> AnyhowResult<()> {
    ensure_parent(path)?;

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
    doc["openai_base_url"] = toml_edit::value(gateway_url);

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

fn infer_codex_hermes_slot(gateway_url: &str) -> &'static str {
    // Same heuristics as OpenClaw team detection.
    if infer_openclaw_slot(gateway_url) == OPENCLAW_TEAM_SLOT {
        CODEX_TEAM_SLOT
    } else {
        CODEX_PERSONAL_SLOT
    }
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
