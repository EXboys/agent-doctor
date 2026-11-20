//! On-demand download, extraction, and cleanup for the headless `exo` runtime.
//!
//! Primary source in mainland China: Alibaba Cloud OSS (`agent-doctor.oss-cn-shenzhen`).
//! Fallback source: official `EXO-latest.dmg` headless mount & runtime extraction so
//! development and overseas machines work even before an OSS tarball is mirrored.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
use agent_doctor_core::lifecycle::use_china_mirrors;
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
use sha2::{Digest, Sha256};
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
use std::fs::File;
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
use std::io::{Read, Write};
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
use std::time::Duration;

/// Latest published EXO.app tag that ships a headless `exo` runtime in the DMG.
/// (v1.0.72 was never cut; Zenoh builds live on main after the libp2p→zenoh migration.)
pub const PINNED_EXO_VERSION: &str = "1.0.71";
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const OSS_BASE_URL: &str = "https://agent-doctor.oss-cn-shenzhen.aliyuncs.com/runtimes/exo";
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const OFFICIAL_DMG_URL: &str = "https://assets.exolabs.net/EXO-latest.dmg";

static CANCEL_INSTALL: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExoInstallManifest {
    pub version: String,
    pub build_id: String,
    pub source: String,
    #[serde(default)]
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocatedExoRuntime {
    pub executable: PathBuf,
    pub runtime_dir: PathBuf,
    pub dashboard_zh_rel_home: Option<String>,
    pub build_id: String,
    pub managed: bool,
}

pub fn cancel_exo_install() {
    CANCEL_INSTALL.store(true, Ordering::SeqCst);
}

pub(crate) fn exo_install_cancelled() -> bool {
    CANCEL_INSTALL.load(Ordering::SeqCst)
}

pub(crate) fn clear_exo_install_cancel() {
    CANCEL_INSTALL.store(false, Ordering::SeqCst);
}

pub fn managed_exo_version_root(version: &str) -> Option<PathBuf> {
    dirs::home_dir().map(|h| {
        h.join(".agent-doctor")
            .join("runtimes")
            .join("exo")
            .join(version)
    })
}

pub fn pinned_version() -> String {
    std::env::var("AGENT_DOCTOR_EXO_VERSION")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| PINNED_EXO_VERSION.to_string())
}

/// Locate an installed headless `exo` runtime:
/// 1. Our managed install under `~/.agent-doctor/runtimes/exo/<version>/exo/exo`
/// 2. System `/Applications/EXO.app/Contents/Resources/exo/exo` (reused headlessly)
///
/// Chinese static dashboard (`dashboard-zh/`) is always looked up under the managed
/// version root so a separately built zh pack can override either install path.
pub fn locate_exo_runtime() -> Option<LocatedExoRuntime> {
    let ver = pinned_version();
    let dashboard_zh_rel_home = managed_dashboard_zh_rel_home(&ver);

    if let Some(root) = managed_exo_version_root(&ver) {
        let runtime_dir = root.join("exo");
        let exe = runtime_dir.join("exo");
        if exe.is_file() {
            let manifest = read_install_manifest(&root);
            let build_id = manifest
                .map(|m| m.build_id)
                .filter(|b| !b.trim().is_empty())
                .unwrap_or_else(|| ver.clone());
            return Some(LocatedExoRuntime {
                executable: exe,
                runtime_dir,
                dashboard_zh_rel_home,
                build_id,
                managed: true,
            });
        }
    }

    let app_root = PathBuf::from("/Applications/EXO.app/Contents");
    let runtime_dir = app_root.join("Resources").join("exo");
    let exe = runtime_dir.join("exo");
    if exe.is_file() {
        let build_id =
            read_app_bundle_build_id(&app_root.join("Info.plist")).unwrap_or_else(|| ver.clone());
        return Some(LocatedExoRuntime {
            executable: exe,
            runtime_dir,
            dashboard_zh_rel_home,
            build_id,
            managed: false,
        });
    }

    None
}

fn managed_dashboard_zh_rel_home(version: &str) -> Option<String> {
    let root = managed_exo_version_root(version)?;
    let zh_index = root.join("dashboard-zh").join("index.html");
    zh_index
        .is_file()
        .then(|| format!(".agent-doctor/runtimes/exo/{version}/dashboard-zh"))
}

/// Relative-to-$HOME path for the Agent Doctor dashboard, if installed.
pub fn pinned_dashboard_zh_rel_home() -> Option<String> {
    managed_dashboard_zh_rel_home(&pinned_version())
}

/// Embedded Agent Doctor dashboard (中文 + 适合本机). Ships with the app so a
/// DMG-only exo install still gets the customized board — OSS packs may omit it.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const PREBUILT_DASHBOARD_ZH_TAR_GZ: &[u8] =
    include_bytes!("../../../../vendor/exo-dashboard-zh/prebuilt/dashboard-zh.tar.gz");

/// Make sure `~/.agent-doctor/runtimes/exo/<pinned>/dashboard-zh/` exists.
/// Prefer an already-installed tree; otherwise extract the embedded prebuilt
/// (and optionally refresh from OSS when that object exists).
pub fn ensure_managed_dashboard_zh() -> Result<()> {
    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    {
        return Ok(());
    }

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        let ver = pinned_version();
        let root =
            managed_exo_version_root(&ver).context("无法定位本机用户主目录以安装算力看板")?;
        let zh_index = root.join("dashboard-zh").join("index.html");
        if zh_index.is_file() {
            return Ok(());
        }
        fs::create_dir_all(&root)
            .with_context(|| format!("创建算力目录失败：{}", root.display()))?;

        // Optional OSS refresh (when the object has been uploaded).
        let oss_url = format!("{OSS_BASE_URL}/{ver}/dashboard-zh.tar.gz");
        let archive = root.join(".dashboard-zh-download.tar.gz");
        let mut installed = false;
        if let Ok(sha) = download_to_file(&oss_url, &archive, &mut |_, _, _| {}) {
            let _ = sha;
            if extract_tar_gz(&archive, &root).is_ok() && zh_index.is_file() {
                installed = true;
            }
        }
        let _ = fs::remove_file(&archive);

        if !installed {
            let embedded = root.join(".dashboard-zh-prebuilt.tar.gz");
            fs::write(&embedded, PREBUILT_DASHBOARD_ZH_TAR_GZ)
                .with_context(|| format!("写入内置算力看板失败：{}", embedded.display()))?;
            extract_tar_gz(&embedded, &root)?;
            let _ = fs::remove_file(&embedded);
        }

        if !zh_index.is_file() {
            bail!("定制算力看板安装失败，请重试开启多机算力。");
        }
        Ok(())
    }
}

/// Download and install the headless `exo` runtime on Apple Silicon macOS.
pub fn install_exo_runtime<F>(mut on_progress: F) -> Result<LocatedExoRuntime>
where
    F: FnMut(u64, Option<u64>, &str),
{
    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    {
        let _ = &mut on_progress;
        bail!("当前电脑不支持运行苹果芯片算力组件，请直接连接同一网络里的苹果电脑。");
    }

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        CANCEL_INSTALL.store(false, Ordering::SeqCst);
        let ver = pinned_version();
        let target_root = managed_exo_version_root(&ver).context("无法定位本机用户主目录")?;
        let parent = target_root
            .parent()
            .context("无法定位算力组件安装父目录")?
            .to_path_buf();
        fs::create_dir_all(&parent)?;

        let staging = parent.join(format!(".staging-{ver}-{}", std::process::id()));
        if staging.exists() {
            let _ = fs::remove_dir_all(&staging);
        }
        fs::create_dir_all(&staging)?;

        let install_res = install_into_staging(&ver, &staging, &mut on_progress);
        if let Err(err) = install_res {
            let _ = fs::remove_dir_all(&staging);
            return Err(err);
        }

        // Keep a previously installed Chinese dashboard across reinstalls.
        // OSS packs may already include dashboard-zh/; only restore when staging lacks it.
        let zh_keep = parent.join(format!(".dashboard-zh-keep-{ver}-{}", std::process::id()));
        let existing_zh = target_root.join("dashboard-zh");
        if existing_zh.join("index.html").is_file() {
            let _ = fs::remove_dir_all(&zh_keep);
            let _ = fs::rename(&existing_zh, &zh_keep);
        }

        if target_root.exists() {
            let _ = fs::remove_dir_all(&target_root);
        }
        fs::rename(&staging, &target_root)
            .with_context(|| format!("移动算力组件目录失败：{}", target_root.display()))?;

        let final_zh = target_root.join("dashboard-zh");
        if !final_zh.join("index.html").is_file() && zh_keep.join("index.html").is_file() {
            let _ = fs::rename(&zh_keep, &final_zh);
        }
        let _ = fs::remove_dir_all(&zh_keep);

        // DMG / incomplete OSS packs have no dashboard-zh — install the embedded one.
        let _ = ensure_managed_dashboard_zh();

        locate_exo_runtime().context("算力组件解压完成，但未找到可执行入口文件")
    }
}

/// Remove `~/.agent-doctor/runtimes/exo`, keeping `~/.exo/models` unless `delete_models` is set.
pub fn uninstall_managed_exo_runtime(delete_models: bool) -> Result<()> {
    if let Some(home) = dirs::home_dir() {
        let managed = home.join(".agent-doctor").join("runtimes").join("exo");
        if managed.exists() {
            fs::remove_dir_all(&managed)
                .with_context(|| format!("清理 {} 失败", managed.display()))?;
        }
        if delete_models {
            let models = home.join(".exo").join("models");
            if models.exists() {
                fs::remove_dir_all(&models)
                    .with_context(|| format!("清理模型目录 {} 失败", models.display()))?;
            }
        }
    }
    Ok(())
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn install_into_staging<F>(ver: &str, staging: &Path, on_progress: &mut F) -> Result<()>
where
    F: FnMut(u64, Option<u64>, &str),
{
    let oss_url = std::env::var("AGENT_DOCTOR_EXO_ARCHIVE_URL")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("{OSS_BASE_URL}/{ver}/exo-darwin-arm64.tar.gz"));

    let expected_sha = std::env::var("AGENT_DOCTOR_EXO_SHA256")
        .ok()
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty());

    // Prefer OSS archive (contains pre-extracted runtime + optional dashboard-zh + LICENSE).
    // If OSS has not been populated for this tag yet (HTTP 404 / network unreachable abroad),
    // fall back cleanly to official EXO-latest.dmg headless extraction.
    let try_oss_first = use_china_mirrors()
        || std::env::var("AGENT_DOCTOR_EXO_ARCHIVE_URL")
            .ok()
            .is_some_and(|s| !s.trim().is_empty());

    if try_oss_first {
        let archive_path = staging.join("exo.tar.gz");
        match download_to_file(&oss_url, &archive_path, on_progress) {
            Ok(sha) => {
                verify_optional_sha(&sha, expected_sha.as_deref())?;
                on_progress(0, None, "extracting");
                extract_tar_gz(&archive_path, staging)?;
                let _ = fs::remove_file(&archive_path);
                ensure_staging_layout(staging, ver, &oss_url, Some(sha))?;
                return Ok(());
            }
            Err(err) => {
                if CANCEL_INSTALL.load(Ordering::SeqCst) {
                    return Err(err);
                }
                let _ = fs::remove_file(&archive_path);
            }
        }
    }

    // Fallback: official DMG headless extraction
    let dmg_path = staging.join("EXO.dmg");
    let sha = download_to_file(OFFICIAL_DMG_URL, &dmg_path, on_progress)?;
    on_progress(0, None, "extracting");
    let build_id = extract_from_dmg(&dmg_path, staging)?.unwrap_or_else(|| ver.to_string());
    let _ = fs::remove_file(&dmg_path);
    write_apache_license_note(staging)?;
    let manifest = ExoInstallManifest {
        version: ver.to_string(),
        build_id,
        source: OFFICIAL_DMG_URL.to_string(),
        sha256: Some(sha),
    };
    fs::write(
        staging.join("install.json"),
        serde_json::to_string_pretty(&manifest)?,
    )?;
    Ok(())
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn download_to_file<F>(url: &str, dest: &Path, on_progress: &mut F) -> Result<String>
where
    F: FnMut(u64, Option<u64>, &str),
{
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(1800))
        .build()
        .context("创建下载客户端失败")?;

    let mut resp = client
        .get(url)
        .send()
        .with_context(|| format!("连接下载源失败：{url}"))?;

    if !resp.status().is_success() {
        bail!("下载源返回 HTTP {}（{url}）", resp.status().as_u16());
    }

    let total = resp.content_length();
    let mut file =
        File::create(dest).with_context(|| format!("创建临时下载文件失败：{}", dest.display()))?;
    let mut hasher = Sha256::new();
    let mut downloaded: u64 = 0;
    let mut buf = [0u8; 64 * 1024];

    on_progress(0, total, "downloading");
    loop {
        if CANCEL_INSTALL.load(Ordering::SeqCst) {
            bail!("已取消下载算力组件。");
        }
        let n = resp.read(&mut buf).context("下载连接中断，请重试")?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).context("写入下载文件失败")?;
        hasher.update(&buf[..n]);
        downloaded += n as u64;
        on_progress(downloaded, total, "downloading");
    }
    file.flush()?;

    let digest = hasher.finalize();
    let mut hex = String::with_capacity(64);
    for b in digest {
        use std::fmt::Write as _;
        let _ = write!(&mut hex, "{b:02x}");
    }
    Ok(hex)
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn verify_optional_sha(actual: &str, expected: Option<&str>) -> Result<()> {
    if let Some(exp) = expected {
        if !actual.eq_ignore_ascii_case(exp) {
            bail!("下载的算力组件校验不通过，请稍后重试。");
        }
    }
    Ok(())
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn extract_tar_gz(archive: &Path, dest: &Path) -> Result<()> {
    let archive_str = archive.to_string_lossy();
    let dest_str = dest.to_string_lossy();
    let out = agent_doctor_core::exec::run_output(
        Path::new("/usr/bin/tar"),
        &["-xzf", &archive_str, "-C", &dest_str],
        Duration::from_secs(180),
    )
    .map_err(|e| anyhow::anyhow!("解压算力组件失败：{e}"))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        bail!("解压算力组件未完成：{}", stderr.trim());
    }
    Ok(())
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn extract_from_dmg(dmg_path: &Path, staging: &Path) -> Result<Option<String>> {
    let mount_point = staging.join(".mnt");
    fs::create_dir_all(&mount_point)?;
    let dmg_str = dmg_path.to_string_lossy().to_string();
    let mnt_str = mount_point.to_string_lossy().to_string();

    let attach = agent_doctor_core::exec::run_output(
        Path::new("/usr/bin/hdiutil"),
        &[
            "attach",
            "-nobrowse",
            "-readonly",
            "-quiet",
            "-mountpoint",
            &mnt_str,
            &dmg_str,
        ],
        Duration::from_secs(60),
    )
    .map_err(|e| anyhow::anyhow!("挂载算力安装包失败：{e}"))?;

    if !attach.status.success() {
        let stderr = String::from_utf8_lossy(&attach.stderr);
        bail!("打开算力安装包失败：{}", stderr.trim());
    }

    let copy_res = (|| -> Result<Option<String>> {
        let contents = mount_point.join("EXO.app").join("Contents");
        let src_exo = contents.join("Resources").join("exo");
        if !src_exo.is_dir() {
            bail!("安装包内未找到 exo 运行时目录。");
        }
        let dst_exo = staging.join("exo");
        let src_str = src_exo.to_string_lossy().to_string();
        let dst_str = dst_exo.to_string_lossy().to_string();
        let ditto = agent_doctor_core::exec::run_output(
            Path::new("/usr/bin/ditto"),
            &[&src_str, &dst_str],
            Duration::from_secs(180),
        )
        .map_err(|e| anyhow::anyhow!("复制算力运行时失败：{e}"))?;
        if !ditto.status.success() {
            let stderr = String::from_utf8_lossy(&ditto.stderr);
            bail!("复制算力运行时未完成：{}", stderr.trim());
        }
        Ok(read_app_bundle_build_id(&contents.join("Info.plist")))
    })();

    let _ = agent_doctor_core::exec::run_output(
        Path::new("/usr/bin/hdiutil"),
        &["detach", "-quiet", "-force", &mnt_str],
        Duration::from_secs(30),
    );
    let _ = fs::remove_dir_all(&mount_point);

    copy_res
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn ensure_staging_layout(
    staging: &Path,
    ver: &str,
    source_url: &str,
    sha256: Option<String>,
) -> Result<()> {
    let direct_exe = staging.join("exo").join("exo");
    if !direct_exe.is_file() {
        bail!("算力压缩包结构不完整（缺少 exo/exo）。");
    }
    if !staging.join("install.json").is_file() {
        let manifest = ExoInstallManifest {
            version: ver.to_string(),
            build_id: ver.to_string(),
            source: source_url.to_string(),
            sha256,
        };
        fs::write(
            staging.join("install.json"),
            serde_json::to_string_pretty(&manifest)?,
        )?;
    }
    write_apache_license_note(staging)?;
    Ok(())
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn write_apache_license_note(staging: &Path) -> Result<()> {
    let notice_path = staging.join("THIRD_PARTY_NOTICE.txt");
    if !notice_path.exists() {
        let text = "This directory contains the unmodified exo runtime from Exo Technologies Ltd \
(https://github.com/exo-explore/exo), distributed under the Apache License, Version 2.0 \
(http://www.apache.org/licenses/LICENSE-2.0).\n";
        let _ = fs::write(notice_path, text);
    }
    Ok(())
}

fn read_install_manifest(root: &Path) -> Option<ExoInstallManifest> {
    let raw = fs::read_to_string(root.join("install.json")).ok()?;
    serde_json::from_str(&raw).ok()
}

/// Extract `EXOBuildTag` + `EXOBuildCommit` (matching `ExoProcessController.buildID` in Swift)
/// from `EXO.app/Contents/Info.plist`.
pub(crate) fn read_app_bundle_build_id(info_plist: &Path) -> Option<String> {
    let xml = fs::read_to_string(info_plist).ok()?;
    let tag = plist_string_value(&xml, "EXOBuildTag")
        .or_else(|| plist_string_value(&xml, "CFBundleShortVersionString"))?;
    let commit = plist_string_value(&xml, "EXOBuildCommit")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    match commit {
        Some(c) => Some(format!("{tag} ({c})")),
        None => Some(tag),
    }
}

fn plist_string_value(xml: &str, key: &str) -> Option<String> {
    let needle = format!("<key>{key}</key>");
    let idx = xml.find(&needle)?;
    let rest = &xml[idx + needle.len()..];
    let s_start = rest.find("<string>")? + "<string>".len();
    let s_end = rest[s_start..].find("</string>")?;
    let val = rest[s_start..s_start + s_end].trim();
    if val.is_empty() {
        None
    } else {
        Some(val.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_exo_build_id_from_info_plist_xml() {
        let tmp = tempfile::tempdir().unwrap();
        let plist = tmp.path().join("Info.plist");
        fs::write(
            &plist,
            r#"<plist><dict>
                <key>EXOBuildTag</key><string>1.0.72</string>
                <key>EXOBuildCommit</key><string>4f1c2a9</string>
            </dict></plist>"#,
        )
        .unwrap();
        assert_eq!(
            read_app_bundle_build_id(&plist).as_deref(),
            Some("1.0.72 (4f1c2a9)")
        );
    }
}
