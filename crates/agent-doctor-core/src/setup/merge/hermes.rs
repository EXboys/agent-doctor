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
    let requested_model = model_id
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .unwrap_or(COMPANY_DEFAULT_MODEL);
    // Hermes rewrites every dot to a hyphen when provider is `anthropic`
    // (`glm-5.3` → `glm-5-3`). Non-Claude models stay on the OpenAI-compatible
    // path under `custom`, which keeps the vendor id unchanged.
    let pin_chat = !model_is_claude(requested_model);
    let model = if pin_chat {
        restore_hyphenated_vendor_model(requested_model)
    } else {
        requested_model.to_string()
    };
    let gateway_url = if pin_chat {
        hermes_chat_base_url(gateway_url)
    } else {
        gateway_url.trim().trim_end_matches('/').to_string()
    };

    // Evotown gateway is OpenAI-compatible; Hermes calls that "custom".
    let effective_provider =
        if pin_chat || provider.trim().is_empty() || provider.trim().eq_ignore_ascii_case("openai")
        {
            "custom"
        } else {
            provider.trim()
        };

    upsert_hermes_slot(&slot, &gateway_url, &model)?;

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
        model_map.insert(YamlValue::from("default"), YamlValue::from(model.as_str()));
        model_map.insert(
            YamlValue::from("base_url"),
            YamlValue::from(gateway_url.as_str()),
        );
        if pin_chat {
            model_map.insert(
                YamlValue::from("api_mode"),
                YamlValue::from("chat_completions"),
            );
        }
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
                title_map.insert(
                    YamlValue::from("base_url"),
                    YamlValue::from(gateway_url.as_str()),
                );
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

/// Claude ids are the ones Hermes is supposed to hyphenate. Everything else
/// (GLM, Qwen, Kimi, MiniMax) must keep the vendor's dotted spelling.
pub(crate) fn model_is_claude(model: &str) -> bool {
    model.trim().to_ascii_lowercase().starts_with("claude")
}

/// `true` when Hermes would send a name the vendor will reject.
pub(crate) fn hermes_model_will_be_rewritten(
    provider: &str,
    api_mode: &str,
    base_url: &str,
    model: &str,
) -> bool {
    let model = model.trim();
    if model.is_empty() || model_is_claude(model) {
        return false;
    }
    let provider = provider.trim().to_ascii_lowercase();
    let api_mode = api_mode.trim().to_ascii_lowercase();
    let url = base_url.trim().trim_end_matches('/').to_ascii_lowercase();
    provider == "anthropic"
        || api_mode == "anthropic_messages"
        || url.ends_with("/anthropic")
        || restore_hyphenated_vendor_model(model) != model
}

/// Map a known Anthropic Messages URL back to that vendor's OpenAI-compatible URL.
pub(crate) fn hermes_chat_base_url(url: &str) -> String {
    let trimmed = url.trim().trim_end_matches('/');
    let lower = trimmed.to_ascii_lowercase();
    if !lower.contains("/anthropic") {
        return trimmed.to_string();
    }
    if lower.contains("open.bigmodel.cn") {
        return "https://open.bigmodel.cn/api/paas/v4".to_string();
    }
    if lower.contains("api.z.ai") {
        return "https://api.z.ai/api/paas/v4".to_string();
    }
    if lower.contains("api.deepseek.com") {
        return "https://api.deepseek.com/v1".to_string();
    }
    if lower.contains("api.minimaxi.com") {
        return "https://api.minimaxi.com/v1".to_string();
    }
    if lower.contains("api.minimax.io") {
        return "https://api.minimax.io/v1".to_string();
    }
    if lower.contains("dashscope.aliyuncs.com") {
        return "https://dashscope.aliyuncs.com/compatible-mode/v1".to_string();
    }
    if lower.contains("api.moonshot.cn") {
        return "https://api.moonshot.cn/v1".to_string();
    }
    if lower.contains("api.moonshot.ai") {
        return "https://api.moonshot.ai/v1".to_string();
    }
    trimmed.to_string()
}

/// Undo Hermes's dot-to-hyphen pass for vendor ids whose canonical form uses a dot.
pub(crate) fn restore_hyphenated_vendor_model(model: &str) -> String {
    let trimmed = model.trim();
    if let Some(restored) = restore_version_dot(trimmed, "glm-") {
        return restored;
    }
    if let Some(restored) = restore_version_dot(trimmed, "kimi-k") {
        return restored;
    }
    if let Some(restored) = restore_version_dot_ci(trimmed, "minimax-m") {
        return restored;
    }
    if let Some(restored) = restore_qwen_version(trimmed) {
        return restored;
    }
    trimmed.to_string()
}

fn restore_version_dot(model: &str, prefix: &str) -> Option<String> {
    let rest = model.strip_prefix(prefix)?;
    let (major, minor) = rest.split_once('-')?;
    if major.is_empty()
        || minor.is_empty()
        || !major.chars().all(|c| c.is_ascii_digit())
        || !minor.chars().all(|c| c.is_ascii_digit())
    {
        return None;
    }
    Some(format!("{prefix}{major}.{minor}"))
}

fn restore_version_dot_ci(model: &str, prefix_lower: &str) -> Option<String> {
    if model.len() < prefix_lower.len() {
        return None;
    }
    let (prefix, rest) = model.split_at(prefix_lower.len());
    if !prefix.eq_ignore_ascii_case(prefix_lower) {
        return None;
    }
    let (major, minor) = rest.split_once('-')?;
    if major.is_empty()
        || minor.is_empty()
        || !major.chars().all(|c| c.is_ascii_digit())
        || !minor.chars().all(|c| c.is_ascii_digit())
    {
        return None;
    }
    Some(format!("{prefix}{major}.{minor}"))
}

fn restore_qwen_version(model: &str) -> Option<String> {
    let rest = model.strip_prefix("qwen")?;
    let (major, tail) = rest.split_once('-')?;
    let (minor, suffix) = tail.split_once('-')?;
    if major.is_empty()
        || minor.is_empty()
        || suffix.is_empty()
        || !major.chars().all(|c| c.is_ascii_digit())
        || !minor.chars().all(|c| c.is_ascii_digit())
    {
        return None;
    }
    Some(format!("qwen{major}.{minor}-{suffix}"))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_glm_dotted_id_and_openai_url() {
        assert_eq!(restore_hyphenated_vendor_model("glm-5-3"), "glm-5.3");
        assert_eq!(restore_hyphenated_vendor_model("glm-5.3"), "glm-5.3");
        assert_eq!(
            restore_hyphenated_vendor_model("glm-5-turbo"),
            "glm-5-turbo"
        );
        assert_eq!(restore_hyphenated_vendor_model("kimi-k2-6"), "kimi-k2.6");
        assert_eq!(
            restore_hyphenated_vendor_model("MiniMax-M2-5"),
            "MiniMax-M2.5"
        );
        assert_eq!(
            restore_hyphenated_vendor_model("qwen3-7-plus"),
            "qwen3.7-plus"
        );
        assert_eq!(
            hermes_chat_base_url("https://open.bigmodel.cn/api/anthropic"),
            "https://open.bigmodel.cn/api/paas/v4"
        );
        assert_eq!(
            hermes_chat_base_url("https://open.bigmodel.cn/api/paas/v4"),
            "https://open.bigmodel.cn/api/paas/v4"
        );
        assert!(hermes_model_will_be_rewritten(
            "anthropic",
            "",
            "https://open.bigmodel.cn/api/anthropic",
            "glm-5.3",
        ));
        assert!(!hermes_model_will_be_rewritten(
            "custom",
            "chat_completions",
            "https://open.bigmodel.cn/api/paas/v4",
            "glm-5.3",
        ));
        assert!(model_is_claude("claude-sonnet-5"));
        assert!(!model_is_claude("glm-5.3"));
    }
}
