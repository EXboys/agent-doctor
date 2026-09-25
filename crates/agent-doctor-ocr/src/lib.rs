//! On-device picture text for Ask. DeepSeek and other text-only models cannot
//! see pictures; we read words on this computer and fold them into the prompt.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

use serde::Serialize;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(any(target_os = "macos", windows)))]
mod unsupported;
#[cfg(windows)]
mod windows;

const MAX_IMAGE_BYTES: u64 = 12 * 1024 * 1024;
const MAX_TEXT_CHARS: usize = 8_000;
const MAX_TOTAL_CHARS: usize = 16_000;

#[derive(Debug, Clone, Serialize)]
pub struct ImageTextReading {
    pub path: String,
    pub name: String,
    pub text: String,
    pub ok: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImageTextReport {
    pub readings: Vec<ImageTextReading>,
    pub available: bool,
}

struct CacheEntry {
    mtime: Option<SystemTime>,
    len: u64,
    text: String,
    ok: bool,
}

static CACHE: OnceLock<Mutex<HashMap<PathBuf, CacheEntry>>> = OnceLock::new();

fn cache() -> &'static Mutex<HashMap<PathBuf, CacheEntry>> {
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn is_image_path(path: &Path) -> bool {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    matches!(
        ext.as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "heic" | "heif" | "bmp" | "tif" | "tiff"
    )
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("image")
        .to_string()
}

fn clip_text(mut text: String) -> String {
    if text.chars().count() > MAX_TEXT_CHARS {
        text = text.chars().take(MAX_TEXT_CHARS).collect();
        text.push('…');
    }
    text
}

fn read_one(path: &Path) -> ImageTextReading {
    let name = file_name(path);
    let display = path.display().to_string();
    if !path.is_file() || !is_image_path(path) {
        return ImageTextReading {
            path: display,
            name,
            text: String::new(),
            ok: false,
        };
    }
    let meta = match fs::metadata(path) {
        Ok(meta) => meta,
        Err(_) => {
            return ImageTextReading {
                path: display,
                name,
                text: String::new(),
                ok: false,
            };
        }
    };
    if meta.len() == 0 || meta.len() > MAX_IMAGE_BYTES {
        return ImageTextReading {
            path: display,
            name,
            text: String::new(),
            ok: false,
        };
    }
    let mtime = meta.modified().ok();
    if let Ok(guard) = cache().lock() {
        if let Some(hit) = guard.get(path) {
            if hit.mtime == mtime && hit.len == meta.len() {
                return ImageTextReading {
                    path: display,
                    name,
                    text: hit.text.clone(),
                    ok: hit.ok,
                };
            }
        }
    }

    let (ok, text) = platform_read_image_text(path);
    let text = clip_text(text);
    if let Ok(mut guard) = cache().lock() {
        guard.insert(
            path.to_path_buf(),
            CacheEntry {
                mtime,
                len: meta.len(),
                text: text.clone(),
                ok,
            },
        );
    }
    ImageTextReading {
        path: display,
        name,
        text,
        ok,
    }
}

#[cfg(target_os = "macos")]
fn platform_read_image_text(path: &Path) -> (bool, String) {
    macos::recognize_text(path)
}

#[cfg(windows)]
fn platform_read_image_text(path: &Path) -> (bool, String) {
    windows::recognize_text(path)
}

#[cfg(not(any(target_os = "macos", windows)))]
fn platform_read_image_text(path: &Path) -> (bool, String) {
    unsupported::recognize_text(path)
}

pub fn platform_available() -> bool {
    cfg!(any(target_os = "macos", windows))
}

pub fn read_image_texts(paths: Vec<String>) -> ImageTextReport {
    let mut readings = Vec::new();
    let mut used = 0usize;
    for raw in paths {
        let path = PathBuf::from(raw.trim());
        if path.as_os_str().is_empty() {
            continue;
        }
        let mut reading = read_one(&path);
        if used >= MAX_TOTAL_CHARS {
            reading.text.clear();
            reading.ok = false;
        } else if reading.text.chars().count() + used > MAX_TOTAL_CHARS {
            let keep = MAX_TOTAL_CHARS.saturating_sub(used);
            reading.text = reading.text.chars().take(keep).collect();
            used = MAX_TOTAL_CHARS;
        } else {
            used += reading.text.chars().count();
        }
        readings.push(reading);
    }
    ImageTextReport {
        readings,
        available: platform_available(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_common_image_extensions() {
        assert!(is_image_path(Path::new("a.PNG")));
        assert!(is_image_path(Path::new("/tmp/shot.jpeg")));
        assert!(!is_image_path(Path::new("notes.txt")));
    }

    #[test]
    fn skips_missing_files() {
        let report = read_image_texts(vec!["/no/such/image.png".into()]);
        assert_eq!(report.readings.len(), 1);
        assert!(!report.readings[0].ok);
        assert!(report.readings[0].text.is_empty());
        assert_eq!(report.available, platform_available());
    }

    #[test]
    fn empty_paths_keep_platform_flag() {
        let report = read_image_texts(vec![]);
        assert!(report.readings.is_empty());
        assert_eq!(report.available, cfg!(any(target_os = "macos", windows)));
    }
}
