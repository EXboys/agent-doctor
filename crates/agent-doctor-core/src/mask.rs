use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Local-only map from vault token → original secret. Never serialized to LLM payloads.
#[derive(Debug, Default, Clone)]
pub struct SecretVault {
    entries: HashMap<String, String>,
    next_id: usize,
}

impl SecretVault {
    pub fn insert(&mut self, value: String) -> String {
        let token = format!("{{{{SECRET:{}}}}}", self.next_id);
        self.next_id += 1;
        self.entries.insert(token.clone(), value);
        token
    }

    pub fn get(&self, token: &str) -> Option<&str> {
        self.entries.get(token).map(String::as_str)
    }

    pub fn restore_tokens_in_text(&self, text: &str) -> String {
        let mut restored = text.to_string();
        for (token, value) in &self.entries {
            restored = restored.replace(token, value);
        }
        restored
    }

    /// Merge secrets discovered during tool calls into the session vault.
    pub fn absorb(&mut self, other: SecretVault) {
        for value in other.into_values() {
            if !self.entries.values().any(|existing| existing == &value) {
                self.insert(value);
            }
        }
    }

    fn into_values(self) -> impl Iterator<Item = String> {
        self.entries.into_values()
    }

    pub fn mask_text(&mut self, raw: &str) -> String {
        mask_inline_secrets_collecting(raw, self)
    }
}

fn mask_inline_secrets_collecting(raw: &str, vault: &mut SecretVault) -> String {
    raw.lines()
        .map(|line| mask_config_line(line, vault))
        .collect::<Vec<_>>()
        .join("\n")
}

/// One line, two shapes: `KEY=VALUE` (env files, TOML) and `key: value`
/// (YAML, pretty-printed JSON). Configs like `auth.json` or `config.yaml`
/// carry the same API keys as `.env` files. The value is replaced in place so
/// quotes, indentation, and trailing commas survive — the token must still
/// look restorable to `restore_tokens_in_text`.
fn mask_config_line(line: &str, vault: &mut SecretVault) -> String {
    if line.contains("{{SECRET:") {
        // Already carries a vault token (an earlier mask round, or tool
        // output echoing masked context) — masking again would wrap the
        // token inside a fresh one.
        return line.to_string();
    }
    for sep in ['=', ':'] {
        if let Some(masked) = mask_kv_line(line, sep, vault) {
            return masked;
        }
    }
    line.to_string()
}

fn mask_kv_line(line: &str, sep: char, vault: &mut SecretVault) -> Option<String> {
    // Scan every separator occurrence: on a one-line JSON blob the first
    // `:` often belongs to an innocent key ("meta") while a later one holds
    // the secret.
    for (sep_idx, _) in line.match_indices(sep) {
        let key = clean_config_key(&line[..sep_idx]);
        if !looks_secret_env_key(&key) {
            continue;
        }
        let tail = &line[sep_idx + sep.len_utf8()..];
        let value = clean_config_value(tail);
        if value.is_empty() || value.contains("{{SECRET:") {
            continue;
        }
        // Locate the value inside the raw tail so everything around it
        // (opening quote, trailing comma) is kept byte-for-byte.
        let Some(value_idx) = tail.find(&value) else {
            continue;
        };
        let value_len = value.len();
        let token = vault.insert(value);
        return Some(format!(
            "{}{}{}{}",
            &line[..sep_idx + sep.len_utf8()],
            &tail[..value_idx],
            token,
            &tail[value_idx + value_len..]
        ));
    }
    None
}

fn clean_config_key(key_raw: &str) -> String {
    key_raw
        .trim()
        .trim_start_matches("export ")
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .to_string()
}

fn clean_config_value(value_raw: &str) -> String {
    let trimmed = value_raw.trim();
    let trimmed = trimmed.strip_suffix(',').unwrap_or(trimmed);
    trimmed.trim_matches('"').trim_matches('\'').to_string()
}

pub fn is_env_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name == ".env" || name.ends_with(".env"))
}

/// Restore LLM-proposed file content using the session vault before local write.
pub fn unmask_file_content(
    path: &Path,
    original: Option<&str>,
    proposed: &str,
    vault: &SecretVault,
) -> String {
    let restored = vault.restore_tokens_in_text(proposed);
    if is_env_file(path) {
        merge_env_with_vault(original.unwrap_or(""), &restored, vault)
    } else {
        restored
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaskedFileSnippet {
    pub path: String,
    pub masked_content: String,
}

/// Mask middle of a secret value: `sk-abc123secret456` → `sk-abc***456`.
pub fn mask_secret_value(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if trimmed.len() <= 8 {
        return "***".to_string();
    }
    // Both ends are byte counts, so they can land inside a multi-byte
    // character: pull them back to a character boundary instead of panicking.
    let prefix_end = crate::text::floor_char_boundary(trimmed, 3);
    let suffix_start = crate::text::ceil_char_boundary(trimmed, trimmed.len().saturating_sub(3));
    format!("{}***{}", &trimmed[..prefix_end], &trimmed[suffix_start..])
}

pub fn looks_secret_env_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    [
        "api_key",
        "apikey",
        "token",
        "secret",
        "password",
        "authorization",
        "bearer",
    ]
    .iter()
    .any(|needle| key.contains(needle))
}

pub fn mask_env_file_content(raw: &str, vault: &mut SecretVault) -> String {
    let mut lines = Vec::new();
    for line in raw.lines() {
        lines.push(mask_env_line(line, vault));
    }
    lines.join("\n")
}

fn mask_env_line(line: &str, vault: &mut SecretVault) -> String {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') || line.contains("{{SECRET:") {
        return line.to_string();
    }
    let assignment = trimmed.strip_prefix("export ").unwrap_or(trimmed);
    let Some((key, value)) = assignment.split_once('=') else {
        return line.to_string();
    };
    let key = key.trim();
    let value = value.trim().trim_matches('"').trim_matches('\'');
    if !looks_secret_env_key(key) || value.is_empty() {
        return line.to_string();
    }
    let token = vault.insert(value.to_string());
    let prefix = line.find(key).map(|idx| &line[..idx]).unwrap_or("");
    format!("{prefix}{key}={token}")
}

/// Merge LLM/proposed `.env` text: restore `{{SECRET:n}}` tokens; reject novel long secrets.
pub fn merge_env_with_vault(original: &str, proposed: &str, vault: &SecretVault) -> String {
    let mut merged_lines = Vec::new();
    let original_lines: Vec<&str> = original.lines().collect();
    let proposed_lines: Vec<&str> = proposed.lines().collect();

    for (idx, proposed_line) in proposed_lines.iter().enumerate() {
        let restored = vault.restore_tokens_in_text(proposed_line);
        if contains_novel_secret(&restored, vault) {
            if let Some(fallback) = original_lines.get(idx) {
                merged_lines.push((*fallback).to_string());
            } else {
                merged_lines.push(restored);
            }
        } else {
            merged_lines.push(restored);
        }
    }

    if proposed_lines.len() < original_lines.len() {
        merged_lines.extend(
            original_lines[proposed_lines.len()..]
                .iter()
                .map(|line| (*line).to_string()),
        );
    }

    let mut body = merged_lines.join("\n");
    if original.ends_with('\n') && !body.ends_with('\n') {
        body.push('\n');
    }
    body
}

fn contains_novel_secret(line: &str, vault: &SecretVault) -> bool {
    let assignment = line.trim().strip_prefix("export ").unwrap_or(line.trim());
    let Some((key, value)) = assignment.split_once('=') else {
        return false;
    };
    if !looks_secret_env_key(key.trim()) {
        return false;
    }
    let value = value.trim().trim_matches('"').trim_matches('\'');
    if value.is_empty() || value.contains("{{SECRET:") {
        return false;
    }
    // Model must not invent a new secret string.
    value.len() >= 12 && !vault.entries.values().any(|original| original == value)
}

pub fn mask_config_file(path: &Path, raw: &str, vault: &mut SecretVault) -> String {
    if path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name == ".env" || name.ends_with(".env"))
    {
        return mask_env_file_content(raw, vault);
    }
    mask_inline_secrets_in_text(raw, vault)
}

fn mask_inline_secrets_in_text(raw: &str, vault: &mut SecretVault) -> String {
    mask_inline_secrets_collecting(raw, vault)
}

pub fn load_masked_config_snippets(
    config_paths: &[PathBuf],
) -> (Vec<MaskedFileSnippet>, SecretVault) {
    let mut vault = SecretVault::default();
    let mut snippets = Vec::new();
    for path in config_paths {
        if !path.exists() {
            continue;
        }
        let Ok(raw) = std::fs::read_to_string(path) else {
            continue;
        };
        snippets.push(MaskedFileSnippet {
            path: path.display().to_string(),
            masked_content: mask_config_file(path, &raw, &mut vault),
        });
    }
    (snippets, vault)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_env_secrets_with_vault_tokens() {
        let mut vault = SecretVault::default();
        let masked = mask_env_file_content("DEEPSEEK_API_KEY=sk-test-secret-key\n", &mut vault);
        assert!(masked.contains("{{SECRET:0}}"));
        assert!(!masked.contains("sk-test-secret-key"));
        assert_eq!(vault.get("{{SECRET:0}}"), Some("sk-test-secret-key"));
    }

    #[test]
    fn merge_restores_vault_tokens_and_rejects_novel_secrets() {
        let mut vault = SecretVault::default();
        let original = "DEEPSEEK_API_KEY=sk-test-secret-key\n";
        let masked = mask_env_file_content(original, &mut vault);
        let proposed = masked.replace("{{SECRET:0}}", "sk-brand-new-invented-key");
        let merged = merge_env_with_vault(original, &proposed, &vault);
        assert!(merged.contains("sk-test-secret-key"));
        assert!(!merged.contains("sk-brand-new-invented-key"));
    }

    #[test]
    fn mask_secret_value_keeps_prefix_and_suffix() {
        assert_eq!(mask_secret_value("sk-abcdefghijklmnop"), "sk-***nop");
    }

    #[test]
    fn mask_secret_value_handles_a_cjk_value() {
        // The mask kept the first and last three bytes, which cut inside a
        // character for a value that is not pure ASCII.
        assert_eq!(mask_secret_value("a中文密钥"), "a***钥");
    }

    #[test]
    fn masks_pretty_json_secrets_and_keeps_structure() {
        let mut vault = SecretVault::default();
        let raw = "{\n  \"apiKey\": \"sk-json-secret-key\",\n  \"model\": \"glm-4\"\n}";
        let masked = mask_inline_secrets_collecting(raw, &mut vault);
        assert!(masked.contains("\"apiKey\": \"{{SECRET:0}}\","));
        assert!(masked.contains("\"model\": \"glm-4\""));
        assert!(!masked.contains("sk-json-secret-key"));
        assert_eq!(vault.get("{{SECRET:0}}"), Some("sk-json-secret-key"));
        // Token restores the original line byte-for-byte.
        let line = masked.lines().nth(1).unwrap();
        assert_eq!(
            vault.restore_tokens_in_text(line),
            "  \"apiKey\": \"sk-json-secret-key\","
        );
    }

    #[test]
    fn masks_yaml_secrets() {
        let mut vault = SecretVault::default();
        let raw = "providers:\n  - api_key: sk-yaml-secret\n  model: glm-4";
        let masked = mask_inline_secrets_collecting(raw, &mut vault);
        assert!(masked.contains("- api_key: {{SECRET:0}}"));
        assert!(!masked.contains("sk-yaml-secret"));
        assert!(masked.contains("model: glm-4"));
    }

    #[test]
    fn masks_toml_secrets_and_innocent_keys_stay() {
        let mut vault = SecretVault::default();
        let raw = "api_key = \"sk-toml-secret\"\nbase_url = \"https://api.example.com\"";
        let masked = mask_inline_secrets_collecting(raw, &mut vault);
        assert!(masked.contains("api_key = \"{{SECRET:0}}\""));
        assert!(!masked.contains("sk-toml-secret"));
        assert!(masked.contains("base_url = \"https://api.example.com\""));
    }

    #[test]
    fn masks_secret_in_one_line_json_after_innocent_key() {
        let mut vault = SecretVault::default();
        let raw = r#"{"meta":"x","apiKey":"sk-inline-secret"}"#;
        let masked = mask_inline_secrets_collecting(raw, &mut vault);
        assert!(masked.contains("{{SECRET:0}}"));
        assert!(!masked.contains("sk-inline-secret"));
    }

    #[test]
    fn already_masked_lines_are_left_alone() {
        let mut vault = SecretVault::default();
        let raw = "DEEPSEEK_API_KEY={{SECRET:0}}";
        let masked = mask_inline_secrets_collecting(raw, &mut vault);
        assert_eq!(masked, "DEEPSEEK_API_KEY={{SECRET:0}}");
        assert_eq!(vault.get("{{SECRET:0}}"), None);
    }
}
