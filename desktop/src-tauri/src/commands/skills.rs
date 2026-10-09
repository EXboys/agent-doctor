use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};

use agent_doctor_core::{
    install_local_skills, remove_local_skill, skill_roots, SkillInstallError, SkillInstallReport,
};
use serde::Serialize;

const MAX_ZIP_FILE_BYTES: u64 = 20 * 1024 * 1024;
const MAX_ZIP_BYTES: u64 = 80 * 1024 * 1024;

#[derive(Serialize)]
pub struct SkillInstallOutcome {
    pub installed: Vec<String>,
    pub replaced: Vec<String>,
    pub skipped: usize,
}

#[tauri::command]
pub async fn skill_install_command(
    scope: String,
    project_name: Option<String>,
    runtime: Option<String>,
    paths: Vec<String>,
) -> Result<SkillInstallOutcome, String> {
    tauri::async_runtime::spawn_blocking(move || install(scope, project_name, runtime, paths))
        .await
        .map_err(|_| "安装被打断了，请再试一次。".to_string())?
}

#[tauri::command]
pub async fn skill_remove_command(
    scope: String,
    project_name: Option<String>,
    runtime: Option<String>,
    skill_id: String,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        remove_local_skill(
            &scope,
            project_name.as_deref(),
            runtime.as_deref(),
            &skill_id,
        )
        .map(|_| ())
        .map_err(|err| match err {
            SkillInstallError::NotFound => code("not-found"),
            SkillInstallError::NoAgent => code("no-agent"),
            SkillInstallError::NoProject => code("no-project"),
            SkillInstallError::Copy(detail) => detail,
            _ => code("not-found"),
        })
    })
    .await
    .map_err(|_| "删除被打断了，请再试一次。".to_string())?
}

fn install(
    scope: String,
    project_name: Option<String>,
    runtime: Option<String>,
    paths: Vec<String>,
) -> Result<SkillInstallOutcome, String> {
    let mut sources = Vec::new();
    let mut temps = Vec::new();
    let mut skipped = 0usize;
    for raw in paths {
        let path = PathBuf::from(&raw);
        if !path.exists() {
            skipped += 1;
            continue;
        }
        if is_zip(&path) {
            let temp = std::env::temp_dir().join(format!(
                "ad-skill-zip-{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or(0)
            ));
            fs::create_dir_all(&temp).map_err(|_| "解压失败，请再试一次。".to_string())?;
            extract_zip(&path, &temp)?;
            if skill_roots(&temp).is_empty() {
                skipped += 1;
                let _ = fs::remove_dir_all(&temp);
                continue;
            }
            sources.push(temp.clone());
            temps.push(temp);
        } else if path.is_dir() {
            if skill_roots(&path).is_empty() {
                skipped += 1;
            } else {
                sources.push(path);
            }
        } else {
            skipped += 1;
        }
    }

    let result = install_local_skills(
        &scope,
        project_name.as_deref(),
        runtime.as_deref(),
        &sources,
    );
    for temp in &temps {
        let _ = fs::remove_dir_all(temp);
    }
    match result {
        Ok(report) => Ok(outcome(report, skipped)),
        Err(SkillInstallError::NotASkill) => Err(code("not-a-skill")),
        Err(SkillInstallError::NoAgent) => Err(code("no-agent")),
        Err(SkillInstallError::NoProject) => Err(code("no-project")),
        Err(SkillInstallError::TooBig) => Err(code("too-big")),
        Err(SkillInstallError::NotFound) => Err(code("not-found")),
        Err(SkillInstallError::UnknownScope) => Err(code("not-a-skill")),
        Err(SkillInstallError::Copy(detail)) => Err(detail),
    }
}

fn outcome(report: SkillInstallReport, skipped: usize) -> SkillInstallOutcome {
    SkillInstallOutcome {
        installed: report.installed,
        replaced: report.replaced,
        skipped,
    }
}

fn code(name: &str) -> String {
    name.to_string()
}

fn is_zip(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("zip"))
}

fn extract_zip(from: &Path, to: &Path) -> Result<(), String> {
    let bad = || "这个 zip 包打不开。".to_string();
    let file = File::open(from).map_err(|_| bad())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|_| bad())?;
    let mut files = 0usize;
    let mut bytes = 0u64;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|_| bad())?;
        if entry.is_dir() {
            continue;
        }
        let Some(relative) = entry.enclosed_name() else {
            continue;
        };
        if relative
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
        {
            continue;
        }
        if entry.size() > MAX_ZIP_FILE_BYTES {
            return Err(code("too-big"));
        }
        files += 1;
        bytes += entry.size();
        if files > 4_000 || bytes > MAX_ZIP_BYTES {
            return Err(code("too-big"));
        }
        let target = to.join(&relative);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|_| bad())?;
        }
        let mut out = File::create(&target).map_err(|_| bad())?;
        io::copy(&mut (&mut entry).take(MAX_ZIP_FILE_BYTES), &mut out).map_err(|_| bad())?;
    }
    Ok(())
}
