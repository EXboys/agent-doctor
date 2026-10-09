//! Install a local skill folder into the global, project, or agent skills directory.

use std::fs;
use std::path::{Path, PathBuf};

use crate::adapters::util::home_join;
use crate::workspace::{load_workspaces, WorkspaceEntry};
use crate::DeepSeekHarnessAdapter;

const MAX_FILES: usize = 4_000;
const MAX_BYTES: u64 = 80 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillInstallReport {
    pub installed: Vec<String>,
    pub replaced: Vec<String>,
    pub skipped: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillInstallError {
    NotASkill,
    NoAgent,
    NoProject,
    UnknownScope,
    TooBig,
    NotFound,
    Copy(String),
}

/// `scope` is `global`, `project`, or `agent`.
/// A project install needs `project_name`. An agent install needs `runtime`.
pub fn install_local_skills(
    scope: &str,
    project_name: Option<&str>,
    runtime: Option<&str>,
    sources: &[PathBuf],
) -> Result<SkillInstallReport, SkillInstallError> {
    let mut roots = Vec::new();
    for source in sources {
        roots.extend(skill_roots(source));
    }
    if roots.is_empty() {
        return Err(SkillInstallError::NotASkill);
    }

    let mut installed = Vec::new();
    let mut replaced = Vec::new();
    for root in roots {
        let id = skill_id(&root);
        let targets = destinations(scope, project_name, runtime, &id)?;
        if targets.is_empty() {
            return Err(if scope == "agent" {
                SkillInstallError::NoAgent
            } else {
                SkillInstallError::NoProject
            });
        }
        let mut did_replace = false;
        for target in targets {
            if same_dir(&root, &target) {
                continue;
            }
            did_replace |= copy_skill(&root, &target)?;
        }
        if did_replace {
            replaced.push(id);
        } else {
            installed.push(id);
        }
    }
    Ok(SkillInstallReport {
        installed,
        replaced,
        skipped: 0,
    })
}

/// Removes the skill from the selected scope. A global delete also removes the shared copy.
pub fn remove_local_skill(
    scope: &str,
    project_name: Option<&str>,
    runtime: Option<&str>,
    skill_id: &str,
) -> Result<usize, SkillInstallError> {
    let skill_id = safe_skill_id(skill_id)?;
    let targets = destinations(scope, project_name, runtime, skill_id)?;
    let mut removed = 0usize;
    for target in &targets {
        let Some(root) = target.parent() else {
            continue;
        };
        removed += remove_skill_tree(root, skill_id)?;
    }
    if scope == "global" {
        if let Ok(layout) = crate::skills::load_local_skills_layout() {
            removed += remove_skill_tree(&layout.skills_dir, skill_id)?;
        }
    }
    if removed == 0 {
        Err(SkillInstallError::NotFound)
    } else {
        Ok(removed)
    }
}

fn safe_skill_id(skill_id: &str) -> Result<&str, SkillInstallError> {
    let id = skill_id.trim();
    if id.is_empty()
        || id.contains('/')
        || id.contains('\\')
        || id.contains("..")
        || id.starts_with('.')
    {
        return Err(SkillInstallError::NotFound);
    }
    Ok(id)
}

/// Deletes `root/<id>` and `root/<category>/<id>` when each one is a skill folder or a link.
pub fn remove_skill_tree(root: &Path, skill_id: &str) -> Result<usize, SkillInstallError> {
    let mut removed = remove_if_skill(&root.join(skill_id))?;
    let Ok(entries) = fs::read_dir(root) else {
        return Ok(removed);
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            removed += remove_if_skill(&path.join(skill_id))?;
        }
    }
    Ok(removed)
}

fn remove_if_skill(path: &Path) -> Result<usize, SkillInstallError> {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return Ok(0);
    };
    if meta.file_type().is_symlink() || path.join("SKILL.md").is_file() {
        remove_path(path)?;
        return Ok(1);
    }
    Ok(0)
}

fn destinations(
    scope: &str,
    project_name: Option<&str>,
    runtime: Option<&str>,
    skill_id: &str,
) -> Result<Vec<PathBuf>, SkillInstallError> {
    match scope {
        "global" => {
            let targets = global_targets(skill_id);
            if targets.is_empty() {
                Err(SkillInstallError::NoAgent)
            } else {
                Ok(targets)
            }
        }
        "agent" => {
            let runtime = runtime.unwrap_or("");
            match agent_target(runtime, skill_id) {
                Some(path) => Ok(vec![path]),
                None => Err(SkillInstallError::NoAgent),
            }
        }
        "project" => {
            let name = project_name.unwrap_or("").trim();
            if name.is_empty() {
                return Err(SkillInstallError::NoProject);
            }
            let doc = load_workspaces().map_err(|err| SkillInstallError::Copy(err.to_string()))?;
            let Some(entry) = doc.workspaces.get(name) else {
                return Err(SkillInstallError::NoProject);
            };
            Ok(project_targets(entry, skill_id))
        }
        _ => Err(SkillInstallError::UnknownScope),
    }
}

fn global_targets(skill_id: &str) -> Vec<PathBuf> {
    let homes = [
        (".claude", ".claude/skills"),
        (".codex", ".codex/skills"),
        (".cursor", ".cursor/skills"),
        (".hermes", ".hermes/skills"),
        (".openclaw", ".openclaw/skills"),
    ];
    let mut targets = Vec::new();
    for (home, skills) in homes {
        if home_join(home).is_dir() {
            targets.push(home_join(skills).join(skill_id));
        }
    }
    if DeepSeekHarnessAdapter::home().is_dir() {
        targets.push(DeepSeekHarnessAdapter::home().join("skills").join(skill_id));
    }
    targets
}

fn agent_target(runtime: &str, skill_id: &str) -> Option<PathBuf> {
    let (home, skills) = match runtime {
        "claude-code" => (home_join(".claude"), home_join(".claude/skills")),
        "codex" => (home_join(".codex"), home_join(".codex/skills")),
        "cursor" => (home_join(".cursor"), home_join(".cursor/skills")),
        "hermes" => (home_join(".hermes"), home_join(".hermes/skills")),
        "openclaw" => (home_join(".openclaw"), home_join(".openclaw/skills")),
        "deepseek-harness" => {
            let home = DeepSeekHarnessAdapter::home();
            let skills = home.join("skills");
            if !home.is_dir() {
                return None;
            }
            return Some(skills.join(skill_id));
        }
        _ => return None,
    };
    home.is_dir().then(|| skills.join(skill_id))
}

fn project_targets(entry: &WorkspaceEntry, skill_id: &str) -> Vec<PathBuf> {
    let mut targets = vec![
        entry.path.join(".claude/skills").join(skill_id),
        entry.path.join(".cursor/skills").join(skill_id),
        entry.path.join(".dsh/skills").join(skill_id),
    ];
    if entry.openclaw_workspace.components().count() > 0 {
        targets.push(entry.openclaw_workspace.join("skills").join(skill_id));
    }
    if entry.codex_home.components().count() > 0 {
        targets.push(entry.codex_home.join("skills").join(skill_id));
    }
    targets
}

/// A skill folder contains `SKILL.md`. A parent folder (or an unpacked zip) may hold several.
pub fn skill_roots(path: &Path) -> Vec<PathBuf> {
    if !path.is_dir() {
        return Vec::new();
    }
    if path.join("SKILL.md").is_file() {
        return vec![path.to_path_buf()];
    }
    let mut found = Vec::new();
    let Ok(entries) = fs::read_dir(path) else {
        return found;
    };
    for entry in entries.flatten() {
        let child = entry.path();
        if !child.is_dir() {
            continue;
        }
        if child.join("SKILL.md").is_file() {
            found.push(child);
            continue;
        }
        let Ok(nested) = fs::read_dir(&child) else {
            continue;
        };
        for inner in nested.flatten() {
            let dir = inner.path();
            if dir.is_dir() && dir.join("SKILL.md").is_file() {
                found.push(dir);
            }
        }
    }
    found
}

fn skill_id(root: &Path) -> String {
    let from_file = read_skill_name(root);
    let raw = from_file
        .as_deref()
        .filter(|name| !name.is_empty())
        .or_else(|| root.file_name().and_then(|name| name.to_str()))
        .unwrap_or("skill");
    let cleaned = sanitize_id(raw);
    if cleaned.is_empty() {
        "skill".to_string()
    } else {
        cleaned
    }
}

fn read_skill_name(root: &Path) -> Option<String> {
    let raw = fs::read_to_string(root.join("SKILL.md")).ok()?;
    let block = raw.strip_prefix("---")?.split_once("\n---")?.0;
    for line in block.lines() {
        let line = line.trim();
        let Some(value) = line.strip_prefix("name:") else {
            continue;
        };
        let value = value.trim().trim_matches('"').trim();
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }
    None
}

fn sanitize_id(raw: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for ch in raw.chars() {
        let mapped = if ch.is_ascii_alphanumeric() {
            dash = false;
            Some(ch.to_ascii_lowercase())
        } else if ch == '-'
            || ch == '_'
            || ch == '.'
            || ch == '/'
            || ch == '\\'
            || ch.is_whitespace()
        {
            if dash || out.is_empty() {
                None
            } else {
                dash = true;
                Some(if ch == '_' || ch == '.' { ch } else { '-' })
            }
        } else {
            None
        };
        if let Some(ch) = mapped {
            out.push(ch);
        }
        if out.len() >= 64 {
            break;
        }
    }
    out.trim_end_matches(['-', '_', '.']).to_string()
}

fn same_dir(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// Returns whether an existing skill was replaced.
fn copy_skill(from: &Path, to: &Path) -> Result<bool, SkillInstallError> {
    let parent = to
        .parent()
        .ok_or_else(|| SkillInstallError::Copy("no parent".into()))?;
    fs::create_dir_all(parent).map_err(|err| SkillInstallError::Copy(err.to_string()))?;
    let staging = parent.join(format!(
        ".{}-installing",
        to.file_name().unwrap_or_default().to_string_lossy()
    ));
    if staging.exists() {
        remove_path(&staging)?;
    }
    let mut budget = Budget::default();
    copy_dir(from, &staging, &mut budget, 0)?;
    let replaced = to.exists() || to.symlink_metadata().is_ok();
    if replaced {
        remove_path(to)?;
    }
    fs::rename(&staging, to).map_err(|err| SkillInstallError::Copy(err.to_string()))?;
    Ok(replaced)
}

fn remove_path(path: &Path) -> Result<(), SkillInstallError> {
    let meta =
        fs::symlink_metadata(path).map_err(|err| SkillInstallError::Copy(err.to_string()))?;
    if meta.is_dir() && !meta.file_type().is_symlink() {
        fs::remove_dir_all(path).map_err(|err| SkillInstallError::Copy(err.to_string()))
    } else {
        fs::remove_file(path).map_err(|err| SkillInstallError::Copy(err.to_string()))
    }
}

#[derive(Default)]
struct Budget {
    files: usize,
    bytes: u64,
}

fn copy_dir(
    from: &Path,
    to: &Path,
    budget: &mut Budget,
    depth: usize,
) -> Result<(), SkillInstallError> {
    if depth > 12 {
        return Ok(());
    }
    fs::create_dir_all(to).map_err(|err| SkillInstallError::Copy(err.to_string()))?;
    for entry in fs::read_dir(from)
        .map_err(|err| SkillInstallError::Copy(err.to_string()))?
        .flatten()
    {
        let name = entry.file_name().to_string_lossy().to_string();
        if skip_name(&name) {
            continue;
        }
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|err| SkillInstallError::Copy(err.to_string()))?;
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            copy_dir(&path, &to.join(&name), budget, depth + 1)?;
        } else if file_type.is_file() {
            copy_file(&path, &to.join(&name), budget)?;
        }
    }
    Ok(())
}

fn copy_file(from: &Path, to: &Path, budget: &mut Budget) -> Result<(), SkillInstallError> {
    let size = fs::metadata(from)
        .map_err(|err| SkillInstallError::Copy(err.to_string()))?
        .len();
    budget.files += 1;
    budget.bytes += size;
    if budget.files > MAX_FILES || budget.bytes > MAX_BYTES {
        return Err(SkillInstallError::TooBig);
    }
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent).map_err(|err| SkillInstallError::Copy(err.to_string()))?;
    }
    fs::copy(from, to).map_err(|err| SkillInstallError::Copy(err.to_string()))?;
    Ok(())
}

fn skip_name(name: &str) -> bool {
    name == ".git" || name == "__MACOSX" || name == "node_modules" || name.starts_with('.')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "ad-skill-{tag}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_skill(dir: &Path, name: &str) {
        fs::create_dir_all(dir).unwrap();
        fs::write(
            dir.join("SKILL.md"),
            format!("---\nname: {name}\n---\n\n# {name}\n"),
        )
        .unwrap();
    }

    #[test]
    fn installs_a_skill_folder_and_replaces_it() {
        let root = temp("one");
        let source = root.join("incoming");
        write_skill(&source, "Calculator");
        fs::write(source.join("notes.txt"), "hello").unwrap();
        let dest_root = root.join("agent").join("skills");
        let dest = dest_root.join("calculator");
        fs::create_dir_all(&dest).unwrap();
        fs::write(dest.join("SKILL.md"), "old").unwrap();

        let replaced = copy_skill(&source, &dest).unwrap();
        assert!(replaced);
        let text = fs::read_to_string(dest.join("SKILL.md")).unwrap();
        assert!(text.contains("name: Calculator"));
        assert_eq!(fs::read_to_string(dest.join("notes.txt")).unwrap(), "hello");
        assert!(!dest_root.join(".calculator-installing").exists());
    }

    #[test]
    fn finds_one_skill_or_several_in_a_parent_folder() {
        let root = temp("roots");
        let single = root.join("single");
        write_skill(&single, "solo");
        assert_eq!(skill_roots(&single), vec![single.clone()]);

        let parent = root.join("pack");
        write_skill(&parent.join("alpha"), "alpha");
        write_skill(&parent.join("wrap").join("beta"), "beta");
        let mut found = skill_roots(&parent);
        found.sort();
        assert_eq!(
            found,
            vec![parent.join("alpha"), parent.join("wrap").join("beta")]
        );
        assert_eq!(skill_id(&parent.join("alpha")), "alpha");
        assert_eq!(skill_id(&single), "solo");
    }

    #[test]
    fn project_targets_follow_the_project() {
        let root = temp("project");
        let entry = WorkspaceEntry {
            path: root.join("repo"),
            hermes_profile: String::new(),
            codex_home: root.join("codex"),
            openclaw_agent_id: String::new(),
            openclaw_workspace: root.join("openclaw"),
        };
        let targets = project_targets(&entry, "demo");
        assert!(targets
            .iter()
            .any(|path| path.ends_with(".claude/skills/demo")));
        assert!(targets
            .iter()
            .any(|path| path.ends_with(".dsh/skills/demo")));
        assert!(targets
            .iter()
            .any(|path| path.ends_with("openclaw/skills/demo")));
        assert!(targets
            .iter()
            .any(|path| path.ends_with("codex/skills/demo")));
    }

    #[test]
    fn drops_names_that_are_not_safe_folder_names() {
        assert_eq!(sanitize_id("../Etc/Passwd"), "etc-passwd");
        assert_eq!(sanitize_id("..."), "");
        assert_eq!(sanitize_id("My Skill"), "my-skill");
    }

    #[test]
    fn removes_a_skill_and_a_nested_copy() {
        let root = temp("remove");
        write_skill(&root.join("calculator"), "calculator");
        write_skill(&root.join("tools").join("calculator"), "calculator");
        fs::create_dir_all(root.join("notes")).unwrap();
        assert_eq!(remove_skill_tree(&root, "calculator").unwrap(), 2);
        assert!(!root.join("calculator").exists());
        assert!(!root.join("tools").join("calculator").exists());
        assert!(root.join("notes").is_dir());
        assert_eq!(remove_skill_tree(&root, "calculator").unwrap(), 0);
    }
}
