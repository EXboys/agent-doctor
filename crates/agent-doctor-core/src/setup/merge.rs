use std::fs;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result as AnyhowResult};
use serde_json::{json, Map, Value as JsonValue};
use serde_yaml::{Mapping, Value as YamlValue};

use crate::adapters::util::home_join;
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

/// Hosts that speak OpenAI Chat Completions but not Codex's required `/v1/responses`.
/// Pointing Codex `wire_api = "responses"` at these yields HTTP 404.
pub fn codex_host_supports_responses_api(gateway_url: &str) -> bool {
    let lower = gateway_url.to_ascii_lowercase();
    // Official DeepSeek API: chat/completions only (verified 2026-07).
    if lower.contains("api.deepseek.com") {
        return false;
    }
    true
}

/// Additive Codex wiring: upsert `model_providers.{company|personal}`, point
/// `model_provider` at the active slot, leave the other slot intact.
///
/// Also mirrors into the active workspace `CODEX_HOME` when present so
/// `workspace use` overlays stay aligned with `~/.codex`.
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

    if slot == CODEX_PERSONAL_SLOT && !codex_host_supports_responses_api(gateway_url) {
        return Ok(RuntimeSetupResult {
            runtime_id: "codex".to_string(),
            display_name: "Codex CLI".to_string(),
            applied: false,
            message: format!(
                "skipped — Codex requires OpenAI Responses API (/v1/responses); \
                 {gateway_url} only exposes chat/completions (404 on /responses). \
                 Use Team/Evotown for Codex, or put a Responses→Chat bridge in front."
            ),
            ..Default::default()
        });
    }

    let path = home_join(".codex/config.toml");
    let backup_path = backup_file(&path)?;
    ensure_parent(&path)?;
    write_codex_provider_config(&path, gateway_url, model_id, &slot)?;

    // Keep workspace overlay in sync when an active workspace isolates CODEX_HOME.
    if let Ok(doc) = crate::workspace::load_workspaces() {
        if let Some(active) = doc.active.as_deref() {
            if let Some(entry) = doc.workspaces.get(active) {
                let ws_config = entry.codex_home.join("config.toml");
                if entry.codex_home.exists() {
                    let _ = write_codex_provider_config(&ws_config, gateway_url, model_id, &slot);
                }
            }
        }
    }

    clear_codex_placeholder_auth()?;

    let env_key = codex_slot_env_key(&slot);
    Ok(RuntimeSetupResult {
        runtime_id: "codex".to_string(),
        display_name: "Codex CLI".to_string(),
        applied: true,
        config_path: Some(path.display().to_string()),
        backup_path: backup_path.map(|p| p.display().to_string()),
        message: format!(
            "set model_provider={slot} + openai_base_url (wire_api=responses, env_key={env_key}, model={model_id})"
        ),
        ..Default::default()
    })
}

fn codex_slot_env_key(slot: &str) -> &'static str {
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

    let display = if slot == CODEX_TEAM_SLOT {
        "Company Gateway"
    } else {
        "Personal Provider"
    };

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
    fn deepseek_official_is_chat_only_for_codex() {
        assert!(!codex_host_supports_responses_api(
            "https://api.deepseek.com/v1"
        ));
        assert!(codex_host_supports_responses_api(
            "https://www.skilllite.ai/api/gateway/v1"
        ));
        assert!(codex_host_supports_responses_api(
            "https://api.openai.com/v1"
        ));
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
