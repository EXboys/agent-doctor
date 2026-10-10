use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::Serialize;

const MAX_READ_BYTES: u64 = 512 * 1024;
const MAX_LIST_ENTRIES: usize = 400;
const MAX_SHEET_BYTES: u64 = 20 * 1024 * 1024;
const MAX_SHEET_ROWS: usize = 500;
const MAX_SHEET_COLS: usize = 60;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceSheet {
    name: String,
    rows: Vec<Vec<String>>,
    truncated: bool,
}

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
    is_image: bool,
    absolute_path: String,
    sheets: Option<Vec<WorkspaceSheet>>,
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

fn is_sheet_name(name: &str) -> bool {
    let ext = Path::new(name)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    matches!(ext.as_str(), "xlsx" | "xlsm" | "xlsb" | "xls" | "ods")
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

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceMatch {
    relative_path: String,
    is_dir: bool,
    /// Set when the path is outside the project; `relative_path` is then absolute.
    external: bool,
}

const MAX_FIND_DEPTH: usize = 6;
const MAX_FIND_VISITS: usize = 20_000;

/// A reply names a file the way a person would: a bare name, a short path, or
/// a full path. Find the one it means inside the project.
#[tauri::command]
pub fn find_workspace_path_command(
    root: String,
    query: String,
) -> Result<Option<WorkspaceMatch>, String> {
    let root_canon = PathBuf::from(root.trim())
        .canonicalize()
        .map_err(|_| invalid_root_message())?;
    let mut wanted = query
        .trim()
        .trim_matches(|c| c == '`' || c == '"' || c == '\'')
        .replace('\\', "/");
    if let Some(rest) = wanted.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            wanted = home.join(rest).to_string_lossy().replace('\\', "/");
        }
    }
    let wanted = wanted.trim_end_matches('/').to_string();
    if wanted.is_empty() || wanted.split('/').any(|part| part == "..") {
        return Ok(None);
    }

    let direct = if Path::new(&wanted).is_absolute() {
        PathBuf::from(&wanted)
    } else {
        root_canon.join(wanted.trim_start_matches("./"))
    };
    if let Ok(found) = direct.canonicalize() {
        if found.starts_with(&root_canon) && found != root_canon {
            return Ok(Some(WorkspaceMatch {
                relative_path: relative_from_root(&root_canon, &found),
                is_dir: found.is_dir(),
                external: false,
            }));
        }
        // An absolute path the agent named, such as /tmp/x.txt. Open it as is.
        if Path::new(&wanted).is_absolute() && found.parent().is_some() {
            return Ok(Some(WorkspaceMatch {
                relative_path: found.to_string_lossy().replace('\\', "/"),
                is_dir: found.is_dir(),
                external: true,
            }));
        }
        return Ok(None);
    }
    if Path::new(&wanted).is_absolute() {
        return Ok(None);
    }

    let suffix = format!("/{}", wanted.trim_start_matches("./"));
    let mut best: Option<WorkspaceMatch> = None;
    let mut queue = std::collections::VecDeque::from([(root_canon.clone(), 0usize)]);
    let mut visits = 0usize;
    while let Some((dir, depth)) = queue.pop_front() {
        let Ok(read_dir) = fs::read_dir(&dir) else {
            continue;
        };
        for item in read_dir.flatten() {
            visits += 1;
            if visits > MAX_FIND_VISITS {
                return Ok(best);
            }
            let path = item.path();
            let name = item.file_name().to_string_lossy().to_string();
            let is_dir = path.is_dir();
            if is_dir && should_skip_dir(&name) {
                continue;
            }
            let relative = relative_from_root(&root_canon, &path);
            if format!("/{relative}").ends_with(&suffix) {
                let shorter = best
                    .as_ref()
                    .is_none_or(|b| relative.len() < b.relative_path.len());
                if shorter {
                    best = Some(WorkspaceMatch {
                        relative_path: relative,
                        is_dir,
                        external: false,
                    });
                }
            }
            if is_dir && depth + 1 < MAX_FIND_DEPTH {
                queue.push_back((path, depth + 1));
            }
        }
        // Breadth-first: the first level that matches holds the shortest path.
        if best.is_some() && queue.front().is_none_or(|(_, d)| *d > depth) {
            return Ok(best);
        }
    }
    Ok(best)
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
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("file")
        .to_string();
    let is_image = is_image_name(&name);
    let is_sheet = is_sheet_name(&name);
    let limit = if is_sheet {
        MAX_SHEET_BYTES
    } else {
        MAX_READ_BYTES
    };
    if !is_image && size_bytes > limit {
        return Err("文件太大，问答里暂时打不开。请在系统里用其他应用查看。".to_string());
    }
    let sheets = if is_sheet {
        Some(read_sheets(&path)?)
    } else {
        None
    };

    let (content, editable) = if is_image || is_sheet {
        (String::new(), false)
    } else {
        let bytes = fs::read(&path).map_err(|_| "无法读取这个文件。".to_string())?;
        match String::from_utf8(bytes) {
            Ok(text) if !text.contains('\0') => (text, true),
            _ => (String::new(), false),
        }
    };

    let language = if is_image {
        "image".to_string()
    } else if is_sheet {
        "sheet".to_string()
    } else {
        language_from_name(&name)
    };
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
        is_image,
        absolute_path: path.to_string_lossy().to_string(),
        sheets,
        size_bytes,
    })
}

fn read_sheets(path: &Path) -> Result<Vec<WorkspaceSheet>, String> {
    use calamine::{open_workbook_auto, Data, Reader};

    let unreadable = || "这个表格打不开，可能已损坏或设置了密码。".to_string();
    let mut workbook = open_workbook_auto(path).map_err(|_| unreadable())?;
    let mut sheets = Vec::new();
    for name in workbook.sheet_names().to_owned() {
        let Ok(range) = workbook.worksheet_range(&name) else {
            continue;
        };
        let truncated = range.height() > MAX_SHEET_ROWS || range.width() > MAX_SHEET_COLS;
        let rows = range
            .rows()
            .take(MAX_SHEET_ROWS)
            .map(|row| {
                row.iter()
                    .take(MAX_SHEET_COLS)
                    .map(|cell| match cell {
                        Data::Empty => String::new(),
                        Data::Error(_) => "#ERROR".to_string(),
                        other => other.to_string(),
                    })
                    .collect()
            })
            .collect();
        sheets.push(WorkspaceSheet {
            name,
            rows,
            truncated,
        });
    }
    if sheets.is_empty() {
        return Err(unreadable());
    }
    Ok(sheets)
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
    fn finds_files_named_in_a_reply() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("ad-ask-find-{stamp}"));
        fs::create_dir_all(dir.join(".daily-digest/logs")).unwrap();
        fs::write(dir.join(".daily-digest/digest.mjs"), "x").unwrap();
        fs::write(dir.join(".daily-digest/logs/2026-10-10.md"), "x").unwrap();
        let root = dir.to_string_lossy().to_string();
        let find = |q: &str| find_workspace_path_command(root.clone(), q.into()).unwrap();

        assert_eq!(
            find("digest.mjs"),
            Some(WorkspaceMatch {
                relative_path: ".daily-digest/digest.mjs".into(),
                is_dir: false,
                external: false,
            })
        );
        let outside = std::env::temp_dir().join(format!("ad-ask-outside-{stamp}.txt"));
        fs::write(&outside, "x").unwrap();
        let hit = find(&outside.to_string_lossy()).unwrap();
        assert!(hit.external && !hit.is_dir);
        let (folder, name) = hit.relative_path.rsplit_once('/').unwrap();
        let opened = read_workspace_file_command(folder.into(), name.into()).unwrap();
        assert_eq!(opened.content, "x");
        let _ = fs::remove_file(&outside);
        assert_eq!(
            find("logs/2026-10-10.md").map(|m| m.relative_path),
            Some(".daily-digest/logs/2026-10-10.md".into())
        );
        assert_eq!(find(".daily-digest").map(|m| m.is_dir), Some(true));
        assert_eq!(find("missing.md"), None);
        assert_eq!(find("../etc/passwd"), None);
        let _ = fs::remove_dir_all(&dir);
    }

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
        let updated = read_workspace_file_command(root.clone(), "src/main.rs".into()).unwrap();
        assert!(updated.content.contains("{ }"));

        fs::write(dir.join("shot.png"), vec![0x89, b'P', b'N', b'G', 0, 0, 1]).unwrap();
        let image = read_workspace_file_command(root.clone(), "shot.png".into()).unwrap();
        assert!(image.is_image && !image.editable);
        assert!(image.absolute_path.ends_with("shot.png"));

        fs::write(dir.join("blob.bin"), vec![0u8, 1, 2]).unwrap();
        let blob = read_workspace_file_command(root.clone(), "blob.bin".into()).unwrap();
        assert!(!blob.is_image && !blob.editable && blob.sheets.is_none());

        write_test_xlsx(&dir.join("data.xlsx"));
        let book = read_workspace_file_command(root, "data.xlsx".into()).unwrap();
        let sheets = book.sheets.expect("xlsx should be read as sheets");
        assert_eq!(sheets[0].name, "Sheet1");
        assert_eq!(
            sheets[0].rows[0],
            vec!["名字".to_string(), "数量".to_string()]
        );
        assert_eq!(sheets[0].rows[1], vec!["苹果".to_string(), "3".to_string()]);
        let _ = fs::remove_dir_all(&dir);
    }

    fn write_test_xlsx(path: &Path) {
        use std::io::Write;
        let parts = [
            (
                "[Content_Types].xml",
                r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>"#,
            ),
            (
                "_rels/.rels",
                r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#,
            ),
            (
                "xl/workbook.xml",
                r#"<?xml version="1.0" encoding="UTF-8"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Sheet1" sheetId="1" r:id="rId1"/></sheets></workbook>"#,
            ),
            (
                "xl/_rels/workbook.xml.rels",
                r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#,
            ),
            (
                "xl/worksheets/sheet1.xml",
                r#"<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>名字</t></is></c><c r="B1" t="inlineStr"><is><t>数量</t></is></c></row><row r="2"><c r="A2" t="inlineStr"><is><t>苹果</t></is></c><c r="B2"><v>3</v></c></row></sheetData></worksheet>"#,
            ),
        ];
        let mut zip = zip::ZipWriter::new(fs::File::create(path).unwrap());
        for (name, body) in parts {
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(body.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }
}
