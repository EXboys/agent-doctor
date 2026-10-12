//! Link the one global skills folder and the one project skills folder into the
//! directory each assistant actually reads.
//!
//! Real files live in `~/.agent-doctor/skills` and `<project>/.agent-doctor/skills`.
//! Claude, Cursor, DeepSeek, and OpenClaw keep those two layers in different
//! folders. Codex and Hermes each read one isolated folder, so both layers are
//! linked there and the project skill wins when the name matches.
//! A link is refreshed when the app opens and after a skill is installed or removed.
//! Replies do not scan skills. That work stays off the path that waits for an answer.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::adapters::util::home_join;
use crate::workspace::{load_workspaces, WorkspaceEntry};
use crate::DeepSeekHarnessAdapter;

/// Link skills into the directories `runtime` will read for this workspace.
/// A missing workspace only publishes global skills. Failures to link one skill
/// are skipped so a chat turn still starts.
pub fn publish_skills_for_turn(runtime: &str, workspace_name: Option<&str>) -> Result<usize> {
    let name = workspace_name
        .map(str::trim)
        .filter(|name| !name.is_empty());
    let doc = name.and_then(|_| load_workspaces().ok());
    let entry = name.and_then(|name| {
        doc.as_ref()
            .and_then(|doc| doc.workspaces.get(name))
            .cloned()
    });
    let cache = crate::skills::load_local_skills_layout()
        .ok()
        .map(|layout| layout.skills_dir);
    let user = user_skill_root(runtime);
    Ok(publish_with_roots(
        runtime,
        entry.as_ref(),
        user.as_deref(),
        cache.as_deref(),
        true,
    ))
}

fn user_skill_root(runtime: &str) -> Option<PathBuf> {
    let path = match runtime {
        "claude-code" => home_join(".claude/skills"),
        "codex" => home_join(".codex/skills"),
        "cursor" => home_join(".cursor/skills"),
        "hermes" => home_join(".hermes/skills"),
        "openclaw" => home_join(".openclaw/skills"),
        "deepseek-harness" => DeepSeekHarnessAdapter::home().join("skills"),
        _ => return None,
    };
    Some(path)
}

enum LiveSkills {
    /// User-level skills and this project's skills stay in different folders.
    Split {
        global: PathBuf,
        project: Option<PathBuf>,
    },
    /// The process only reads one skills folder (isolated Codex or Hermes profile).
    Merged { root: PathBuf },
}

fn live_skills(
    runtime: &str,
    entry: Option<&WorkspaceEntry>,
    user_skills: &Path,
) -> Option<LiveSkills> {
    match runtime {
        "codex" => Some(match entry {
            Some(entry) if !entry.codex_home.as_os_str().is_empty() => LiveSkills::Merged {
                root: entry.codex_home.join("skills"),
            },
            _ => LiveSkills::Split {
                global: user_skills.to_path_buf(),
                project: None,
            },
        }),
        "hermes" => Some(match entry {
            Some(entry) if !entry.hermes_profile.is_empty() => LiveSkills::Merged {
                root: home_join(".hermes/profiles")
                    .join(&entry.hermes_profile)
                    .join("skills"),
            },
            _ => LiveSkills::Split {
                global: user_skills.to_path_buf(),
                project: None,
            },
        }),
        "claude-code" => Some(LiveSkills::Split {
            global: user_skills.to_path_buf(),
            project: entry.map(|entry| entry.path.join(".claude/skills")),
        }),
        "cursor" => Some(LiveSkills::Split {
            global: user_skills.to_path_buf(),
            project: entry.map(|entry| entry.path.join(".cursor/skills")),
        }),
        "deepseek-harness" => Some(LiveSkills::Split {
            global: user_skills.to_path_buf(),
            project: entry.map(|entry| entry.path.join(".dsh/skills")),
        }),
        "openclaw" => Some(LiveSkills::Split {
            global: user_skills.to_path_buf(),
            project: entry.and_then(|entry| {
                if entry.openclaw_workspace.as_os_str().is_empty() {
                    None
                } else {
                    Some(entry.openclaw_workspace.join("skills"))
                }
            }),
        }),
        _ => None,
    }
}

fn project_skill_roots(entry: &WorkspaceEntry) -> Vec<PathBuf> {
    let mut roots = vec![
        entry.path.join(".agent-doctor/skills"),
        entry.path.join(".claude/skills"),
        entry.path.join(".cursor/skills"),
        entry.path.join(".dsh/skills"),
    ];
    if !entry.openclaw_workspace.as_os_str().is_empty() {
        roots.push(entry.openclaw_workspace.join("skills"));
    }
    if !entry.codex_home.as_os_str().is_empty() {
        roots.push(entry.codex_home.join("skills"));
    }
    if !entry.hermes_profile.is_empty() {
        roots.push(
            home_join(".hermes/profiles")
                .join(&entry.hermes_profile)
                .join("skills"),
        );
    }
    roots
}

/// `user_skills` is that assistant's own global skills folder. `cache` is the shared
/// `~/.agent-doctor/skills` store. Project folders on `entry` are included when set.
fn publish_with_roots(
    runtime: &str,
    entry: Option<&WorkspaceEntry>,
    user_skills: Option<&Path>,
    cache: Option<&Path>,
    include_project: bool,
) -> usize {
    let Some(user_skills) = user_skills else {
        return 0;
    };
    let Some(live) = live_skills(runtime, entry, user_skills) else {
        return 0;
    };

    // Canonical store first, then older real folders that were copied before links.
    let mut global_roots = Vec::new();
    if let Some(cache) = cache {
        global_roots.push(cache.to_path_buf());
    }
    global_roots.push(user_skills.to_path_buf());
    let global = index_skills(&global_roots);
    let project = if include_project {
        entry
            .map(project_skill_roots)
            .map(|roots| index_skills(&roots))
            .unwrap_or_default()
    } else {
        BTreeMap::new()
    };
    let mut allowed = global_roots;
    if include_project {
        if let Some(entry) = entry {
            allowed.extend(project_skill_roots(entry));
        }
    }

    match live {
        LiveSkills::Merged { root } => {
            // Project skills replace a global skill with the same name, so the
            // global copy is not linked first and then overwritten.
            let mut global_only = global.clone();
            for id in project.keys() {
                global_only.remove(id);
            }
            link_map(&global_only, &root, &allowed) + link_map(&project, &root, &allowed)
        }
        LiveSkills::Split {
            global: dest,
            project: project_dest,
        } => {
            let mut linked = link_map(&global, &dest, &allowed);
            if include_project {
                if let Some(dest) = project_dest {
                    linked += link_map(&project, &dest, &allowed);
                }
            }
            linked
        }
    }
}

fn index_skills(roots: &[PathBuf]) -> BTreeMap<String, PathBuf> {
    let mut out = BTreeMap::new();
    for root in roots {
        scan_skills(root, &mut out);
    }
    out
}

fn scan_skills(root: &Path, out: &mut BTreeMap<String, PathBuf>) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if !skill_id_ok(&name) || !is_real_dir(&path) {
            continue;
        }
        if path.join("SKILL.md").is_file() {
            out.entry(name).or_insert(path);
            continue;
        }
        let Ok(children) = fs::read_dir(&path) else {
            continue;
        };
        for child in children.flatten() {
            let child_path = child.path();
            let child_name = child.file_name().to_string_lossy().to_string();
            if skill_id_ok(&child_name)
                && is_real_dir(&child_path)
                && child_path.join("SKILL.md").is_file()
            {
                out.entry(child_name).or_insert(child_path);
            }
        }
    }
}

fn skill_id_ok(id: &str) -> bool {
    !id.is_empty()
        && !id.starts_with('.')
        && !id.contains('/')
        && !id.contains('\\')
        && !id.contains("..")
}

fn is_real_dir(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .map(|meta| meta.is_dir() && !meta.file_type().is_symlink())
        .unwrap_or(false)
}

const RUNTIMES: [&str; 6] = [
    "claude-code",
    "codex",
    "cursor",
    "hermes",
    "openclaw",
    "deepseek-harness",
];

/// Refresh every assistant after the user installs or deletes a skill.
/// Startup uses [`repair_skill_links_at_launch`] so protected folders are left alone.
pub fn repair_skill_links() -> usize {
    let doc = load_workspaces().unwrap_or_default();
    let mut linked = repair_global_only();
    for name in doc.workspaces.keys() {
        for runtime in RUNTIMES {
            linked += publish_skills_for_turn(runtime, Some(name)).unwrap_or(0);
        }
    }
    linked
}

/// Refresh links without reading project folders that would raise a folder-access prompt.
/// Global skills still link into each assistant's own directory and into isolated homes.
pub fn repair_skill_links_at_launch() -> usize {
    let doc = load_workspaces().unwrap_or_default();
    let mut linked = repair_global_only();
    for (name, entry) in &doc.workspaces {
        if path_needs_folder_permission(&entry.path) {
            linked += repair_global_only_for(name);
            continue;
        }
        for runtime in RUNTIMES {
            linked += publish_skills_for_turn(runtime, Some(name)).unwrap_or(0);
        }
    }
    linked
}

fn repair_global_only() -> usize {
    let mut linked = 0usize;
    for runtime in RUNTIMES {
        linked += publish_skills_for_turn(runtime, None).unwrap_or(0);
    }
    linked
}

fn repair_global_only_for(workspace_name: &str) -> usize {
    let mut linked = 0usize;
    for runtime in RUNTIMES {
        linked += publish_global_for_workspace(runtime, workspace_name);
    }
    linked
}

fn publish_global_for_workspace(runtime: &str, workspace_name: &str) -> usize {
    let doc = load_workspaces().ok();
    let entry = doc
        .as_ref()
        .and_then(|doc| doc.workspaces.get(workspace_name))
        .cloned();
    let cache = crate::skills::load_local_skills_layout()
        .ok()
        .map(|layout| layout.skills_dir);
    let user = user_skill_root(runtime);
    publish_with_roots(
        runtime,
        entry.as_ref(),
        user.as_deref(),
        cache.as_deref(),
        false,
    )
}

/// True when reading or creating this path can raise a system folder-access prompt.
/// The check uses the path text only and does not touch the folder.
pub(crate) fn path_needs_folder_permission(path: &Path) -> bool {
    let Some(home) = dirs::home_dir() else {
        return false;
    };
    let Ok(rest) = path.strip_prefix(&home) else {
        return false;
    };
    let mut parts = rest.components();
    let Some(first) = parts.next() else {
        return false;
    };
    let name = first.as_os_str().to_string_lossy();
    const PROTECTED: &[&str] = &["Documents", "Desktop", "Downloads", "文稿", "桌面", "下载"];
    if PROTECTED.iter().any(|item| name == *item) {
        return true;
    }
    name == "Library"
        && parts.next().map(|part| part.as_os_str())
            == Some(std::ffi::OsStr::new("Mobile Documents"))
}

/// Remove links (and older copied folders) for one skill. Does not delete the
/// real folder in `~/.agent-doctor/skills` or `<project>/.agent-doctor/skills`.
pub(crate) fn forget_skill_links(
    scope: &str,
    project_name: Option<&str>,
    runtime: Option<&str>,
    skill_id: &str,
) -> usize {
    if !skill_id_ok(skill_id) {
        return 0;
    }
    let doc = load_workspaces().unwrap_or_default();
    let project = project_name.and_then(|name| doc.workspaces.get(name.trim()).cloned());
    let runtimes: Vec<&str> = if scope == "agent" {
        vec![runtime.unwrap_or("")]
    } else {
        RUNTIMES.to_vec()
    };
    let mut removed = 0usize;
    for runtime in runtimes {
        let Some(user) = user_skill_root(runtime) else {
            continue;
        };
        if scope == "global" || scope == "agent" {
            if let Some(live) = live_skills(runtime, None, &user) {
                removed += forget_at(scope, skill_id, &live);
            }
        }
        let workspace = if scope == "project" {
            project.clone().into_iter().collect::<Vec<_>>()
        } else {
            doc.workspaces.values().cloned().collect::<Vec<_>>()
        };
        for entry in workspace {
            if let Some(live) = live_skills(runtime, Some(&entry), &user) {
                removed += forget_at(scope, skill_id, &live);
            }
        }
    }
    removed
}

fn forget_at(scope: &str, skill_id: &str, live: &LiveSkills) -> usize {
    match live {
        LiveSkills::Split { global, project } => match scope {
            "global" | "agent" => remove_skill_slot(&global.join(skill_id), true),
            "project" => project
                .as_ref()
                .map(|root| remove_skill_slot(&root.join(skill_id), true))
                .unwrap_or(0),
            _ => 0,
        },
        LiveSkills::Merged { root } => {
            // A real folder here may belong to the other layer. Only a link is safe to drop.
            remove_skill_slot(&root.join(skill_id), false)
        }
    }
}

/// `real_too` also deletes a copied skill folder. A merged home only drops links.
fn remove_skill_slot(path: &Path, real_too: bool) -> usize {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return 0;
    };
    if meta.file_type().is_symlink() {
        return usize::from(fs::remove_file(path).is_ok());
    }
    if real_too && meta.is_dir() && path.join("SKILL.md").is_file() {
        return usize::from(fs::remove_dir_all(path).is_ok());
    }
    0
}

fn link_map(skills: &BTreeMap<String, PathBuf>, dest_root: &Path, allowed: &[PathBuf]) -> usize {
    let mut linked = 0usize;
    for (id, source) in skills {
        if !skill_id_ok(id) {
            continue;
        }
        let target = dest_root.join(id);
        if same_dir(source, &target) {
            continue;
        }
        // Point at the real folder. A path that escapes the skill store is not linked.
        let Ok(src) = fs::canonicalize(source) else {
            continue;
        };
        if !allowed.iter().any(|root| {
            fs::canonicalize(root)
                .ok()
                .is_some_and(|root| src.starts_with(&root))
        }) {
            continue;
        }
        if let Ok(root) = fs::canonicalize(dest_root) {
            if src.starts_with(&root) {
                continue;
            }
        }
        if link_skill(&src, &target) {
            linked += 1;
        }
    }
    linked
}

fn same_dir(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// Returns true when a new link (or Windows copy) was created.
fn link_skill(source: &Path, target: &Path) -> bool {
    if same_dir(source, target) {
        return false;
    }
    if let Some(parent) = target.parent() {
        if fs::create_dir_all(parent).is_err() {
            return false;
        }
    }
    if target.exists() || fs::symlink_metadata(target).is_ok() {
        let Ok(meta) = fs::symlink_metadata(target) else {
            return false;
        };
        if !meta.file_type().is_symlink() {
            return false;
        }
        if same_dir(source, target) {
            return false;
        }
        if fs::remove_file(target).is_err() {
            return false;
        }
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(source, target).is_ok()
    }
    #[cfg(not(unix))]
    {
        copy_dir(source, target).is_ok()
    }
}

#[cfg(not(unix))]
fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            continue;
        }
        let dest = to.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir(&entry.path(), &dest)?;
        } else if file_type.is_file() {
            fs::copy(entry.path(), dest)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skill(dir: &Path, id: &str, body: &str) {
        let folder = dir.join(id);
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("SKILL.md"), body).unwrap();
    }

    fn entry(tmp: &Path, codex_home: &Path) -> WorkspaceEntry {
        WorkspaceEntry {
            path: tmp.join("proj"),
            hermes_profile: String::new(),
            codex_home: codex_home.to_path_buf(),
            openclaw_agent_id: String::new(),
            openclaw_workspace: PathBuf::new(),
        }
    }

    #[test]
    fn codex_turn_sees_global_cache_and_project_skills() {
        let tmp = tempfile::tempdir().unwrap();
        let user = tmp.path().join("user-skills");
        let cache = tmp.path().join("cache");
        let project = tmp.path().join("proj");
        let home = tmp.path().join("codex-home");
        skill(&user, "shared", "# shared\n");
        skill(&cache, "mall", "# mall\n");
        skill(&project.join(".cursor/skills"), "local", "# local\n");
        skill(&project.join(".agent-doctor/skills"), "notes", "# notes\n");
        skill(&user, "calc", "# global calc\n");
        skill(&project.join(".claude/skills"), "calc", "# project calc\n");

        let workspace = entry(tmp.path(), &home);
        let linked = publish_with_roots("codex", Some(&workspace), Some(&user), Some(&cache), true);
        assert_eq!(linked, 5);
        assert_eq!(
            fs::read_to_string(home.join("skills/shared/SKILL.md")).unwrap(),
            "# shared\n"
        );
        assert_eq!(
            fs::read_to_string(home.join("skills/mall/SKILL.md")).unwrap(),
            "# mall\n"
        );
        assert_eq!(
            fs::read_to_string(home.join("skills/local/SKILL.md")).unwrap(),
            "# local\n"
        );
        assert_eq!(
            fs::read_to_string(home.join("skills/calc/SKILL.md")).unwrap(),
            "# project calc\n"
        );
        assert_eq!(
            fs::read_to_string(home.join("skills/notes/SKILL.md")).unwrap(),
            "# notes\n"
        );
        assert_eq!(
            publish_with_roots("codex", Some(&workspace), Some(&user), Some(&cache), true),
            0
        );
    }

    #[test]
    fn claude_keeps_global_skills_out_of_the_project() {
        let tmp = tempfile::tempdir().unwrap();
        let user = tmp.path().join("user-skills");
        let cache = tmp.path().join("cache");
        let project = tmp.path().join("proj");
        skill(&user, "shared", "# shared\n");
        skill(&cache, "mall", "# mall\n");
        skill(&project.join(".cursor/skills"), "local", "# local\n");

        let workspace = entry(tmp.path(), &tmp.path().join("unused-codex"));
        let linked = publish_with_roots(
            "claude-code",
            Some(&workspace),
            Some(&user),
            Some(&cache),
            true,
        );
        assert_eq!(linked, 2);
        assert!(user.join("mall/SKILL.md").is_file());
        assert!(!user.join("local").exists());
        assert!(project.join(".claude/skills/local/SKILL.md").is_file());
        assert!(!project.join(".claude/skills/shared").exists());
        assert!(!project.join(".claude/skills/mall").exists());
    }

    #[test]
    fn a_real_skill_folder_is_left_in_place() {
        let tmp = tempfile::tempdir().unwrap();
        let user = tmp.path().join("user-skills");
        let home = tmp.path().join("codex-home");
        skill(&user, "shared", "# new\n");
        skill(&home.join("skills"), "shared", "# already here\n");

        let workspace = entry(tmp.path(), &home);
        assert_eq!(
            publish_with_roots("codex", Some(&workspace), Some(&user), None, true),
            0
        );
        assert_eq!(
            fs::read_to_string(home.join("skills/shared/SKILL.md")).unwrap(),
            "# already here\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn repairs_a_dangling_skill_link() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = tmp.path().join("cache");
        let user = tmp.path().join("user-skills");
        let home = tmp.path().join("codex-home");
        skill(&cache, "mall", "# mall\n");
        let workspace = entry(tmp.path(), &home);
        assert!(
            publish_with_roots("codex", Some(&workspace), Some(&user), Some(&cache), true) >= 1
        );
        fs::remove_file(home.join("skills/mall")).unwrap();
        std::os::unix::fs::symlink(tmp.path().join("missing"), home.join("skills/mall")).unwrap();
        assert!(
            publish_with_roots("codex", Some(&workspace), Some(&user), Some(&cache), true) >= 1
        );
        assert_eq!(
            fs::read_to_string(home.join("skills/mall/SKILL.md")).unwrap(),
            "# mall\n"
        );
    }

    #[test]
    fn protected_folders_are_recognized_without_reading_them() {
        let Some(home) = dirs::home_dir() else {
            return;
        };
        assert!(path_needs_folder_permission(&home.join("Documents/demo")));
        assert!(path_needs_folder_permission(&home.join("Desktop/demo")));
        assert!(path_needs_folder_permission(&home.join("Downloads/demo")));
        assert!(path_needs_folder_permission(
            &home.join("Library/Mobile Documents/com~apple~CloudDocs/demo")
        ));
        assert!(!path_needs_folder_permission(&home.join("code/demo")));
        assert!(!path_needs_folder_permission(
            &home.join(".agent-doctor/skills/demo")
        ));
    }

    #[test]
    fn launch_style_link_does_not_open_the_project_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = tmp.path().join("cache");
        let user = tmp.path().join("user-skills");
        let project = tmp.path().join("proj");
        skill(&cache, "mall", "# mall\n");
        skill(&project.join(".agent-doctor/skills"), "local", "# local\n");
        let workspace = entry(tmp.path(), &tmp.path().join("codex-home"));
        let linked = publish_with_roots(
            "claude-code",
            Some(&workspace),
            Some(&user),
            Some(&cache),
            false,
        );
        assert_eq!(linked, 1);
        assert!(user.join("mall/SKILL.md").is_file());
        assert!(!project.join(".claude").exists());
    }
}
