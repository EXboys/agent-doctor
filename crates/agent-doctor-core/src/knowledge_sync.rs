//! Sync the local knowledge folder to the signed-in TeamUps account.
//! The folder on disk stays what the assistant reads. The account stores a copy.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::skills::saved_teamups_provider_credentials;

const MAX_FILE_BYTES: u64 = 5 * 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 100 * 1024 * 1024;
const MAX_FILES: usize = 4000;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeSyncReport {
    pub state: String,
    pub synced_at_ms: Option<u64>,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileMeta {
    sha256: String,
    size: u64,
    mtime_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Action {
    Upload(String),
    Download(String),
    DeleteLocal(String),
    DeleteRemote(String),
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    enabled: bool,
    #[serde(default)]
    hold: bool,
    #[serde(default)]
    files: Vec<RemoteFile>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RemoteFile {
    path: String,
    sha256: String,
    size: u64,
    mtime_ms: u64,
}

#[derive(Debug, Deserialize)]
struct ApiError {
    error: Option<String>,
    code: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct Baseline {
    files: BTreeMap<String, BaselineFile>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct BaselineFile {
    sha256: String,
    size: u64,
    #[serde(rename = "mtimeMs")]
    mtime_ms: u64,
}

fn report(state: &str, detail: impl Into<String>) -> KnowledgeSyncReport {
    KnowledgeSyncReport {
        state: state.to_string(),
        synced_at_ms: (state == "synced").then(now_ms),
        detail: detail.into(),
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0xf) as usize] as char);
    }
    out
}

fn sha256_bytes(bytes: &[u8]) -> String {
    hex_encode(&Sha256::digest(bytes))
}

fn knowledge_root(project_path: Option<&str>) -> Result<PathBuf, String> {
    let base = match project_path.map(str::trim).filter(|p| !p.is_empty()) {
        Some(path) => {
            let path = PathBuf::from(path);
            if !path.is_dir() {
                return Err("这个项目的文件夹找不到了。".into());
            }
            path
        }
        None => dirs::home_dir().ok_or_else(|| "找不到你的个人文件夹。".to_string())?,
    };
    Ok(base.join(".agent-doctor").join("knowledge"))
}

fn normalize_remote(raw: &str) -> Option<String> {
    let trimmed = raw.trim().trim_end_matches('/').trim_end_matches(".git");
    let trimmed = trimmed.trim();
    if trimmed.is_empty() || trimmed.contains("..") {
        return None;
    }
    Some(trimmed.to_string())
}

fn git_origin(project: &Path) -> Option<String> {
    let output = Command::new("git")
        .args([
            "-C",
            &project.to_string_lossy(),
            "remote",
            "get-url",
            "origin",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    normalize_remote(&String::from_utf8_lossy(&output.stdout))
}

fn project_local_id(project: &Path) -> Result<String, String> {
    let dir = project.join(".agent-doctor");
    let file = dir.join("project-id");
    if let Ok(existing) = fs::read_to_string(&file) {
        let id = existing.trim().to_string();
        if id.len() >= 8 && id.chars().all(|c| c.is_ascii_alphanumeric()) {
            return Ok(id);
        }
    }
    fs::create_dir_all(&dir)
        .map_err(|_| "没法记下这个项目，请检查文件夹能不能写入。".to_string())?;
    let id = format!("{:x}{:x}", now_ms(), std::process::id());
    fs::write(&file, &id).map_err(|_| "没法记下这个项目，请检查文件夹能不能写入。".to_string())?;
    Ok(id)
}

fn scope_for(project_path: Option<&str>) -> Result<(String, String), String> {
    let Some(path) = project_path.map(str::trim).filter(|p| !p.is_empty()) else {
        return Ok(("global".into(), "所有项目".into()));
    };
    let project = PathBuf::from(path);
    let label = project
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "项目".into());
    if let Some(remote) = git_origin(&project) {
        return Ok((format!("git:{remote}"), label));
    }
    Ok((format!("id:{}", project_local_id(&project)?), label))
}

fn safe_relative(path: &str) -> bool {
    if path == "AGENTS.md" {
        return true;
    }
    if path.contains('\\') || path.starts_with('/') || path.contains('\0') {
        return false;
    }
    let parts: Vec<&str> = path.split('/').collect();
    if parts
        .iter()
        .any(|part| part.is_empty() || *part == "." || *part == "..")
    {
        return false;
    }
    if let Some(rest) = path.strip_prefix("wiki/") {
        return rest.ends_with(".md") && !rest.ends_with('/');
    }
    path.starts_with("raw/") && !path.ends_with('/')
}

fn collect_local(root: &Path) -> Result<BTreeMap<String, FileMeta>, String> {
    let mut files = BTreeMap::new();
    if !root.is_dir() {
        return Ok(files);
    }
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let entries = fs::read_dir(&dir).map_err(|_| "知识库文件夹打不开。".to_string())?;
        for entry in entries {
            let entry = entry.map_err(|_| "知识库文件夹打不开。".to_string())?;
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with('.') {
                continue;
            }
            let meta =
                fs::symlink_metadata(&path).map_err(|_| "知识库里有个文件读不出。".to_string())?;
            if meta.file_type().is_symlink() {
                continue;
            }
            if meta.is_dir() {
                pending.push(path);
                continue;
            }
            if !meta.is_file() {
                continue;
            }
            let relative = path
                .strip_prefix(root)
                .map_err(|_| "知识库里有个文件读不出。".to_string())?
                .components()
                .map(|c| c.as_os_str().to_string_lossy().to_string())
                .collect::<Vec<_>>()
                .join("/");
            if !safe_relative(&relative) {
                continue;
            }
            if meta.len() > MAX_FILE_BYTES {
                return Err("有的文件超过 5MB，没有同步。".into());
            }
            if files.len() >= MAX_FILES {
                return Err("文件太多，没有同步。".into());
            }
            let mut bytes = Vec::new();
            File::open(&path)
                .and_then(|mut file| file.read_to_end(&mut bytes))
                .map_err(|_| "知识库里有个文件读不出。".to_string())?;
            let mtime_ms = meta
                .modified()
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            files.insert(
                relative,
                FileMeta {
                    sha256: sha256_bytes(&bytes),
                    size: meta.len(),
                    mtime_ms,
                },
            );
        }
    }
    Ok(files)
}

/// Newer mtime wins when both sides changed. A missing file is a delete only
/// when this computer last synced that exact copy.
fn plan_sync(
    local: &BTreeMap<String, FileMeta>,
    remote: &BTreeMap<String, FileMeta>,
    baseline: &BTreeMap<String, FileMeta>,
) -> Vec<Action> {
    let mut names = BTreeMap::new();
    for key in local.keys().chain(remote.keys()) {
        names.insert(key.clone(), ());
    }
    let mut actions = Vec::new();
    for path in names.into_keys() {
        let local_file = local.get(&path);
        let remote_file = remote.get(&path);
        let base = baseline.get(&path);
        match (local_file, remote_file) {
            (Some(local_file), Some(remote_file)) if local_file.sha256 == remote_file.sha256 => {}
            (Some(local_file), Some(remote_file)) => {
                if local_file.mtime_ms >= remote_file.mtime_ms {
                    actions.push(Action::Upload(path));
                } else {
                    actions.push(Action::Download(path));
                }
            }
            (Some(local_file), None) => {
                if base.is_some_and(|base| base.sha256 == local_file.sha256) {
                    actions.push(Action::DeleteLocal(path));
                } else {
                    actions.push(Action::Upload(path));
                }
            }
            (None, Some(remote_file)) => {
                if base.is_some_and(|base| base.sha256 == remote_file.sha256) {
                    actions.push(Action::DeleteRemote(path));
                } else {
                    actions.push(Action::Download(path));
                }
            }
            (None, None) => {}
        }
    }
    actions
}

fn projected_bytes(
    local: &BTreeMap<String, FileMeta>,
    remote: &BTreeMap<String, FileMeta>,
    actions: &[Action],
) -> u64 {
    let mut bytes = remote.values().map(|file| file.size).sum::<u64>();
    for action in actions {
        match action {
            Action::Upload(path) => {
                let next = local.get(path).map(|file| file.size).unwrap_or(0);
                let previous = remote.get(path).map(|file| file.size).unwrap_or(0);
                bytes = bytes - previous + next;
            }
            Action::DeleteRemote(path) => {
                bytes = bytes.saturating_sub(remote.get(path).map(|file| file.size).unwrap_or(0));
            }
            Action::Download(_) | Action::DeleteLocal(_) => {}
        }
    }
    bytes
}

fn baseline_path(scope: &str) -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or_else(|| "找不到你的个人文件夹。".to_string())?;
    Ok(home
        .join(".agent-doctor")
        .join("knowledge-sync")
        .join(format!("{}.json", sha256_bytes(scope.as_bytes()))))
}

fn read_baseline(scope: &str) -> BTreeMap<String, FileMeta> {
    let Ok(path) = baseline_path(scope) else {
        return BTreeMap::new();
    };
    let Ok(text) = fs::read_to_string(path) else {
        return BTreeMap::new();
    };
    let Ok(parsed) = serde_json::from_str::<Baseline>(&text) else {
        return BTreeMap::new();
    };
    parsed
        .files
        .into_iter()
        .map(|(path, file)| {
            (
                path,
                FileMeta {
                    sha256: file.sha256,
                    size: file.size,
                    mtime_ms: file.mtime_ms,
                },
            )
        })
        .collect()
}

fn write_baseline(scope: &str, files: &BTreeMap<String, FileMeta>) -> Result<(), String> {
    let path = baseline_path(scope)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|_| "没法记下同步进度。".to_string())?;
    }
    let baseline = Baseline {
        files: files
            .iter()
            .map(|(path, file)| {
                (
                    path.clone(),
                    BaselineFile {
                        sha256: file.sha256.clone(),
                        size: file.size,
                        mtime_ms: file.mtime_ms,
                    },
                )
            })
            .collect(),
    };
    let text = serde_json::to_string(&baseline).map_err(|_| "没法记下同步进度。".to_string())?;
    fs::write(path, text).map_err(|_| "没法记下同步进度。".to_string())
}

fn clear_baseline(scope: &str) {
    if let Ok(path) = baseline_path(scope) {
        let _ = fs::remove_file(path);
    }
}

fn http() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|_| "连不上账号。".to_string())
}

fn query(scope: &str, label: &str, relative: Option<&str>) -> String {
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    serializer.append_pair("scope", scope);
    serializer.append_pair("label", label);
    if let Some(relative) = relative {
        serializer.append_pair("path", relative);
    }
    serializer.finish()
}

fn api_error(status: reqwest::StatusCode, body: &str) -> KnowledgeSyncReport {
    let parsed: ApiError = serde_json::from_str(body).unwrap_or(ApiError {
        error: None,
        code: None,
    });
    if parsed.code.as_deref() == Some("too_big") || status.as_u16() == 413 {
        return report(
            "too_big",
            parsed
                .error
                .unwrap_or_else(|| "这份知识库超过 100MB，没有同步。".into()),
        );
    }
    if parsed.code.as_deref() == Some("off") {
        return report("off", "账号里的同步关着。要打开，去个人中心。");
    }
    if parsed.code.as_deref() == Some("held") {
        return report(
            "held",
            "账号里的这份已删掉。要再同步，去个人中心点重新同步。",
        );
    }
    if status.as_u16() == 401 || status.as_u16() == 403 {
        return report("signed_out", "登录后，这份知识库会跟着账号到别的电脑。");
    }
    if status.as_u16() == 404 {
        return report("failed", "账号暂时没接上，这份知识库还只在这台电脑上。");
    }
    report(
        "failed",
        parsed
            .error
            .filter(|text| !text.is_empty() && !text.starts_with('<'))
            .unwrap_or_else(|| "这次没同步到账号。".into()),
    )
}

fn fetch_manifest(
    base: &str,
    license: &str,
    scope: &str,
    label: &str,
) -> Result<Manifest, KnowledgeSyncReport> {
    let url = format!(
        "{base}/api/v1/doctor/knowledge/manifest?{}",
        query(scope, label, None)
    );
    let response = http()
        .map_err(|detail| report("failed", detail))?
        .get(url)
        .header("Authorization", format!("Bearer {license}"))
        .header("Accept", "application/json")
        .send()
        .map_err(|_| report("failed", "连不上账号，这份知识库还只在这台电脑上。"))?;
    let status = response.status();
    let body = response.text().unwrap_or_default();
    if !status.is_success() {
        return Err(api_error(status, &body));
    }
    serde_json::from_str(&body).map_err(|_| report("failed", "账号返回的内容读不懂，这次没同步。"))
}

fn upload_file(
    base: &str,
    license: &str,
    scope: &str,
    label: &str,
    relative: &str,
    bytes: &[u8],
    meta: &FileMeta,
) -> Result<(), KnowledgeSyncReport> {
    let url = format!(
        "{base}/api/v1/doctor/knowledge/file?{}",
        query(scope, label, Some(relative))
    );
    let response = http()
        .map_err(|detail| report("failed", detail))?
        .put(url)
        .header("Authorization", format!("Bearer {license}"))
        .header("x-sha256", &meta.sha256)
        .header("x-mtime-ms", meta.mtime_ms.to_string())
        .header("content-type", "application/octet-stream")
        .body(bytes.to_vec())
        .send()
        .map_err(|_| report("failed", "连不上账号，这份知识库还只在这台电脑上。"))?;
    let status = response.status();
    if status.is_success() {
        return Ok(());
    }
    let body = response.text().unwrap_or_default();
    Err(api_error(status, &body))
}

fn download_file(
    base: &str,
    license: &str,
    scope: &str,
    label: &str,
    relative: &str,
    root: &Path,
) -> Result<(), KnowledgeSyncReport> {
    let url = format!(
        "{base}/api/v1/doctor/knowledge/file?{}",
        query(scope, label, Some(relative))
    );
    let response = http()
        .map_err(|detail| report("failed", detail))?
        .get(url)
        .header("Authorization", format!("Bearer {license}"))
        .send()
        .map_err(|_| report("failed", "连不上账号，这份知识库还只在这台电脑上。"))?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().unwrap_or_default();
        return Err(api_error(status, &body));
    }
    let mtime_ms = response
        .headers()
        .get("x-mtime-ms")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0);
    let bytes = response
        .bytes()
        .map_err(|_| report("failed", "账号上的文件没下完。"))?;
    if !safe_relative(relative) {
        return Err(report("failed", "账号上有个文件路径不对，这次没同步。"));
    }
    let dest = root.join(relative);
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|_| report("failed", "没法把知识库写到这台电脑。"))?;
    }
    let mut file =
        File::create(&dest).map_err(|_| report("failed", "没法把知识库写到这台电脑。"))?;
    std::io::Write::write_all(&mut file, &bytes)
        .map_err(|_| report("failed", "没法把知识库写到这台电脑。"))?;
    drop(file);
    if mtime_ms > 0 {
        let secs = (mtime_ms / 1000) as i64;
        let nanos = ((mtime_ms % 1000) * 1_000_000) as u32;
        let _ = filetime::set_file_mtime(dest, filetime::FileTime::from_unix_time(secs, nanos));
    }
    Ok(())
}

fn delete_remote(
    base: &str,
    license: &str,
    scope: &str,
    label: &str,
    relative: &str,
) -> Result<(), KnowledgeSyncReport> {
    let url = format!(
        "{base}/api/v1/doctor/knowledge/file?{}",
        query(scope, label, Some(relative))
    );
    let response = http()
        .map_err(|detail| report("failed", detail))?
        .delete(url)
        .header("Authorization", format!("Bearer {license}"))
        .send()
        .map_err(|_| report("failed", "连不上账号，这份知识库还只在这台电脑上。"))?;
    let status = response.status();
    if status.is_success() {
        return Ok(());
    }
    let body = response.text().unwrap_or_default();
    Err(api_error(status, &body))
}

fn delete_local(root: &Path, relative: &str) -> Result<(), KnowledgeSyncReport> {
    if !safe_relative(relative) {
        return Ok(());
    }
    let dest = root.join(relative);
    if dest.is_file() {
        fs::remove_file(&dest).map_err(|_| report("failed", "没法更新这台电脑上的知识库。"))?;
    }
    Ok(())
}

/// Pull and push one knowledge folder. `project_path` none is the global folder.
pub fn sync_knowledge(project_path: Option<&str>) -> KnowledgeSyncReport {
    let _guard = SYNC_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    match sync_knowledge_locked(project_path) {
        Ok(report) => report,
        Err(report) => report,
    }
}

static SYNC_LOCK: Mutex<()> = Mutex::new(());

fn sync_knowledge_locked(
    project_path: Option<&str>,
) -> Result<KnowledgeSyncReport, KnowledgeSyncReport> {
    let Some((base, license)) = saved_teamups_provider_credentials()
        .map_err(|_| report("failed", "连不上账号，这份知识库还只在这台电脑上。"))?
    else {
        return Ok(report(
            "signed_out",
            "登录后，这份知识库会跟着账号到别的电脑。",
        ));
    };
    let base = base.trim_end_matches('/').to_string();
    let (scope, label) = scope_for(project_path).map_err(|detail| report("failed", detail))?;
    let root = knowledge_root(project_path).map_err(|detail| report("failed", detail))?;
    let manifest = fetch_manifest(&base, &license, &scope, &label)?;
    if !manifest.enabled {
        return Ok(report("off", "账号里的同步关着。要打开，去个人中心。"));
    }
    if manifest.hold {
        clear_baseline(&scope);
        return Ok(report(
            "held",
            "账号里的这份已删掉。要再同步，去个人中心点重新同步。",
        ));
    }
    let local = collect_local(&root).map_err(|detail| report("failed", detail))?;
    let remote = manifest
        .files
        .into_iter()
        .filter(|file| safe_relative(&file.path))
        .map(|file| {
            (
                file.path,
                FileMeta {
                    sha256: file.sha256,
                    size: file.size,
                    mtime_ms: file.mtime_ms,
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    let baseline = read_baseline(&scope);
    if local.is_empty() && remote.is_empty() {
        return Ok(report("idle", ""));
    }
    let actions = plan_sync(&local, &remote, &baseline);
    if projected_bytes(&local, &remote, &actions) > MAX_TOTAL_BYTES {
        return Ok(report("too_big", "这份知识库超过 100MB，没有同步。"));
    }
    for action in &actions {
        match action {
            Action::Upload(path) => {
                let bytes = fs::read(root.join(path))
                    .map_err(|_| report("failed", "知识库里有个文件读不出。"))?;
                let meta = local
                    .get(path)
                    .ok_or_else(|| report("failed", "知识库里有个文件读不出。"))?;
                upload_file(&base, &license, &scope, &label, path, &bytes, meta)?;
            }
            Action::Download(path) => download_file(&base, &license, &scope, &label, path, &root)?,
            Action::DeleteRemote(path) => delete_remote(&base, &license, &scope, &label, path)?,
            Action::DeleteLocal(path) => delete_local(&root, path)?,
        }
    }
    let after = collect_local(&root).map_err(|detail| report("failed", detail))?;
    write_baseline(&scope, &after).map_err(|detail| report("failed", detail))?;
    Ok(report("synced", ""))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(sha: &str, size: u64, mtime_ms: u64) -> FileMeta {
        FileMeta {
            sha256: sha.into(),
            size,
            mtime_ms,
        }
    }

    #[test]
    fn same_bytes_need_no_transfer() {
        let mut local = BTreeMap::new();
        let mut remote = BTreeMap::new();
        local.insert("wiki/a.md".into(), meta("a", 1, 10));
        remote.insert("wiki/a.md".into(), meta("a", 1, 2));
        assert!(plan_sync(&local, &remote, &BTreeMap::new()).is_empty());
    }

    #[test]
    fn the_newer_copy_wins_when_both_changed() {
        let mut local = BTreeMap::new();
        let mut remote = BTreeMap::new();
        local.insert("wiki/a.md".into(), meta("local", 1, 50));
        remote.insert("wiki/a.md".into(), meta("remote", 1, 20));
        assert_eq!(
            plan_sync(&local, &remote, &BTreeMap::new()),
            vec![Action::Upload("wiki/a.md".into())]
        );
        local.insert("wiki/a.md".into(), meta("local", 1, 5));
        assert_eq!(
            plan_sync(&local, &remote, &BTreeMap::new()),
            vec![Action::Download("wiki/a.md".into())]
        );
    }

    #[test]
    fn a_delete_is_pushed_only_after_this_computer_synced_that_copy() {
        let mut local = BTreeMap::new();
        let mut remote = BTreeMap::new();
        let mut baseline = BTreeMap::new();
        remote.insert("wiki/a.md".into(), meta("a", 1, 10));
        baseline.insert("wiki/a.md".into(), meta("a", 1, 10));
        assert_eq!(
            plan_sync(&local, &remote, &baseline),
            vec![Action::DeleteRemote("wiki/a.md".into())]
        );

        local.insert("wiki/a.md".into(), meta("a", 1, 10));
        remote.clear();
        assert_eq!(
            plan_sync(&local, &remote, &baseline),
            vec![Action::DeleteLocal("wiki/a.md".into())]
        );
    }

    #[test]
    fn the_first_sync_keeps_a_file_that_exists_on_only_one_side() {
        let mut local = BTreeMap::new();
        local.insert("wiki/a.md".into(), meta("a", 1, 10));
        assert_eq!(
            plan_sync(&local, &BTreeMap::new(), &BTreeMap::new()),
            vec![Action::Upload("wiki/a.md".into())]
        );
    }

    #[test]
    fn git_remotes_drop_the_dot_git_suffix() {
        assert_eq!(
            normalize_remote("https://github.com/a/b.git"),
            Some("https://github.com/a/b".into())
        );
        assert_eq!(normalize_remote("https://github.com/../secret"), None);
    }
}
