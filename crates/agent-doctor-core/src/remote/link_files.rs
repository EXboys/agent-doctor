//! Minimal project-folder file helpers for the link daemon.
//! Kept here so the loopback service does not depend on the desktop crate.

use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::{bail, Context, Result};

use super::link_protocol::{LinkDirEntry, LinkFilePayload};

const MAX_READ_BYTES: u64 = 512 * 1024;
const MAX_LIST_ENTRIES: usize = 400;

fn resolve_under_root(root: &str, relative: &str) -> Result<PathBuf> {
    let root_trim = root.trim();
    if root_trim.is_empty() {
        bail!("当前还没有可用的文件夹，请先选好项目。");
    }
    let root_canon = PathBuf::from(root_trim)
        .canonicalize()
        .context("当前文件夹打不开，请换一个项目再试。")?;

    let rel = relative.trim().trim_start_matches(['/', '\\']);
    let mut target = root_canon.clone();
    if !rel.is_empty() && rel != "." {
        for component in Path::new(rel).components() {
            match component {
                Component::ParentDir => {
                    if !target.pop() {
                        bail!("已经在最外层文件夹了。");
                    }
                }
                Component::Normal(name) => target.push(name),
                Component::CurDir => {}
                Component::RootDir | Component::Prefix(_) => bail!("路径无效。"),
            }
        }
    }

    let target_canon = target.canonicalize().context("找不到这个文件或文件夹。")?;
    if !target_canon.starts_with(&root_canon) {
        bail!("只能查看当前文件夹里的内容。");
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
        "md" | "markdown" => "markdown",
        "json" => "json",
        "yaml" | "yml" => "yaml",
        "toml" => "toml",
        "html" | "htm" => "html",
        "css" => "css",
        "sh" | "bash" | "zsh" => "bash",
        _ => "text",
    }
    .to_string()
}

fn is_image_name(name: &str) -> bool {
    let ext = Path::new(name)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    matches!(
        ext.as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" | "svg" | "avif"
    )
}

pub fn list_dir(root: &str, relative: &str) -> Result<Vec<LinkDirEntry>> {
    let dir = resolve_under_root(root, relative)?;
    if !dir.is_dir() {
        bail!("这不是一个文件夹。");
    }
    let root_canon = PathBuf::from(root.trim())
        .canonicalize()
        .context("当前文件夹打不开")?;
    let mut entries = Vec::new();
    let read = fs::read_dir(&dir).context("打不开这个文件夹")?;
    for item in read.flatten() {
        if entries.len() >= MAX_LIST_ENTRIES {
            break;
        }
        let path = item.path();
        let name = item.file_name().to_string_lossy().to_string();
        if name == ".DS_Store" {
            continue;
        }
        entries.push(LinkDirEntry {
            name,
            relative_path: relative_from_root(&root_canon, &path),
            is_dir: path.is_dir(),
        });
    }
    entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    });
    Ok(entries)
}

pub fn read_file(root: &str, relative: &str) -> Result<LinkFilePayload> {
    let path = resolve_under_root(root, relative)?;
    if path.is_dir() {
        bail!("这是一个文件夹，请点开里面的文件。");
    }
    let meta = fs::metadata(&path).context("读不了这个文件")?;
    let size = meta.len();
    let name = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| relative.to_string());
    let is_image = is_image_name(&name);
    let absolute_path = path.to_string_lossy().to_string();
    if is_image {
        return Ok(LinkFilePayload {
            relative_path: relative.replace('\\', "/"),
            name,
            content: String::new(),
            language: "text".into(),
            editable: false,
            is_image: true,
            absolute_path,
            size_bytes: size,
        });
    }
    if size > MAX_READ_BYTES {
        bail!("文件太大，暂时不能在这里打开。");
    }
    let bytes = fs::read(&path).context("读不了这个文件")?;
    if bytes.contains(&0) {
        return Ok(LinkFilePayload {
            relative_path: relative.replace('\\', "/"),
            name,
            content: String::new(),
            language: "text".into(),
            editable: false,
            is_image: false,
            absolute_path,
            size_bytes: size,
        });
    }
    let content = String::from_utf8_lossy(&bytes).into_owned();
    Ok(LinkFilePayload {
        relative_path: relative.replace('\\', "/"),
        name: name.clone(),
        content,
        language: language_from_name(&name),
        editable: true,
        is_image: false,
        absolute_path,
        size_bytes: size,
    })
}

pub fn write_file(root: &str, relative: &str, content: &str) -> Result<()> {
    let path = resolve_under_root(root, relative)?;
    if path.is_dir() {
        bail!("不能把一个文件夹当文件保存。");
    }
    if fs::metadata(&path).map(|m| m.len()).unwrap_or(0) > MAX_READ_BYTES {
        bail!("文件太大，暂时不能保存。");
    }
    fs::write(&path, content).context("保存失败，请检查文件是否被占用。")?;
    Ok(())
}

/// Browse absolute directories on this computer (for "add project on B").
pub fn list_absolute_dir(path: &str) -> Result<Vec<LinkDirEntry>> {
    let trimmed = path.trim();
    let dir = if trimmed.is_empty() || trimmed == "~" {
        dirs::home_dir().context("找不到用户主目录")?
    } else if let Some(rest) = trimmed.strip_prefix("~/") {
        dirs::home_dir().context("找不到用户主目录")?.join(rest)
    } else {
        PathBuf::from(trimmed)
    };
    let dir = dir
        .canonicalize()
        .with_context(|| format!("找不到文件夹：{trimmed}"))?;
    if !dir.is_dir() {
        bail!("这不是一个文件夹。");
    }
    let mut entries = Vec::new();
    for item in fs::read_dir(&dir).context("打不开这个文件夹")?.flatten() {
        if entries.len() >= MAX_LIST_ENTRIES {
            break;
        }
        let path = item.path();
        let name = item.file_name().to_string_lossy().to_string();
        if name.starts_with('.') && name != ".." {
            // Keep hidden folders out of the beginner picker.
            continue;
        }
        entries.push(LinkDirEntry {
            name,
            relative_path: path.to_string_lossy().replace('\\', "/"),
            is_dir: path.is_dir(),
        });
    }
    entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    });
    Ok(entries)
}
