use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value as JsonValue;

use crate::adapters::util::home_join;

use super::WorkspaceEntry;

#[derive(Debug, Clone, serde::Serialize)]
pub struct PathReferenceIssue {
    pub source: String,
    pub path: String,
}

pub fn scan_workspace_path_references(entry: &WorkspaceEntry) -> Vec<PathReferenceIssue> {
    let mut issues = Vec::new();

    let project_mcp = entry.path.join(".mcp.json");
    if project_mcp.exists() {
        issues.extend(scan_json_path_references(&project_mcp, "project .mcp.json"));
    }

    let openclaw_config = home_join(".openclaw/openclaw.json");
    if openclaw_config.exists() {
        issues.extend(scan_json_path_references(&openclaw_config, "openclaw.json"));
    }

    if entry.openclaw_workspace.is_dir() {
        issues.extend(scan_directory_json_references(
            &entry.openclaw_workspace,
            "openclaw workspace",
        ));
    }

    let hermes_config = home_join(".hermes/profiles")
        .join(&entry.hermes_profile)
        .join("config.yaml");
    if hermes_config.exists() {
        if let Ok(raw) = fs::read_to_string(&hermes_config) {
            if let Ok(value) = serde_yaml::from_str::<serde_yaml::Value>(&raw) {
                issues.extend(scan_yaml_path_references(
                    &value,
                    &format!("hermes profile {}", entry.hermes_profile),
                ));
            }
        }
    }

    issues
        .into_iter()
        .filter(|issue| !path_exists(&issue.path))
        .collect()
}

fn scan_directory_json_references(dir: &Path, source: &str) -> Vec<PathReferenceIssue> {
    let Ok(read_dir) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut issues = Vec::new();
    for entry in read_dir.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
            issues.extend(scan_json_path_references(&path, source));
        }
    }
    issues
}

fn scan_json_path_references(path: &Path, source: &str) -> Vec<PathReferenceIssue> {
    let Ok(raw) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_str::<JsonValue>(&raw) else {
        return Vec::new();
    };
    let mut refs = Vec::new();
    collect_json_paths("", &value, &mut refs);
    refs.into_iter()
        .map(|path| PathReferenceIssue {
            source: source.to_string(),
            path,
        })
        .collect()
}

fn scan_yaml_path_references(value: &serde_yaml::Value, source: &str) -> Vec<PathReferenceIssue> {
    let mut refs = Vec::new();
    collect_yaml_paths("", value, &mut refs);
    refs.into_iter()
        .map(|path| PathReferenceIssue {
            source: source.to_string(),
            path,
        })
        .collect()
}

fn collect_json_paths(key_path: &str, value: &JsonValue, out: &mut Vec<String>) {
    match value {
        JsonValue::Object(map) => {
            for (key, value) in map {
                collect_json_paths(&join_key(key_path, key), value, out);
            }
        }
        JsonValue::Array(items) => {
            for item in items {
                collect_json_paths(key_path, item, out);
            }
        }
        JsonValue::String(text) if is_interesting_path_key(key_path) && is_path_like(text) => {
            out.push(expand_home(text));
        }
        _ => {}
    }
}

fn collect_yaml_paths(key_path: &str, value: &serde_yaml::Value, out: &mut Vec<String>) {
    match value {
        serde_yaml::Value::Mapping(map) => {
            for (key, value) in map {
                let key = key.as_str().unwrap_or_default();
                collect_yaml_paths(&join_key(key_path, key), value, out);
            }
        }
        serde_yaml::Value::Sequence(items) => {
            for item in items {
                collect_yaml_paths(key_path, item, out);
            }
        }
        serde_yaml::Value::String(text)
            if is_interesting_path_key(key_path) && is_path_like(text) =>
        {
            out.push(expand_home(text));
        }
        _ => {}
    }
}

fn join_key(base: &str, key: &str) -> String {
    if base.is_empty() {
        key.to_string()
    } else {
        format!("{base}.{key}")
    }
}

fn is_interesting_path_key(key_path: &str) -> bool {
    let key_path = key_path.to_ascii_lowercase();
    [
        "mcp",
        "skill",
        "skills",
        "manifest",
        "path",
        "command",
        "workspace",
    ]
    .iter()
    .any(|needle| key_path.contains(needle))
}

fn is_path_like(value: &str) -> bool {
    value.starts_with('/')
        || value.starts_with("~/")
        || value.starts_with("./")
        || value.starts_with("../")
}

fn expand_home(value: &str) -> String {
    if let Some(rest) = value.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest).display().to_string();
        }
    }
    value.to_string()
}

fn path_exists(path: &str) -> bool {
    let expanded = expand_home(path);
    let candidate = PathBuf::from(&expanded);
    candidate.exists() || candidate.is_symlink()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn flags_missing_mcp_path_reference() {
        let temp = env::temp_dir().join(format!("ad-paths-{}", std::process::id()));
        let _ = fs::remove_dir_all(&temp);
        fs::create_dir_all(&temp).unwrap();
        fs::write(
            temp.join(".mcp.json"),
            r#"{"mcpServers":{"x":{"command":"/no/such/mcp-binary"}}}"#,
        )
        .unwrap();

        let entry = WorkspaceEntry {
            title: String::new(),
            path: temp.clone(),
            hermes_profile: "demo".into(),
            codex_home: temp.join("codex"),
            openclaw_agent_id: "demo".into(),
            openclaw_workspace: temp.join("openclaw"),
        };

        let issues = scan_workspace_path_references(&entry);
        assert!(issues.iter().any(|issue| issue.path.contains("mcp-binary")));

        let _ = fs::remove_dir_all(&temp);
    }
}
