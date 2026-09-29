use std::fs;

use anyhow::{Context, Result as AnyhowResult};
use serde_yaml::{Mapping, Value as YamlValue};

use crate::adapters::util::home_join;
use crate::adapters::HermesAdapter;
use crate::setup::{backup_file, ensure_parent, RuntimeSetupResult};

use super::*;

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

pub(crate) fn hermes_slots_path() -> std::path::PathBuf {
    home_join(".hermes/agent-doctor-slots.yaml")
}

pub(crate) fn upsert_hermes_slot(slot: &str, gateway_url: &str, model: &str) -> AnyhowResult<()> {
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
