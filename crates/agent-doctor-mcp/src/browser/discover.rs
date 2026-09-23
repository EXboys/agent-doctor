use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};

use super::paths::{resolve_profile_directory, resolve_user_data_dir};
use super::types::{BrowserDiscovery, BrowserFamily};

/// Discover Chrome / Edge / Chromium on the local machine.
pub fn discover_chrome() -> Result<BrowserDiscovery> {
    discover_browser(BrowserFamily::Auto)
}

/// Discover a Chromium-family browser, optionally pinning Chrome vs Edge.
pub fn discover_browser(family: BrowserFamily) -> Result<BrowserDiscovery> {
    if let Ok(custom) = std::env::var("AGENT_DOCTOR_BROWSER_BINARY") {
        let path = PathBuf::from(custom.trim());
        if path.as_os_str().is_empty() {
            // fall through
        } else if path.exists() {
            return discovery_from_binary(path);
        } else {
            anyhow::bail!(
                "AGENT_DOCTOR_BROWSER_BINARY points to missing binary: {}",
                path.display()
            );
        }
    }

    let binary_path = find_chrome_binary(family).with_context(|| {
        format!(
            "{} not found on this system",
            match family {
                BrowserFamily::Edge => "Microsoft Edge",
                BrowserFamily::Chromium => "Chromium",
                BrowserFamily::Chrome => "Google Chrome",
                BrowserFamily::Auto => "Chrome/Edge/Chromium",
            }
        )
    })?;
    discovery_from_binary(binary_path)
}

fn discovery_from_binary(binary_path: PathBuf) -> Result<BrowserDiscovery> {
    let user_data_dir = resolve_user_data_dir(None, Some(&binary_path));
    let profile_directory = resolve_profile_directory(None);
    let version = detect_chrome_version(&binary_path);

    Ok(BrowserDiscovery {
        binary_path,
        user_data_dir,
        profile_directory,
        version,
    })
}

pub(crate) fn find_chrome_binary(family: BrowserFamily) -> Result<PathBuf> {
    let mut candidates: Vec<PathBuf> = windows_browser_install_paths(family);
    candidates.extend(
        browser_binary_candidates(family)
            .into_iter()
            .map(PathBuf::from),
    );

    for candidate in &candidates {
        if candidate.is_absolute() && candidate.exists() {
            return Ok(candidate.clone());
        }
    }

    if let Ok(path) = std::env::var("PATH") {
        for candidate in &candidates {
            if candidate.is_absolute() {
                continue;
            }
            for dir in std::env::split_paths(&path) {
                let full = dir.join(candidate);
                if full.exists() {
                    return Ok(full);
                }
                #[cfg(windows)]
                {
                    let as_exe = dir.join(format!("{}.exe", candidate.display()));
                    if as_exe.exists() {
                        return Ok(as_exe);
                    }
                }
            }
        }
    }

    anyhow::bail!(
        "Could not find {} binary. Install Google Chrome, Microsoft Edge, or Chromium.",
        family.as_str()
    )
}

/// Official Windows install locations. GUI PATH almost never includes these.
pub(crate) fn windows_browser_install_paths(family: BrowserFamily) -> Vec<PathBuf> {
    #[cfg(not(windows))]
    {
        let _ = family;
        Vec::new()
    }
    #[cfg(windows)]
    {
        let mut roots: Vec<PathBuf> = Vec::new();
        for key in ["PROGRAMFILES", "PROGRAMFILES(X86)", "LOCALAPPDATA"] {
            if let Ok(value) = std::env::var(key) {
                let trimmed = value.trim();
                if !trimmed.is_empty() {
                    roots.push(PathBuf::from(trimmed));
                }
            }
        }
        let home = home_dir();
        if !home.as_os_str().is_empty() {
            roots.push(home.join("AppData").join("Local"));
        }
        browser_paths_for_windows_roots(family, &roots)
    }
}

#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn browser_paths_for_windows_roots(
    family: BrowserFamily,
    roots: &[PathBuf],
) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let mut push_unique = |path: PathBuf| {
        if !paths.contains(&path) {
            paths.push(path);
        }
    };

    for root in roots {
        if matches!(family, BrowserFamily::Chrome | BrowserFamily::Auto) {
            push_unique(root.join("Google/Chrome/Application/chrome.exe"));
        }
        if matches!(family, BrowserFamily::Edge | BrowserFamily::Auto) {
            push_unique(root.join("Microsoft/Edge/Application/msedge.exe"));
        }
        if matches!(family, BrowserFamily::Chromium | BrowserFamily::Auto) {
            push_unique(root.join("Chromium/Application/chrome.exe"));
            push_unique(root.join("BraveSoftware/Brave-Browser/Application/brave.exe"));
        }
    }
    paths
}

pub(crate) fn browser_binary_candidates(family: BrowserFamily) -> Vec<&'static str> {
    if cfg!(target_os = "macos") {
        match family {
            BrowserFamily::Chrome => vec![
                "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
                "/Applications/Chrome.app/Contents/MacOS/Chrome",
            ],
            BrowserFamily::Edge => {
                vec!["/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge"]
            }
            BrowserFamily::Chromium => vec![
                "/Applications/Chromium.app/Contents/MacOS/Chromium",
                "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
            ],
            BrowserFamily::Auto => vec![
                "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
                "/Applications/Chrome.app/Contents/MacOS/Chrome",
                "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
                "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
                "/Applications/Chromium.app/Contents/MacOS/Chromium",
            ],
        }
    } else if cfg!(target_os = "linux") {
        match family {
            BrowserFamily::Chrome => vec!["google-chrome", "google-chrome-stable", "chrome"],
            BrowserFamily::Edge => vec!["microsoft-edge", "microsoft-edge-stable", "msedge"],
            BrowserFamily::Chromium => vec!["chromium", "chromium-browser"],
            BrowserFamily::Auto => vec![
                "google-chrome",
                "google-chrome-stable",
                "microsoft-edge",
                "microsoft-edge-stable",
                "msedge",
                "chromium",
                "chromium-browser",
                "chrome",
            ],
        }
    } else {
        match family {
            BrowserFamily::Chrome => vec!["chrome", "google-chrome"],
            BrowserFamily::Edge => vec!["msedge", "microsoft-edge"],
            BrowserFamily::Chromium => vec!["chromium"],
            BrowserFamily::Auto => vec!["chrome", "msedge", "chromium"],
        }
    }
}

fn detect_chrome_version(binary: &Path) -> Option<String> {
    // Never spawn Google Chrome / Chromium just to read --version.
    // Executing the app binary on macOS often handoffs to a running instance
    // and focuses a browser window (Resources tab / status checks).
    chrome_version_from_app_bundle(binary).or_else(|| chrome_version_via_plutil(binary))
}

fn chrome_version_via_plutil(binary: &Path) -> Option<String> {
    let mut dir = binary.parent()?.to_path_buf();
    for _ in 0..4 {
        if dir.extension().and_then(|e| e.to_str()) == Some("app") {
            let plist = dir.join("Contents/Info.plist");
            if !plist.exists() {
                return None;
            }
            let output = Command::new("plutil")
                .args(["-extract", "CFBundleShortVersionString", "raw", "-o", "-"])
                .arg(&plist)
                .output()
                .ok()?;
            if !output.status.success() {
                return None;
            }
            let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if version.is_empty() {
                return None;
            }
            return Some(format!("Google Chrome {version}"));
        }
        dir = dir.parent()?.to_path_buf();
    }
    None
}

fn chrome_version_from_app_bundle(binary: &Path) -> Option<String> {
    let mut dir = binary.parent()?.to_path_buf();
    // .../Foo.app/Contents/MacOS/Google Chrome → climb to Foo.app
    for _ in 0..4 {
        if dir.extension().and_then(|e| e.to_str()) == Some("app") {
            let plist = dir.join("Contents/Info.plist");
            return read_bundle_short_version(&plist);
        }
        dir = dir.parent()?.to_path_buf();
    }
    None
}

fn read_bundle_short_version(plist_path: &std::path::Path) -> Option<String> {
    let raw = std::fs::read_to_string(plist_path).ok()?;
    // Keep this dependency-free: Info.plist for Chrome is XML with the key nearby.
    let key = "<key>CFBundleShortVersionString</key>";
    let idx = raw.find(key)?;
    let after = &raw[idx + key.len()..];
    let start = after.find("<string>")? + "<string>".len();
    let end = after[start..].find("</string>")?;
    let version = after[start..start + end].trim();
    if version.is_empty() {
        None
    } else {
        Some(format!("Google Chrome {version}"))
    }
}

/// Default profile used when launching Chrome for MCP (everyday Chrome).
pub(crate) fn default_user_data_dir() -> PathBuf {
    super::paths::resolve_user_data_dir(None, find_chrome_binary(BrowserFamily::Auto).ok().as_ref())
}
