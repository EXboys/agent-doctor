use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result as AnyhowResult};
use serde_json::{json, Map, Value as JsonValue};
use serde_yaml::{Mapping, Value as YamlValue};
use toml::Value as TomlValue;

use crate::adapters::util::home_join;
use crate::adapters::HermesAdapter;
use crate::setup::{backup_file, ensure_parent, RuntimeSetupResult};

/// Stable custom provider id written into `models.providers`.
pub const OPENCLAW_PROVIDER_ID: &str = "agent-doctor";

/// OpenClaw's `gateway` key is the local control-plane listener (port/mode/bind),
/// not the LLM base URL. Custom/company endpoints belong under `models.providers`.
///
/// Provider `apiKey` is an env ref to `OPENAI_API_KEY`. When `api_key` is non-empty,
/// it is written to `~/.openclaw/.env` and the LaunchAgent `service-env` file so the
/// running gateway does not keep a stale company/personal key.
pub fn apply_openclaw(
    gateway_url: &str,
    api_key: &str,
    model_id: Option<&str>,
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
        .unwrap_or("default");

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

        providers_obj.insert(
            OPENCLAW_PROVIDER_ID.to_string(),
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
            json!({ "primary": format!("{OPENCLAW_PROVIDER_ID}/{model}") }),
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
        "set models.providers.{OPENCLAW_PROVIDER_ID} baseUrl={gateway_url} model={model}"
    );
    if !api_key.trim().is_empty() {
        sync_openclaw_openai_api_key(api_key.trim())?;
        message.push_str("; synced OPENAI_API_KEY to ~/.openclaw/.env (+ service-env if present)");
    }

    Ok(RuntimeSetupResult {
        runtime_id: "openclaw".to_string(),
        display_name: "OpenClaw".to_string(),
        applied: true,
        config_path: Some(path.display().to_string()),
        backup_path: backup_path.map(|p| p.display().to_string()),
        message,
    })
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
    let path = home_join(".hermes/config.yaml");
    let backup_path = backup_file(&path)?;
    ensure_parent(&path)?;

    let mut root: YamlValue = if path.exists() {
        let raw = fs::read_to_string(&path)?;
        serde_yaml::from_str(&raw).unwrap_or_else(|_| YamlValue::Mapping(Mapping::new()))
    } else {
        YamlValue::Mapping(Mapping::new())
    };

    {
        let model = root
            .as_mapping_mut()
            .context("Hermes config root must be a mapping")?
            .entry(YamlValue::from("model"))
            .or_insert_with(|| YamlValue::Mapping(Mapping::new()));
        let model_map = model
            .as_mapping_mut()
            .context("Hermes model section must be a mapping")?;

        // Evotown gateway is OpenAI-compatible; Hermes calls that "custom"
        // (not "openai" — that slug is unknown to Hermes).
        let effective_provider =
            if provider.trim().is_empty() || provider.trim().eq_ignore_ascii_case("openai") {
                "custom"
            } else {
                provider.trim()
            };
        model_map.insert(
            YamlValue::from("provider"),
            YamlValue::from(effective_provider),
        );
        if let Some(id) = model_id.map(str::trim).filter(|m| !m.is_empty()) {
            model_map.insert(YamlValue::from("default"), YamlValue::from(id));
        } else if !model_map.contains_key(YamlValue::from("default")) {
            model_map.insert(YamlValue::from("default"), YamlValue::from("gpt-4o-mini"));
        }
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

    let provider_name = root
        .get("model")
        .and_then(|model| model.get("provider"))
        .and_then(YamlValue::as_str)
        .unwrap_or("custom")
        .to_string();

    fs::write(&path, serde_yaml::to_string(&root)?)?;
    // Custom endpoints authenticate via OPENAI_API_KEY.
    let env_provider = if provider_name == "custom" {
        "openai"
    } else {
        provider_name.as_str()
    };
    HermesAdapter::apply_api_key(env_provider, api_key)?;

    Ok(RuntimeSetupResult {
        runtime_id: "hermes".to_string(),
        display_name: "Hermes Agent".to_string(),
        applied: true,
        config_path: Some(path.display().to_string()),
        backup_path: backup_path.map(|p| p.display().to_string()),
        message: format!(
            "set model.provider={provider_name}, base_url={gateway_url}, and updated ~/.hermes/.env"
        ),
    })
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
    })
}

pub fn apply_codex(
    gateway_url: &str,
    _api_key: &str,
    model: Option<&str>,
) -> AnyhowResult<RuntimeSetupResult> {
    let path = home_join(".codex/config.toml");
    let backup_path = backup_file(&path)?;
    ensure_parent(&path)?;

    let mut root: TomlValue = if path.exists() {
        let raw = fs::read_to_string(&path)?;
        toml::from_str(&raw).unwrap_or(TomlValue::Table(toml::map::Map::new()))
    } else {
        TomlValue::Table(toml::map::Map::new())
    };

    let table = root
        .as_table_mut()
        .context("Codex config root must be a table")?;

    // Prefer Evotown-routable defaults; gpt-4o-* often fails upstream on company gateways.
    let model_id = model
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .unwrap_or("deepseek-v4-flash");
    table.insert("model".to_string(), TomlValue::String(model_id.to_string()));
    table.insert(
        "model_provider".to_string(),
        TomlValue::String("company".to_string()),
    );

    let mut company = toml::map::Map::new();
    company.insert(
        "name".to_string(),
        TomlValue::String("Company Gateway".to_string()),
    );
    company.insert(
        "base_url".to_string(),
        TomlValue::String(gateway_url.to_string()),
    );
    company.insert(
        "env_key".to_string(),
        // Evotown agent env already exports OPENAI_API_KEY=evk_…
        TomlValue::String("OPENAI_API_KEY".to_string()),
    );
    company.insert(
        "requires_openai_auth".to_string(),
        TomlValue::Boolean(false),
    );
    // OpenAI Codex CLI (≥0.84) only accepts Responses wire API.
    company.insert(
        "wire_api".to_string(),
        TomlValue::String("responses".to_string()),
    );
    company.insert("supports_websockets".to_string(), TomlValue::Boolean(false));

    let mut providers = toml::map::Map::new();
    providers.insert("company".to_string(), TomlValue::Table(company));
    table.insert("model_providers".to_string(), TomlValue::Table(providers));

    fs::write(&path, toml::to_string_pretty(&root)?)?;

    clear_codex_placeholder_auth()?;

    Ok(RuntimeSetupResult {
        runtime_id: "codex".to_string(),
        display_name: "Codex CLI".to_string(),
        applied: true,
        config_path: Some(path.display().to_string()),
        backup_path: backup_path.map(|p| p.display().to_string()),
        message: format!(
            "set company gateway (wire_api=responses, model={model_id}); uses OPENAI_API_KEY from env"
        ),
    })
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
