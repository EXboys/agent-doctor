use serde::Serialize;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Read};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const KNOWLEDGE_DIR: &str = ".agent-doctor/knowledge";
const MAX_SOURCE_FILE_BYTES: u64 = 5 * 1024 * 1024;
const MAX_IMPORT_BYTES: u64 = 80 * 1024 * 1024;
const MAX_IMPORT_FILES: usize = 3000;
const MAX_PAGES: usize = 600;
const MAX_PAGE_BYTES: u64 = 512 * 1024;
const MAX_INDEX_CHARS: usize = 4000;
const MAX_INLINE_WIKI_CHARS: usize = 12_000;
const SKIP_DIRS: &[&str] = &[
    "node_modules",
    "target",
    "dist",
    "build",
    "__pycache__",
    "__MACOSX",
    "venv",
];

const SCHEMA: &str = r#"# Knowledge wiki rules

This folder is a knowledge wiki that an AI assistant keeps for one user. The user does not edit it by hand; you own it.

## Layout
- `raw/` holds the sources the user added: chat transcripts, documents, folders, and unpacked zip files. Read them. Never edit, move, or delete anything in `raw/`.
- `wiki/` holds the pages you write. Markdown only.
- `wiki/index.md` is the table of contents. Every page appears here once as a relative link with a one-line summary, grouped under `##` headings by topic.
- `wiki/log.md` is the history. Append one entry per run: `## YYYY-MM-DD <what was added>`, then bullets naming the pages you created or changed.

## Pages
- Start every page with `# Title`, then a two or three sentence summary.
- One topic per page: a concept, a component, a process, a decision, a person or team, a term.
- Group pages in topic folders once there are more than a handful (for example `wiki/overview/`, `wiki/components/`, `wiki/guides/`). File names are lowercase-kebab-case `.md`.
- Link related pages with relative links such as `[Billing](../components/billing.md)`.
- End each page with a `## Sources` list linking the `raw/` files it is based on, relative to the page.
- Keep facts the user can act on: names, numbers, rules, steps, decisions and the reasons for them. Skip filler.

## Adding sources
1. Read each new source completely. For a code folder, start with its README, entry points, config, and main modules.
2. Decide which existing pages the source changes and update them in place. Do not create near-duplicates.
3. Create pages for topics that do not exist yet.
4. When a source contradicts a page, keep both claims, say which source says what, and mark it with `> Conflict:`.
5. For a code project, cover what it does, project structure, architecture, key components, how to run it, and common problems.
6. For a chat transcript, keep lasting knowledge only: decisions, preferences, facts, how-tos. Drop small talk and one-off steps.
7. Update `wiki/index.md` and append to `wiki/log.md`.

## Limits
- Only write inside `wiki/`. Do not touch anything outside this folder.
- Do not install anything, start servers, or use the network. Reading files is enough.
"#;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgePage {
    path: String,
    title: String,
    modified_ms: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgePages {
    root: String,
    pages: Vec<KnowledgePage>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeImport {
    root: String,
    sources: Vec<String>,
    files: usize,
    skipped: usize,
    /// PDF or Word files whose text could not be read (scans, broken files).
    unreadable: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeContext {
    wiki: String,
    index: String,
    full: Option<String>,
}

fn knowledge_root(project_path: Option<&str>) -> Result<PathBuf, String> {
    let base = match project_path.map(str::trim).filter(|p| !p.is_empty()) {
        Some(path) => {
            let path = PathBuf::from(path);
            if !path.is_dir() {
                return Err("这个项目的文件夹找不到了，请在左边重新添加项目。".to_string());
            }
            path
        }
        None => dirs::home_dir().ok_or_else(|| "找不到你的个人文件夹。".to_string())?,
    };
    Ok(base.join(KNOWLEDGE_DIR))
}

fn prepare_root(project_path: Option<&str>) -> Result<PathBuf, String> {
    let root = knowledge_root(project_path)?;
    let cannot = |_| "没法建知识库文件夹，请检查这个文件夹能不能写入。".to_string();
    fs::create_dir_all(root.join("raw")).map_err(cannot)?;
    fs::create_dir_all(root.join("wiki")).map_err(cannot)?;
    let schema = root.join("AGENTS.md");
    if !schema.exists() {
        fs::write(&schema, SCHEMA).map_err(cannot)?;
    }
    Ok(root)
}

fn display(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

fn rel_string(base: &Path, path: &Path) -> String {
    path.strip_prefix(base)
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_default()
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

fn skip_name(name: &str) -> bool {
    name.starts_with('.') || SKIP_DIRS.contains(&name)
}

fn slug(name: &str) -> String {
    let mut out = String::new();
    for ch in name.chars() {
        if ch.is_alphanumeric() || ch == '.' || ch == '_' {
            out.push(ch);
        } else if !out.ends_with('-') {
            out.push('-');
        }
        if out.chars().count() >= 60 {
            break;
        }
    }
    let trimmed = out.trim_matches(['-', '.']).to_string();
    if trimmed.is_empty() {
        "source".to_string()
    } else {
        trimmed
    }
}

/// A fresh `raw/<stamp>-<name>` path that does not exist yet.
fn fresh_source_path(raw: &Path, name: &str) -> PathBuf {
    let stem = format!("{}-{}", now_ms(), slug(name));
    let mut candidate = raw.join(&stem);
    let mut n = 2;
    while candidate.exists() {
        candidate = raw.join(format!("{stem}-{n}"));
        n += 1;
    }
    candidate
}

#[derive(Default)]
struct Budget {
    files: usize,
    bytes: u64,
    skipped: usize,
}

impl Budget {
    fn take(&mut self, size: u64) -> bool {
        if size > MAX_SOURCE_FILE_BYTES
            || self.files >= MAX_IMPORT_FILES
            || self.bytes + size > MAX_IMPORT_BYTES
        {
            self.skipped += 1;
            return false;
        }
        self.files += 1;
        self.bytes += size;
        true
    }
}

fn copy_file(from: &Path, to: &Path, budget: &mut Budget) -> io::Result<()> {
    let size = fs::metadata(from)?.len();
    if !budget.take(size) {
        return Ok(());
    }
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(from, to)?;
    Ok(())
}

fn copy_dir(from: &Path, to: &Path, budget: &mut Budget, depth: usize) -> io::Result<()> {
    if depth > 12 {
        return Ok(());
    }
    for entry in fs::read_dir(from)?.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if skip_name(&name) {
            continue;
        }
        let path = entry.path();
        let file_type = entry.file_type()?;
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

fn extract_zip(from: &Path, to: &Path, budget: &mut Budget) -> Result<(), String> {
    let bad_zip = || "这个 zip 包打不开，可能已经损坏。".to_string();
    let file = File::open(from).map_err(|_| bad_zip())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|_| bad_zip())?;
    for i in 0..archive.len() {
        let Ok(mut entry) = archive.by_index(i) else {
            budget.skipped += 1;
            continue;
        };
        if entry.is_dir() {
            continue;
        }
        let Some(relative) = entry.enclosed_name() else {
            budget.skipped += 1;
            continue;
        };
        let hidden = relative.components().any(|c| match c {
            Component::Normal(part) => skip_name(&part.to_string_lossy()),
            _ => true,
        });
        if hidden || !budget.take(entry.size()) {
            continue;
        }
        let target = to.join(&relative);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|_| bad_zip())?;
        }
        let mut out = File::create(&target).map_err(|_| bad_zip())?;
        io::copy(&mut (&mut entry).take(MAX_SOURCE_FILE_BYTES), &mut out).map_err(|_| bad_zip())?;
    }
    Ok(())
}

const TEXT_COPY_SUFFIX: &str = ".txt";
const CONVERT_TIMEOUT: Duration = Duration::from_secs(60);
const PDF_TEXT_SCRIPT: &str = r#"ObjC.import("PDFKit");
function run(argv) {
  const doc = $.PDFDocument.alloc.initWithURL($.NSURL.fileURLWithPath(argv[0]));
  if (!doc || doc.isNil()) return "";
  const text = doc.string;
  return text && !text.isNil() ? ObjC.unwrap(text) : "";
}"#;

fn lower_ext(path: &Path) -> String {
    path.extension()
        .and_then(|s| s.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default()
}

/// Stdout of `cmd`, or `None` when it fails or runs past the timeout.
fn run_with_timeout(mut cmd: Command) -> Option<String> {
    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        buf
    });
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(_)) | Err(_) => return None,
            Ok(None) if started.elapsed() > CONVERT_TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
        }
    }
    let text = String::from_utf8_lossy(&reader.join().ok()?)
        .trim()
        .to_string();
    (!text.is_empty()).then_some(text)
}

/// Plain text of a PDF or Word file, read with tools every Mac already has.
fn document_text(path: &Path) -> Option<String> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let mut cmd = match lower_ext(path).as_str() {
        "pdf" => {
            let mut cmd = Command::new("osascript");
            cmd.args(["-l", "JavaScript", "-e", PDF_TEXT_SCRIPT])
                .arg(path);
            cmd
        }
        "doc" | "docx" | "rtf" | "odt" => {
            let mut cmd = Command::new("textutil");
            cmd.args(["-convert", "txt", "-stdout"]).arg(path);
            cmd
        }
        _ => return None,
    };
    cmd.stdin(Stdio::null());
    run_with_timeout(cmd)
}

/// Agents cannot read PDF or Word files, and left alone they burn the run trying
/// to convert them. Put a `.txt` copy beside each one; returns how many failed.
fn add_text_copies(path: &Path, depth: usize) -> usize {
    if depth > 12 {
        return 0;
    }
    if path.is_dir() {
        let Ok(entries) = fs::read_dir(path) else {
            return 0;
        };
        return entries
            .flatten()
            .map(|entry| add_text_copies(&entry.path(), depth + 1))
            .sum();
    }
    if !matches!(
        lower_ext(path).as_str(),
        "pdf" | "doc" | "docx" | "rtf" | "odt"
    ) {
        return 0;
    }
    let mut copy = path.as_os_str().to_owned();
    copy.push(TEXT_COPY_SUFFIX);
    let copy = PathBuf::from(copy);
    if copy.exists() {
        return 0;
    }
    match document_text(path) {
        Some(text) if fs::write(&copy, &text).is_ok() => 0,
        _ => 1,
    }
}

fn is_zip(path: &Path) -> bool {
    path.extension()
        .and_then(|s| s.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("zip"))
}

fn page_title(path: &Path) -> Option<String> {
    let file = File::open(path).ok()?;
    for line in BufReader::new(file).lines().take(40) {
        let line = line.ok()?;
        if let Some(title) = line.trim().strip_prefix("# ") {
            let title = title.trim();
            if !title.is_empty() {
                return Some(title.to_string());
            }
        }
    }
    None
}

fn collect_pages(wiki: &Path, dir: &Path, depth: usize, out: &mut Vec<KnowledgePage>) {
    if depth > 6 || out.len() >= MAX_PAGES {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            collect_pages(wiki, &path, depth + 1, out);
            continue;
        }
        if !name.to_ascii_lowercase().ends_with(".md") {
            continue;
        }
        let modified_ms = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let stem = name.trim_end_matches(".md").to_string();
        out.push(KnowledgePage {
            path: rel_string(wiki, &path),
            title: page_title(&path).unwrap_or(stem),
            modified_ms,
        });
        if out.len() >= MAX_PAGES {
            return;
        }
    }
}

fn list_pages(wiki: &Path) -> Vec<KnowledgePage> {
    let mut pages = Vec::new();
    collect_pages(wiki, wiki, 0, &mut pages);
    let rank = |path: &str| match path {
        "index.md" => 0,
        "log.md" => 2,
        _ => 1,
    };
    pages.sort_by(|a, b| {
        rank(&a.path)
            .cmp(&rank(&b.path))
            .then_with(|| a.path.cmp(&b.path))
    });
    pages
}

fn resolve_page(wiki: &Path, relative: &str) -> Result<PathBuf, String> {
    let rel = Path::new(relative.trim());
    if rel.as_os_str().is_empty()
        || !rel.components().all(|c| matches!(c, Component::Normal(_)))
        || !relative.to_ascii_lowercase().ends_with(".md")
    {
        return Err("找不到这一页。".to_string());
    }
    Ok(wiki.join(rel))
}

fn clip_chars(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text.to_string(),
    }
}

#[tauri::command]
pub fn knowledge_pages_command(project_path: Option<String>) -> Result<KnowledgePages, String> {
    let root = knowledge_root(project_path.as_deref())?;
    Ok(KnowledgePages {
        pages: list_pages(&root.join("wiki")),
        root: display(&root),
    })
}

#[tauri::command]
pub fn knowledge_read_page_command(
    project_path: Option<String>,
    path: String,
) -> Result<String, String> {
    let wiki = knowledge_root(project_path.as_deref())?.join("wiki");
    let page = resolve_page(&wiki, &path)?;
    let size = fs::metadata(&page)
        .map_err(|_| "找不到这一页。".to_string())?
        .len();
    if size > MAX_PAGE_BYTES {
        return Err("这一页太长，这里打不开。".to_string());
    }
    fs::read_to_string(&page).map_err(|_| "这一页读不出来。".to_string())
}

/// Before a rebuild; also gives sources added before text copies existed their copy.
#[tauri::command]
pub async fn knowledge_prepare_command(project_path: Option<String>) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let root = prepare_root(project_path.as_deref())?;
        add_text_copies(&root.join("raw"), 0);
        Ok(display(&root))
    })
    .await
    .map_err(|err| err.to_string())?
}

/// Copying a big folder and reading PDFs takes a while; keep it off the UI thread.
#[tauri::command]
pub async fn knowledge_import_command(
    project_path: Option<String>,
    paths: Vec<String>,
) -> Result<KnowledgeImport, String> {
    tauri::async_runtime::spawn_blocking(move || import_paths(project_path.as_deref(), paths))
        .await
        .map_err(|err| err.to_string())?
}

fn import_paths(project_path: Option<&str>, paths: Vec<String>) -> Result<KnowledgeImport, String> {
    let root = prepare_root(project_path)?;
    let raw = root.join("raw");
    let mut budget = Budget::default();
    let mut sources = Vec::new();
    let mut unreadable = 0;
    for item in paths {
        let from = PathBuf::from(item.trim());
        let name = from
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let before = budget.files;
        let result = if from.is_dir() {
            let to = fresh_source_path(&raw, &name);
            copy_dir(&from, &to, &mut budget, 0)
                .map_err(|_| "有个文件夹没能复制进来。".to_string())
                .map(|_| to)
        } else if is_zip(&from) {
            let stem = from
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or(name);
            let to = fresh_source_path(&raw, &stem);
            extract_zip(&from, &to, &mut budget).map(|_| to)
        } else if from.is_file() {
            let to = fresh_source_path(&raw, &name);
            copy_file(&from, &to, &mut budget)
                .map_err(|_| "有个文件没能复制进来。".to_string())
                .map(|_| to)
        } else {
            budget.skipped += 1;
            continue;
        };
        let to = result?;
        if budget.files > before {
            unreadable += add_text_copies(&to, 0);
            sources.push(rel_string(&root, &to));
        }
    }
    Ok(KnowledgeImport {
        root: display(&root),
        sources,
        files: budget.files,
        skipped: budget.skipped,
        unreadable,
    })
}

#[tauri::command]
pub fn knowledge_add_source_command(
    project_path: Option<String>,
    name: String,
    content: String,
) -> Result<KnowledgeImport, String> {
    let root = prepare_root(project_path.as_deref())?;
    let mut target = fresh_source_path(&root.join("raw"), &name);
    target.set_extension("md");
    fs::write(&target, content).map_err(|_| "资料没能存下来，请再试一次。".to_string())?;
    Ok(KnowledgeImport {
        sources: vec![rel_string(&root, &target)],
        root: display(&root),
        files: 1,
        skipped: 0,
        unreadable: 0,
    })
}

#[tauri::command]
pub fn knowledge_save_page_command(
    project_path: Option<String>,
    path: String,
    content: String,
) -> Result<(), String> {
    let root = prepare_root(project_path.as_deref())?;
    let page = resolve_page(&root.join("wiki"), &path)?;
    if let Some(parent) = page.parent() {
        fs::create_dir_all(parent).map_err(|_| "这一页没能保存。".to_string())?;
    }
    fs::write(&page, content).map_err(|_| "这一页没能保存。".to_string())
}

/// What a chat turn should know about this wiki; `None` when it has no pages yet.
#[tauri::command]
pub fn knowledge_context_command(
    project_path: Option<String>,
) -> Result<Option<KnowledgeContext>, String> {
    let Ok(root) = knowledge_root(project_path.as_deref()) else {
        return Ok(None);
    };
    let wiki = root.join("wiki");
    let pages: Vec<KnowledgePage> = list_pages(&wiki)
        .into_iter()
        .filter(|page| page.path != "log.md")
        .collect();
    if pages.is_empty() {
        return Ok(None);
    }
    let index = fs::read_to_string(wiki.join("index.md")).unwrap_or_default();
    let mut catalog = clip_chars(index.trim(), MAX_INDEX_CHARS);
    let unlisted: Vec<String> = pages
        .iter()
        .filter(|page| page.path != "index.md" && !index.contains(&page.path))
        .map(|page| format!("- [{}]({})", page.title, page.path))
        .collect();
    if !unlisted.is_empty() {
        if !catalog.is_empty() {
            catalog.push_str("\n\n");
        }
        catalog.push_str(&unlisted.join("\n"));
    }

    let mut full = String::new();
    for page in &pages {
        let Ok(text) = fs::read_to_string(wiki.join(&page.path)) else {
            continue;
        };
        full.push_str(&format!("<<< {} >>>\n{}\n\n", page.path, text.trim()));
        if full.chars().count() > MAX_INLINE_WIKI_CHARS {
            full.clear();
            break;
        }
    }

    Ok(Some(KnowledgeContext {
        wiki: display(&wiki),
        index: catalog,
        full: (!full.is_empty()).then(|| full.trim_end().to_string()),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ad-knowledge-{tag}-{}", now_ms()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn imports_folder_zip_and_lists_pages() {
        let project = temp_dir("project");
        let project_str = Some(display(&project));

        let docs = project.join("docs-src");
        fs::create_dir_all(docs.join("node_modules")).unwrap();
        fs::write(docs.join("a.md"), "hello").unwrap();
        fs::write(docs.join("node_modules/skip.js"), "x").unwrap();

        let zip_path = project.join("pack.zip");
        {
            let mut writer = zip::ZipWriter::new(File::create(&zip_path).unwrap());
            let options = zip::write::SimpleFileOptions::default();
            writer.start_file("inner/b.txt", options).unwrap();
            writer.write_all(b"world").unwrap();
            writer.start_file("__MACOSX/._b.txt", options).unwrap();
            writer.write_all(b"junk").unwrap();
            writer.finish().unwrap();
        }

        let report = import_paths(
            project_str.as_deref(),
            vec![display(&docs), display(&zip_path)],
        )
        .unwrap();
        assert_eq!(report.files, 2);
        assert_eq!(report.sources.len(), 2);
        let root = project.join(KNOWLEDGE_DIR);
        assert!(root.join("AGENTS.md").is_file());
        assert!(root.join(&report.sources[1]).join("inner/b.txt").is_file());

        assert!(knowledge_context_command(project_str.clone())
            .unwrap()
            .is_none());
        knowledge_save_page_command(
            project_str.clone(),
            "guides/start.md".into(),
            "# Start\nhi".into(),
        )
        .unwrap();
        let pages = knowledge_pages_command(project_str.clone()).unwrap().pages;
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].title, "Start");
        let context = knowledge_context_command(project_str.clone())
            .unwrap()
            .unwrap();
        assert!(context.index.contains("guides/start.md"));
        assert!(context.full.unwrap().contains("hi"));

        assert!(knowledge_read_page_command(project_str, "../AGENTS.md".into()).is_err());
        let _ = fs::remove_dir_all(project);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn writes_text_copy_beside_documents() {
        let project = temp_dir("docs");
        let src = project.join("src");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("note.rtf"), r"{\rtf1\ansi hello wiki}").unwrap();
        fs::write(src.join("scan.pdf"), b"not a real pdf").unwrap();

        let report = import_paths(Some(&display(&project)), vec![display(&src)]).unwrap();
        let copied = project.join(KNOWLEDGE_DIR).join(&report.sources[0]);
        let text = fs::read_to_string(copied.join("note.rtf.txt")).unwrap();
        assert!(text.contains("hello wiki"));
        assert!(!copied.join("scan.pdf.txt").exists());
        assert_eq!(report.unreadable, 1);
        let _ = fs::remove_dir_all(project);
    }
}
