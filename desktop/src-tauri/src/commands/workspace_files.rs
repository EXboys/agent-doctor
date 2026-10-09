use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::Serialize;

const MAX_READ_BYTES: u64 = 512 * 1024;
const MAX_LIST_ENTRIES: usize = 400;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceDirEntry {
    name: String,
    relative_path: String,
    is_dir: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceFilePayload {
    relative_path: String,
    name: String,
    content: String,
    language: String,
    editable: bool,
    size_bytes: u64,
}

fn invalid_root_message() -> String {
    "当前还没有可用的文件夹，请先选好工作区或发一条消息。".to_string()
}

fn resolve_under_root(root: &str, relative: &str) -> Result<PathBuf, String> {
    let root_trim = root.trim();
    if root_trim.is_empty() || root_trim == "—" {
        return Err(invalid_root_message());
    }
    let root_canon = PathBuf::from(root_trim)
        .canonicalize()
        .map_err(|_| "当前文件夹打不开，请换一个工作区再试。".to_string())?;

    let rel = relative.trim().trim_start_matches(['/', '\\']);
    let mut target = root_canon.clone();
    if !rel.is_empty() && rel != "." {
        let rel_path = Path::new(rel);
        for component in rel_path.components() {
            match component {
                Component::ParentDir => {
                    if !target.pop() {
                        return Err("已经在最外层文件夹了。".to_string());
                    }
                }
                Component::Normal(name) => target.push(name),
                Component::CurDir => {}
                Component::RootDir | Component::Prefix(_) => {
                    return Err("路径无效。".to_string());
                }
            }
        }
    }

    let target_canon = target
        .canonicalize()
        .map_err(|_| "找不到这个文件或文件夹。".to_string())?;
    if !target_canon.starts_with(&root_canon) {
        return Err("只能查看当前文件夹里的内容。".to_string());
    }
    Ok(target_canon)
}

fn relative_from_root(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_default()
}

fn language_from_name(name: &str) -> String {
    let ext = Path::new(name)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "ts" | "mts" | "cts" => "typescript",
        "tsx" => "tsx",
        "js" | "mjs" | "cjs" => "javascript",
        "jsx" => "jsx",
        "rs" => "rust",
        "py" => "python",
        "go" => "go",
        "java" => "java",
        "kt" | "kts" => "kotlin",
        "swift" => "swift",
        "c" | "h" => "c",
        "cpp" | "cc" | "cxx" | "hpp" => "cpp",
        "cs" => "csharp",
        "rb" => "ruby",
        "php" => "php",
        "sql" => "sql",
        "sh" | "bash" | "zsh" => "bash",
        "json" => "json",
        "yaml" | "yml" => "yaml",
        "toml" => "toml",
        "md" | "markdown" => "markdown",
        "html" | "htm" => "html",
        "css" => "css",
        "scss" => "scss",
        "xml" => "xml",
        "vue" => "vue",
        "dockerfile" => "dockerfile",
        "" if name.eq_ignore_ascii_case("dockerfile") => "dockerfile",
        _ => "text",
    }
    .to_string()
}

fn should_skip_dir(name: &str) -> bool {
    matches!(
        name,
        "node_modules" | ".git" | "target" | "dist" | "build" | ".next" | "__pycache__"
    )
}

#[tauri::command]
pub fn list_workspace_dir_command(
    root: String,
    relative: Option<String>,
) -> Result<Vec<WorkspaceDirEntry>, String> {
    let rel = relative.unwrap_or_default();
    let workspace_root = PathBuf::from(root.trim())
        .canonicalize()
        .map_err(|_| invalid_root_message())?;
    let dir = resolve_under_root(&root, &rel)?;
    if !dir.is_dir() {
        return Err("这不是一个文件夹。".to_string());
    }

    let mut entries = Vec::new();
    let read_dir = fs::read_dir(&dir).map_err(|_| "无法读取这个文件夹。".to_string())?;

    for item in read_dir {
        let Ok(item) = item else { continue };
        let path = item.path();
        let name = item.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        let is_dir = path.is_dir();
        if is_dir && should_skip_dir(&name) {
            continue;
        }
        let relative_path = relative_from_root(&workspace_root, &path);
        entries.push(WorkspaceDirEntry {
            name,
            relative_path,
            is_dir,
        });
    }

    entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a
            .name
            .to_ascii_lowercase()
            .cmp(&b.name.to_ascii_lowercase()),
    });

    if entries.len() > MAX_LIST_ENTRIES {
        entries.truncate(MAX_LIST_ENTRIES);
    }
    Ok(entries)
}

#[tauri::command]
pub fn read_workspace_file_command(
    root: String,
    relative: String,
) -> Result<WorkspaceFilePayload, String> {
    let path = resolve_under_root(&root, &relative)?;
    if path.is_dir() {
        return Err("这是一个文件夹，请点进去查看。".to_string());
    }
    let metadata = fs::metadata(&path).map_err(|_| "找不到这个文件。".to_string())?;
    let size_bytes = metadata.len();
    if size_bytes > MAX_READ_BYTES {
        return Err("文件太大，问答里暂时打不开。请在系统里用其他应用查看。".to_string());
    }

    let bytes = fs::read(&path).map_err(|_| "无法读取这个文件。".to_string())?;
    let editable = !bytes.contains(&0);
    let content = if editable {
        String::from_utf8(bytes).map_err(|_| "这是二进制文件，问答里不能编辑。".to_string())?
    } else {
        String::new()
    };

    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("file")
        .to_string();
    let language = language_from_name(&name);
    let root_canon = PathBuf::from(root.trim())
        .canonicalize()
        .map_err(|_| invalid_root_message())?;
    let relative_path = relative_from_root(&root_canon, &path);

    Ok(WorkspaceFilePayload {
        relative_path,
        name,
        content,
        language,
        editable,
        size_bytes,
    })
}

#[tauri::command]
pub fn write_workspace_file_command(
    root: String,
    relative: String,
    content: String,
) -> Result<(), String> {
    let path = resolve_under_root(&root, &relative)?;
    if path.is_dir() {
        return Err("不能把一个文件夹当文件保存。".to_string());
    }
    if fs::metadata(&path).map(|m| m.len()).unwrap_or(0) > MAX_READ_BYTES {
        return Err("文件太大，问答里暂时不能保存。".to_string());
    }
    fs::write(&path, content).map_err(|_| "保存失败，请检查文件是否被占用。".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn lists_and_reads_under_root() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("ad-ask-files-{stamp}"));
        let nested = dir.join("src");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("main.rs"), "fn main() {}").unwrap();
        let root = dir.to_string_lossy().to_string();

        let entries = list_workspace_dir_command(root.clone(), Some(String::new())).unwrap();
        assert!(entries.iter().any(|e| e.name == "src" && e.is_dir));

        let file = read_workspace_file_command(root.clone(), "src/main.rs".into()).unwrap();
        assert_eq!(file.language, "rust");
        assert!(file.content.contains("fn main"));

        write_workspace_file_command(root.clone(), "src/main.rs".into(), "fn main() { }\n".into())
            .unwrap();
        let updated = read_workspace_file_command(root, "src/main.rs".into()).unwrap();
        assert!(updated.content.contains("{ }"));
        let _ = fs::remove_dir_all(&dir);
    }
}
