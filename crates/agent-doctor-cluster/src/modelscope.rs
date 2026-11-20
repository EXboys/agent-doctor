//! Mainland model downloads from ModelScope.
//!
//! exo and llama.cpp both speak the Hugging Face protocol. ModelScope does not,
//! so when the China download route is on we fetch the files ourselves and hand
//! each engine a local directory.

use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::Deserialize;

const API_ROOT: &str = "https://www.modelscope.cn";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RemoteFile {
    pub path: String,
    pub size: u64,
}

#[derive(Deserialize)]
struct ListResponse {
    #[serde(rename = "Data")]
    data: Option<ListData>,
}

#[derive(Deserialize)]
struct ListData {
    #[serde(rename = "Files")]
    files: Option<Vec<ListedFile>>,
}

#[derive(Deserialize)]
struct ListedFile {
    #[serde(rename = "Path")]
    path: String,
    #[serde(rename = "Size", default)]
    size: u64,
    #[serde(rename = "Type")]
    kind: String,
}

/// `org/name` or `org/name:QUANT`.
pub(crate) fn split_model_spec(model_id: &str) -> Result<(String, Option<String>)> {
    let model_id = model_id.trim();
    let (repo, quant) = match model_id.split_once(':') {
        Some((repo, quant)) => (repo.trim(), Some(quant.trim().to_string())),
        None => (model_id, None),
    };
    if !valid_repo(repo) {
        bail!("这个模型名称不能用。");
    }
    if quant
        .as_deref()
        .is_some_and(|q| q.is_empty() || q.contains('/') || q.contains('\\'))
    {
        bail!("这个模型规格不能用。");
    }
    Ok((repo.to_string(), quant))
}

fn valid_repo(repo: &str) -> bool {
    let mut parts = repo.split('/');
    let Some(owner) = parts.next() else {
        return false;
    };
    let Some(name) = parts.next() else {
        return false;
    };
    parts.next().is_none()
        && !owner.is_empty()
        && !name.is_empty()
        && !owner.contains("..")
        && !name.contains("..")
        && !repo.contains('\\')
        && !repo.contains(' ')
}

/// exo stores a model at `{models}/{org--name}` (`ModelId.normalize` swaps `/` for `--`).
pub(crate) fn exo_model_dirname(repo: &str) -> String {
    repo.replace('/', "--")
}

pub(crate) fn exo_models_root() -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os("EXO_DEFAULT_MODELS_DIR") {
        return Ok(PathBuf::from(dir));
    }
    if let Some(home) = std::env::var_os("EXO_HOME") {
        return Ok(PathBuf::from(home).join("models"));
    }
    let user = dirs::home_dir().context("无法定位本机用户目录")?;
    if cfg!(target_os = "linux") {
        let data = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| user.join(".local").join("share"));
        Ok(data.join("exo").join("models"))
    } else {
        Ok(user.join(".exo").join("models"))
    }
}

fn llama_model_dir(repo: &str) -> Result<PathBuf> {
    let user = dirs::home_dir().context("无法定位本机用户目录")?;
    Ok(user
        .join(".agent-doctor")
        .join("models")
        .join("llama")
        .join(exo_model_dirname(repo)))
}

/// Keep GGUF shards for one quant, first shard first.
pub(crate) fn select_gguf_shards(paths: &[String], quant: &str) -> Vec<String> {
    let needle = quant.to_ascii_lowercase();
    let matched: Vec<String> = paths
        .iter()
        .filter(|path| {
            let Some(name) = path.rsplit(['/', '\\']).next() else {
                return false;
            };
            let name = name.to_ascii_lowercase();
            name.ends_with(".gguf") && name.contains(&needle)
        })
        .cloned()
        .collect();
    let whole: Vec<String> = matched
        .iter()
        .filter(|path| !is_split_gguf(path))
        .cloned()
        .collect();
    let mut chosen = if whole.is_empty() { matched } else { whole };
    chosen.sort_by_key(|a| a.to_ascii_lowercase());
    chosen
}

fn is_split_gguf(path: &str) -> bool {
    path.rsplit(['/', '\\'])
        .next()
        .unwrap_or(path)
        .to_ascii_lowercase()
        .contains("-of-")
}

/// True when a previous download finished and no partial file is left behind.
pub(crate) fn llama_weights_ready(model_id: &str) -> bool {
    let Ok((repo, _)) = split_model_spec(model_id) else {
        return false;
    };
    let Ok(dir) = llama_model_dir(&repo) else {
        return false;
    };
    let Ok(entries) = fs::read_dir(dir) else {
        return false;
    };
    let mut gguf = false;
    let mut partial = false;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        if name.ends_with(".partial") {
            partial = true;
        }
        if name.ends_with(".gguf") && entry.metadata().is_ok_and(|meta| meta.len() > 1024 * 1024) {
            gguf = true;
        }
    }
    gguf && !partial
}

fn safe_relative(rel: &str) -> Option<PathBuf> {
    if rel.is_empty() || rel.starts_with('/') || rel.starts_with('\\') || rel.contains('\\') {
        return None;
    }
    let mut out = PathBuf::new();
    for seg in rel.split('/') {
        if seg.is_empty() || seg == "." || seg == ".." || seg.contains('\0') {
            return None;
        }
        out.push(seg);
    }
    if out.as_os_str().is_empty() {
        None
    } else {
        Some(out)
    }
}

pub(crate) fn ensure_llama_gguf<F, C>(
    model_id: &str,
    on_progress: &mut F,
    is_cancelled: &C,
) -> Result<PathBuf>
where
    F: FnMut(u64, Option<u64>, &str),
    C: Fn() -> bool,
{
    let (repo, quant) = split_model_spec(model_id)?;
    let Some(quant) = quant else {
        bail!("这个模型规格不能用。");
    };
    let dest = llama_model_dir(&repo)?;
    let files = list_files(&repo)?;
    let wanted = select_gguf_shards(
        &files.iter().map(|f| f.path.clone()).collect::<Vec<_>>(),
        &quant,
    );
    if wanted.is_empty() {
        bail!("国内源里没有找到这个规格的模型文件。");
    }
    let selected: Vec<RemoteFile> = wanted
        .iter()
        .filter_map(|path| files.iter().find(|f| f.path == *path).cloned())
        .collect();
    download_files(&repo, &dest, &selected, on_progress, is_cancelled)?;
    let first = dest.join(safe_relative(&selected[0].path).context("模型文件路径无效")?);
    Ok(first)
}

/// Weights plus the small config and tokenizer files exo reads from disk.
/// Skips `original/` pytorch trees, pictures, and readmes that make a download look stuck.
pub(crate) fn exo_snapshot_files(files: &[RemoteFile]) -> Vec<RemoteFile> {
    files
        .iter()
        .filter(|file| exo_file_needed(&file.path))
        .cloned()
        .collect()
}

fn exo_file_needed(path: &str) -> bool {
    if path.split('/').any(|seg| {
        let seg = seg.to_ascii_lowercase();
        seg == "original" || seg == ".git" || seg == "__pycache__" || seg == "onnx"
    }) {
        return false;
    }
    let name = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
    if name.is_empty() || name.starts_with('.') {
        return false;
    }
    if name.ends_with(".safetensors") || name.ends_with(".safetensors.index.json") {
        return true;
    }
    const KEEP: &[&str] = &[
        "config.json",
        "generation_config.json",
        "tokenizer.json",
        "tokenizer_config.json",
        "tokenizer.model",
        "special_tokens_map.json",
        "added_tokens.json",
        "vocab.json",
        "merges.txt",
        "chat_template.jinja",
        "preprocessor_config.json",
    ];
    KEEP.contains(&name.as_str()) || name.ends_with(".tiktoken")
}

fn has_safetensor_weight(files: &[RemoteFile]) -> bool {
    files.iter().any(|file| {
        let name = file
            .path
            .rsplit('/')
            .next()
            .unwrap_or(&file.path)
            .to_ascii_lowercase();
        name.ends_with(".safetensors")
    })
}

pub(crate) fn ensure_exo_snapshot<F, C>(
    model_id: &str,
    on_progress: &mut F,
    is_cancelled: &C,
) -> Result<PathBuf>
where
    F: FnMut(u64, Option<u64>, &str),
    C: Fn() -> bool,
{
    let (repo, _) = split_model_spec(model_id)?;
    let dest = exo_models_root()?.join(exo_model_dirname(&repo));
    let listed = list_files(&repo)?;
    if listed.is_empty() {
        bail!("国内源里没有找到这个模型。可以换成 Hugging Face 再试。");
    }
    let files = exo_snapshot_files(&listed);
    if !has_safetensor_weight(&files) {
        bail!("国内源里没有这个模型的权重。可以换成 Hugging Face 再试。");
    }
    download_files(&repo, &dest, &files, on_progress, is_cancelled)?;
    Ok(dest)
}

fn list_files(repo: &str) -> Result<Vec<RemoteFile>> {
    let client = http_client()?;
    let url = format!("{API_ROOT}/api/v1/models/{repo}/repo/files?Recursive=true&Revision=master");
    let resp = client
        .get(&url)
        .send()
        .context("连不上国内模型源，请检查网络后再试。")?;
    if resp.status().as_u16() == 429 {
        bail!("国内模型源暂时限流了，过一会儿再试。");
    }
    if !resp.status().is_success() {
        bail!("国内模型源没有返回文件列表，请过一会儿再试。");
    }
    let body: ListResponse = resp.json().context("国内模型源的文件列表读不出来。")?;
    let files = body.data.and_then(|d| d.files).unwrap_or_default();
    let mut out = Vec::new();
    for file in files {
        if file.kind != "blob" {
            continue;
        }
        if safe_relative(&file.path).is_none() {
            continue;
        }
        out.push(RemoteFile {
            path: file.path,
            size: file.size,
        });
    }
    Ok(out)
}

fn download_files<F, C>(
    repo: &str,
    dest: &Path,
    files: &[RemoteFile],
    on_progress: &mut F,
    is_cancelled: &C,
) -> Result<()>
where
    F: FnMut(u64, Option<u64>, &str),
    C: Fn() -> bool,
{
    fs::create_dir_all(dest).context("无法创建模型目录")?;
    let total: u64 = files.iter().map(|f| f.size).sum();
    let total = if total > 0 { Some(total) } else { None };
    let mut done = 0u64;
    on_progress(0, total, "model");
    let client = http_client()?;
    for file in files {
        if is_cancelled() {
            bail!("已取消下载。");
        }
        let rel = safe_relative(&file.path).context("模型文件路径无效")?;
        let path = dest.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).context("无法创建模型目录")?;
        }
        if file.size > 0 && fs::metadata(&path).is_ok_and(|m| m.len() == file.size) {
            done += file.size;
            on_progress(done, total, "model");
            continue;
        }
        let file_done = download_one(
            &client,
            repo,
            file,
            &path,
            done,
            total,
            on_progress,
            is_cancelled,
        )?;
        done += file_done;
        on_progress(done, total, "model");
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn download_one<F, C>(
    client: &reqwest::blocking::Client,
    repo: &str,
    file: &RemoteFile,
    dest: &Path,
    done_before: u64,
    total: Option<u64>,
    on_progress: &mut F,
    is_cancelled: &C,
) -> Result<u64>
where
    F: FnMut(u64, Option<u64>, &str),
    C: Fn() -> bool,
{
    let partial = partial_path(dest);
    let mut have = fs::metadata(&partial).map(|m| m.len()).unwrap_or(0);
    if file.size > 0 && have > file.size {
        let _ = fs::remove_file(&partial);
        have = 0;
    }
    let url = resolve_url(repo, &file.path)?;
    let mut req = client.get(url);
    if have > 0 {
        req = req.header("Range", format!("bytes={have}-"));
    }
    let mut resp = req.send().context("模型下载中断，请再试一次。")?;
    if resp.status().as_u16() == 416 {
        let _ = fs::remove_file(&partial);
        have = 0;
        resp = client
            .get(resolve_url(repo, &file.path)?)
            .send()
            .context("模型下载中断，请再试一次。")?;
    }
    if resp.status().as_u16() == 429 {
        bail!("国内模型源暂时限流了，过一会儿再试。");
    }
    if !resp.status().is_success() {
        bail!("模型下载失败，请过一会儿再试。");
    }
    let append = resp.status().as_u16() == 206 && have > 0;
    if !append {
        have = 0;
    }
    let mut out = OpenOptions::new()
        .create(true)
        .write(true)
        .append(append)
        .truncate(!append)
        .open(&partial)
        .context("无法保存模型文件")?;
    let mut buf = [0u8; 64 * 1024];
    let mut got = have;
    loop {
        if is_cancelled() {
            bail!("已取消下载。");
        }
        let n = match resp.read(&mut buf) {
            Ok(n) => n,
            Err(err) if download_stalled(&err) => {
                bail!("下载停住了。可以换成另一个下载地址再试。");
            }
            Err(err) => return Err(err).context("模型下载中断，请再试一次。"),
        };
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n]).context("写入模型文件失败")?;
        got += n as u64;
        on_progress(done_before + got, total, "model");
    }
    out.flush().context("写入模型文件失败")?;
    if file.size > 0 && got != file.size {
        bail!("模型文件不完整，请再试一次。");
    }
    fs::rename(&partial, dest).context("保存模型文件失败")?;
    Ok(got)
}

fn partial_path(dest: &Path) -> PathBuf {
    let name = dest.file_name().and_then(|s| s.to_str()).unwrap_or("model");
    dest.with_file_name(format!("{name}.partial"))
}

fn resolve_url(repo: &str, path: &str) -> Result<reqwest::Url> {
    let mut url = reqwest::Url::parse(&format!("{API_ROOT}/models/{repo}/resolve/master/"))
        .context("国内模型源地址无效")?;
    {
        let mut segs = url
            .path_segments_mut()
            .map_err(|_| anyhow::anyhow!("国内模型源地址无效"))?;
        segs.pop_if_empty();
        for seg in path.split('/') {
            segs.push(seg);
        }
    }
    Ok(url)
}

fn download_stalled(err: &std::io::Error) -> bool {
    err.kind() == std::io::ErrorKind::TimedOut
        || err.to_string().to_ascii_lowercase().contains("timed out")
}

fn http_client() -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(20))
        // The blocking client applies this deadline to every body read, not to
        // the whole file. A quiet socket fails here; bytes that keep arriving
        // start the wait over.
        .timeout(Duration::from_secs(90))
        .user_agent("agent-doctor")
        .build()
        .context("创建下载客户端失败")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gguf_quant_keeps_matching_shards_in_order() {
        let paths = vec![
            "qwen2.5-7b-instruct-q4_k_m-00002-of-00002.gguf".to_string(),
            "qwen2.5-7b-instruct-q8_0-00001-of-00001.gguf".to_string(),
            "README.md".to_string(),
            "qwen2.5-7b-instruct-q4_k_m-00001-of-00002.gguf".to_string(),
        ];
        let got = select_gguf_shards(&paths, "Q4_K_M");
        assert_eq!(
            got,
            vec![
                "qwen2.5-7b-instruct-q4_k_m-00001-of-00002.gguf",
                "qwen2.5-7b-instruct-q4_k_m-00002-of-00002.gguf",
            ]
        );
    }

    #[test]
    fn model_spec_splits_quant_and_normalizes_exo_dir() {
        let (repo, quant) = split_model_spec("Qwen/Qwen2.5-7B-Instruct-GGUF:Q4_K_M").unwrap();
        assert_eq!(repo, "Qwen/Qwen2.5-7B-Instruct-GGUF");
        assert_eq!(quant.as_deref(), Some("Q4_K_M"));
        assert_eq!(
            exo_model_dirname("mlx-community/Qwen2.5-7B-Instruct-4bit"),
            "mlx-community--Qwen2.5-7B-Instruct-4bit"
        );
        assert!(split_model_spec("../evil/name").is_err());
        assert!(safe_relative("../config.json").is_none());
        assert!(safe_relative("config.json").is_some());
    }

    #[test]
    fn exo_snapshot_keeps_weights_and_skips_the_original_tree() {
        let files = vec![
            RemoteFile {
                path: "model.safetensors".into(),
                size: 10,
            },
            RemoteFile {
                path: "config.json".into(),
                size: 1,
            },
            RemoteFile {
                path: "original/model.safetensors".into(),
                size: 99,
            },
            RemoteFile {
                path: "README.md".into(),
                size: 1,
            },
            RemoteFile {
                path: "model.safetensors.index.json".into(),
                size: 1,
            },
        ];
        let got = exo_snapshot_files(&files);
        let paths: Vec<&str> = got.iter().map(|file| file.path.as_str()).collect();
        assert_eq!(
            paths,
            vec![
                "model.safetensors",
                "config.json",
                "model.safetensors.index.json"
            ]
        );
        assert!(has_safetensor_weight(&got));
    }

    #[test]
    fn whole_gguf_is_preferred_over_split_shards() {
        let paths = vec![
            "qwen2.5-coder-7b-instruct-q4_k_m-00002-of-00002.gguf".to_string(),
            "qwen2.5-coder-7b-instruct-q4_k_m.gguf".to_string(),
            "qwen2.5-coder-7b-instruct-q4_k_m-00001-of-00002.gguf".to_string(),
        ];
        let got = select_gguf_shards(&paths, "Q4_K_M");
        assert_eq!(got, vec!["qwen2.5-coder-7b-instruct-q4_k_m.gguf"]);
    }
}
