use std::fs;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result as AnyhowResult};
use serde_json::{json, Map, Value as JsonValue};

use crate::adapters::util::home_join;
use crate::setup::{backup_file, ensure_parent, RuntimeSetupResult};

use super::*;

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

pub(crate) fn infer_openclaw_slot(gateway_url: &str) -> &'static str {
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

/// Keep OpenClaw's env-ref `OPENAI_API_KEY` in sync with the active Agent Doctor key.
pub(crate) fn sync_openclaw_openai_api_key(api_key: &str) -> AnyhowResult<()> {
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
pub(crate) fn restart_openclaw_gateway_for_key_sync() -> AnyhowResult<String> {
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
pub(crate) fn restart_openclaw_via_launchctl() -> AnyhowResult<String> {
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

pub(crate) fn run_command_with_timeout(
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
pub(crate) fn which_openclaw() -> Option<std::path::PathBuf> {
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
pub(crate) fn ensure_openclaw_local_gateway(obj: &mut Map<String, JsonValue>) {
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

pub(crate) fn generate_openclaw_gateway_token() -> String {
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
