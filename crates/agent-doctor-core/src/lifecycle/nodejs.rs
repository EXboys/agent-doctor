//! Bootstrap a user-local Node.js when `npm` is missing (Claude/Codex/dsh install).
//!
//! Windows uses the official zip (no Administrator / winget). Unix uses the
//! official `.tar.gz` via `tar`. Installed under [`managed_nodejs_root`].

use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::Deserialize;

use crate::adapters::util::{find_binary, managed_nodejs_root, prepend_user_path};
use crate::exec::command_for_path;

const NODE_DIST_INDEX: &str = "https://nodejs.org/dist/index.json";
const NODE_DIST_BASE: &str = "https://nodejs.org/dist";
/// Used when nodejs.org/dist/index.json is unreachable.
const NODE_LTS_FALLBACK: &str = "v22.19.0";

pub fn npm_available() -> bool {
    find_binary("npm").is_some()
}

/// Ensure `npm` is on PATH, installing a local Node LTS if needed.
pub fn ensure_npm() -> Result<PathBuf> {
    ensure_npm_with_progress(|_, _| {})
}

/// `on_progress(message, percent)` — percent is 0–100 for this Node bootstrap step.
pub fn ensure_npm_with_progress<F>(mut on_progress: F) -> Result<PathBuf>
where
    F: FnMut(&str, u8),
{
    if let Some(npm) = find_binary("npm") {
        on_progress(&format!("Using existing npm at {}", npm.display()), 100);
        return Ok(npm);
    }

    on_progress(
        "npm not found — installing a local Node.js LTS (no admin required)…",
        2,
    );
    let dest = managed_nodejs_root();
    if npm_in_dir(&dest).is_none() {
        install_managed_nodejs(&dest, &mut on_progress)?;
    }
    prepend_user_path(&dest);
    prepend_user_path(&dest.join("bin"));

    find_binary("npm")
        .or_else(|| npm_in_dir(&dest))
        .with_context(|| {
            format!(
                "Node.js was installed to {} but npm was still not found",
                dest.display()
            )
        })
}

fn npm_in_dir(root: &Path) -> Option<PathBuf> {
    find_binary_in(&[root.to_path_buf(), root.join("bin")], "npm")
}

fn find_binary_in(dirs: &[PathBuf], name: &str) -> Option<PathBuf> {
    for dir in dirs {
        #[cfg(windows)]
        {
            for ext in [".cmd", ".exe", ".bat"] {
                let candidate = dir.join(format!("{name}{ext}"));
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn install_managed_nodejs<F>(dest: &Path, on_progress: &mut F) -> Result<()>
where
    F: FnMut(&str, u8),
{
    let client = reqwest::blocking::Client::builder()
        .user_agent(concat!("agent-doctor/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(180))
        .connect_timeout(Duration::from_secs(20))
        .build()
        .context("failed to build HTTP client for Node.js download")?;

    on_progress("Looking up Node.js LTS…", 4);
    let version = resolve_lts_version(&client).unwrap_or_else(|_| NODE_LTS_FALLBACK.to_string());
    let archive = node_archive_name(&version);
    let url = format!("{NODE_DIST_BASE}/{version}/{archive}");
    on_progress(&format!("Downloading Node.js {version}…"), 6);

    let bytes = download_bytes(&client, &url, on_progress)?;
    on_progress(
        &format!(
            "Downloaded {} — extracting to {}",
            format_bytes(bytes.len() as u64),
            dest.display()
        ),
        82,
    );

    let staging = dest
        .parent()
        .unwrap_or(dest)
        .join(format!(".nodejs-staging-{}", std::process::id()));
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&staging).with_context(|| format!("create {}", staging.display()))?;

    let extract_result = (|| {
        if archive.ends_with(".zip") {
            extract_zip(&bytes, &staging, on_progress)
        } else {
            extract_tar_gz(&bytes, &staging, on_progress)
        }
    })();
    if let Err(error) = extract_result {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }

    let unpacked = find_unpacked_root(&staging).context("Node.js archive had no node binary")?;
    if dest.exists() {
        fs::remove_dir_all(dest).with_context(|| format!("remove previous {}", dest.display()))?;
    }
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    if fs::rename(&unpacked, dest).is_err() {
        copy_dir_all(&unpacked, dest)?;
    }
    let _ = fs::remove_dir_all(&staging);
    on_progress(
        &format!("Node.js {version} ready at {}", dest.display()),
        100,
    );
    Ok(())
}

#[derive(Debug, Deserialize)]
struct DistIndexEntry {
    version: String,
    lts: serde_json::Value,
}

fn resolve_lts_version(client: &reqwest::blocking::Client) -> Result<String> {
    let entries: Vec<DistIndexEntry> = client
        .get(NODE_DIST_INDEX)
        .send()
        .context("nodejs.org index")?
        .error_for_status()
        .context("nodejs.org index HTTP")?
        .json()
        .context("nodejs.org index JSON")?;
    let entry = entries
        .into_iter()
        .find(|entry| entry.lts.is_string())
        .context("no LTS release in nodejs.org index")?;
    Ok(entry.version)
}

fn node_archive_name(version: &str) -> String {
    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        _ => "x64",
    };
    match std::env::consts::OS {
        "windows" => format!("node-{version}-win-{arch}.zip"),
        "macos" => format!("node-{version}-darwin-{arch}.tar.gz"),
        _ => format!("node-{version}-linux-{arch}.tar.gz"),
    }
}

fn download_bytes<F>(
    client: &reqwest::blocking::Client,
    url: &str,
    on_progress: &mut F,
) -> Result<Vec<u8>>
where
    F: FnMut(&str, u8),
{
    use std::io::Read;
    use std::time::Instant;

    let mut response = client
        .get(url)
        .send()
        .with_context(|| format!("download {url}"))?
        .error_for_status()
        .with_context(|| format!("download {url} HTTP"))?;
    let total = response.content_length();
    let mut buf = vec![0u8; 64 * 1024];
    let mut out = Vec::new();
    if let Some(len) = total {
        out.reserve(len as usize);
    }
    let mut last_emit = Instant::now()
        .checked_sub(Duration::from_secs(1))
        .unwrap_or_else(Instant::now);
    let mut last_pct = 255u8;
    loop {
        let n = response.read(&mut buf).context("read Node.js download")?;
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n]);
        let pct = download_percent(out.len() as u64, total);
        if last_pct == 255 || last_emit.elapsed() >= Duration::from_millis(250) || pct >= 80 {
            on_progress(&format_download_status(out.len() as u64, total), pct);
            last_emit = Instant::now();
            last_pct = pct;
        }
    }
    on_progress(&format_download_status(out.len() as u64, total), 80);
    Ok(out)
}

fn download_percent(read: u64, total: Option<u64>) -> u8 {
    match total {
        Some(total) if total > 0 => (6 + read.saturating_mul(74) / total).min(80) as u8,
        _ => 20,
    }
}

fn format_bytes(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.0} KB", bytes as f64 / 1024.0)
    }
}

fn format_download_status(read: u64, total: Option<u64>) -> String {
    match total {
        Some(total) if total > 0 => format!(
            "Downloading Node.js {} / {}",
            format_bytes(read),
            format_bytes(total)
        ),
        _ => format!("Downloading Node.js {}…", format_bytes(read)),
    }
}

fn extract_zip<F>(bytes: &[u8], dest: &Path, on_progress: &mut F) -> Result<()>
where
    F: FnMut(&str, u8),
{
    let reader = Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(reader).context("invalid Node.js zip")?;
    let total = archive.len().max(1);
    for index in 0..archive.len() {
        let mut file = archive.by_index(index).context("read Node.js zip entry")?;
        let Some(rel) = file.enclosed_name() else {
            continue;
        };
        let outpath = dest.join(rel);
        if file.name().ends_with('/') {
            fs::create_dir_all(&outpath)?;
            continue;
        }
        if let Some(parent) = outpath.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut outfile = fs::File::create(&outpath)?;
        std::io::copy(&mut file, &mut outfile)?;
        if index == 0 || index == total - 1 || index % 25 == 0 {
            let pct = (82 + (index as u64 * 14 / total as u64) as u8).min(96);
            on_progress(
                &format!("Extracting Node.js ({}/{})", index + 1, total),
                pct,
            );
        }
    }
    Ok(())
}

fn extract_tar_gz<F>(bytes: &[u8], dest: &Path, on_progress: &mut F) -> Result<()>
where
    F: FnMut(&str, u8),
{
    let archive_path = dest.join("node.tar.gz");
    fs::write(&archive_path, bytes).context("write node tarball")?;
    on_progress("Extracting Node.js archive…", 86);
    let status = {
        let mut cmd = command_for_path(Path::new("tar"));
        cmd.args([
            "-xzf",
            &archive_path.to_string_lossy(),
            "-C",
            &dest.to_string_lossy(),
        ]);
        cmd.status().context("run tar")?
    };
    let _ = fs::remove_file(&archive_path);
    if !status.success() {
        bail!("tar exited with {status}");
    }
    Ok(())
}

fn find_unpacked_root(staging: &Path) -> Option<PathBuf> {
    if node_present(staging) {
        return Some(staging.to_path_buf());
    }
    let entries = fs::read_dir(staging).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() && node_present(&path) {
            return Some(path);
        }
    }
    None
}

fn node_present(root: &Path) -> bool {
    root.join("node.exe").is_file()
        || root.join("node").is_file()
        || root.join("bin").join("node").is_file()
        || npm_in_dir(root).is_some()
}

fn copy_dir_all(src: &Path, dest: &Path) -> Result<()> {
    fs::create_dir_all(dest)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dest.join(entry.file_name());
        if from.is_dir() {
            copy_dir_all(&from, &to)?;
        } else {
            fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archive_name_matches_this_os() {
        let name = node_archive_name("v22.19.0");
        assert!(name.starts_with("node-v22.19.0-"));
        if cfg!(windows) {
            assert!(name.ends_with(".zip"));
            assert!(name.contains("-win-"));
        } else if cfg!(target_os = "macos") {
            assert!(name.ends_with(".tar.gz"));
            assert!(name.contains("-darwin-"));
        } else {
            assert!(name.ends_with(".tar.gz"));
            assert!(name.contains("-linux-"));
        }
    }

    #[test]
    fn managed_root_is_user_local() {
        let root = managed_nodejs_root();
        let display = root.to_string_lossy();
        assert!(
            display.contains("AgentDoctor") || display.contains("agent-doctor"),
            "{display}"
        );
        assert!(display.contains("nodejs"));
    }

    #[test]
    fn ensure_npm_reuses_existing_binary() {
        if find_binary("npm").is_none() {
            return;
        }
        let path = ensure_npm().expect("existing npm");
        assert!(path.exists());
    }

    #[test]
    fn download_percent_scales_into_node_step() {
        assert_eq!(download_percent(0, Some(100)), 6);
        assert_eq!(download_percent(50, Some(100)), 43);
        assert_eq!(download_percent(100, Some(100)), 80);
        assert_eq!(download_percent(10, None), 20);
    }

    #[test]
    fn format_download_status_includes_totals() {
        let text = format_download_status(5 * 1024 * 1024, Some(30 * 1024 * 1024));
        assert!(text.contains("5.0 MB"), "{text}");
        assert!(text.contains("30.0 MB"), "{text}");
    }
}
