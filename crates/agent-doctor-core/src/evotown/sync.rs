use std::fs;
use std::io::Cursor;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::client::EvotownClient;
use super::config::EvotownConfig;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncOptions {
    pub dry_run: bool,
    pub only_skills: Vec<String>,
    pub runtime_target: Option<String>,
    pub bundle_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillSyncOutcome {
    pub skill_id: String,
    pub version: String,
    pub outcome: String,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncReport {
    pub base_url: String,
    pub bundle_id: String,
    pub runtime_target: String,
    pub skills_dir: String,
    pub lock_path: String,
    pub installed: usize,
    pub skipped: usize,
    pub failed: usize,
    pub outcomes: Vec<SkillSyncOutcome>,
}

pub fn execute_sync(config: &EvotownConfig, options: &SyncOptions) -> Result<SyncReport> {
    let runtime_target = options
        .runtime_target
        .as_deref()
        .unwrap_or(config.runtime_target.as_str())
        .to_string();
    let bundle_id = options
        .bundle_id
        .as_deref()
        .unwrap_or(config.bundle_id.as_str())
        .to_string();

    let client = EvotownClient::new(&config.base_url, &config.api_key)?;
    let manifest_path =
        format!("/api/v1/market/bundles/{bundle_id}/manifest?runtime_target={runtime_target}");
    let manifest_body = client.get_json(&manifest_path)?;
    let manifest = manifest_body
        .get("manifest")
        .cloned()
        .unwrap_or(Value::Null);

    let skills = manifest
        .get("skills")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut state = load_lock_state(&config.skills_lock_path)?;
    state["bundle_id"] = json!(manifest
        .get("bundle_id")
        .and_then(Value::as_str)
        .unwrap_or(bundle_id.as_str()));
    state["channel"] = json!(manifest
        .get("channel")
        .and_then(Value::as_str)
        .unwrap_or("stable"));
    state["runtime_target"] = json!(runtime_target);
    state["updated_at"] = json!(utc_now());

    let lock = state
        .as_object_mut()
        .context("skills lock must be an object")?
        .entry("skills")
        .or_insert_with(|| json!({}));
    let lock_map = lock
        .as_object_mut()
        .context("skills lock.skills must be an object")?;

    let only: std::collections::HashSet<String> = options
        .only_skills
        .iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect();

    let mut installed = 0usize;
    let mut skipped = 0usize;
    let mut failed = 0usize;
    let mut outcomes = Vec::new();

    for entry in skills {
        let skill_id = entry
            .get("skill_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if skill_id.is_empty() {
            continue;
        }
        if !only.is_empty() && !only.contains(&skill_id) {
            continue;
        }

        let version = entry
            .get("version")
            .and_then(Value::as_str)
            .unwrap_or("0.0.0")
            .to_string();
        let mut package_url = entry
            .get("package_url")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if package_url.starts_with("builtin://") || package_url.is_empty() {
            package_url = market_download_path(&skill_id);
        }

        match install_skill_entry(SkillInstallRequest {
            client: &client,
            base_url: &config.base_url,
            skills_dir: &config.skills_dir,
            lock_map,
            skill_id: &skill_id,
            version: &version,
            package_url: &package_url,
            expected_sha256: entry.get("package_sha256").and_then(Value::as_str),
            expected_signature: entry
                .get("signature")
                .or_else(|| entry.get("package_signature"))
                .and_then(Value::as_str),
            dry_run: options.dry_run,
        }) {
            Ok(outcome) => {
                outcomes.push(SkillSyncOutcome {
                    skill_id: skill_id.clone(),
                    version: version.clone(),
                    outcome: outcome.outcome.clone(),
                    detail: outcome.detail,
                });
                match outcome.outcome.as_str() {
                    "installed" => installed += 1,
                    "skipped" => skipped += 1,
                    _ => failed += 1,
                }
            }
            Err(error) => {
                failed += 1;
                outcomes.push(SkillSyncOutcome {
                    skill_id,
                    version,
                    outcome: "failed".to_string(),
                    detail: Some(error.to_string()),
                });
            }
        }
    }

    if !options.dry_run {
        save_lock_state(&config.skills_lock_path, &state)?;
    }

    Ok(SyncReport {
        base_url: config.base_url.clone(),
        bundle_id: state
            .get("bundle_id")
            .and_then(Value::as_str)
            .unwrap_or(bundle_id.as_str())
            .to_string(),
        runtime_target,
        skills_dir: config.skills_dir.display().to_string(),
        lock_path: config.skills_lock_path.display().to_string(),
        installed,
        skipped,
        failed,
        outcomes,
    })
}

struct InstallOutcome {
    outcome: String,
    detail: Option<String>,
}

struct SkillInstallRequest<'a> {
    client: &'a EvotownClient,
    base_url: &'a str,
    skills_dir: &'a Path,
    lock_map: &'a mut serde_json::Map<String, Value>,
    skill_id: &'a str,
    version: &'a str,
    package_url: &'a str,
    expected_sha256: Option<&'a str>,
    expected_signature: Option<&'a str>,
    dry_run: bool,
}

fn install_skill_entry(request: SkillInstallRequest<'_>) -> Result<InstallOutcome> {
    let SkillInstallRequest {
        client,
        base_url,
        skills_dir,
        lock_map,
        skill_id,
        version,
        package_url,
        expected_sha256,
        expected_signature,
        dry_run,
    } = request;

    let resolved = resolve_package_url(base_url, package_url)
        .with_context(|| format!("no package URL for {skill_id}"))?;

    let prev = lock_map.get(skill_id).cloned().unwrap_or(Value::Null);
    let prev_version = prev.get("version").and_then(Value::as_str).unwrap_or("");
    let prev_url = prev
        .get("package_url")
        .and_then(Value::as_str)
        .unwrap_or("");
    let target = skills_dir.join(skill_id);
    if prev_version == version
        && prev_url == package_url
        && target.is_dir()
        && target
            .read_dir()
            .map(|mut dir| dir.next().is_some())
            .unwrap_or(false)
    {
        return Ok(InstallOutcome {
            outcome: "skipped".to_string(),
            detail: Some("up to date".to_string()),
        });
    }

    if dry_run {
        return Ok(InstallOutcome {
            outcome: "installed".to_string(),
            detail: Some(format!("would download from {resolved}")),
        });
    }

    let blob = client.get_bytes(&resolved)?;
    let digest = hex_sha256(&blob);

    if let Some(expected) = expected_sha256.filter(|value| !value.is_empty()) {
        if digest != expected {
            anyhow::bail!("sha256 mismatch for {skill_id}");
        }
    }
    if let Some(signature) = expected_signature.filter(|value| !value.is_empty()) {
        if !verify_package_signature(&digest, signature) {
            anyhow::bail!("signature verification failed for {skill_id}");
        }
    }

    if target.exists() {
        fs::remove_dir_all(&target).ok();
    }
    extract_zip_bytes(&blob, &target)?;

    lock_map.insert(
        skill_id.to_string(),
        json!({
            "version": version,
            "package_url": package_url,
            "sha256": digest,
            "installed_at": utc_now(),
        }),
    );

    Ok(InstallOutcome {
        outcome: "installed".to_string(),
        detail: Some(resolved),
    })
}

fn market_download_path(skill_id: &str) -> String {
    format!("/api/v1/market/skills/{skill_id}/download")
}

fn resolve_package_url(base_url: &str, package_url: &str) -> Result<String> {
    if package_url.is_empty() || package_url.starts_with("builtin://") {
        anyhow::bail!("missing package URL");
    }
    if package_url.starts_with("http://") || package_url.starts_with("https://") {
        return Ok(package_url.to_string());
    }
    let base = base_url.trim_end_matches('/');
    if package_url.starts_with('/') {
        Ok(format!("{base}{package_url}"))
    } else {
        Ok(format!("{base}/{package_url}"))
    }
}

fn extract_zip_bytes(content: &[u8], target_dir: &Path) -> Result<()> {
    fs::create_dir_all(target_dir)?;
    let reader = Cursor::new(content);
    let mut archive = zip::ZipArchive::new(reader).context("invalid skill zip archive")?;
    for index in 0..archive.len() {
        let mut file = archive
            .by_index(index)
            .context("failed to read zip entry")?;
        let outpath = match file.enclosed_name() {
            Some(path) => target_dir.join(path),
            None => continue,
        };
        if file.name().ends_with('/') {
            fs::create_dir_all(&outpath)?;
            continue;
        }
        if let Some(parent) = outpath.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut outfile = fs::File::create(&outpath)?;
        std::io::copy(&mut file, &mut outfile)?;
    }
    Ok(())
}

fn verify_package_signature(hex_digest: &str, signature: &str) -> bool {
    let secret = std::env::var("EVOTOWN_SKILL_SIGNING_SECRET")
        .unwrap_or_default()
        .trim()
        .to_string();
    if secret.is_empty() || signature.trim().is_empty() {
        return true;
    }
    use base64::Engine;
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    type HmacSha256 = Hmac<Sha256>;
    let Ok(mut mac) = HmacSha256::new_from_slice(secret.as_bytes()) else {
        return false;
    };
    mac.update(hex_digest.as_bytes());
    let expected =
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());
    constant_time_eq(expected.as_bytes(), signature.trim().as_bytes())
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right.iter())
        .fold(0u8, |acc, (a, b)| acc | (a ^ b))
        == 0
}

fn hex_sha256(content: &[u8]) -> String {
    let digest = Sha256::digest(content);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn load_lock_state(path: &Path) -> Result<Value> {
    if !path.exists() {
        return Ok(json!({ "skills": {} }));
    }
    let raw = fs::read_to_string(path)?;
    serde_json::from_str(&raw).or_else(|_| Ok(json!({ "skills": {} })))
}

fn save_lock_state(path: &Path, state: &Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_string_pretty(state)? + "\n")?;
    Ok(())
}

fn utc_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // Good enough for lock metadata; full RFC3339 not required for parity with Python script.
    format!("{seconds}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_relative_package_urls() {
        assert_eq!(
            resolve_package_url(
                "https://evotown.example",
                "/api/v1/market/skills/foo/download"
            )
            .unwrap(),
            "https://evotown.example/api/v1/market/skills/foo/download"
        );
    }
}
