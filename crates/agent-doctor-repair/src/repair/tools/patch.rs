use std::path::Path;

use anyhow::{bail, Context, Result};
use serde_json::Value;

pub fn patch_structured_file(path: &Path, key_path: &str, raw_value: &str) -> Result<String> {
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    match extension.as_str() {
        "yaml" | "yml" => patch_yaml_file(path, key_path, raw_value),
        "json" => patch_json_file(path, key_path, raw_value),
        "toml" => patch_toml_file(path, key_path, raw_value),
        _ => bail!(
            "patch_config supports .yaml/.yml, .json, .toml only (got {})",
            path.display()
        ),
    }
}

fn patch_yaml_file(path: &Path, key_path: &str, raw_value: &str) -> Result<String> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let mut value: serde_yaml::Value =
        serde_yaml::from_str(&raw).context("failed to parse YAML config")?;
    set_value_at_path(&mut value, key_path, parse_scalar_value(raw_value)?)?;
    serde_yaml::to_string(&value).context("failed to serialize YAML config")
}

fn patch_json_file(path: &Path, key_path: &str, raw_value: &str) -> Result<String> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let mut value: Value = serde_json::from_str(&raw).context("failed to parse JSON config")?;
    let scalar = parse_scalar_value(raw_value)?;
    let json_scalar = serde_json::to_value(&scalar).context("failed to convert patch value")?;
    set_json_value_at_path(&mut value, key_path, json_scalar)?;
    serde_json::to_string_pretty(&value).context("failed to serialize JSON config")
}

fn patch_toml_file(path: &Path, key_path: &str, raw_value: &str) -> Result<String> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let mut doc: toml::Value = raw.parse().context("failed to parse TOML config")?;
    set_toml_value_at_path(&mut doc, key_path, parse_toml_value(raw_value)?)?;
    toml::to_string_pretty(&doc).context("failed to serialize TOML config")
}

fn parse_scalar_value(raw_value: &str) -> Result<serde_yaml::Value> {
    serde_yaml::from_str(raw_value)
        .or_else(|_| Ok(serde_yaml::Value::String(raw_value.to_string())))
}

fn parse_toml_value(raw_value: &str) -> Result<toml::Value> {
    raw_value
        .parse::<toml::Value>()
        .or_else(|_| Ok(toml::Value::String(raw_value.to_string())))
}

fn set_value_at_path(
    root: &mut serde_yaml::Value,
    key_path: &str,
    value: serde_yaml::Value,
) -> Result<()> {
    let parts: Vec<&str> = key_path
        .split('.')
        .filter(|part| !part.is_empty())
        .collect();
    if parts.is_empty() {
        bail!("key_path must not be empty");
    }

    let mut cursor = root;
    for part in &parts[..parts.len() - 1] {
        let mapping = cursor
            .as_mapping_mut()
            .context("key_path traverses non-mapping YAML node")?;
        let key = serde_yaml::Value::String((*part).to_string());
        if !mapping.contains_key(&key) {
            mapping.insert(key.clone(), serde_yaml::Mapping::new().into());
        }
        cursor = mapping
            .get_mut(&key)
            .context("failed to traverse YAML key_path")?;
    }

    let mapping = cursor
        .as_mapping_mut()
        .context("final key_path segment must be a mapping key")?;
    mapping.insert(
        serde_yaml::Value::String(parts[parts.len() - 1].to_string()),
        value,
    );
    Ok(())
}

fn set_json_value_at_path(root: &mut Value, key_path: &str, value: Value) -> Result<()> {
    let parts: Vec<&str> = key_path
        .split('.')
        .filter(|part| !part.is_empty())
        .collect();
    if parts.is_empty() {
        bail!("key_path must not be empty");
    }

    let mut cursor = root;
    for part in &parts[..parts.len() - 1] {
        let object = cursor
            .as_object_mut()
            .context("key_path traverses non-object JSON node")?;
        if !object.contains_key(*part) {
            object.insert((*part).to_string(), Value::Object(Default::default()));
        }
        cursor = object
            .get_mut(*part)
            .context("failed to traverse JSON key_path")?;
    }

    let object = cursor
        .as_object_mut()
        .context("final key_path segment must be an object key")?;
    object.insert(parts[parts.len() - 1].to_string(), value);
    Ok(())
}

fn set_toml_value_at_path(
    root: &mut toml::Value,
    key_path: &str,
    value: toml::Value,
) -> Result<()> {
    let parts: Vec<&str> = key_path
        .split('.')
        .filter(|part| !part.is_empty())
        .collect();
    if parts.is_empty() {
        bail!("key_path must not be empty");
    }

    let mut cursor = root;
    for part in &parts[..parts.len() - 1] {
        let table = cursor
            .as_table_mut()
            .context("key_path traverses non-table TOML node")?;
        if !table.contains_key(*part) {
            table.insert((*part).to_string(), toml::Table::new().into());
        }
        cursor = table
            .get_mut(*part)
            .context("failed to traverse TOML key_path")?;
    }

    let table = cursor
        .as_table_mut()
        .context("final key_path segment must be a table key")?;
    table.insert(parts[parts.len() - 1].to_string(), value);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    #[test]
    fn patches_yaml_key_path() {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("config.yaml");
        let mut file = std::fs::File::create(&path).expect("create");
        writeln!(file, "model:\n  provider: openai\n").expect("write");

        let updated = patch_structured_file(&path, "model.provider", "deepseek").expect("patch");
        assert!(updated.contains("provider: deepseek"));
    }
}
