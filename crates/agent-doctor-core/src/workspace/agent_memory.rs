use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::Result;
use serde::Serialize;
use serde_json::Value;

use crate::DeepSeekHarnessAdapter;

use super::path::find_git_root;
use super::{load_workspaces, WorkspaceEntry};

const MAX_SNIPPET_BYTES: usize = 28_000;
const MAX_SNIPPETS: usize = 20;
const MAX_FILES_SCAN: usize = 48;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentMemorySnippet {
    pub label: String,
    pub body: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentMemoryReport {
    pub runtime: String,
    pub snippets: Vec<AgentMemorySnippet>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub empty_hint: Option<String>,
}

pub fn list_runtime_agent_memory(
    runtime_id: &str,
    workspace_name: Option<&str>,
) -> Result<AgentMemoryReport> {
    let doc = load_workspaces()?;
    let entry = resolve_workspace_entry(&doc, workspace_name);
    let mut snippets = Vec::new();

    match runtime_id {
        "claude-code" => {
            if let Some(entry) = entry.as_ref() {
                collect_claude_memory(&entry.path, &mut snippets);
            }
        }
        "hermes" => {
            if let Some(entry) = entry.as_ref() {
                let profile = entry.hermes_profile.trim();
                if !profile.is_empty() {
                    collect_hermes_memory(profile, &mut snippets);
                }
            }
        }
        "codex" => {
            if let Some(entry) = entry.as_ref() {
                collect_codex_memory(&entry.codex_home, &mut snippets);
            }
        }
        "openclaw" => {
            if let Some(entry) = entry.as_ref() {
                collect_openclaw_memory(&entry.openclaw_workspace, &mut snippets);
            }
        }
        "deepseek-harness" => {
            if let Some(entry) = entry.as_ref() {
                collect_dsh_memory(&entry.path, &mut snippets);
            }
        }
        other => {
            return Ok(AgentMemoryReport {
                runtime: other.to_string(),
                snippets: vec![],
                empty_hint: Some(format!("暂不支持同步 {other} 的记忆。")),
            });
        }
    }

    trim_snippets(&mut snippets);
    let empty_hint = if snippets.is_empty() {
        Some(empty_hint_for_runtime(runtime_id, entry.as_ref()))
    } else {
        None
    };

    Ok(AgentMemoryReport {
        runtime: runtime_id.to_string(),
        snippets,
        empty_hint,
    })
}

fn resolve_workspace_entry(
    doc: &super::WorkspacesDocument,
    workspace_name: Option<&str>,
) -> Option<WorkspaceEntry> {
    if let Some(name) = workspace_name.filter(|n| !n.trim().is_empty()) {
        return doc.workspaces.get(name).cloned();
    }
    if let Some(active) = doc.active.as_deref() {
        if let Some(entry) = doc.workspaces.get(active) {
            return Some(entry.clone());
        }
    }
    doc.workspaces.values().next().cloned()
}

fn empty_hint_for_runtime(runtime_id: &str, entry: Option<&WorkspaceEntry>) -> String {
    match runtime_id {
        "claude-code" => {
            if entry.is_none() {
                "还没有注册项目。先在左侧建好项目，Claude 在项目里记的内容才会出现在这里。".into()
            } else {
                "Claude 还没在这个项目里写过 auto memory（一般在 ~/.claude/projects/…/memory/）。你可以先在 Claude 里聊几句，或在本页「记一条」。".into()
            }
        }
        "hermes" => {
            if entry.is_none() {
                "还没有注册项目。Hermes 按工作区 profile 存记忆，需要先选好项目。".into()
            } else {
                "Hermes 这个 profile 里还没有 SOUL.md、USER.md、MEMORY.md 或 memories 文件夹内容。".into()
            }
        }
        "codex" => "Codex 的 memories 文件夹里还没有可读内容（在工作区的 CODEX_HOME 下）。".into(),
        "openclaw" => "OpenClaw 工作区里还没有 MEMORY.md 或 memory 笔记。".into(),
        "deepseek-harness" => {
            "DeepSeek Harness 没有单独的 memory 目录；项目里的 CLAUDE.md 会显示在这里。长期约定也可以在本页「记一条」。".into()
        }
        _ => "这个助手还没有可同步的记忆。".into(),
    }
}

fn collect_claude_memory(project_path: &Path, snippets: &mut Vec<AgentMemorySnippet>) {
    push_markdown_file(project_path.join("CLAUDE.md"), "CLAUDE.md", snippets);
    push_markdown_file(
        project_path.join("CLAUDE.local.md"),
        "CLAUDE.local.md",
        snippets,
    );

    let memory_dir = claude_auto_memory_dir(project_path);
    push_markdown_tree(&memory_dir, "Claude memory", snippets);
}

fn claude_auto_memory_dir(project_path: &Path) -> PathBuf {
    if let Some(custom) = claude_auto_memory_directory_setting(project_path) {
        return custom;
    }
    let identity = claude_project_identity_path(project_path);
    let slug = claude_storage_slug(&identity);
    crate::adapters::util::home_join(".claude/projects")
        .join(slug)
        .join("memory")
}

pub fn claude_project_identity_path(project_path: &Path) -> PathBuf {
    find_git_root(project_path)
        .or_else(|| project_path.canonicalize().ok())
        .unwrap_or_else(|| project_path.to_path_buf())
}

pub fn claude_storage_slug(path: &Path) -> String {
    path.to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

fn claude_auto_memory_directory_setting(project_path: &Path) -> Option<PathBuf> {
    for settings in [
        project_path.join(".claude/settings.local.json"),
        project_path.join(".claude/settings.json"),
        crate::adapters::util::home_join(".claude/settings.json"),
    ] {
        if let Some(path) = read_auto_memory_directory_from_settings(&settings) {
            return Some(path);
        }
    }
    None
}

fn read_auto_memory_directory_from_settings(settings_path: &Path) -> Option<PathBuf> {
    let raw = fs::read_to_string(settings_path).ok()?;
    let value: Value = serde_json::from_str(&raw).ok()?;
    let key = value
        .get("autoMemoryDirectory")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())?;
    expand_home_path(key)
}

fn expand_home_path(raw: &str) -> Option<PathBuf> {
    if raw == "~" {
        return dirs::home_dir();
    }
    if let Some(rest) = raw.strip_prefix("~/") {
        return dirs::home_dir().map(|home| home.join(rest));
    }
    let path = PathBuf::from(raw);
    if path.is_absolute() {
        Some(path)
    } else {
        None
    }
}

fn collect_hermes_memory(profile: &str, snippets: &mut Vec<AgentMemorySnippet>) {
    let home = crate::adapters::util::home_join(".hermes/profiles").join(profile);
    push_markdown_file(home.join("SOUL.md"), "SOUL.md", snippets);
    push_markdown_file(home.join("USER.md"), "USER.md", snippets);
    push_markdown_file(home.join("MEMORY.md"), "MEMORY.md", snippets);
    push_markdown_file(home.join("IDENTITY.md"), "IDENTITY.md", snippets);
    push_markdown_tree(&home.join("memories"), "Hermes memories", snippets);
}

fn collect_codex_memory(codex_home: &Path, snippets: &mut Vec<AgentMemorySnippet>) {
    push_markdown_tree(&codex_home.join("memories"), "Codex memories", snippets);
}

fn collect_openclaw_memory(workspace: &Path, snippets: &mut Vec<AgentMemorySnippet>) {
    push_markdown_file(workspace.join("USER.md"), "USER.md", snippets);
    push_markdown_file(workspace.join("MEMORY.md"), "MEMORY.md", snippets);
    push_markdown_file(workspace.join("AGENTS.md"), "AGENTS.md", snippets);
    push_markdown_tree(&workspace.join("memory"), "OpenClaw memory", snippets);
}

fn collect_dsh_memory(project_path: &Path, snippets: &mut Vec<AgentMemorySnippet>) {
    push_markdown_file(project_path.join("CLAUDE.md"), "CLAUDE.md", snippets);
    push_markdown_file(project_path.join("AGENTS.md"), "AGENTS.md", snippets);
    let home = DeepSeekHarnessAdapter::home();
    push_markdown_file(home.join("MEMORY.md"), "DSH MEMORY.md", snippets);
    push_markdown_tree(&home.join("memories"), "DSH memories", snippets);
}

fn push_markdown_tree(dir: &Path, group: &str, snippets: &mut Vec<AgentMemorySnippet>) {
    if !dir.is_dir() {
        return;
    }
    let mut files = Vec::new();
    walk_markdown_files(dir, &mut files);
    files.sort_by_key(|a| std::cmp::Reverse(a.1));
    for (path, _) in files.into_iter().take(MAX_FILES_SCAN) {
        if snippets.len() >= MAX_SNIPPETS {
            break;
        }
        let label = path
            .strip_prefix(dir)
            .ok()
            .and_then(|rel| rel.to_str())
            .map(|rel| format!("{group}/{rel}"))
            .unwrap_or_else(|| group.to_string());
        push_markdown_file(path, &label, snippets);
    }
}

fn walk_markdown_files(current: &Path, out: &mut Vec<(PathBuf, SystemTime)>) {
    let read_dir = match fs::read_dir(current) {
        Ok(dir) => dir,
        Err(_) => return,
    };
    for entry in read_dir.flatten() {
        let path = entry.path();
        let meta = entry.metadata().ok();
        let modified = meta
            .and_then(|m| m.modified().ok())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        if path.is_dir() {
            walk_markdown_files(&path, out);
        } else if path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
        {
            out.push((path, modified));
        }
    }
}

fn push_markdown_file(path: PathBuf, label: &str, snippets: &mut Vec<AgentMemorySnippet>) {
    if snippets.len() >= MAX_SNIPPETS {
        return;
    }
    let body = match read_text_cap(&path) {
        Some(text) => text,
        None => return,
    };
    if is_boilerplate_memory(&body) {
        return;
    }
    snippets.push(AgentMemorySnippet {
        label: label.to_string(),
        body,
    });
}

fn read_text_cap(path: &Path) -> Option<String> {
    let meta = fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() == 0 {
        return None;
    }
    let bytes = fs::read(path).ok()?;
    if bytes.contains(&0) {
        return None;
    }
    let mut text = String::from_utf8(bytes).ok()?;
    if text.len() > MAX_SNIPPET_BYTES {
        text.truncate(MAX_SNIPPET_BYTES);
        text.push_str("\n\n…");
    }
    Some(text.trim().to_string()).filter(|s| !s.is_empty())
}

fn is_boilerplate_memory(body: &str) -> bool {
    const MARKERS: [&str; 3] = [
        "Managed by Agent Doctor workspace.",
        "# Project memory (OpenClaw workspace)",
        "# Agents (OpenClaw workspace)",
    ];
    let meaningful: String = body
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| !MARKERS.iter().any(|mark| line.contains(mark)))
        .collect::<Vec<_>>()
        .join("\n");
    meaningful.trim().len() < 8
}

fn trim_snippets(snippets: &mut Vec<AgentMemorySnippet>) {
    if snippets.len() > MAX_SNIPPETS {
        snippets.truncate(MAX_SNIPPETS);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_slug_matches_observed_encoding() {
        let slug = claude_storage_slug(Path::new("/Users/airlu/Documents/agent-doctor"));
        assert_eq!(slug, "-Users-airlu-Documents-agent-doctor");
    }

    #[test]
    fn boilerplate_openclaw_seed_is_skipped() {
        assert!(is_boilerplate_memory(
            "# Project memory (OpenClaw workspace)\n\nManaged by Agent Doctor workspace.\n"
        ));
        assert!(!is_boilerplate_memory(
            "# Notes\n\nUser prefers concise replies.\n"
        ));
    }
}
